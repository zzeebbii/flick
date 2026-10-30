//! Spec types (Fig's structure, plain data only) and the lazy Spec store (ADR 0006).
use serde::{Deserialize, Deserializer};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Specs not used for this long are dropped from memory.
const EVICT_AFTER: Duration = Duration::from_secs(30 * 60);
/// How often the list of commands on PATH is re-read.
const PATH_REFRESH: Duration = Duration::from_secs(5 * 60);

fn one_or_many<'de, D: Deserializer<'de>, T: Deserialize<'de>>(d: D) -> Result<Vec<T>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum OneOrMany<T> {
        One(T),
        Many(Vec<T>),
    }
    Ok(match Option::<OneOrMany<T>>::deserialize(d)? {
        None => vec![],
        Some(OneOrMany::One(x)) => vec![x],
        Some(OneOrMany::Many(v)) => v,
    })
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Subcommand {
    #[serde(deserialize_with = "one_or_many")]
    pub name: Vec<Box<str>>,
    pub description: Option<Box<str>>,
    pub subcommands: Vec<Subcommand>,
    pub options: Vec<Opt>,
    #[serde(deserialize_with = "one_or_many")]
    pub args: Vec<Arg>,
    pub load_spec: Option<LoadSpec>,
    pub hidden: bool,
    pub priority: Option<f32>,
    pub insert_value: Option<Box<str>>,
}

#[derive(Deserialize)]
#[serde(untagged)]
pub enum LoadSpec {
    Path(Box<str>),
    Inline(Box<Subcommand>),
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Opt {
    #[serde(deserialize_with = "one_or_many")]
    pub name: Vec<Box<str>>,
    pub description: Option<Box<str>>,
    #[serde(deserialize_with = "one_or_many")]
    pub args: Vec<Arg>,
    pub is_persistent: bool,
    pub is_repeatable: serde_json::Value,
    pub hidden: bool,
    pub priority: Option<f32>,
    pub insert_value: Option<Box<str>>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Arg {
    pub name: Option<Box<str>>,
    pub description: Option<Box<str>>,
    pub suggestions: Vec<SuggestionSpec>,
    pub is_optional: bool,
    pub is_variadic: bool,
    pub is_command: bool,
    pub load_spec: Option<LoadSpec>,
    #[serde(deserialize_with = "one_or_many")]
    pub template: Vec<Box<str>>,
    pub generators: Vec<Generator>,
}

#[derive(Deserialize)]
#[serde(untagged)]
pub enum SuggestionSpec {
    Name(Box<str>),
    Full(Box<FullSuggestion>),
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct FullSuggestion {
    #[serde(deserialize_with = "one_or_many")]
    pub name: Vec<Box<str>>,
    pub description: Option<Box<str>>,
    pub insert_value: Option<Box<str>>,
    pub hidden: bool,
    pub priority: Option<f32>,
}

/// Runs a command and turns its output into Suggestions (see `generate.rs`).
#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Generator {
    pub script: Option<Script>,
    pub split_on: Option<Box<str>>,
    #[serde(deserialize_with = "one_or_many")]
    pub template: Vec<Box<str>>,
    pub parse: Option<Parse>,
    pub cache: Option<CacheSpec>,
    /// Milliseconds before the command is killed.
    pub script_timeout: Option<u64>,
    /// Fig ran JavaScript here; it falls back to Native completion until ported to `parse`.
    pub needs_code: bool,
}

/// Declarative replacement for Fig's `postProcess` (ADR 0003).
#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Parse {
    /// Regex applied to each line; non-matching lines are dropped. Named groups `name`
    /// (default: the whole match) and `description`.
    pub pattern: Option<Box<str>>,
    /// Regex; matching lines are dropped.
    pub exclude: Option<Box<str>>,
    /// Description when the pattern has no `description` group.
    pub description: Option<Box<str>>,
    /// Parse the output as JSON instead of lines.
    pub json: Option<JsonParse>,
}

/// JSON output: one document, or one per line (`docker ... --format '{{json .}}'`).
#[derive(Deserialize, Default)]
#[serde(default)]
pub struct JsonParse {
    /// Dot path to the list or object to read, e.g. `scripts`.
    pub path: Option<Box<str>>,
    /// Field for the name; `$key` for an object's keys.
    pub name: Box<str>,
    /// Field for the description; `$value` for an object's values.
    pub description: Option<Box<str>>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct CacheSpec {
    /// Milliseconds a result stays fresh.
    pub ttl: Option<u64>,
}

#[derive(Deserialize, Clone)]
#[serde(untagged)]
pub enum Script {
    Shell(Box<str>),
    Argv(Vec<Box<str>>),
    Command {
        command: Box<str>,
        #[serde(default)]
        args: Vec<Box<str>>,
    },
}

impl Script {
    pub fn argv(&self) -> Vec<String> {
        match self {
            Script::Shell(s) => vec!["sh".into(), "-c".into(), s.to_string()],
            Script::Argv(v) => v.iter().map(|a| a.to_string()).collect(),
            Script::Command { command, args } => std::iter::once(command)
                .chain(args)
                .map(|a| a.to_string())
                .collect(),
        }
    }
}

impl Opt {
    pub fn repeatable(&self) -> bool {
        !matches!(
            self.is_repeatable,
            serde_json::Value::Null | serde_json::Value::Bool(false)
        )
    }
}

/// Loads Specs from disk on first use and keeps them until they go unused.
pub struct Store {
    dir: PathBuf,
    /// User specs (YAML or JSON); they replace a shipped Spec of the same name.
    user_dir: Option<PathBuf>,
    /// CLI names that have a top-level Spec, sorted.
    pub names: Vec<String>,
    loaded: Mutex<HashMap<String, (Arc<Subcommand>, Instant)>>,
    commands: Mutex<Option<(Instant, Arc<Vec<String>>)>>,
}

impl Store {
    pub fn new(dir: PathBuf, user_dir: Option<PathBuf>) -> Self {
        let mut names: Vec<String> = [Some(&dir), user_dir.as_ref()]
            .into_iter()
            .flatten()
            .flat_map(|d| std::fs::read_dir(d).into_iter().flatten().flatten())
            .filter_map(|e| {
                let f = e.file_name();
                let f = f.to_str()?;
                [".json", ".yaml", ".yml"]
                    .iter()
                    .find_map(|x| f.strip_suffix(x))
                    .map(String::from)
            })
            .collect();
        names.sort();
        names.dedup();
        Store {
            dir,
            user_dir,
            names,
            loaded: Mutex::default(),
            commands: Mutex::default(),
        }
    }

