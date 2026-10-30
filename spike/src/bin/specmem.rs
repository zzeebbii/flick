// Spike: memory cost of holding converted Specs in typed structs. Throwaway.
// Usage: specmem <specs-dir> [cli...]   (no cli names = load every .json under the dir)
// Run under `/usr/bin/time -l` to read max RSS.
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::{env, fs, time::Instant};

#[derive(Deserialize)]
#[serde(untagged)]
enum OneOrMany<T> {
    One(T),
    Many(Vec<T>),
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Suggestion {
    Name(Box<str>),
    Full {
        name: Option<OneOrMany<Box<str>>>,
        description: Option<Box<str>>,
    },
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
struct Generator {
    script: Option<OneOrMany<Box<str>>>,
    split_on: Option<Box<str>>,
    template: Option<OneOrMany<Box<str>>>,
    #[serde(default)]
    needs_code: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
struct Arg {
    name: Option<Box<str>>,
    description: Option<Box<str>>,
    #[serde(default)]
    is_optional: bool,
    #[serde(default)]
    is_variadic: bool,
    suggestions: Option<Vec<Suggestion>>,
    template: Option<OneOrMany<Box<str>>>,
    generators: Option<Vec<Generator>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
struct Opt {
    name: OneOrMany<Box<str>>,
    description: Option<Box<str>>,
    args: Option<OneOrMany<Arg>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
struct Subcommand {
    name: OneOrMany<Box<str>>,
    description: Option<Box<str>>,
    subcommands: Option<Vec<Subcommand>>,
    options: Option<Vec<Opt>>,
    args: Option<OneOrMany<Arg>>,
    load_spec: Option<serde_json::Value>,
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, out);
        } else if p.extension().is_some_and(|x| x == "json") {
            out.push(p);
        }
    }
}

fn main() {
    let mut args = env::args().skip(1);
    let dir = PathBuf::from(args.next().expect("usage: specmem <specs-dir> [cli...]"));
    let clis: Vec<String> = args.collect();
    let mut files = vec![];
    if clis.is_empty() {
        walk(&dir, &mut files);
    } else {
        files = clis.iter().map(|c| dir.join(format!("{c}.json"))).collect();
    }
    let t = Instant::now();
    let mut bytes = 0;
    let specs: Vec<Subcommand> = files
        .iter()
        .filter_map(|f| {
            let s = fs::read(f).ok()?;
            bytes += s.len();
            match serde_json::from_slice(&s) {
                Ok(spec) => Some(spec),
                Err(e) => {
                    eprintln!("{}: {e}", f.display());
                    None
                }
            }
        })
        .collect();
    println!(
        "loaded {} of {} spec files ({:.1} MB JSON) in {:?}",
        specs.len(),
        files.len(),
        bytes as f64 / 1e6,
        t.elapsed()
    );
}
