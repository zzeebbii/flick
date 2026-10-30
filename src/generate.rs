//! Generators: run a Spec's command, parse its output, cache the result (Q16).
//!
//! Generators never block typing. The first request starts the command in the background
//! and answers without its results; when it finishes, `notify` tells the shell to ask again.
//! Results stay fresh for the Generator's `cache.ttl`; after that the old result is still
//! shown while a new one is fetched.
use crate::spec::{Generator, JsonParse, Parse};
use std::collections::HashMap;
use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const DEFAULT_TTL: Duration = Duration::from_secs(5);
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);
/// Unused results are dropped after this long.
const FORGET_AFTER: Duration = Duration::from_secs(10 * 60);
const MAX_OUTPUT: u64 = 1 << 20;

/// (name, description)
pub type Items = Arc<Vec<(String, String)>>;
pub type Notify = Arc<dyn Fn() + Send + Sync>;

struct Entry {
    fetched: Option<Instant>,
    used: Instant,
    items: Option<Items>,
    running: bool,
}

#[derive(Default)]
pub struct Generators {
    cache: Mutex<HashMap<(Vec<String>, String), Entry>>,
}

impl Generators {
    /// The latest result for `g` run in `dir`, starting a run if there is none or it is stale.
    /// The flag is true while a run is in flight (its finish will `notify`).
    pub fn get(
        self: &Arc<Self>,
        g: &Generator,
        dir: &str,
        notify: &Notify,
    ) -> (Option<Items>, bool) {
        let Some(script) = g.script.as_ref().map(|s| s.argv()) else {
            return (None, false);
        };
        if g.needs_code || script.is_empty() {
            return (None, false);
        }
        let ttl = g
            .cache
            .as_ref()
            .and_then(|c| c.ttl)
            .map_or(DEFAULT_TTL, Duration::from_millis);
        let now = Instant::now();
        let key = (script, dir.to_string());
        let mut cache = self.cache.lock().unwrap();
        cache.retain(|_, e| e.running || now.duration_since(e.used) < FORGET_AFTER);
        let e = cache.entry(key.clone()).or_insert(Entry {
            fetched: None,
            used: now,
            items: None,
            running: false,
        });
        e.used = now;
        let fresh = e.fetched.is_some_and(|t| now.duration_since(t) < ttl);
        if !fresh && !e.running {
            e.running = true;
            let (this, notify) = (Arc::clone(self), Arc::clone(notify));
            let timeout = g
                .script_timeout
                .map_or(DEFAULT_TIMEOUT, Duration::from_millis);
            let parse = ParseRules::new(g);
            thread::spawn(move || {
                let out = run(&key.0, &key.1, timeout);
                let items = Arc::new(parse.apply(&out));
                let changed = {
                    let mut cache = this.cache.lock().unwrap();
                    let e = cache.get_mut(&key);
                    let changed = e.as_ref().is_none_or(|e| e.items.as_ref() != Some(&items));
                    if let Some(e) = e {
                        e.items = Some(items);
                        e.fetched = Some(Instant::now());
                        e.running = false;
                    }
                    changed
                };
                if changed {
                    notify();
                }
            });
        }
        (e.items.clone(), e.running)
    }
}

/// Runs `argv` in `dir` and returns its stdout, or "" on failure or timeout.
fn run(argv: &[String], dir: &str, timeout: Duration) -> String {
    let mut cmd = Command::new(&argv[0]);
    cmd.args(&argv[1..])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    if !dir.is_empty() {
        cmd.current_dir(dir);
    }
    let Ok(mut child) = cmd.spawn() else {
        return String::new();
    };
    let mut stdout = child.stdout.take().unwrap();
    let reader = thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = (&mut stdout).take(MAX_OUTPUT).read_to_end(&mut buf);
        buf
    });
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if start.elapsed() < timeout => thread::sleep(Duration::from_millis(5)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return String::new();
            }
        }
    }
    String::from_utf8_lossy(&reader.join().unwrap_or_default()).into_owned()
}

/// A Generator's parse settings, compiled once per run.
struct ParseRules {
    split_on: Option<String>,
    pattern: Option<regex_lite::Regex>,
    exclude: Option<regex_lite::Regex>,
    description: String,
    json: Option<(Option<String>, String, Option<String>)>,
}

