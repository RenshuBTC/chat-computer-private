//! Damages real firmware tables at random and runs them, to check the interpreter never panics (a panic in the kernel
//! stops the PC): every damaged table must load, initialize and answer with values or errors.
//!
//!     cargo run --release --example fuzz -- [--rounds 200] [--seed 1] DSDT [SSDT...]
//!
//! Each round damages one of the tables (bytes changed, a run of bytes copied over another, or the end cut off), loads
//! them all, initializes, and evaluates every device's standard objects, every GPE method and the embedded
//! controller's query methods. A table that made it panic is saved as fuzz-crash-SEED-ROUND.aml in the current
//! folder, and the panic's message printed (the workspace builds with panic = "abort", so the first panic ends it).

use aml::{Aml, Host, PciAddress, Space, Value};
use std::collections::HashMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Mutex;

/// Where the last panic happened, and what it said; the damaged table of the round going on, and its file's name.
static LAST: Mutex<String> = Mutex::new(String::new());
static ROUND: Mutex<Option<(String, Vec<u8>)>> = Mutex::new(None);

#[derive(Default)]
struct Simulated {
    bytes: HashMap<(u8, u64), u8>,
    time: u64,
}

impl Host for Simulated {
    fn read(&mut self, space: Space, address: u64, bits: u8) -> Option<u64> {
        let s = space.number();
        Some((0..bits as u64 / 8).rev().fold(0u64, |n, i| n << 8 | *self.bytes.get(&(s, address.wrapping_add(i))).unwrap_or(&0) as u64))
    }
    fn write(&mut self, space: Space, address: u64, bits: u8, value: u64) -> bool {
        let s = space.number();
        for i in 0..bits as u64 / 8 {
            self.bytes.insert((s, address.wrapping_add(i)), (value >> (8 * i)) as u8);
        }
        true
    }
    fn read_pci(&mut self, at: PciAddress, offset: u16, bits: u8) -> Option<u64> {
        let base = (at.bus as u64) << 20 | (at.device as u64) << 15 | (at.function as u64) << 12;
        self.read(Space::Pci, base + offset as u64, bits)
    }
    fn write_pci(&mut self, at: PciAddress, offset: u16, bits: u8, value: u64) -> bool {
        let base = (at.bus as u64) << 20 | (at.device as u64) << 15 | (at.function as u64) << 12;
        self.write(Space::Pci, base + offset as u64, bits, value)
    }
    fn sleep(&mut self, ms: u64) {
        self.time = self.time.wrapping_add(ms.wrapping_mul(10_000));
    }
    fn stall(&mut self, us: u64) {
        self.time = self.time.wrapping_add(us.wrapping_mul(10));
    }
    fn timer(&mut self) -> u64 {
        self.time = self.time.wrapping_add(1);
        self.time
    }
    fn notify(&mut self, _path: &str, _value: u64) {}
    fn debug(&mut self, _text: &str) {}
}

/// A small random number generator (xorshift), so a seed gives the same rounds again.
struct Random(u64);

impl Random {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        if n == 0 { 0 } else { (self.next() % n as u64) as usize }
    }
}

/// Damages a table's code (its header kept, its length fixed up).
fn damage(table: &[u8], r: &mut Random) -> Vec<u8> {
    let mut t = table.to_vec();
    let body = t.len().saturating_sub(36).max(1);
    match r.below(4) {
        0 => {
            for _ in 0..1 + r.below(8) {
                let at = 36 + r.below(body);
                if at < t.len() {
                    t[at] = r.next() as u8;
                }
            }
        }
        1 => {
            // A byte that matters often: a length, an opcode, a name's first letter.
            let at = 36 + r.below(body);
            if at < t.len() {
                t[at] = [0x00, 0x01, 0x08, 0x10, 0x12, 0x14, 0x5B, 0x5C, 0x5E, 0x2E, 0x2F, 0xFF, 0x80, 0x40, 0xC0][r.below(15)];
            }
        }
        2 => {
            let from = 36 + r.below(body);
            let to = 36 + r.below(body);
            let n = 1 + r.below(64);
            for i in 0..n {
                if from + i < t.len() && to + i < t.len() {
                    t[to + i] = t[from + i];
                }
            }
        }
        _ => {
            let keep = 36 + r.below(body);
            t.truncate(keep.min(t.len()));
        }
    }
    let len = t.len() as u32;
    if t.len() >= 8 {
        t[4..8].copy_from_slice(&len.to_le_bytes());
    }
    t
}

