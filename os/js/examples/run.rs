//! Runs a script with the engine: `cargo run -p js --example run -- FILE` (or `-e 'code'`), printing what it
//! prints (`print(...)`, `console.log(...)`) and its result or the exception it threw. With `-m FILE` it runs the
//! file as a module (the modules it imports read from their files).
use js::object::{Class, Key, PromiseState};
use js::value::{Gc, Value};
use js::vm::Vm;

fn show(vm: &mut Vm, v: &Value) -> String {
    if let Value::Obj(g) = v
        && matches!(vm.heap.get(*g).class, Class::Error)
    {
        let stack = vm.get(*g, Key::Str(js::names::STACK), v).unwrap_or(Value::Undefined);
        if let Value::Str(s) = stack {
            return s.to_rust();
        }
    }
    match vm.to_string(v) {
        Ok(s) => s.to_rust(),
        Err(_) => String::from("<value>"),
    }
}

fn print(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> Result<Value, Value> {
    let parts: Vec<String> = args.iter().map(|a| show(vm, a)).collect();
    println!("{}", parts.join(" "));
    Ok(Value::Undefined)
}

fn read_module(_vm: &mut Vm, key: &str) -> Result<String, String> {
    let url = js::module::split_key(key).0;
    std::fs::read_to_string(url).map_err(|e| format!("Couldn't read {url}: {e}"))
}

/// A module run: its graph loaded, linked and evaluated (the promise of its evaluation, once the jobs have run).
fn run_module(vm: &mut Vm, path: &str) -> Result<Value, Value> {
    let path = std::fs::canonicalize(path).map(|p| p.to_string_lossy().to_string()).unwrap_or(path.to_string());
    vm.modules.script_base = path.clone();
    vm.modules.fetch_sync = Some(read_module);
    let id = vm.module_load(&path);
    let Some(pos) = vm.modules.ready.iter().position(|(i, _)| *i == id) else { return Err(Value::str("the module's graph didn't load")) };
    let (_, r) = vm.modules.ready.remove(pos);
    let m = r?;
    vm.module_link(m)?;
    let p = vm.module_evaluate(m);
    vm.run_jobs();
    match &vm.heap.get(p).class {
        Class::Promise(d) => match &d.state {
            PromiseState::Rejected(e) => Err(e.clone()),
            PromiseState::Fulfilled(v) => Ok(v.clone()),
            PromiseState::Pending => Ok(Value::str("(still waiting)")),
        },
        _ => Ok(Value::Undefined),
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut vm = Vm::new();
    vm.now = || std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64() * 1000.0).unwrap_or(0.0);
    let g = vm.global;
    vm.method(g, "print", 1, print);
    let console = vm.new_object();
    vm.method(console, "log", 1, print);
    vm.put(g, "console", Value::Obj(console), js::object::HIDDEN);
    let t = std::time::Instant::now();
    let r = match args.get(1).map(|s| s.as_str()) {
        Some("-m") => run_module(&mut vm, &args[2]),
        Some("-e") => vm.run_script(&args[2], "script"),
        _ => {
            let src = std::fs::read_to_string(&args[1]).expect("a script");
            vm.run_script(&src, "script")
        }
    };
    vm.run_jobs();
    match r {
        Ok(v) => {
            let s = show(&mut vm, &v);
            println!("=> {s}");
        }
        Err(e) => {
            let s = show(&mut vm, &e);
            println!("Uncaught {s}");
        }
    }
    eprintln!("({:.1} ms, {} objects)", t.elapsed().as_secs_f64() * 1000.0, vm.heap.count());
}
