//! Turns the line being typed into Suggestions: Spec walking for the current word, History for
//! the whole line, one ranking for both.
use crate::config::Sources;
use crate::generate::{self, Generators, Notify};
use crate::history::History;
use crate::spec::{Arg, LoadSpec, Opt, Store, Subcommand, SuggestionSpec};
use std::cell::Cell;
use std::sync::Arc;

/// What completing needs besides the words: the Specs, the directory, and Generators
/// (absent in tests and `flick complete`, where nothing runs in the background).
pub struct Ctx<'a> {
    pub store: &'a Arc<Store>,
    pub dir: &'a str,
    pub gens: Option<(&'a Arc<Generators>, &'a Notify)>,
    /// Set when a Generator is still running for this request.
    pub pending: Cell<bool>,
    pub sources: Sources,
    /// CLIs whose Generators never run (`[generators] disable`).
    pub disabled: &'a [String],
}

impl Ctx<'_> {
    pub fn new<'a>(store: &'a Arc<Store>, dir: &'a str, sources: Sources) -> Ctx<'a> {
        Ctx {
            store,
            dir,
            gens: None,
            pending: Cell::default(),
            sources,
            disabled: &[],
        }
    }

    /// Same request, but no Generators run.
    fn without_generators(&self) -> Ctx<'_> {
        Ctx {
            gens: None,
            ..Ctx::new(self.store, self.dir, self.sources)
        }
    }
}

/// Everything on: what tests and `flick complete` use.
pub const ALL: Sources = Sources {
    history: true,
    specs: true,
    generators: true,
    files: true,
    native: true,
};

#[derive(Debug, PartialEq)]
pub struct Suggestion {
    /// Replaces the current word, or the whole line for a History line (kind `l`).
    pub insert: String,
    pub description: String,
    /// `l` History line, `c` command or subcommand, `o` option, `a` fixed argument value,
    /// `v` Generator value, `f` file, `d` folder. Themes colour by it.
    pub kind: char,
    priority: f32,
    usage: f32,
}

pub struct Request<'a> {
    pub dir: &'a str,
    /// Everything left of the cursor, as typed.
    pub line: &'a str,
    /// Words of the current command (see `complete`).
    pub words: &'a [String],
    pub now: u64,
}

/// History lines shown in the Menu, after the best one at the top.
const EXTRA_LINES: usize = 3;

/// The Menu, in order. The best History line (if any) comes first, so it is also the Ghost
/// text; then Spec Suggestions ordered by how much the user has used each; then a few more
/// History lines.
pub fn suggest(cx: &Ctx, history: &History, r: &Request) -> Vec<Suggestion> {
    let mut words = if cx.sources.specs {
        complete(cx, r.words)
    } else {
        vec![]
    };
    if !cx.sources.history {
        return words;
    }
    let typed: Vec<&str> = r
        .words
        .iter()
        .map(String::as_str)
        .skip_while(|w| is_assignment(w))
        .collect();
    let before = typed[..typed.len().saturating_sub(1)].join(" ");
    for s in &mut words {
        let line = if before.is_empty() {
            s.insert.clone()
        } else {
            format!("{before} {}", s.insert)
        };
        s.usage = history.usage(&line, r.dir, r.now);
    }
    words.sort_by(|a, b| b.usage.total_cmp(&a.usage)); // stable: ties keep Spec order

    let mut lines = history
        .lines(r.line, r.dir, r.now, 1 + EXTRA_LINES)
        .into_iter()
        .map(|(cmd, desc)| Suggestion {
            insert: cmd.to_string(),
            description: desc,
            kind: 'l',
            priority: 0.0,
            usage: 0.0,
        });
    lines.next().into_iter().chain(words).chain(lines).collect()
}

