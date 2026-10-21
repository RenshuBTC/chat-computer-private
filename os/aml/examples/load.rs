//! Loads a PC's ACPI tables (the DSDT first, then SSDTs) into the interpreter, initializes the namespace, and
//! evaluates objects, printing each result as tests compare them: for checking the interpreter against ACPICA's
//! acpiexec on real firmware (tests/corpus.py).
//!
//!     cargo run --example load -- [--devices] [--eval PATH]... TABLE...
//!
//! Operation regions read as zeros until written, as acpiexec simulates them (PCI configuration space's by offset alone,
//! as acpiexec keeps it).

use aml::{Aml, Host, PciAddress, Space, Value};
use std::collections::HashMap;

#[derive(Default)]
struct Simulated {
    bytes: HashMap<(u8, u64), u8>,
    time: u64,
}

impl Host for Simulated {
    fn read(&mut self, space: Space, address: u64, bits: u8) -> Option<u64> {
        let s = space.number();
        Some((0..bits as u64 / 8).rev().fold(0u64, |n, i| n << 8 | *self.bytes.get(&(s, address + i)).unwrap_or(&0) as u64))
    }
    fn write(&mut self, space: Space, address: u64, bits: u8, value: u64) -> bool {
        let s = space.number();
        for i in 0..bits as u64 / 8 {
            self.bytes.insert((s, address + i), (value >> (8 * i)) as u8);
        }
        true
    }
    // (acpiexec keeps a region's bytes by its offset, whichever device it's of: so here too.)
    fn read_pci(&mut self, _at: PciAddress, offset: u16, bits: u8) -> Option<u64> {
        self.read(Space::Pci, offset as u64, bits)
    }
    fn write_pci(&mut self, _at: PciAddress, offset: u16, bits: u8, value: u64) -> bool {
        self.write(Space::Pci, offset as u64, bits, value)
    }
    fn sleep(&mut self, ms: u64) {
        self.time += ms * 10_000;
    }
    fn stall(&mut self, us: u64) {
        self.time += us * 10;
    }
    fn timer(&mut self) -> u64 {
        self.time += 1;
        self.time
    }
    fn notify(&mut self, _path: &str, _value: u64) {}
    fn debug(&mut self, _text: &str) {}
}

fn canon(aml: &Aml, v: &Value) -> String {
    match v {
        Value::Integer(n) => format!("I:{n:X}"),
        Value::String(s) => format!("\"{s}\""),
        Value::Buffer(b) => format!("B[{}]", b.borrow().iter().map(|x| format!("{x:02X}")).collect::<Vec<String>>().join(" ")),
        Value::Package(p) => format!("P[{}]", p.borrow().iter().map(|i| canon(aml, i)).collect::<Vec<String>>().join(", ")),
        Value::Uninitialized => String::from("U"),
        Value::Reference(aml::Reference::Node(n)) => format!("R:{}", aml.path(*n)),
        Value::Reference(_) => String::from("R"),
        Value::Object(n) => format!("O:{}", aml.path(*n)),
    }
}

fn main() {
    let mut args = std::env::args().skip(1).peekable();
    let mut evals = Vec::new();
    let mut tables = Vec::new();
    let mut devices = false;
    let mut standard = false;
    let mut trace = false;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--eval" => evals.push(args.next().expect("a path after --eval")),
            "--devices" => devices = true,
            "--standard" => standard = true,
            "--trace" => trace = true,
            _ => tables.push(a),
        }
    }
    let mut host = Simulated::default();
    let mut aml = Aml::new();
    if trace {
        aml.evaluated = Some(Vec::new());
    }
    for t in &tables {
        let bytes = std::fs::read(t).expect("a table");
        if let Err(e) = aml.load(&bytes, &mut host) {
            println!("load {t}: {e:?}");
        }
    }
    // As acpiexec sets up: each present embedded controller's space connected under it (its _REG), then CMOS's
    // throughout (acpiexec connects the serial buses', GPIO's and the like's too, but simulates them in its own way:
    // differences in what uses them are expected, so they're listed), then the _INIs.
    for ec in aml.present_devices("PNP0C09", &mut host) {
        aml.connect_under(ec, Space::EmbeddedControl, &mut host);
    }
    // (acpiexec also looks for the first PCI Express root bridge, and for generic event devices, whose _CRS it reads:
    // what those _HID, _CID, _STA and _CRS methods do is done here too.)
    let _ = aml.find_devices("PNP0A08", 1, &mut host);
    for ged in aml.present_devices("ACPI0013", &mut host) {
        let _ = aml.evaluate_child(ged, "_CRS", Vec::new(), &mut host);
    }
    aml.connect(Space::Cmos, &mut host);
    let present = aml.initialize(&[], &mut host);
    // acpiexec's own tests then read the sleep states' values (\_S0 to \_S5), and ask every device its _STA.
    for state in 0..6 {
        let _ = aml.evaluate(&format!("\\_S{state}_"), Vec::new(), &mut host);
    }
    let _ = aml.find_devices("", usize::MAX, &mut host);
    for e in aml.evaluated.take().unwrap_or_default() {
        println!("evaluated {e}");
    }
    println!("objects {}", aml.objects());
    println!("present {present}");
    let unlike: Vec<String> = aml
        .region_spaces()
        .into_iter()
        .filter(|s| !matches!(s, Space::Memory | Space::Io | Space::Pci | Space::EmbeddedControl | Space::Cmos | Space::Table))
        .map(|s| format!("{}", s.number()))
        .collect();
    println!("unlike {}", unlike.join(" "));
    for p in &aml.problems {
        println!("problem {p}");
    }
    if devices || standard {
        let mut all = Vec::new();
        let mut queue = vec![0usize];
        while let Some(n) = queue.pop() {
            for c in aml.children(n) {
                if aml.is_device(c) {
                    all.push(aml.path(c));
                }
                queue.push(c);
            }
        }
        all.sort();
        for d in &all {
            if devices {
                println!("device {d}");
            }
            if standard {
                // Each device's standard objects that it has, to evaluate.
                let node = aml.find(d).unwrap();
                let names: Vec<String> = aml.children(node).iter().map(|&c| aml.name(c)).collect();
                for n in ["_HID", "_CID", "_UID", "_ADR", "_STA", "_BBN", "_SEG", "_CRS", "_PRS", "_PRT", "_BIF", "_BIX", "_BST", "_PSR", "_LID", "_GPE"] {
                    if names.iter().any(|x| x == n) {
                        evals.push(format!("{d}.{n}"));
                    }
                }
            }
        }
    }
    for path in &evals {
        match aml.evaluate(path, Vec::new(), &mut host) {
            Ok(v) => println!("= {path} {}", canon(&aml, &v)),
            Err(e) => println!("= {path} error {e:?}"),
        }
    }
}
