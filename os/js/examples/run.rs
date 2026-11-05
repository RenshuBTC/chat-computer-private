//! Runs a script with the engine: `cargo run -p js --example run -- FILE` (or `-e 'code'`), printing what it
//! prints (`print(...)`, `console.log(...)`) and its result or the exception it threw.
use js::object::{Class, Key};
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

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let src = if args.get(1).map(|s| s.as_str()) == Some("-e") { args[2].clone() } else { std::fs::read_to_string(&args[1]).expect("a script") };
    let mut vm = Vm::new();
    vm.now = || std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64() * 1000.0).unwrap_or(0.0);
    let g = vm.global;
    vm.method(g, "print", 1, print);
    let console = vm.new_object();
    vm.method(console, "log", 1, print);
    vm.put(g, "console", Value::Obj(console), js::object::HIDDEN);
    let t = std::time::Instant::now();
    let r = vm.run_script(&src, "script");
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
