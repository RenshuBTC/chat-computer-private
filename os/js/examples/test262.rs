//! Runs Ecma's conformance tests (test262, a copy of github.com/tc39/test262 given as the first argument) through
//! the engine: for now, whether each test parses (or, a test that must fail to parse, doesn't). Prints how many
//! passed, by directory, and the first few failures (with --failures N, N of them).
//!
//!     cargo run --release -p js --example test262 -- PATH/TO/test262 [test/language] [--failures 40]
use js::parser::parse;
use js::string::Atoms;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<_> = rd.flatten().map(|e| e.path()).collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            walk(&p, out);
        } else if p.extension().is_some_and(|e| e == "js") && !p.to_string_lossy().contains("_FIXTURE") {
            out.push(p);
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let root = PathBuf::from(&args[1]);
    let sub = args.get(2).filter(|a| !a.starts_with("--")).cloned().unwrap_or_else(|| String::from("test/language"));
    let show: usize = args.iter().position(|a| a == "--failures").and_then(|i| args.get(i + 1)).and_then(|v| v.parse().ok()).unwrap_or(15);
    // (--valid: only tests that should parse are shown failing: what real code would hit.)
    let valid_only = args.iter().any(|a| a == "--valid");
    let mut wrongly_refused = 0usize;
    let mut files = Vec::new();
    walk(&root.join(&sub), &mut files);
    let (mut pass, mut fail) = (0usize, 0usize);
    let mut by_dir: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    let mut shown = 0;
    for f in &files {
        let Ok(src) = std::fs::read_to_string(f) else { continue };
        let meta = src.split("/*---").nth(1).and_then(|m| m.split("---*/").next()).unwrap_or("");
        let negative_parse = meta.contains("phase: parse");
        let flags = meta.lines().find(|l| l.trim_start().starts_with("flags:")).unwrap_or("");
        let module = flags.contains("module");
        let only_strict = flags.contains("onlyStrict");
        let raw = flags.contains("raw");
        // Features the engine doesn't parse on purpose (proposals not in ECMAScript 2024).
        let features = meta.lines().find(|l| l.trim_start().starts_with("features:")).unwrap_or("");
        if ["decorators", "source-phase-imports", "import-defer", "explicit-resource-management", "import-assertions"].iter().any(|x| features.contains(x)) {
            continue;
        }
        let mut atoms = Atoms::new();
        let text = if only_strict && !raw { format!("\"use strict\";\n{src}") } else { src.clone() };
        let r = parse(&text, &mut atoms, module, false);
        let ok = r.is_ok() != negative_parse;
        let dir = f.strip_prefix(&root).unwrap_or(f).parent().map(|p| p.to_string_lossy().to_string()).unwrap_or_default();
        let key = dir.split('/').take(4).collect::<Vec<_>>().join("/");
        let e = by_dir.entry(key).or_default();
        if ok {
            pass += 1;
            e.0 += 1;
        } else {
            fail += 1;
            e.1 += 1;
            if r.is_err() {
                wrongly_refused += 1;
            }
            if shown < show && (!valid_only || r.is_err()) {
                shown += 1;
                match &r {
                    Ok(_) => println!("FAIL (parsed, should not) {}", f.display()),
                    Err(err) => {
                        let (l, c) = js::parser::line_col(&text, err.at);
                        println!("FAIL {} ({l}:{c}): {}", f.display(), err.message);
                    }
                }
            }
        }
    }
    for (d, (p, f)) in &by_dir {
        if *f > 0 {
            println!("{d}: {p} passed, {f} failed");
        }
    }
    println!("{pass} of {} passed ({:.1}%); {wrongly_refused} valid ones refused", pass + fail, pass as f64 * 100.0 / (pass + fail).max(1) as f64);
}
