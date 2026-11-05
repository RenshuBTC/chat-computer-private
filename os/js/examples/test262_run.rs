//! Runs Ecma's conformance tests (test262) through the whole engine: each test with the harness files it includes,
//! in a VM of its own; a test passes if it runs to its end without throwing (or throws what its metadata says it
//! must, or for an async test, prints that it completed). Prints how many passed, by directory, and failures.
//!
//!     cargo run --release -p js --example test262_run -- PATH/TO/test262 [test/built-ins/Array] [--failures 40]
//!         [--strict] (also runs each test in strict mode, as test262 asks)  [--list FILE] (failures to a file)
use js::object::{Class, Key};
use js::value::{Gc, Value};
use js::vm::Vm;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc;

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    if dir.is_file() {
        out.push(dir.to_path_buf());
        return;
    }
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

/// Features the engine doesn't have (yet, or on purpose).
const SKIP_FEATURES: &[&str] = &[
    "Temporal",
    "ShadowRealm",
    "SharedArrayBuffer",
    "Atomics",
    "decorators",
    "source-phase-imports",
    "import-defer",
    "explicit-resource-management",
    "import-assertions",
    "import-text",
    "import-bytes",
    "cross-realm",
    "Intl.DurationFormat",
    "Intl.Locale-info",
    "Intl.Era-monthcode",
    "IsHTMLDDA",
    "tail-call-optimization",
    "FinalizationRegistry.prototype.cleanupSome",
    "host-gc-required",
    "Array.fromAsync",
    "iterator-sequencing",
    "iterator-includes",
    "Iterator.prototype.join",
    "iterator-chunking",
    "joint-iteration",
    "uint8array-base64",
    "Math.sumPrecise",
    "Float16Array",
    "Error.isError",
    "RegExp.escape",
    "upsert",
    "immutable-arraybuffer",
    "canonical-tz",
    "json-parse-with-source",
    "promise-try",
    "nonextensible-applies-to-private",
    "regexp-modifiers",
    "regexp-duplicate-named-groups",
    "Atomics.pause",
];

struct Meta {
    includes: Vec<String>,
    flags: String,
    negative: Option<(String, String)>,
    features: String,
}

fn meta(src: &str) -> Meta {
    let m = src.split("/*---").nth(1).and_then(|m| m.split("---*/").next()).unwrap_or("");
    let mut includes = Vec::new();
    let mut flags = String::new();
    let mut features = String::new();
    let mut negative = None;
    let lines: Vec<&str> = m.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        let l = lines[i].trim();
        if let Some(rest) = l.strip_prefix("includes:") {
            let rest = rest.trim();
            if rest.starts_with('[') {
                includes.extend(rest.trim_matches(|c| c == '[' || c == ']').split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()));
            } else {
                while i + 1 < lines.len() && lines[i + 1].trim().starts_with('-') {
                    i += 1;
                    includes.push(lines[i].trim().trim_start_matches('-').trim().to_string());
                }
            }
        } else if let Some(rest) = l.strip_prefix("flags:") {
            flags = rest.to_string();
        } else if let Some(rest) = l.strip_prefix("features:") {
            features = rest.to_string();
            if !rest.contains('[') {
                while i + 1 < lines.len() && lines[i + 1].trim().starts_with('-') {
                    i += 1;
                    features.push(' ');
                    features.push_str(lines[i].trim().trim_start_matches('-').trim());
                }
            }
        } else if l.starts_with("negative:") {
            let mut phase = String::new();
            let mut ty = String::new();
            while i + 1 < lines.len() && (lines[i + 1].starts_with(' ') || lines[i + 1].starts_with('\t')) {
                i += 1;
                let t = lines[i].trim();
                if let Some(p) = t.strip_prefix("phase:") {
                    phase = p.trim().to_string();
                } else if let Some(p) = t.strip_prefix("type:") {
                    ty = p.trim().to_string();
                }
            }
            negative = Some((phase, ty));
        }
        i += 1;
    }
    Meta { includes, flags, negative, features }
}

