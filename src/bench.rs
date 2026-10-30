//! `flick bench`: measures flick against the soft budgets in REQUIREMENTS.md (Q29).
//!
//! Starts its own background process on a temporary socket, with a temporary copy of the
//! user's History, so nothing real is touched.
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use std::{env, fs, thread};

const LINES: &[&str] = &[
    "g",
    "gi",
    "git ch",
    "git checkout ",
    "git commit --am",
    "docker r",
    "brew ins",
    "npm run ",
    "ls -",
    "cd ",
    "aws s3 c",
    "FOO=1 git sta",
    "kubectl get ",
];
const BUDGET_KEYPRESS_MS: f64 = 10.0;
const BUDGET_STARTUP_MS: f64 = 20.0;
const BUDGET_MEMORY_MB: f64 = 15.0;

/// Returns the exit code: 0 if every budget holds.
pub fn run() -> i32 {
    let tmp = env::temp_dir().join(format!("flick-bench-{}", std::process::id()));
    let _ = fs::create_dir_all(&tmp);
    let code = bench(&tmp).unwrap_or_else(|e| {
        eprintln!("flick bench: {e}");
        2
    });
    let _ = fs::remove_dir_all(&tmp);
    code
}

fn bench(tmp: &Path) -> Result<i32, String> {
    let exe = env::current_exe().map_err(|e| e.to_string())?;
    let sock = tmp.join("s.sock");
    let history = tmp.join("history.tsv");
    let real = crate::history_path();
    if real.exists() {
        fs::copy(&real, &history).map_err(|e| format!("copying History: {e}"))?;
    }
    let mut daemon = Command::new(&exe)
        .arg("serve")
        .arg(&sock)
        .env("FLICK_HISTORY", &history)
        .env("FLICK_HISTFILE", crate::zsh_history_path()) // read only, for the first-run import
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| e.to_string())?;
    let stream = (0..100)
        .find_map(|_| {
            UnixStream::connect(&sock).ok().or_else(|| {
                thread::sleep(Duration::from_millis(20));
                None
            })
        })
        .ok_or("the background process did not start")?;
    let mut reader = BufReader::new(stream.try_clone().map_err(|e| e.to_string())?);
    let mut writer = stream;
    let dir = env::current_dir()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let mut ask = |line: &str| -> Duration {
        let mut words: Vec<&str> = line.split(' ').collect();
        if words.is_empty() {
            words.push("");
        }
        let req = format!("c\t{dir}\t{line}\t{}\n", words.join("\t"));
        let t = Instant::now();
        writer.write_all(req.as_bytes()).unwrap();
        let mut buf = String::new();
        loop {
            buf.clear();
            if reader.read_line(&mut buf).unwrap_or(0) == 0 || buf == "\n" {
                break;
            }
        }
        t.elapsed()
    };

    for l in LINES {
        ask(l); // first use loads Specs and starts Generators
    }
    thread::sleep(Duration::from_millis(500));
    let mut times: Vec<f64> = (0..2000)
        .map(|i| ask(LINES[i % LINES.len()]).as_secs_f64() * 1000.0)
        .collect();
    times.sort_by(f64::total_cmp);
    let pct = |p: usize| times[(times.len() * p / 100).min(times.len() - 1)];
    let rss_before = rss_mb(daemon.id());
    for i in 0..20_000 {
        ask(LINES[i % LINES.len()]);
    }
    let rss_after = rss_mb(daemon.id());
    let _ = daemon.kill();
    let _ = daemon.wait();

    let startup = startup_ms(tmp);

    println!(
        "flick bench (History: {})\n",
        if real.exists() {
            "copy of yours"
        } else {
            "empty"
        }
    );
    let mut ok = true;
    let mut row = |what: &str, value: String, pass: Option<bool>, budget: &str| {
        let mark = match pass {
            Some(true) => "ok  ",
            Some(false) => "OVER",
            None => "    ",
        };
        ok &= pass != Some(false);
        println!("  {mark}  {what:<38} {value:<28} {budget}");
    };
    row(
        "keypress → answer, p50",
        format!("{:.3} ms", pct(50)),
        None,
        "",
    );
    row(
        "keypress → answer, p95",
        format!("{:.3} ms", pct(95)),
        Some(pct(95) <= BUDGET_KEYPRESS_MS),
        "≤ 10 ms",
    );
    row(
        "keypress → answer, max",
        format!("{:.3} ms", pct(100)),
        None,
        "",
    );
    row(
        "memory after 2k requests",
        format!("{rss_before:.1} MB"),
        Some(rss_before <= BUDGET_MEMORY_MB),
        "≤ 15 MB",
    );
    row(
        "memory after 20k more",
        format!("{rss_after:.1} MB ({:+.1} MB)", rss_after - rss_before),
        Some(rss_after <= BUDGET_MEMORY_MB),
        "≤ 15 MB, flat",
    );
    match startup {
        Some(ms) => row(
            "added shell startup",
            format!("{ms:.1} ms"),
            Some(ms <= BUDGET_STARTUP_MS),
            "≤ 20 ms",
        ),
        None => row(
            "added shell startup",
            "skipped: flick.zsh or zsh not found".into(),
            None,
            "",
        ),
    }
    println!("\nKeypress time is the socket round trip; zsh adds ~0.3 ms to draw the Menu.");
    Ok(if ok { 0 } else { 1 })
}

fn rss_mb(pid: u32) -> f64 {
    let out = Command::new("ps")
        .args(["-o", "rss=", "-p", &pid.to_string()])
        .output();
    out.ok()
        .and_then(|o| {
            String::from_utf8_lossy(&o.stdout)
                .trim()
                .parse::<f64>()
                .ok()
        })
        .map_or(0.0, |kb| kb / 1024.0)
}

/// Median time to `source flick.zsh` in a fresh `zsh -f`, minus an empty `zsh -f`.
fn startup_ms(tmp: &Path) -> Option<f64> {
    let plugin = flick_zsh()?;
    let time = |script: &str| -> Option<f64> {
        let mut runs: Vec<f64> = (0..30)
            .map(|_| {
                let t = Instant::now();
                let status = Command::new("zsh")
                    .args(["-f", "-c", script])
                    .env("XDG_CACHE_HOME", tmp) // keep the generated config cache out of ~/.cache
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status();
                status
                    .ok()
                    .filter(|s| s.success())
                    .map(|_| t.elapsed().as_secs_f64() * 1000.0)
            })
            .collect::<Option<_>>()?;
        runs.sort_by(f64::total_cmp);
        Some(runs[runs.len() / 2])
    };
    let with = time(&format!("source {:?}", plugin.to_string_lossy()))?;
    let without = time(":")?;
    Some((with - without).max(0.0))
}

fn flick_zsh() -> Option<PathBuf> {
    let exe = env::current_exe().ok()?.canonicalize().ok()?;
    let candidates = [
        exe.ancestors().nth(3).map(|r| r.join("flick.zsh")), // <repo>/target/release/flick
        exe.parent().map(|d| d.join("flick.zsh")),
        Some(crate::home().join(".local/share/flick/flick.zsh")),
    ];
    candidates.into_iter().flatten().find(|p| p.is_file())
}
