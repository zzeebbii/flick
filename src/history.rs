//! flick's own History: every command with its directory, exit code and time (Q11, Q24).
//!
//! On disk: one History entry per line, `time<TAB>exit<TAB>dir<TAB>command`, with `\`, tab and
//! newline escaped. In memory: one aggregate per distinct command, sorted by command so a
//! prefix is a contiguous range.
use std::collections::hash_map::DefaultHasher;
use std::fs::{self, File, OpenOptions};
use std::hash::{Hash, Hasher};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::Path;

/// Directories remembered per command, for the current-directory boost.
const MAX_DIRS: usize = 8;

struct Entry {
    cmd: Box<str>,
    uses: u32,
    fails: u32,
    last: u64,
    dirs: Vec<(u64, u32)>, // (hash of directory, uses there)
}

pub struct History {
    /// Commands containing any of these (case-insensitive) are never recorded (Q24).
    pub ignore: Vec<String>,
    entries: Vec<Entry>,
    file: Option<File>,
}

impl History {
    /// Loads `path`. If it doesn't exist yet, first imports zsh's history from `import_from`.
    pub fn open(path: &Path, import_from: Option<&Path>, ignore: Vec<String>) -> History {
        let mut h = History {
            ignore,
            ..History::default()
        };
        if !path.exists() {
            if let Some(parent) = path.parent() {
                let _ = fs::create_dir_all(parent);
                let _ = fs::set_permissions(parent, fs::Permissions::from_mode(0o700));
            }
            let imported = import_from
                .and_then(|p| fs::read(p).ok())
                .map(|b| parse_zsh_history(&b));
            let mut out = String::new();
            for (time, cmd) in imported.unwrap_or_default() {
                if h.recordable(&cmd) {
                    out.push_str(&format!("{time}\t\t\t{}\n", escape(&cmd)));
                }
            }
            let _ = write_private(path, out.as_bytes());
        }
        if let Ok(f) = File::open(path) {
            let mut rows: Vec<(u64, Option<i32>, String, String)> = vec![];
            for line in BufReader::new(f).split(b'\n').map_while(Result::ok) {
                let line = String::from_utf8_lossy(&line);
                let mut f = line.splitn(4, '\t');
                let (Some(t), Some(exit), Some(dir), Some(cmd)) =
                    (f.next(), f.next(), f.next(), f.next())
                else {
                    continue;
                };
                rows.push((
                    t.parse().unwrap_or(0),
                    exit.parse().ok(),
                    unescape(dir),
                    unescape(cmd),
                ));
            }
            // A pattern added to `[history] ignore` later also hides what was recorded before it.
            rows.retain(|r| h.recordable(&r.3));
            // Sorted by command, every `add` appends instead of inserting in the middle.
            rows.sort_by(|a, b| a.3.trim_end().cmp(b.3.trim_end()));
            for (t, exit, dir, cmd) in rows {
                h.add(t, exit, &dir, &cmd);
            }
        }
        h.file = OpenOptions::new().append(true).open(path).ok();
        h
    }

    /// Records a command the user just ran, in memory and on disk.
    pub fn record(&mut self, time: u64, exit: Option<i32>, dir: &str, cmd: &str) {
        let cmd = cmd.trim_end();
        if !self.recordable(cmd) {
            return;
        }
        self.add(time, exit, dir, cmd);
        if let Some(f) = &mut self.file {
            let exit = exit.map(|e| e.to_string()).unwrap_or_default();
            let _ = writeln!(f, "{time}\t{exit}\t{}\t{}", escape(dir), escape(cmd));
        }
    }

    fn add(&mut self, time: u64, exit: Option<i32>, dir: &str, cmd: &str) {
        let cmd = cmd.trim_end();
        if cmd.is_empty() {
            return;
        }
        let i = match self.entries.binary_search_by(|e| (*e.cmd).cmp(cmd)) {
            Ok(i) => i,
            Err(i) => {
                let entry = Entry {
                    cmd: cmd.into(),
                    uses: 0,
                    fails: 0,
                    last: 0,
                    dirs: vec![],
                };
                self.entries.insert(i, entry);
                i
            }
        };
        let e = &mut self.entries[i];
        e.uses += 1;
        e.fails += u32::from(exit.is_some_and(|x| x != 0));
        e.last = e.last.max(time);
        if !dir.is_empty() {
            let d = hash(dir);
            if let Some((_, n)) = e.dirs.iter_mut().find(|(h, _)| *h == d) {
                *n += 1;
            } else if e.dirs.len() < MAX_DIRS {
                e.dirs.push((d, 1));
            }
        }
    }

