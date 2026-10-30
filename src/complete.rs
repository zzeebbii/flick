//! Turns the words of the command being typed into Suggestions by walking its Spec.
use crate::spec::{Arg, LoadSpec, Opt, Store, Subcommand, SuggestionSpec};

#[derive(Debug, PartialEq)]
pub struct Suggestion {
    /// Text that replaces the current word.
    pub insert: String,
    pub description: String,
    priority: f32,
}

/// `words` are the words of the current command, already alias-expanded by the shell.
/// The last word is the one being typed; it is empty right after a space.
pub fn complete(store: &Store, words: &[String]) -> Vec<Suggestion> {
    // `FOO=1 BAR=2 cmd ...`: skip leading environment assignments.
    let words = &words[words.iter().take_while(|w| is_assignment(w)).count()..];
    let Some((current, done)) = words.split_last() else {
        return vec![];
    };
    let mut out = match done.split_first() {
        None => commands(store, current),
        Some((cmd, rest)) => match store.get(cmd) {
            Some(spec) => walk(store, &spec, rest, current, vec![]),
            None => vec![],
        },
    };
    rank(&mut out, current);
    out
}

fn is_assignment(w: &str) -> bool {
    w.split_once('=').is_some_and(|(name, _)| {
        !name.is_empty()
            && !name.starts_with(|c: char| c.is_ascii_digit())
            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    })
}

fn commands(store: &Store, current: &str) -> Vec<Suggestion> {
    store
        .names
        .iter()
        .filter(|n| n.starts_with(current))
        .map(|n| suggestion(n, None, None))
        .collect()
}

/// Walks the finished words `done` down from `cmd`, then suggests for `current`.
fn walk<'a>(
    store: &Store,
    cmd: &'a Subcommand,
    done: &[String],
    current: &str,
    mut persistent: Vec<&'a Opt>,
) -> Vec<Suggestion> {
    let mut used: Vec<&Opt> = vec![];
    let mut arg_index = 0;
    let mut pending: Option<&Arg> = None; // an option is waiting for its value
    let mut only_args = false; // after `--`

    for (i, word) in done.iter().enumerate() {
        if pending.take().is_some() {
            continue;
        }
        if !only_args && word == "--" {
            only_args = true;
            continue;
        }
        if !only_args && word.len() > 1 && word.starts_with('-') {
            let (name, inline_value) = match word.split_once('=') {
                Some((n, _)) => (n, true),
                None => (word.as_str(), false),
            };
            if let Some(opt) = find_option(cmd, &persistent, name) {
                used.push(opt);
                if !inline_value {
                    pending = opt.args.first().filter(|a| !a.is_optional);
                }
            }
            continue;
        }
        if arg_index == 0
            && !only_args
            && let Some(sub) = cmd.subcommands.iter().find(|s| has_name(&s.name, word))
        {
            persistent.extend(cmd.options.iter().filter(|o| o.is_persistent));
            return descend(store, sub, &done[i + 1..], current, persistent);
        }
        if let Some(arg) = cmd.args.get(arg_index) {
            if arg.is_command {
                let mut words = done[i..].to_vec();
                words.push(current.to_string());
                return complete(store, &words);
            }
            if let Some(LoadSpec::Path(p)) = &arg.load_spec {
                return match store.get(p) {
                    Some(spec) => walk(store, &spec, &done[i + 1..], current, vec![]),
                    None => vec![],
                };
            }
            if !arg.is_variadic {
                arg_index += 1;
            }
        }
    }

    if let Some(arg) = pending {
        return arg_suggestions(arg);
    }
    if current.starts_with('-') && !only_args {
        return cmd
            .options
            .iter()
            .chain(persistent.iter().copied())
            .filter(|o| !o.hidden && (o.repeatable() || !used.iter().any(|u| std::ptr::eq(*u, *o))))
            .flat_map(|o| {
                o.name.iter().map(move |n| {
                    suggestion(n, o.description.as_deref(), o.priority)
                        .with_insert(o.insert_value.as_deref())
                })
            })
            .collect();
    }
    let mut out = vec![];
    if arg_index == 0 && !only_args {
        for s in cmd.subcommands.iter().filter(|s| !s.hidden) {
            for n in &s.name {
                out.push(
                    suggestion(n, s.description.as_deref(), s.priority)
                        .with_insert(s.insert_value.as_deref()),
                );
            }
        }
    }
    if let Some(arg) = cmd.args.get(arg_index) {
        if arg.is_command {
            out.extend(commands(store, current));
        } else {
            out.extend(arg_suggestions(arg));
        }
    }
    out
}

