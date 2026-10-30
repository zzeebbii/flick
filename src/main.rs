//! flick: the background process that serves Suggestions to every zsh (ADR 0002).
//!
//! Usage:
//!   flick serve <socket>       run the background process
//!   flick complete <word>...   print Suggestions once, for debugging; the last word is the one being typed
//!
//! Protocol: one request per line, fields separated by tabs. `dir`, `line` and `cmd` are escaped
//! (`\\`, `\t`, `\n`); words have no tabs or newlines.
//!   c <dir> <line> <word>...      complete; answered with `kind<TAB>insert<TAB>description` lines
//!                                 (kind `w` replaces the current word, `l` the whole line), then
//!                                 an empty line
//!   r <time> <exit> <dir> <cmd>   record a command that just ran; no answer
mod complete;
mod history;
mod spec;

use history::{History, unescape};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use std::{env, fs, process, thread};

const MAX_SUGGESTIONS: usize = 50;
/// Exit after this long with no shells connected (Q18).
const IDLE_EXIT: Duration = Duration::from_secs(10 * 60);

struct State {
    store: spec::Store,
    history: Mutex<History>,
}

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    let state = State {
        store: spec::Store::new(specs_dir()),
        history: Mutex::new(History::open(&history_path(), Some(&zsh_history_path()))),
    };
    match args.first().map(String::as_str) {
        Some("serve") if args.len() == 2 => serve(Path::new(&args[1]), Arc::new(state)),
        Some("complete") if args.len() >= 2 => {
            let dir = env::current_dir().unwrap_or_default();
            let line = args[1..].join(" ");
            let r = complete::Request {
                dir: &dir.to_string_lossy(),
                line: &line,
                words: &args[1..],
                now: now_secs(),
            };
            for s in complete::suggest(&state.store, &state.history.lock().unwrap(), &r) {
                println!(
                    "{}\t{}\t{}",
                    if s.whole_line { 'l' } else { 'w' },
                    s.insert,
                    s.description
                );
            }
        }
        _ => {
            eprintln!("usage: flick serve <socket> | flick complete <word>...");
            process::exit(2);
        }
    }
}

fn home() -> PathBuf {
    env::var_os("HOME").unwrap_or_default().into()
}

/// `$FLICK_HISTORY`, else `$XDG_DATA_HOME/flick/history.tsv`, else `~/.local/share/flick/history.tsv`.
fn history_path() -> PathBuf {
    if let Some(p) = env::var_os("FLICK_HISTORY") {
        return p.into();
    }
    let data =
        env::var_os("XDG_DATA_HOME").map_or_else(|| home().join(".local/share"), PathBuf::from);
    data.join("flick/history.tsv")
}

/// zsh's history file to import on first run: `$FLICK_HISTFILE` (the shell passes its `$HISTFILE`).
fn zsh_history_path() -> PathBuf {
    env::var_os("FLICK_HISTFILE").map_or_else(|| home().join(".zsh_history"), PathBuf::from)
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
    home().join(".local/share/flick/specs")
}

fn serve(socket: &Path, state: Arc<State>) {
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
        let (state, clients, last_seen) = (
            Arc::clone(&state),
            Arc::clone(&clients),
            Arc::clone(&last_seen),
        );
        clients.fetch_add(1, Ordering::Relaxed);
        thread::spawn(move || {
            let _ = handle(stream, &state);
            clients.fetch_sub(1, Ordering::Relaxed);
            last_seen.store(now_secs(), Ordering::Relaxed);
        });
    }
}

fn handle(stream: UnixStream, state: &State) -> std::io::Result<()> {
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
        let fields: Vec<&str> = line.trim_end_matches('\n').split('\t').collect();
        match fields.as_slice() {
            ["r", time, exit, dir, cmd] => {
                let mut h = state.history.lock().unwrap();
                h.record(
                    time.parse().unwrap_or_else(|_| now_secs()),
                    exit.parse().ok(),
                    &unescape(dir),
                    &unescape(cmd),
                );
            }
            ["c", dir, typed, words @ ..] if !words.is_empty() => {
                let (dir, typed) = (unescape(dir), unescape(typed));
                let words: Vec<String> = words.iter().map(|w| w.to_string()).collect();
                let r = complete::Request {
                    dir: &dir,
                    line: &typed,
                    words: &words,
                    now: now_secs(),
                };
                let suggestions =
                    complete::suggest(&state.store, &state.history.lock().unwrap(), &r);
                let mut resp = String::new();
                for s in suggestions.into_iter().take(MAX_SUGGESTIONS) {
                    let clean = |t: &str| t.replace(['\t', '\n', '\r'], " ");
                    let kind = if s.whole_line { 'l' } else { 'w' };
                    resp.push_str(&format!(
                        "{kind}\t{}\t{}\n",
                        clean(&s.insert),
                        clean(&s.description)
                    ));
                }
                resp.push('\n');
                out.write_all(resp.as_bytes())?;
            }
            // Unknown or malformed: still answer a `c`-shaped request so the shell never waits forever.
            [kind, ..] if *kind != "r" => out.write_all(b"\n")?,
            _ => {}
        }
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}
