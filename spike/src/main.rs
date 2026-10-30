// Spike: measure shell <-> daemon round-trip. Throwaway code.
// Protocol: client sends one line (the buffer), daemon replies with up to 8
// suggestion lines followed by an empty line.
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::sync::Arc;
use std::{env, fs, thread};

fn load_history(path: &str) -> Vec<(String, u32)> {
    let bytes = fs::read(path).unwrap_or_default();
    let mut counts: HashMap<String, u32> = HashMap::new();
    for line in String::from_utf8_lossy(&bytes).lines() {
        // Extended history format: ": <time>:<duration>;<command>"
        let cmd = match line.strip_prefix(": ") {
            Some(rest) => rest.split_once(';').map_or(rest, |(_, c)| c),
            None => line,
        };
        if cmd.is_empty() || cmd.ends_with('\\') {
            continue;
        }
        *counts.entry(cmd.to_string()).or_default() += 1;
    }
    let mut v: Vec<_> = counts.into_iter().collect();
    v.sort_by_key(|a| std::cmp::Reverse(a.1));
    v
}

// Prefix matches first (by count), then subsequence matches.
fn suggest<'a>(hist: &'a [(String, u32)], q: &str) -> Vec<&'a str> {
    if q.is_empty() {
        return vec![];
    }
    let mut out: Vec<&str> = hist
        .iter()
        .filter(|(c, _)| c.starts_with(q) && c != q)
        .take(8)
        .map(|(c, _)| c.as_str())
        .collect();
    if out.len() < 8 {
        let fuzzy = hist
            .iter()
            .filter(|(c, _)| !c.starts_with(q) && is_subseq(q, c));
        out.extend(fuzzy.take(8 - out.len()).map(|(c, _)| c.as_str()));
    }
    out
}

fn is_subseq(q: &str, s: &str) -> bool {
    let mut it = s.chars();
    q.chars().all(|qc| it.any(|sc| sc == qc))
}

fn main() {
    let sock = env::args()
        .nth(1)
        .expect("usage: flickd <socket> [histfile]");
    let histfile = env::args()
        .nth(2)
        .unwrap_or_else(|| format!("{}/.zsh_history", env::var("HOME").unwrap()));
    let hist = Arc::new(load_history(&histfile));
    let _ = fs::remove_file(&sock);
    let listener = UnixListener::bind(&sock).expect("bind");
    for stream in listener.incoming().flatten() {
        let hist = Arc::clone(&hist);
        thread::spawn(move || {
            let mut w = stream.try_clone().unwrap();
            for line in BufReader::new(stream).lines().map_while(Result::ok) {
                let mut resp = String::new();
                for s in suggest(&hist, &line) {
                    resp.push_str(s);
                    resp.push('\n');
                }
                resp.push('\n');
                if w.write_all(resp.as_bytes()).is_err() {
                    break;
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ranks_prefix_before_fuzzy() {
        let h = vec![
            ("gco main".into(), 5),
            ("git commit".into(), 3),
            ("git checkout".into(), 1),
        ];
        assert_eq!(suggest(&h, "git c"), vec!["git commit", "git checkout"]);
        assert_eq!(
            suggest(&h, "gc"),
            vec!["gco main", "git commit", "git checkout"]
        );
    }
}