impl ParseRules {
    fn new(g: &Generator) -> Self {
        let p = g.parse.as_ref();
        let re = |f: fn(&Parse) -> &Option<Box<str>>| {
            p.and_then(|p| f(p).as_deref())
                .and_then(|r| regex_lite::Regex::new(r).ok())
        };
        ParseRules {
            split_on: g.split_on.as_deref().map(String::from),
            pattern: re(|p| &p.pattern),
            exclude: re(|p| &p.exclude),
            description: p
                .and_then(|p| p.description.as_deref())
                .unwrap_or_default()
                .to_string(),
            json: p.and_then(|p| p.json.as_ref()).map(|j: &JsonParse| {
                (
                    j.path.as_deref().map(String::from),
                    j.name.to_string(),
                    j.description.as_deref().map(String::from),
                )
            }),
        }
    }

    fn apply(&self, out: &str) -> Vec<(String, String)> {
        let mut items = match &self.json {
            Some((path, name, desc)) => json_items(out, path.as_deref(), name, desc.as_deref()),
            None => self.line_items(out),
        };
        let mut seen = std::collections::HashSet::new();
        items.retain(|(n, _)| !n.is_empty() && seen.insert(n.clone()));
        items
    }

    fn line_items(&self, out: &str) -> Vec<(String, String)> {
        let pieces: Vec<&str> = match &self.split_on {
            Some(sep) if !sep.is_empty() => out.split(sep.as_str()).collect(),
            _ => out.lines().collect(),
        };
        pieces
            .into_iter()
            .filter(|l| !l.trim().is_empty())
            .filter(|l| !self.exclude.as_ref().is_some_and(|r| r.is_match(l)))
            .filter_map(|l| match &self.pattern {
                None => Some((l.trim().to_string(), self.description.clone())),
                Some(re) => {
                    let c = re.captures(l)?;
                    let name = c
                        .name("name")
                        .or_else(|| c.get(0))?
                        .as_str()
                        .trim()
                        .to_string();
                    let desc = c
                        .name("description")
                        .map_or(self.description.clone(), |d| d.as_str().trim().to_string());
                    Some((name, desc))
                }
            })
            .collect()
    }
}

fn json_items(
    out: &str,
    path: Option<&str>,
    name: &str,
    desc: Option<&str>,
) -> Vec<(String, String)> {
    let docs: Vec<serde_json::Value> = match serde_json::from_str(out) {
        Ok(v) => vec![v],
        Err(_) => out
            .lines()
            .filter_map(|l| serde_json::from_str(l).ok())
            .collect(), // JSON lines
    };
    let text = |v: &serde_json::Value| match v {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Null => String::new(),
        other => other.to_string(),
    };
    let mut items = vec![];
    for doc in docs {
        let mut v = &doc;
        for key in path.into_iter().flat_map(|p| p.split('.')) {
            v = &v[key];
        }
        match v {
            serde_json::Value::Object(map) if name == "$key" => {
                for (k, val) in map {
                    items.push((
                        k.clone(),
                        if desc == Some("$value") {
                            text(val)
                        } else {
                            String::new()
                        },
                    ));
                }
            }
            serde_json::Value::Array(list) => {
                for o in list {
                    items.push((text(&o[name]), desc.map_or(String::new(), |d| text(&o[d]))));
                }
            }
            o @ serde_json::Value::Object(_) => {
                items.push((text(&o[name]), desc.map_or(String::new(), |d| text(&o[d]))))
            }
            _ => {}
        }
    }
    items
}

/// Files and folders for the word being typed, relative to `dir` (Fig's `filepaths` and
/// `folders` templates). Returns (insert, description); folders end in `/`.
pub fn files(dir: &str, current: &str, folders_only: bool) -> Vec<(String, String)> {
    let (base, partial) = current
        .rsplit_once('/')
        .map_or(("", current), |(b, p)| (b, p));
    let base = if current.contains('/') {
        format!("{base}/")
    } else {
        String::new()
    };
    let home = std::env::var("HOME").unwrap_or_default();
    let resolved = match base.strip_prefix("~/") {
        Some(rest) => format!("{home}/{rest}"),
        None if base.starts_with('/') => base.clone(),
        None => format!("{dir}/{base}"),
    };
    let Ok(entries) = std::fs::read_dir(if resolved.is_empty() { "." } else { &resolved }) else {
        return vec![];
    };
    let mut out: Vec<(String, String)> = entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_str()?.to_string();
            if !name.starts_with(partial) || (name.starts_with('.') && !partial.starts_with('.')) {
                return None;
            }
            let is_dir = e.path().is_dir(); // follows symlinks
            if folders_only && !is_dir {
                return None;
            }
            let insert = format!(
                "{base}{}{}",
                escape_word(&name),
                if is_dir { "/" } else { "" }
            );
            Some((insert, if is_dir { "Folder" } else { "File" }.to_string()))
        })
        .collect();
    out.sort();
    out
}