    fn range<'a>(&'a self, prefix: &'a str) -> impl Iterator<Item = &'a Entry> + 'a {
        let start = self.entries.partition_point(|e| *e.cmd < *prefix);
        self.entries[start..]
            .iter()
            .take_while(move |e| e.cmd.starts_with(prefix))
    }

    /// Whole commands that start with `prefix`, best first. Commands that only ever failed are left out.
    pub fn lines<'a>(
        &'a self,
        prefix: &'a str,
        dir: &str,
        now: u64,
        limit: usize,
    ) -> Vec<(&'a str, String)> {
        if prefix.trim().is_empty() {
            return vec![];
        }
        let d = hash(dir);
        let mut hits: Vec<(&Entry, f32)> = self
            .range(prefix)
            .filter(|e| *e.cmd != *prefix && e.fails < e.uses && !e.cmd.contains('\n'))
            .map(|e| (e, score(e, d, now)))
            .collect();
        hits.sort_by(|a, b| b.1.total_cmp(&a.1));
        hits.into_iter()
            .take(limit)
            .map(|(e, _)| (&*e.cmd, describe(e, now)))
            .collect()
    }

    /// How much the user has run `line` itself or anything that continues it as a word, e.g.
    /// `git checkout` counts `git checkout` and `git checkout main` but not `git checkout-index`.
    pub fn usage(&self, line: &str, dir: &str, now: u64) -> f32 {
        let d = hash(dir);
        let with_space = format!("{line} ");
        let exact = self.entries.binary_search_by(|e| (*e.cmd).cmp(line)).ok();
        exact.map_or(0.0, |i| score(&self.entries[i], d, now))
            + self
                .range(&with_space)
                .map(|e| score(e, d, now))
                .sum::<f32>()
    }
}

/// Frecency: uses weighted by how recently the command last ran, doubled in a directory it ran in.
fn score(e: &Entry, dir: u64, now: u64) -> f32 {
    let age = now.saturating_sub(e.last);
    let recency = match age {
        a if a < 86_400 => 4.0,
        a if a < 7 * 86_400 => 2.0,
        a if a < 30 * 86_400 => 1.0,
        _ => 0.5,
    };
    let here = if e.dirs.iter().any(|(h, _)| *h == dir) {
        2.0
    } else {
        1.0
    };
    (e.uses - e.fails.min(e.uses)) as f32 * recency * here
}

fn describe(e: &Entry, now: u64) -> String {
    let age = now.saturating_sub(e.last);
    let ago = match age {
        0 if e.last == 0 => return format!("history · {}×", e.uses),
        a if a < 3600 => format!("{}m ago", a / 60),
        a if a < 86_400 => format!("{}h ago", a / 3600),
        a => format!("{}d ago", a / 86_400),
    };
    format!("history · {}× · {ago}", e.uses)
}

impl Default for History {
    fn default() -> Self {
        History {
            ignore: crate::config::HistoryConfig::default().ignore,
            entries: vec![],
            file: None,
        }
    }
}

impl History {
    fn recordable(&self, cmd: &str) -> bool {
        let lower = cmd.to_lowercase();
        !cmd.trim().is_empty()
            && !cmd.starts_with(' ')
            && !self
                .ignore
                .iter()
                .any(|p| lower.contains(&p.to_lowercase()))
    }
}

fn hash(s: &str) -> u64 {
    let mut h = DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?
        .write_all(bytes)
}

pub fn escape(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('\t', "\\t")
        .replace('\n', "\\n")
}

pub fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('t') => out.push('\t'),
            Some('n') => out.push('\n'),
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    out
}