/// `words` are the words of the current command, already alias-expanded by the shell.
/// The last word is the one being typed; it is empty right after a space.
pub fn complete(cx: &Ctx, words: &[String]) -> Vec<Suggestion> {
    // `FOO=1 BAR=2 cmd ...`: skip leading environment assignments.
    let words = &words[words.iter().take_while(|w| is_assignment(w)).count()..];
    let Some((current, done)) = words.split_last() else {
        return vec![];
    };
    let mut out = match done.split_first() {
        None => commands(cx.store, current),
        Some((cmd, rest)) => match cx.store.get(cmd) {
            Some(spec) if cx.disabled.contains(cmd) => {
                walk(&cx.without_generators(), &spec, rest, current, vec![])
            }
            Some(spec) => walk(cx, &spec, rest, current, vec![]),
            // No Spec: nothing here, so Tab hands over to zsh's own completion (Native completion).
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

fn commands(store: &Arc<Store>, current: &str) -> Vec<Suggestion> {
    store
        .commands()
        .iter()
        .filter(|n| n.starts_with(current))
        .map(|n| suggestion('c', n, None, None))
        .collect()
}

/// Walks the finished words `done` down from `cmd`, then suggests for `current`.
fn walk<'a>(
    cx: &Ctx,
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
            return descend(cx, sub, &done[i + 1..], current, persistent);
        }
        if let Some(arg) = cmd.args.get(arg_index) {
            if arg.is_command {
                let mut words = done[i..].to_vec();
                words.push(current.to_string());
                return complete(cx, &words);
            }
            if let Some(LoadSpec::Path(p)) = &arg.load_spec {
                return match cx.store.get(p) {
                    Some(spec) => walk(cx, &spec, &done[i + 1..], current, vec![]),
                    None => vec![],
                };
            }
            if !arg.is_variadic {
                arg_index += 1;
            }
        }
    }

    if let Some(arg) = pending {
        return arg_suggestions(cx, arg, current);
    }
    if current.starts_with('-') && !only_args {
        return cmd
            .options
            .iter()
            .chain(persistent.iter().copied())
            .filter(|o| !o.hidden && (o.repeatable() || !used.iter().any(|u| std::ptr::eq(*u, *o))))
            .flat_map(|o| {
                o.name.iter().map(move |n| {
                    suggestion('o', n, o.description.as_deref(), o.priority)
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
                    suggestion('c', n, s.description.as_deref(), s.priority)
                        .with_insert(s.insert_value.as_deref()),
                );
            }
        }
    }
    if let Some(arg) = cmd.args.get(arg_index) {
        if arg.is_command {
            out.extend(commands(cx.store, current));
        } else {
            out.extend(arg_suggestions(cx, arg, current));
        }
    }
    out
}

/// Enters a subcommand, following its `loadSpec` if it lives in another file.
fn descend<'a>(
    cx: &Ctx,
    sub: &'a Subcommand,
    done: &[String],
    current: &str,
    persistent: Vec<&'a Opt>,
) -> Vec<Suggestion> {
    match &sub.load_spec {
        Some(LoadSpec::Path(p)) => match cx.store.get(p) {
            // Persistent options don't cross into a separately loaded file.
            Some(spec) => walk(cx, &spec, done, current, vec![]),
            None => walk(cx, sub, done, current, persistent),
        },
        Some(LoadSpec::Inline(spec)) => walk(cx, spec, done, current, persistent),
        None => walk(cx, sub, done, current, persistent),
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

fn arg_suggestions(cx: &Ctx, arg: &Arg, current: &str) -> Vec<Suggestion> {
    let mut out: Vec<Suggestion> = arg
        .suggestions
        .iter()
        .flat_map(|s| match s {
            SuggestionSpec::Name(n) => vec![suggestion('a', n, None, None)],
            SuggestionSpec::Full(f) if !f.hidden => f
                .name
                .iter()
                .map(|n| {
                    suggestion('a', n, f.description.as_deref(), f.priority)
                        .with_insert(f.insert_value.as_deref())
                })
                .collect(),
            SuggestionSpec::Full(_) => vec![],
        })
        .collect();
    for t in &arg.template {
        out.extend(template_suggestions(cx, t, current));
    }
    for g in &arg.generators {
        for t in &g.template {
            out.extend(template_suggestions(cx, t, current));
        }
        if let Some((gens, notify)) = cx.gens {
            let (items, running) = gens.get(g, cx.dir, notify);
            cx.pending.set(cx.pending.get() || running);
            out.extend(
                // Above files and static values, as Fig ranked them: a branch beats README.md.
                items
                    .iter()
                    .flat_map(|i| i.iter())
                    .map(|(n, d)| suggestion('v', n, Some(d), Some(75.0))),
            );
        }
    }
    out
}

fn template_suggestions(cx: &Ctx, template: &str, current: &str) -> Vec<Suggestion> {
    if !cx.sources.files {
        return vec![];
    }
    match template {
        "filepaths" => file_suggestions(cx, current, false),
        "folders" => file_suggestions(cx, current, true),
        _ => vec![], // `history` and `help` templates: not supported
    }
}

fn file_suggestions(cx: &Ctx, current: &str, folders_only: bool) -> Vec<Suggestion> {
    generate::files(cx.dir, current, folders_only)
        .into_iter()
        .map(|(n, d): (String, String)| {
            let kind = if n.ends_with('/') { 'd' } else { 'f' };
            suggestion(kind, &n, Some(&d), None)
        })
        .collect()
}

fn suggestion(
    kind: char,
    name: &str,
    description: Option<&str>,
    priority: Option<f32>,
) -> Suggestion {
    Suggestion {
        insert: name.to_string(),
        description: description
            .unwrap_or_default()
            .lines()
            .next()
            .unwrap_or_default()
            .to_string(),
        kind,
        priority: priority.unwrap_or(50.0),
        usage: 0.0,
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

    fn store() -> Arc<Store> {
        Arc::new(Store::new(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("specs"),
            None,
        ))
    }

    fn names(line: &str) -> Vec<String> {
        let mut words: Vec<String> = line.split(' ').map(String::from).collect();
        if words.is_empty() {
            words.push(String::new());
        }
        let s = store();
        complete(&Ctx::new(&s, env!("CARGO_MANIFEST_DIR"), ALL), &words)
            .into_iter()
            .map(|s| s.insert)
            .collect()
    }

    #[test]
    fn command_names() {
        assert!(names("gi").contains(&"git".to_string()));
        assert!(names("l").contains(&"ls".to_string())); // no Spec needed: it's on PATH
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
        complete(&Ctx::new(&s, "", ALL), &["git".into(), "ch".into()]);
        assert_eq!(s.loaded_count(), 1);
    }

    #[test]
    fn history_ranks_words_and_leads_with_a_line() {
        let s = store();
        let mut h = History::default();
        h.record(100, Some(0), "/r", "git checkout main");
        let req = |line: &'static str| line.split(' ').map(String::from).collect::<Vec<_>>();
        let words = req("gi");
        let got = suggest(
            &Ctx::new(&s, "/r", ALL),
            &h,
            &Request {
                dir: "/r",
                line: "gi",
                words: &words,
                now: 200,
            },
        );
        assert!(got[0].kind == 'l' && got[0].insert == "git checkout main");
        assert_eq!(got[1].insert, "git"); // used, so it beats `gibo`, which sorts first
        let words = req("git che");
        let got = suggest(
            &Ctx::new(&s, "/r", ALL),
            &h,
            &Request {
                dir: "/r",
                line: "git che",
                words: &words,
                now: 200,
            },
        );
        assert_eq!(got[1].insert, "checkout"); // used, so it beats `check-attr` etc.
    }

    #[test]
    fn files_for_specless_commands_and_templates() {
        // Run from the repo root: `src/` exists there.
        assert!(names("nosuchcmd sr").is_empty()); // left to Native completion
        assert!(names("cat src/ma").contains(&"src/main.rs".to_string()));
        assert!(names("cd sr").contains(&"src/".to_string()));
    }

    #[test]
    fn generator_results_join_the_menu() {
        let s = store();
        let gens = Arc::new(Generators::default());
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        let tx = std::sync::Mutex::new(tx);
        let notify: Notify = Arc::new(move || {
            let _ = tx.lock().unwrap().send(());
        });
        let dir = env!("CARGO_MANIFEST_DIR");
        let cx = Ctx {
            gens: Some((&gens, &notify)),
            ..Ctx::new(&s, dir, ALL)
        };
        let words: Vec<String> = ["git", "checkout", ""].map(String::from).to_vec();
        complete(&cx, &words); // starts `git branch` in the background
        rx.recv_timeout(std::time::Duration::from_secs(5))
            .expect("Generator finished");
        let branch = std::process::Command::new("git")
            .args(["branch", "--show-current"])
            .current_dir(dir)
            .output()
            .unwrap();
        let branch = String::from_utf8_lossy(&branch.stdout).trim().to_string();
        let got: Vec<String> = complete(&cx, &words)
            .into_iter()
            .map(|s| s.insert)
            .collect();
        assert!(got.contains(&branch), "{branch} not in {got:?}");
    }

    #[test]
    fn user_specs_in_yaml() {
        let user = std::env::temp_dir().join(format!("flick-user-specs-{}", std::process::id()));
        std::fs::create_dir_all(&user).unwrap();
        std::fs::write(
            user.join("mytool.yaml"),
            "name: mytool\nsubcommands:\n  - name: start\n    description: Start it\n  - name: stop\noptions:\n  - name: [-v, --verbose]\n",
        )
        .unwrap();
        let s = Arc::new(Store::new(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("specs"),
            Some(user.clone()),
        ));
        let cx = Ctx::new(&s, "", ALL);
        let words = |l: &str| l.split(' ').map(String::from).collect::<Vec<_>>();
        let got: Vec<String> = complete(&cx, &words("mytool st"))
            .into_iter()
            .map(|s| s.insert)
            .collect();
        assert_eq!(got, vec!["start", "stop"]);
        assert!(
            complete(&cx, &words("mytool --v"))
                .iter()
                .any(|s| s.insert == "--verbose")
        );
        assert!(
            complete(&cx, &words("myt"))
                .iter()
                .any(|s| s.insert == "mytool")
        );
        std::fs::remove_dir_all(&user).unwrap();
    }

    #[test]
    fn sources_can_be_switched_off() {
        let s = store();
        let mut h = History::default();
        h.record(100, Some(0), "", "git checkout main");
        let words: Vec<String> = ["git", "ch"].map(String::from).to_vec();
        let r = Request {
            dir: "",
            line: "git ch",
            words: &words,
            now: 200,
        };
        let only_history = Sources {
            specs: false,
            generators: false,
            files: false,
            ..ALL
        };
        let got = suggest(&Ctx::new(&s, "", only_history), &h, &r);
        assert!(got.iter().all(|s| s.kind == 'l') && !got.is_empty());
        let no_history = Sources {
            history: false,
            ..ALL
        };
        assert!(
            suggest(&Ctx::new(&s, "", no_history), &h, &r)
                .iter()
                .all(|s| s.kind != 'l')
        );
    }

    #[test]
    fn kinds_tell_the_shell_what_each_suggestion_is() {
        let s = store();
        let cx = Ctx::new(&s, env!("CARGO_MANIFEST_DIR"), ALL);
        let kinds = |l: &str| {
            let words: Vec<String> = l.split(' ').map(String::from).collect();
            complete(&cx, &words)
                .into_iter()
                .map(|s| (s.insert, s.kind))
                .collect::<Vec<_>>()
        };
        assert!(kinds("git ch").contains(&("checkout".into(), 'c')));
        assert!(kinds("git commit --am").contains(&("--amend".into(), 'o')));
        assert!(kinds("cat sr").contains(&("src/".into(), 'd')));
        assert!(kinds("cat Cargo.t").contains(&("Cargo.toml".into(), 'f')));
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