    /// `path` is a CLI name ("git") or a sub-spec path ("aws/s3").
    pub fn get(&self, path: &str) -> Option<Arc<Subcommand>> {
        if path.is_empty() || path.contains("..") || path.starts_with('/') {
            return None;
        }
        let now = Instant::now();
        let mut loaded = self.loaded.lock().unwrap();
        loaded.retain(|_, (_, used)| now.duration_since(*used) < EVICT_AFTER);
        if let Some((spec, used)) = loaded.get_mut(path) {
            *used = now;
            return Some(Arc::clone(spec));
        }
        let spec: Arc<Subcommand> = Arc::new(self.read(path)?);
        loaded.insert(path.to_string(), (Arc::clone(&spec), now));
        Some(spec)
    }

    fn read(&self, path: &str) -> Option<Subcommand> {
        if let Some(user) = &self.user_dir {
            for ext in ["yaml", "yml"] {
                if let Ok(text) = std::fs::read_to_string(user.join(format!("{path}.{ext}"))) {
                    return serde_norway::from_str(&text).ok();
                }
            }
            if let Ok(bytes) = std::fs::read(user.join(format!("{path}.json"))) {
                return serde_json::from_slice(&bytes).ok();
            }
        }
        serde_json::from_slice(&std::fs::read(self.dir.join(format!("{path}.json"))).ok()?).ok()
    }

    /// Every command name: CLIs with a Spec plus executables on `PATH`, sorted. Re-read in the
    /// background when stale, so no keystroke pays for scanning PATH after the first.
    pub fn commands(self: &Arc<Self>) -> Arc<Vec<String>> {
        let mut cached = self.commands.lock().unwrap();
        match &mut *cached {
            Some((at, list)) => {
                if at.elapsed() >= PATH_REFRESH {
                    *at = Instant::now(); // one refresh at a time
                    let this = Arc::clone(self);
                    std::thread::spawn(move || {
                        let fresh = Arc::new(this.scan_commands());
                        *this.commands.lock().unwrap() = Some((Instant::now(), fresh));
                    });
                }
                Arc::clone(list)
            }
            None => {
                let list = Arc::new(self.scan_commands());
                *cached = Some((Instant::now(), Arc::clone(&list)));
                list
            }
        }
    }

    fn scan_commands(&self) -> Vec<String> {
        use std::os::unix::fs::PermissionsExt;
        let mut all = self.names.clone();
        for dir in std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()) {
            for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
                let executable = e
                    .metadata()
                    .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0);
                if let (true, Some(name)) = (executable, e.file_name().to_str()) {
                    all.push(name.to_string());
                }
            }
        }
        all.sort();
        all.dedup();
        all
    }

    #[cfg(test)]
    pub fn loaded_count(&self) -> usize {
        self.loaded.lock().unwrap().len()
    }
}