/// zsh's history file: optional `: <time>:<duration>;` prefix, multi-line commands joined with
/// a trailing backslash, and "metafied" bytes (0x83 followed by the byte XOR 0x20).
pub fn parse_zsh_history(bytes: &[u8]) -> Vec<(u64, String)> {
    let mut raw = Vec::with_capacity(bytes.len());
    let mut it = bytes.iter();
    while let Some(&b) = it.next() {
        raw.push(if b == 0x83 {
            it.next().map_or(b, |n| n ^ 0x20)
        } else {
            b
        });
    }
    let text = String::from_utf8_lossy(&raw);
    let mut out = vec![];
    let mut pending: Option<(u64, String)> = None;
    for line in text.split('\n') {
        let (time, cmd, part) = match pending.take() {
            Some((t, mut c)) => {
                c.push('\n');
                (t, c, line)
            }
            None => match line.strip_prefix(": ").and_then(|r| r.split_once(';')) {
                Some((meta, cmd)) => (
                    meta.split(':')
                        .next()
                        .and_then(|t| t.parse().ok())
                        .unwrap_or(0),
                    String::new(),
                    cmd,
                ),
                None => (0, String::new(), line),
            },
        };
        let mut cmd = cmd;
        match part.strip_suffix('\\') {
            Some(head) => {
                cmd.push_str(head);
                pending = Some((time, cmd));
            }
            None => {
                cmd.push_str(part);
                if !cmd.is_empty() {
                    out.push((time, cmd));
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    const DAY: u64 = 86_400;

    fn mem(entries: &[(u64, Option<i32>, &str, &str)]) -> History {
        let mut h = History::default();
        for &(t, e, d, c) in entries {
            h.record(t, e, d, c);
        }
        h
    }

    #[test]
    fn parses_zsh_history() {
        // "→" is e2 86 92; zsh stores 0x86 and 0x92 metafied as 83 a6 and 83 b2.
        let bytes = b": 1700000000:0;git status\n: 1700000100:0;echo a\\\nb\nplain cmd\n: 1700000200:0;echo \xe2\x83\xa6\x83\xb2\n";
        let got = parse_zsh_history(bytes);
        assert_eq!(got[0], (1_700_000_000, "git status".into()));
        assert_eq!(got[1], (1_700_000_100, "echo a\nb".into()));
        assert_eq!(got[2], (0, "plain cmd".into()));
        assert_eq!(got[3].1, "echo →");
    }

    #[test]
    fn secrets_and_leading_space_are_not_recorded() {
        let h = mem(&[
            (1, Some(0), "", " secret-ish"),
            (1, Some(0), "", "export API_TOKEN=x"),
            (1, Some(0), "", "mysql --password=x"),
            (
                1,
                Some(0),
                "",
                "curl -H 'Authorization: Bearer abc' https://api.github.com",
            ),
            (1, Some(0), "", "gh auth login --with-token ghp_abc123"),
        ]);
        assert!(h.entries.is_empty());
    }

    #[test]
    fn frecency_ranks_lines() {
        let now = 100 * DAY;
        let h = mem(&[
            (now - 60 * DAY, Some(0), "", "git push --force"),
            (now - 60 * DAY, Some(0), "", "git push --force"),
            (now - 60 * DAY, Some(0), "", "git push --force"),
            (now - 3600, Some(0), "", "git pull"),
        ]);
        let got: Vec<&str> = h
            .lines("git p", "", now, 5)
            .into_iter()
            .map(|(c, _)| c)
            .collect();
        // 3 uses two months ago (1.5) lose to 1 use an hour ago (4).
        assert_eq!(got, vec!["git pull", "git push --force"]);
    }

    #[test]
    fn current_directory_boost_and_failed_commands() {
        let now = 10 * DAY;
        let h = mem(&[
            (now - 3600, Some(0), "/a", "make test"),
            (now - 3600, Some(0), "/b", "make build"),
            (now - 3600, Some(1), "/b", "make typo"),
        ]);
        let in_b: Vec<&str> = h
            .lines("make ", "/b", now, 5)
            .into_iter()
            .map(|(c, _)| c)
            .collect();
        assert_eq!(in_b, vec!["make build", "make test"]); // `make typo` only ever failed
        let in_a: Vec<&str> = h
            .lines("make ", "/a", now, 5)
            .into_iter()
            .map(|(c, _)| c)
            .collect();
        assert_eq!(in_a[0], "make test");
    }

    #[test]
    fn usage_counts_whole_words_only() {
        let h = mem(&[
            (1, Some(0), "", "git checkout main"),
            (1, Some(0), "", "git checkout-index -a"),
        ]);
        assert!(h.usage("git checkout", "", 1) > 0.0);
        assert!(h.usage("git checkout-index", "", 1) > 0.0);
        assert_eq!(h.usage("git check", "", 1), 0.0);
    }

    #[test]
    fn import_then_reload_from_disk() {
        let dir = std::env::temp_dir().join(format!("flick-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let zsh = dir.join("zsh_history");
        fs::write(&zsh, ": 5:0;git status\n: 6:0;git stash\n").unwrap();
        let path = dir.join("h/history.tsv");
        let mut h = History::open(&path, Some(&zsh), vec![]);
        h.record(7, Some(0), "/tmp", "echo a\tb\nc");
        drop(h);
        let h = History::open(&path, None, vec![]);
        assert_eq!(h.entries.len(), 3);
        assert!(h.entries.iter().any(|e| &*e.cmd == "echo a\tb\nc"));
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        fs::remove_dir_all(&dir).unwrap();
    }
}
