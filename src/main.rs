//! flick: the background process that serves Suggestions to every zsh (ADR 0002).
//!
//! Usage:
//!   flick serve <socket>       run the background process
//!   flick complete <word>...   print Suggestions once, for debugging; the last word is the one being typed
//!
//! Protocol: the shell sends one line per request, the words of the current command joined
//! by tabs. flick answers with one `insert<TAB>description` line per Suggestion and then
//! an empty line.
mod complete;
mod spec;

use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use std::{env, fs, process, thread};

const MAX_SUGGESTIONS: usize = 50;
/// Exit after this long with no shells connected (Q18).
const IDLE_EXIT: Duration = Duration::from_secs(10 * 60);

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    let store = spec::Store::new(specs_dir());
    match args.first().map(String::as_str) {
        Some("serve") if args.len() == 2 => serve(Path::new(&args[1]), Arc::new(store)),
        Some("complete") if args.len() >= 2 => {
            for s in complete::complete(&store, &args[1..]) {
                println!("{}\t{}", s.insert, s.description);
            }
        }
        _ => {
            eprintln!("usage: flick serve <socket> | flick complete <word>...");
            process::exit(2);
        }
    }
}

/// `$FLICK_SPECS`, else `specs/` next to the repo the binary was built in, else the install location.
fn specs_dir() -> PathBuf {
    if let Some(d) = env::var_os("FLICK_SPECS") {
        return d.into();
    }
    let exe = env::current_exe().ok().and_then(|e| e.canonicalize().ok());
    // target/release/flick -> <repo>/specs
    if let Some(repo) = exe.as_deref().and_then(|e| e.ancestors().nth(3))
        && repo.join("specs").is_dir()
    {
        return repo.join("specs");
    }
    let home = env::var_os("HOME").unwrap_or_default();
    Path::new(&home).join(".local/share/flick/specs")
}

fn serve(socket: &Path, store: Arc<spec::Store>) {
    // Another flick already answers on this socket: let it serve.
    // ponytail: two shells starting flick in the same instant can still race; add a lock file if seen.
    if UnixStream::connect(socket).is_ok() {
        return;
    }
    let _ = fs::remove_file(socket);
    let listener = match UnixListener::bind(socket) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("flick: cannot listen on {}: {e}", socket.display());
            process::exit(1);
        }
    };
    let _ = fs::set_permissions(socket, fs::Permissions::from_mode(0o600));

    let clients = Arc::new(AtomicUsize::new(0));
    let last_seen = Arc::new(AtomicU64::new(now_secs()));
    {
        let (clients, last_seen, socket) = (
            Arc::clone(&clients),
            Arc::clone(&last_seen),
            socket.to_owned(),
        );
        thread::spawn(move || {
            loop {
                thread::sleep(Duration::from_secs(60));
                let idle = now_secs().saturating_sub(last_seen.load(Ordering::Relaxed));
                if clients.load(Ordering::Relaxed) == 0 && idle >= IDLE_EXIT.as_secs() {
                    let _ = fs::remove_file(&socket);
                    process::exit(0);
                }
            }
        });
    }

    for stream in listener.incoming().flatten() {
        let (store, clients, last_seen) = (
            Arc::clone(&store),
            Arc::clone(&clients),
            Arc::clone(&last_seen),
        );
        clients.fetch_add(1, Ordering::Relaxed);
        thread::spawn(move || {
            let _ = handle(stream, &store);
            clients.fetch_sub(1, Ordering::Relaxed);
            last_seen.store(now_secs(), Ordering::Relaxed);
        });
    }
}

fn handle(stream: UnixStream, store: &spec::Store) -> std::io::Result<()> {
    let mut out = stream.try_clone()?;
    let mut reader = BufReader::new(stream);
    let mut buf = Vec::new();
    loop {
        buf.clear();
        if reader.read_until(b'\n', &mut buf)? == 0 {
            return Ok(());
        }
        // Lossy: a stray non-UTF-8 byte must not drop the connection.
        let line = String::from_utf8_lossy(&buf);
        let words: Vec<String> = line
            .trim_end_matches('\n')
            .split('\t')
            .map(String::from)
            .collect();
        let mut resp = String::new();
        for s in complete::complete(store, &words)
            .into_iter()
            .take(MAX_SUGGESTIONS)
        {
            let clean = |t: &str| t.replace(['\t', '\n', '\r'], " ");
            resp.push_str(&format!(
                "{}\t{}\n",
                clean(&s.insert),
                clean(&s.description)
            ));
        }
        resp.push('\n');
        out.write_all(resp.as_bytes())?;
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}
