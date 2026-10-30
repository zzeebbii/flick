//! Spec types (Fig's structure, plain data only) and the lazy Spec store (ADR 0006).
use serde::{Deserialize, Deserializer};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Specs not used for this long are dropped from memory.
const EVICT_AFTER: Duration = Duration::from_secs(30 * 60);

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

/// Runs a command and parses its output (step 4). Parsed now so Specs load intact.
#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
#[allow(dead_code)]
pub struct Generator {
    pub script: Option<Script>,
    pub split_on: Option<Box<str>>,
    #[serde(deserialize_with = "one_or_many")]
    pub template: Vec<Box<str>>,
    pub needs_code: bool,
}

#[derive(Deserialize)]
#[serde(untagged)]
#[allow(dead_code)]
pub enum Script {
    Shell(Box<str>),
    Argv(Vec<Box<str>>),
    Command {
        command: Box<str>,
        #[serde(default)]
        args: Vec<Box<str>>,
    },
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
    /// CLI names that have a top-level Spec, sorted.
    pub names: Vec<String>,
    loaded: Mutex<HashMap<String, (Arc<Subcommand>, Instant)>>,
}

impl Store {
    pub fn new(dir: PathBuf) -> Self {
        let mut names: Vec<String> = std::fs::read_dir(&dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| {
                e.file_name()
                    .to_str()?
                    .strip_suffix(".json")
                    .map(String::from)
            })
            .collect();
        names.sort();
        Store {
            dir,
            names,
            loaded: Mutex::default(),
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
        let bytes = std::fs::read(self.dir.join(format!("{path}.json"))).ok()?;
        let spec: Arc<Subcommand> = Arc::new(serde_json::from_slice(&bytes).ok()?);
        loaded.insert(path.to_string(), (Arc::clone(&spec), now));
        Some(spec)
    }

    #[cfg(test)]
    pub fn loaded_count(&self) -> usize {
        self.loaded.lock().unwrap().len()
    }
}
