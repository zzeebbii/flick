//! flick: the background process that serves Suggestions to every zsh (ADR 0002).
//!
//! Usage:
//!   flick serve <socket>       run the background process
//!   flick complete <word>...   print Suggestions once, for debugging; the last word is the one being typed
//!   flick config zsh|check|example
//!                              shell settings / validate config.toml / print a commented example
//!   flick bench                measure speed and memory against the budgets
//!
//! Protocol: one request per line, fields separated by tabs. `dir`, `line` and `cmd` are escaped
//! (`\\`, `\t`, `\n`); words have no tabs or newlines.
//!   c <dir> <line> <word>...      complete; answered with `kind<TAB>insert<TAB>description` lines
//!                                 (kind `l` replaces the whole line, any other kind the current
//!                                 word; see `complete::Suggestion::kind`), then an empty line
//!   r <time> <exit> <dir> <cmd>   record a command that just ran; no answer
//!
//! An answer ends with a `~` line before the empty line when a Generator is still running.
//! When it finishes, flick sends a line `!` between answers: asking again gives new Suggestions.
mod bench;
mod complete;
mod config;
mod generate;
mod history;
mod spec;

use config::Config;
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
    store: Arc<spec::Store>,
    history: Mutex<History>,
    gens: Arc<generate::Generators>,
    /// config.toml and its modification time; re-read when the file changes.
    config: Mutex<(Option<SystemTime>, Config)>,
}

impl State {
    fn new() -> State {
        let path = config::config_path();
        let cfg = Config::load(&path).unwrap_or_default();
        State {
            store: Arc::new(spec::Store::new(
                specs_dir(),
                Some(config::config_dir().join("specs")),
            )),
            history: Mutex::new(History::open(
                &history_path(),
                Some(&zsh_history_path()),
                cfg.history.ignore.clone(),
            )),
            gens: Arc::default(),
            config: Mutex::new((mtime(&path), cfg)),
        }
    }

    /// The current Source switches and disabled CLIs, re-reading config.toml if it changed.
    fn settings(&self) -> (config::Sources, Vec<String>) {
        let path = config::config_path();
        let mut c = self.config.lock().unwrap();
        let now = mtime(&path);
        if now != c.0 {
            // A broken edit keeps the last good config; `flick config check` shows the error.
            if let Ok(cfg) = Config::load(&path) {
                self.history.lock().unwrap().ignore = cfg.history.ignore.clone();
                c.1 = cfg;
            }
            c.0 = now;
        }
        (c.1.sources(), c.1.generators.disable.clone())
    }
}

fn mtime(p: &Path) -> Option<SystemTime> {
    fs::metadata(p).and_then(|m| m.modified()).ok()
}

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    let arg = |i: usize| args.get(i).map(String::as_str);
    match (arg(0), arg(1)) {
        (Some("serve"), Some(socket)) => serve(Path::new(socket), Arc::new(State::new())),
        (Some("complete"), Some(_)) => {
            let state = State::new();
            let dir = env::current_dir().unwrap_or_default();
            let line = args[1..].join(" ");
            let r = complete::Request {
                dir: &dir.to_string_lossy(),
                line: &line,
                words: &args[1..],
                now: now_secs(),
            };
            let cx = complete::Ctx::new(&state.store, r.dir, complete::ALL);
            for s in complete::suggest(&cx, &state.history.lock().unwrap(), &r) {
                println!("{}\t{}\t{}", s.kind, s.insert, s.description);
            }
        }
        (Some("config"), Some("example")) => print!("{}", config::EXAMPLE),
        (Some("config"), Some(what @ ("zsh" | "check"))) => {
            let path = config::config_path();
            let loaded = Config::load(&path).and_then(|c| c.theme().map(|t| (c, t)));
            if what == "check" {
                match loaded {
                    Ok((c, _)) if !path.exists() => {
                        println!(
                            "{}: no file yet, using defaults (`flick config example` prints one)",
                            path.display()
                        );
                        println!("Sources: {:?}", c.sources());
                    }
                    Ok((c, _)) => println!("{}: ok\nSources: {:?}", path.display(), c.sources()),
                    Err(e) => {
                        eprintln!("{e}");
                        process::exit(1);
                    }
                }
                return;
            }
            match loaded {
                Ok((c, t)) => print!("{}", config::to_zsh(&c, &t, None)),
                Err(e) => print!(
                    "{}",
                    config::to_zsh(&Config::default(), &config::Theme::default(), Some(&e))
                ),
            }
        }
        (Some("bench"), None) => process::exit(bench::run()),
        _ => {
            eprintln!(
                "usage: flick serve <socket> | flick complete <word>... | flick config zsh|check|example | flick bench"
            );
            process::exit(2);
        }
    }
}

pub fn home() -> PathBuf {
    env::var_os("HOME").unwrap_or_default().into()
}

/// `$FLICK_HISTORY`, else `$XDG_DATA_HOME/flick/history.tsv`, else `~/.local/share/flick/history.tsv`.
pub fn history_path() -> PathBuf {
    if let Some(p) = env::var_os("FLICK_HISTORY") {
        return p.into();
    }
    let data =
        env::var_os("XDG_DATA_HOME").map_or_else(|| home().join(".local/share"), PathBuf::from);
    data.join("flick/history.tsv")
}

/// zsh's history file to import on first run: `$FLICK_HISTFILE` (the shell passes its `$HISTFILE`).
pub fn zsh_history_path() -> PathBuf {
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
    // Shared with Generator threads, which write `!` when they finish; whole lines only.
    let out = Arc::new(Mutex::new(stream.try_clone()?));
    let notify: generate::Notify = {
        let out = Arc::clone(&out);
        Arc::new(move || {
            let _ = out.lock().unwrap().write_all(b"!\n");
        })
    };
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
                state.settings(); // picks up a changed `[history] ignore`
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
                let (sources, disabled) = state.settings();
                let gens = sources.generators.then_some((&state.gens, &notify));
                let cx = complete::Ctx {
                    gens,
                    disabled: &disabled,
                    ..complete::Ctx::new(&state.store, &dir, sources)
                };
                let suggestions = complete::suggest(&cx, &state.history.lock().unwrap(), &r);
                let mut resp = String::new();
                for s in suggestions.into_iter().take(MAX_SUGGESTIONS) {
                    let clean = |t: &str| t.replace(['\t', '\n', '\r'], " ");
                    let kind = s.kind;
                    resp.push_str(&format!(
                        "{kind}\t{}\t{}\n",
                        clean(&s.insert),
                        clean(&s.description)
                    ));
                }
                if cx.pending.get() {
                    resp.push_str("~\n"); // a Generator will finish later and send `!`
                }
                resp.push('\n');
                out.lock().unwrap().write_all(resp.as_bytes())?;
            }
            // Unknown or malformed: still answer a `c`-shaped request so the shell never waits forever.
            [kind, ..] if *kind != "r" => out.lock().unwrap().write_all(b"\n")?,
            _ => {}
        }
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}