/// Enters a subcommand, following its `loadSpec` if it lives in another file.
fn descend<'a>(
    store: &Store,
    sub: &'a Subcommand,
    done: &[String],
    current: &str,
    persistent: Vec<&'a Opt>,
) -> Vec<Suggestion> {
    match &sub.load_spec {
        Some(LoadSpec::Path(p)) => match store.get(p) {
            // Persistent options don't cross into a separately loaded file.
            Some(spec) => walk(store, &spec, done, current, vec![]),
            None => walk(store, sub, done, current, persistent),
        },
        Some(LoadSpec::Inline(spec)) => walk(store, spec, done, current, persistent),
        None => walk(store, sub, done, current, persistent),
    }
}

fn find_option<'a>(cmd: &'a Subcommand, persistent: &[&'a Opt], name: &str) -> Option<&'a Opt> {
    cmd.options
        .iter()
        .chain(persistent.iter().copied())
        .find(|o| has_name(&o.name, name))
}

fn has_name(names: &[Box<str>], word: &str) -> bool {
    names.iter().any(|n| &**n == word)
}

fn arg_suggestions(arg: &Arg) -> Vec<Suggestion> {
    arg.suggestions
        .iter()
        .flat_map(|s| match s {
            SuggestionSpec::Name(n) => vec![suggestion(n, None, None)],
            SuggestionSpec::Full(f) if !f.hidden => f
                .name
                .iter()
                .map(|n| {
                    suggestion(n, f.description.as_deref(), f.priority)
                        .with_insert(f.insert_value.as_deref())
                })
                .collect(),
            SuggestionSpec::Full(_) => vec![],
        })
        .collect()
}

fn suggestion(name: &str, description: Option<&str>, priority: Option<f32>) -> Suggestion {
    Suggestion {
        insert: name.to_string(),
        description: description
            .unwrap_or_default()
            .lines()
            .next()
            .unwrap_or_default()
            .to_string(),
        priority: priority.unwrap_or(50.0),
    }
}

impl Suggestion {
    /// Fig's `insertValue` may contain `{cursor}` and newlines; v1 keeps only single-line values.
    fn with_insert(mut self, insert: Option<&str>) -> Self {
        if let Some(v) = insert.filter(|v| !v.contains('\n')) {
            self.insert = v.replace("{cursor}", "");
        }
        self
    }
}

/// Keeps Suggestions that start with `current` (falling back to a case-insensitive match),
/// ordered by Spec priority, then alphabetically. History-based ranking comes in step 3.
fn rank(out: &mut Vec<Suggestion>, current: &str) {
    if out.iter().any(|s| s.insert.starts_with(current)) {
        out.retain(|s| s.insert.starts_with(current));
    } else {
        let lower = current.to_lowercase();
        out.retain(|s| s.insert.to_lowercase().starts_with(&lower));
    }
    out.retain(|s| s.insert != current);
    out.sort_by(|a, b| {
        b.priority
            .total_cmp(&a.priority)
            .then_with(|| a.insert.cmp(&b.insert))
    });
    out.dedup_by(|a, b| a.insert == b.insert);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn store() -> Store {
        Store::new(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("specs"))
    }

    fn names(line: &str) -> Vec<String> {
        let mut words: Vec<String> = line.split(' ').map(String::from).collect();
        if words.is_empty() {
            words.push(String::new());
        }
        complete(&store(), &words)
            .into_iter()
            .map(|s| s.insert)
            .collect()
    }

    #[test]
    fn command_names() {
        assert!(names("gi").contains(&"git".to_string()));
    }

    #[test]
    fn subcommands_and_options() {
        assert!(names("git ch").contains(&"checkout".to_string()));
        assert!(names("git commit --am").contains(&"--amend".to_string()));
        assert!(names("git commit -").iter().all(|n| n.starts_with('-')));
    }

    #[test]
    fn used_option_not_suggested_again() {
        assert!(!names("git commit --amend --am").contains(&"--amend".to_string()));
    }

    #[test]
    fn option_value_is_not_a_subcommand() {
        // `-C <path>` takes a value, so `status` is still the subcommand after it.
        assert!(names("git -C /tmp sta").contains(&"status".to_string()));
    }

    #[test]
    fn env_assignment_and_command_prefix() {
        assert!(names("FOO=1 git ch").contains(&"checkout".to_string()));
        assert!(names("sudo git ch").contains(&"checkout".to_string()));
    }

    #[test]
    fn load_spec_across_files() {
        assert!(names("aws s3 c").contains(&"cp".to_string()));
    }

    #[test]
    fn specs_load_lazily() {
        let s = store();
        complete(&s, &["git".into(), "ch".into()]);
        assert_eq!(s.loaded_count(), 1);
    }

    #[test]
    fn every_spec_parses() {
        let s = store();
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("specs");
        let mut failed = vec![];
        let mut stack = vec![dir.clone()];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(d).unwrap().flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else if p.extension().is_some_and(|x| x == "json") {
                    let rel = p.strip_prefix(&dir).unwrap().with_extension("");
                    if s.get(rel.to_str().unwrap()).is_none() {
                        failed.push(rel);
                    }
                }
            }
        }
        assert!(failed.is_empty(), "unparsable specs: {failed:?}");
    }
}