/// Backslash-escapes characters the shell would otherwise interpret.
fn escape_word(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if " \t'\"\\$`!&|;<>()[]{}*?#~".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules(json: &str) -> ParseRules {
        ParseRules::new(&serde_json::from_str::<Generator>(json).unwrap())
    }

    #[test]
    fn branches_pattern() {
        let r = rules(
            r#"{"parse": {"pattern": "^[*+ ] (?:remotes/[^/ ]+/)?(?P<name>\\S+)", "exclude": "HEAD", "description": "Branch"}}"#,
        );
        let out = "* main\n  feature/x\n+ wt-branch\n  remotes/origin/HEAD -> origin/main\n  remotes/origin/main\n";
        let got = r.apply(out);
        let names: Vec<&str> = got.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, vec!["main", "feature/x", "wt-branch"]);
        assert_eq!(got[0].1, "Branch");
    }

    #[test]
    fn json_lines_and_object_keys() {
        let r = rules(r#"{"parse": {"json": {"name": "Names", "description": "Image"}}}"#);
        let got = r.apply(
            "{\"Names\":\"web\",\"Image\":\"nginx\"}\n{\"Names\":\"db\",\"Image\":\"postgres\"}\n",
        );
        assert_eq!(
            got,
            vec![
                ("web".into(), "nginx".into()),
                ("db".into(), "postgres".into())
            ]
        );
        let r = rules(
            r#"{"parse": {"json": {"path": "scripts", "name": "$key", "description": "$value"}}}"#,
        );
        let got = r.apply(r#"{"name": "x", "scripts": {"build": "tsc", "test": "jest"}}"#);
        assert_eq!(
            got,
            vec![
                ("build".into(), "tsc".into()),
                ("test".into(), "jest".into())
            ]
        );
    }

    #[test]
    fn split_on_without_parse() {
        assert_eq!(
            rules(r#"{"splitOn": ","}"#).apply("a, b,a"),
            vec![("a".into(), String::new()), ("b".into(), String::new())]
        );
    }

    #[test]
    fn runs_with_timeout() {
        assert_eq!(
            run(&["echo".into(), "hi".into()], "", DEFAULT_TIMEOUT),
            "hi\n"
        );
        let t = Instant::now();
        assert_eq!(
            run(
                &["sleep".into(), "5".into()],
                "",
                Duration::from_millis(100)
            ),
            ""
        );
        assert!(t.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn async_fetch_then_cached() {
        let gens = Arc::new(Generators::default());
        let g: Generator = serde_json::from_str(r#"{"script": ["printf", "a\\nb\\n"]}"#).unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let tx = Mutex::new(tx);
        let notify: Notify = Arc::new(move || tx.lock().unwrap().send(()).unwrap());
        assert_eq!(gens.get(&g, "", &notify), (None, true)); // first ask starts the run
        rx.recv_timeout(Duration::from_secs(2))
            .expect("notified when done");
        let (items, running) = gens.get(&g, "", &notify);
        assert_eq!((items.unwrap().len(), running), (2, false)); // now cached
    }

    #[test]
    fn lists_files_and_folders() {
        let d = std::env::temp_dir().join(format!("flick-files-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("src")).unwrap();
        std::fs::write(d.join("src/main.rs"), "").unwrap();
        std::fs::write(d.join("my file.txt"), "").unwrap();
        std::fs::write(d.join(".hidden"), "").unwrap();
        let dir = d.to_str().unwrap();
        let names = |cur: &str, f: bool| {
            files(dir, cur, f)
                .into_iter()
                .map(|(n, _)| n)
                .collect::<Vec<_>>()
        };
        assert_eq!(names("", false), vec!["my\\ file.txt", "src/"]);
        assert_eq!(names("", true), vec!["src/"]);
        assert_eq!(names("src/m", false), vec!["src/main.rs"]);
        assert_eq!(names(".h", false), vec![".hidden"]);
        std::fs::remove_dir_all(&d).unwrap();
    }
}