fn run(tables: &[Vec<u8>]) -> usize {
    let mut host = Simulated::default();
    let mut aml = Aml::new();
    // (Smaller budgets than the kernel's, to get through more rounds: loops still end as they would.)
    aml.limits(1_000_000, 200_000);
    for t in tables {
        let _ = aml.load(t, &mut host);
    }
    for ec in aml.present_devices("PNP0C09", &mut host) {
        aml.connect_under(ec, Space::EmbeddedControl, &mut host);
    }
    aml.connect(Space::Cmos, &mut host);
    aml.initialize(&[Space::Pci], &mut host);
    let mut evaluated = 0;
    let mut queue = vec![0usize];
    let mut devices = Vec::new();
    while let Some(n) = queue.pop() {
        for c in aml.children(n) {
            if aml.is_device(c) {
                devices.push(c);
            }
            queue.push(c);
        }
        if devices.len() > 5000 {
            break;
        }
    }
    for d in devices {
        for c in aml.children(d) {
            let name = aml.name(c);
            if ["_HID", "_CID", "_UID", "_ADR", "_STA", "_CRS", "_PRS", "_PRT", "_BIF", "_BIX", "_BST", "_PSR", "_LID", "_GPE", "_PRW", "_DSM"]
                .contains(&name.as_str())
            {
                let args = if name == "_DSM" {
                    vec![Value::buffer(vec![0; 16]), Value::Integer(1), Value::Integer(0), Value::package(Vec::new())]
                } else {
                    Vec::new()
                };
                let _ = aml.evaluate_node(c, args, &mut host);
                evaluated += 1;
            }
        }
    }
    for (path, args) in [("\\_PTS", vec![Value::Integer(5)]), ("\\_S5", vec![]), ("\\_WAK", vec![Value::Integer(5)])] {
        let _ = aml.evaluate(path, args, &mut host);
    }
    // Every GPE and EC query method too: the kernel runs them when the firmware signals.
    let mut events = Vec::new();
    if let Some(gpe) = aml.find("\\_GPE") {
        events.extend(aml.children(gpe));
    }
    for ec in aml.present_devices("PNP0C09", &mut host) {
        events.extend(aml.children(ec).into_iter().filter(|&c| aml.name(c).starts_with("_Q")));
    }
    for e in events {
        if aml.is_method(e) {
            let _ = aml.evaluate_node(e, Vec::new(), &mut host);
            evaluated += 1;
        }
    }
    evaluated
}

fn main() {
    let mut args = std::env::args().skip(1);
    let (mut rounds, mut seed, mut paths) = (200usize, 1u64, Vec::new());
    while let Some(a) = args.next() {
        match a.as_str() {
            "--rounds" => rounds = args.next().and_then(|n| n.parse().ok()).expect("a number after --rounds"),
            "--seed" => seed = args.next().and_then(|n| n.parse().ok()).expect("a number after --seed"),
            _ => paths.push(a),
        }
    }
    let tables: Vec<Vec<u8>> = paths.iter().map(|p| std::fs::read(p).expect("a table")).collect();
    std::panic::set_hook(Box::new(|info| {
        *LAST.lock().unwrap() = info.to_string();
        if let Ok(round) = ROUND.try_lock()
            && let Some((name, table)) = round.as_ref()
        {
            let _ = std::fs::write(name, table);
            println!("panic: {} (the damaged table: {name})", info.to_string().replace('\n', " "));
        }
    }));
    let started = std::time::Instant::now();
    println!("undamaged: {} objects evaluated, {:.1} s", run(&tables), started.elapsed().as_secs_f64());
    let mut r = Random(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    let mut crashes = 0;
    let mut evaluated = 0;
    for round in 0..rounds {
        let which = r.below(tables.len());
        let mut set = tables.clone();
        set[which] = damage(&tables[which], &mut r);
        *ROUND.lock().unwrap() = Some((format!("fuzz-crash-{seed}-{round}.aml"), set[which].clone()));
        let result = catch_unwind(AssertUnwindSafe(|| run(&set)));
        if let Ok(n) = result {
            evaluated += n;
        } else {
            let message = LAST.lock().unwrap().replace('\n', " ");
            let name = format!("fuzz-crash-{seed}-{round}.aml");
            std::fs::write(&name, &set[which]).unwrap();
            println!("round {round}: panic: {message} (table {} damaged: {name})", paths[which]);
            crashes += 1;
        }
    }
    println!("{rounds} rounds, {crashes} panics, {evaluated} objects evaluated, {:.0} s", started.elapsed().as_secs_f64());
}