fn print(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> Result<Value, Value> {
    let s = match args.first() {
        Some(v) => vm.to_string(v).map(|s| s.to_rust()).unwrap_or_default(),
        None => String::new(),
    };
    let key = vm.atom("__printed__");
    let g = vm.global;
    let prev = match vm.get(g, Key::Str(key), &Value::Obj(g)) {
        Ok(Value::Str(p)) => p.to_rust(),
        _ => String::new(),
    };
    vm.put(g, "__printed__", Value::str(&format!("{prev}{s}\n")), js::object::ALL);
    Ok(Value::Undefined)
}

fn eval_script(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> Result<Value, Value> {
    let src = match args.first() {
        Some(v) => vm.to_string(v)?.to_rust(),
        None => String::new(),
    };
    vm.run_script(&src, "evalScript")
}

fn detach(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> Result<Value, Value> {
    if let Some(Value::Obj(g)) = args.first()
        && let Class::ArrayBuffer(data, _) = &mut vm.heap.get_mut(*g).class
    {
        *data = None;
        return Ok(Value::Null);
    }
    Err(vm.type_error("not an ArrayBuffer"))
}

fn gc(vm: &mut Vm, _this: &Value, _args: &[Value], _f: Gc) -> Result<Value, Value> {
    vm.collect();
    Ok(Value::Undefined)
}

fn error_name(vm: &mut Vm, e: &Value) -> String {
    let Value::Obj(g) = e else { return format!("(threw {})", vm.describe(e)) };
    let ctor = vm.get(*g, Key::Str(js::names::CONSTRUCTOR), e).unwrap_or(Value::Undefined);
    let name = match ctor {
        Value::Obj(c) => vm.get(c, Key::Str(js::names::NAME), &Value::Obj(c)).unwrap_or(Value::Undefined),
        _ => Value::Undefined,
    };
    match name {
        Value::Str(s) => s.to_rust(),
        _ => String::from("?"),
    }
}

fn message(vm: &mut Vm, e: &Value) -> String {
    let Value::Obj(g) = e else { return vm.describe(e) };
    let m = vm.get(*g, Key::Str(js::names::MESSAGE), e).unwrap_or(Value::Undefined);
    match m {
        Value::Str(s) => s.to_rust(),
        _ => String::new(),
    }
}

/// A module's text, from its file (its address is its path).
fn read_module(_vm: &mut Vm, key: &str) -> Result<String, String> {
    let url = js::module::split_key(key).0;
    std::fs::read_to_string(url).map_err(|e| format!("Couldn't read {url}: {e}"))
}

/// Runs one test in one mode: None if it passed, else why not.
fn run_one(root: &Path, path: &Path, src: &str, m: &Meta, strict: bool, harness: &mut BTreeMap<String, String>) -> Option<String> {
    let module = m.flags.contains("module");
    let mut text = String::new();
    if strict && !module {
        text.push_str("\"use strict\";\n");
    }
    let raw = m.flags.contains("raw");
    if !raw {
        let mut incs = vec![String::from("assert.js"), String::from("sta.js")];
        if m.flags.contains("async") {
            incs.push(String::from("doneprintHandle.js"));
        }
        incs.extend(m.includes.iter().cloned());
        for inc in incs {
            let h = harness.entry(inc.clone()).or_insert_with(|| std::fs::read_to_string(root.join("harness").join(&inc)).unwrap_or_default());
            text.push_str(h);
            text.push('\n');
        }
    }
    if !module {
        text.push_str(src);
    }
    let path = std::fs::canonicalize(path).unwrap_or(path.to_path_buf()).to_string_lossy().to_string();
    let mut vm = Vm::new();
    vm.modules.script_base = path.clone();
    vm.modules.fetch_sync = Some(read_module);
    vm.now = || std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64() * 1000.0).unwrap_or(0.0);
    vm.step_limit = 20_000_000;
    vm.max_nesting = 120;
    let g = vm.global;
    vm.method(g, "print", 1, print);
    let d = vm.new_object();
    vm.put(d, "global", Value::Obj(g), js::object::HIDDEN);
    vm.method(d, "evalScript", 1, eval_script);
    vm.method(d, "detachArrayBuffer", 1, detach);
    vm.method(d, "gc", 0, gc);
    vm.put(g, "$262", Value::Obj(d), js::object::HIDDEN);
    let negative = m.negative.clone();
    // A parse-phase negative test: the parser must refuse it.
    if let Some((phase, _)) = &negative
        && (phase == "parse" || phase == "early")
        && module
    {
        let mut atoms = js::string::Atoms::new();
        if let Ok(script) = js::parser::parse_module(src, &mut atoms)
            && js::compiler::compile_module(&script, &mut atoms, std::rc::Rc::from(src)).is_ok()
        {
            return Some(String::from("parsed, but must not"));
        }
        return None;
    }
    if let Some((phase, _)) = &negative
        && (phase == "parse" || phase == "early")
    {
        let mut atoms = js::string::Atoms::new();
        let parsed = js::parser::parse(&text, &mut atoms, false, false);
        if let Ok(script) = parsed {
            // (Some early errors are the compiler's.)
            if js::compiler::compile_script(&script, &mut atoms, std::rc::Rc::from(text.as_str()), false).is_ok() {
                return Some(String::from("parsed, but must not"));
            }
        }
        return None;
    }
    let r = if module {
        // The harness as a script, then the test as a module (and those it imports, from their files).
        match vm.run_script(&text, "harness") {
            Err(e) => Err(e),
            Ok(_) => run_module(&mut vm, &path, src),
        }
    } else {
        vm.run_script(&text, "test")
    };
    vm.run_jobs();
    let r = match (r, module) {
        (Ok(Value::Obj(p)), true) => match &vm.heap.get(p).class {
            Class::Promise(d) => match &d.state {
                js::object::PromiseState::Rejected(e) => Err(e.clone()),
                js::object::PromiseState::Fulfilled(_) => Ok(Value::Undefined),
                js::object::PromiseState::Pending => {
                    if m.flags.contains("async") {
                        Ok(Value::Undefined)
                    } else {
                        return Some(String::from("the module's evaluation never finished"));
                    }
                }
            },
            _ => Ok(Value::Undefined),
        },
        (r, _) => r,
    };
    match (r, &negative) {
        (Ok(_), None) => {
            if m.flags.contains("async") {
                let key = vm.atom("__printed__");
                let printed = match vm.get(g, Key::Str(key), &Value::Obj(g)) {
                    Ok(Value::Str(p)) => p.to_rust(),
                    _ => String::new(),
                };
                if printed.contains("Test262:AsyncTestComplete") {
                    return None;
                }
                return Some(format!("async: {}", printed.trim()));
            }
            None
        }
        (Ok(_), Some((_, ty))) => Some(format!("ran, but must throw {ty}")),
        (Err(e), Some((_, ty))) => {
            let n = error_name(&mut vm, &e);
            if &n == ty { None } else { Some(format!("threw {n} ({}), not {ty}", message(&mut vm, &e))) }
        }
        (Err(e), None) => {
            let n = error_name(&mut vm, &e);
            Some(format!("{n}: {}", message(&mut vm, &e)))
        }
    }
}

/// A module test: its graph loaded, linked and evaluated (the promise of its evaluation).
fn run_module(vm: &mut Vm, path: &str, src: &str) -> Result<Value, Value> {
    vm.module_add(path, path, src);
    let id = vm.module_load(path);
    let Some(pos) = vm.modules.ready.iter().position(|(i, _)| *i == id) else { return Err(Value::str("the module's graph didn't load")) };
    let (_, r) = vm.modules.ready.remove(pos);
    let m = r?;
    vm.module_link(m)?;
    Ok(Value::Obj(vm.module_evaluate(m)))
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let root = PathBuf::from(&args[1]);
    let sub = args.get(2).filter(|a| !a.starts_with("--")).cloned().unwrap_or_else(|| String::from("test/built-ins"));
    let show: usize = args.iter().position(|a| a == "--failures").and_then(|i| args.get(i + 1)).and_then(|v| v.parse().ok()).unwrap_or(15);
    let both = args.iter().any(|a| a == "--strict");
    let list = args.iter().position(|a| a == "--list").and_then(|i| args.get(i + 1)).cloned();
    let mut files = Vec::new();
    walk(&root.join(&sub), &mut files);
    let (mut pass, mut fail, mut skipped) = (0usize, 0usize, 0usize);
    let mut by_dir: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    let mut failures: Vec<String> = Vec::new();
    let mut harness: BTreeMap<String, String> = BTreeMap::new();
    let started = std::time::Instant::now();
    for f in &files {
        let Ok(src) = std::fs::read_to_string(f) else { continue };
        let m = meta(&src);
        if SKIP_FEATURES.iter().any(|x| m.features.split(|c: char| c == ',' || c == '[' || c == ']' || c.is_whitespace()).any(|w| w == *x)) {
            skipped += 1;
            continue;
        }
        let modes: Vec<bool> = if m.flags.contains("onlyStrict") {
            vec![true]
        } else if m.flags.contains("noStrict") || m.flags.contains("raw") || !both {
            vec![false]
        } else {
            vec![false, true]
        };
        let rel = f.strip_prefix(&root).unwrap_or(f).to_string_lossy().to_string();
        let mut why = None;
        for strict in modes {
            // A thread with a deep stack, and a panic caught as a failure.
            let (tx, rx) = mpsc::channel();
            let (root2, src2, mut h2, f2) = (root.clone(), src.clone(), harness.clone(), f.clone());
            let m2 = Meta { includes: m.includes.clone(), flags: m.flags.clone(), negative: m.negative.clone(), features: m.features.clone() };
            let t = std::thread::Builder::new().stack_size(256 << 20).spawn(move || {
                let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run_one(&root2, &f2, &src2, &m2, strict, &mut h2)));
                let _ = tx.send((r.unwrap_or_else(|p| Some(format!("PANIC {}", p.downcast_ref::<String>().cloned().or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string())).unwrap_or_default()))), h2));
            });
            let r = match t {
                Ok(h) => {
                    let r = rx.recv_timeout(std::time::Duration::from_secs(20));
                    let _ = h.join();
                    match r {
                        Ok((r, h2)) => {
                            harness = h2;
                            r
                        }
                        Err(_) => Some(String::from("timed out")),
                    }
                }
                Err(_) => Some(String::from("couldn't start")),
            };
            if let Some(w) = r {
                why = Some(if strict { format!("(strict) {w}") } else { w });
                break;
            }
        }
        let dir = rel.rsplit_once('/').map(|x| x.0.to_string()).unwrap_or_default();
        let key = dir.split('/').take(3).collect::<Vec<_>>().join("/");
        let e = by_dir.entry(key).or_default();
        match why {
            None => {
                pass += 1;
                e.0 += 1;
            }
            Some(w) => {
                fail += 1;
                e.1 += 1;
                failures.push(format!("{rel}: {w}"));
            }
        }
    }
    for (d, (p, f)) in &by_dir {
        println!("{:6} {:6}  {:5.1}%  {d}", p, f, 100.0 * *p as f64 / (*p + *f).max(1) as f64);
    }
    for f in failures.iter().take(show) {
        println!("FAIL {f}");
    }
    if let Some(l) = list {
        let _ = std::fs::write(l, failures.join("\n"));
    }
    println!("{pass} passed, {fail} failed, {skipped} skipped ({:.1}%) in {:.1} s", 100.0 * pass as f64 / (pass + fail).max(1) as f64, started.elapsed().as_secs_f64());
}
