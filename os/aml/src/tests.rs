//! Checked against ACPICA's own interpreter (acpiexec, from acpica-tools): each test table (tests/*.asl, compiled by
//! iasl with its optimizations off, so the code has the operators as written) is loaded into both, and its T methods
//! run in turn in both, in one session each: every result must be the same. Operation regions are simulated as
//! acpiexec simulates them: bytes that read as zeros until written. (Skipped, with a note, where iasl and acpiexec
//! aren't installed.)

extern crate std;

use super::*;
use std::collections::HashMap;
use std::format;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::string::{String, ToString};
use std::vec::Vec;

/// The machine, as acpiexec simulates it: each address space's bytes, zeros until written.
#[derive(Default)]
struct TestHost {
    bytes: HashMap<(u8, u64), u8>,
    notified: Vec<(String, u64)>,
    debug: Vec<String>,
    time: u64,
    /// How many times the global lock was taken and given back.
    locks: u32,
    unlocks: u32,
}

impl Host for TestHost {
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
    fn read_pci(&mut self, at: PciAddress, offset: u16, bits: u8) -> Option<u64> {
        let base = (at.bus as u64) << 20 | (at.device as u64) << 15 | (at.function as u64) << 12;
        self.read(Space::Pci, base + offset as u64, bits)
    }
    fn write_pci(&mut self, at: PciAddress, offset: u16, bits: u8, value: u64) -> bool {
        let base = (at.bus as u64) << 20 | (at.device as u64) << 15 | (at.function as u64) << 12;
        self.write(Space::Pci, base + offset as u64, bits, value)
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
    fn notify(&mut self, path: &str, value: u64) {
        self.notified.push((String::from(path), value));
    }
    fn debug(&mut self, text: &str) {
        self.debug.push(String::from(text));
    }
    fn acquire_global_lock(&mut self, _timeout_ms: u16) -> bool {
        self.locks += 1;
        true
    }
    fn release_global_lock(&mut self) {
        self.unlocks += 1;
    }
}

fn tools() -> bool {
    let ok = Command::new("iasl").arg("-v").output().is_ok() && Command::new("acpiexec").arg("-v").output().is_ok();
    if !ok {
        std::println!("iasl or acpiexec (acpica-tools) isn't installed: skipped");
    }
    ok
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("chatcomputer-aml-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A test table, compiled: its AML.
fn compile(name: &str, dir: &Path) -> Vec<u8> {
    let source = format!("{}/tests/{name}.asl", env!("CARGO_MANIFEST_DIR"));
    let out = dir.join(name);
    let r = Command::new("iasl").args(["-oa", "-p"]).arg(&out).arg(&source).output().unwrap();
    assert!(r.status.success(), "iasl {source}: {}", String::from_utf8_lossy(&r.stdout));
    std::fs::read(out.with_extension("aml")).unwrap()
}

/// A value as both sides are compared: I:hex, "string", B[hex], P[...], or error.
fn canon(aml: &Aml, v: &Value) -> String {
    match v {
        Value::Integer(n) => format!("I:{n:X}"),
        Value::String(s) => format!("\"{s}\""),
        Value::Buffer(b) => format!("B[{}]", b.borrow().iter().map(|x| format!("{x:02X}")).collect::<Vec<String>>().join(" ")),
        Value::Package(p) => format!("P[{}]", p.borrow().iter().map(|i| canon(aml, i)).collect::<Vec<String>>().join(", ")),
        Value::Uninitialized => String::from("U"),
        Value::Reference(r) => match aml.deref(r) {
            Ok(v) => canon(aml, &v),
            Err(_) => String::from("R"),
        },
        Value::Object(n) => format!("O:{}", aml.path(*n)),
    }
}

/// acpiexec's printed result (from its "[Type] ..." line on), in the same form; how many lines it took.
fn parse(lines: &[&str]) -> (String, usize) {
    let line = lines[0].trim_start();
    if let Some(rest) = line.strip_prefix("[Integer] = ") {
        let n = u64::from_str_radix(rest.split_whitespace().next().unwrap_or("0"), 16).unwrap_or(0);
        return (format!("I:{n:X}"), 1);
    }
    if line.starts_with("[String]") {
        let a = line.find('"').unwrap_or(0);
        let b = line.rfind('"').unwrap_or(a);
        return (format!("\"{}\"", &line[a + 1..b.max(a + 1)]), 1);
    }
    if line.starts_with("[Buffer] Length ") {
        let count = usize::from_str_radix(line["[Buffer] Length ".len()..].split_whitespace().next().unwrap_or("0"), 16).unwrap_or(0);
        let mut bytes = Vec::new();
        let mut used = 0;
        for l in lines {
            if bytes.len() >= count && used > 0 {
                break;
            }
            used += 1;
            let Some(colon) = l.find(": ") else { continue };
            let hex = &l[colon + 2..];
            let hex = hex.split("//").next().unwrap_or("");
            bytes.extend(hex.split_whitespace().filter(|h| h.len() == 2).map(String::from));
            if count == 0 {
                break;
            }
        }
        bytes.truncate(count);
        return (format!("B[{}]", bytes.join(" ")), used.max(1));
    }
    if let Some(rest) = line.strip_prefix("[Package] Contains ") {
        let count: usize = rest.split_whitespace().next().unwrap_or("0").parse().unwrap_or(0);
        let mut at = 1;
        let mut items = Vec::new();
        for _ in 0..count {
            if at >= lines.len() {
                break;
            }
            let (item, used) = parse(&lines[at..]);
            items.push(item);
            at += used;
        }
        return (format!("P[{}]", items.join(", ")), at);
    }
    if line.starts_with("[Null Object]") || line.contains("Uninitialized") {
        return (String::from("U"), 1);
    }
    (format!("?{line}"), 1)
}

/// What acpiexec gave for each method evaluated, by name, in one session (its commands on its standard input: its
/// batch mode's command line is too short for them all).
fn acpiexec(aml: &Path, methods: &[String]) -> HashMap<String, String> {
    use std::io::Write;
    let mut commands: String = methods.iter().map(|m| format!("evaluate \\{m}\n")).collect();
    commands += "quit\n";
    let mut child = Command::new("acpiexec")
        .arg("-dr")
        .arg(aml)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(commands.as_bytes()).unwrap();
    let out = child.wait_with_output().unwrap();
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    let lines: Vec<&str> = text.lines().collect();
    let mut found = HashMap::new();
    for (i, l) in lines.iter().enumerate() {
        if let Some(rest) = l.strip_prefix("No object was returned from evaluation of \\") {
            found.insert(rest.trim().to_string(), String::from("U"));
        } else if let Some(rest) = l.strip_prefix("Evaluation of \\") {
            let name = rest.split_whitespace().next().unwrap_or("").to_string();
            if rest.contains("failed with status") {
                found.insert(name, String::from("error"));
            } else if rest.contains("returned object") {
                let (v, _) = parse(&lines[i + 1..]);
                found.insert(name, v);
            }
        }
    }
    found
}

/// Loads a test table into both, runs its T methods in both, and says each difference.
fn check(name: &str) -> Vec<String> {
    let dir = scratch(name);
    let code = compile(name, &dir);
    let mut host = TestHost::default();
    let mut aml = Aml::new();
    aml.load(&code, &mut host).unwrap();
    assert!(aml.problems.is_empty(), "{name}: {:?}", aml.problems);
    aml.initialize(&[], &mut host);
    let mut methods: Vec<String> = aml.children(0).into_iter().map(|n| aml.name(n)).filter(|n| n.starts_with('T') && n[1..].chars().all(|c| c.is_ascii_digit())).collect();
    methods.sort();
    let theirs = acpiexec(&dir.join(format!("{name}.aml")), &methods);
    let mut differences = Vec::new();
    for m in &methods {
        let ours = match aml.evaluate(&format!("\\{m}"), Vec::new(), &mut host) {
            Ok(v) => canon(&aml, &v),
            Err(_) => String::from("error"),
        };
        let want = theirs.get(m).cloned().unwrap_or_else(|| String::from("(nothing from acpiexec)"));
        if ours != want {
            differences.push(format!("{name} {m}: ours {ours}, acpiexec's {want}"));
        }
    }
    let _ = std::fs::remove_dir_all(dir);
    differences
}

#[test]
fn as_acpica_runs_it() {
    if !tools() {
        return;
    }
    let mut differences = Vec::new();
    for name in ["basics", "fields", "methods", "narrow"] {
        differences.extend(check(name));
    }
    assert!(differences.is_empty(), "{} differences:\n{}", differences.len(), differences.join("\n"));
}

#[test]
fn eisa_ids() {
    assert_eq!(eisa_id(0x0A0CD041), "PNP0C0A");
    assert_eq!(eisa_id(0x030AD041), "PNP0A03");
}

/// Loads one of the tests' tables, compiled, into a fresh namespace (None where iasl isn't installed).
fn loaded(name: &str, host: &mut TestHost) -> Option<Aml> {
    if Command::new("iasl").arg("-v").output().is_err() {
        std::println!("iasl (acpica-tools) isn't installed: skipped");
        return None;
    }
    let dir = scratch(name);
    let code = compile(name, &dir);
    let _ = std::fs::remove_dir_all(dir);
    let mut aml = Aml::new();
    aml.load(&code, host).unwrap();
    assert!(aml.problems.is_empty(), "{name}: {:?}", aml.problems);
    Some(aml)
}

#[test]
fn resources_as_iasl_writes_them() {
    let mut host = TestHost::default();
    let Some(mut aml) = loaded("resources", &mut host) else { return };
    let mut get = |path: &str| {
        let v = aml.evaluate(path, Vec::new(), &mut host).unwrap();
        resources(&v.bytes().unwrap())
    };
    assert_eq!(get("\\EC00"), vec![Resource::Io { min: 0x62, max: 0x62, length: 1 }, Resource::Io { min: 0x66, max: 0x66, length: 1 }]);
    assert_eq!(
        get("\\MIX0"),
        vec![
            Resource::Io { min: 0x60, max: 0x60, length: 1 },
            Resource::Interrupt { numbers: vec![1], edge: true, active_low: false, shared: false, wake: false },
            Resource::Interrupt { numbers: vec![9, 11], edge: false, active_low: true, shared: true, wake: false },
            Resource::Memory { min: 0xFED0_0000, max: 0xFED0_0000, length: 0x400, writable: true },
            Resource::Memory { min: 0xC_0000, max: 0xD_FFFF, length: 0x2_0000, writable: false },
            Resource::Interrupt { numbers: vec![0x10, 0x11], edge: true, active_low: false, shared: false, wake: false },
            Resource::Range { kind: RangeKind::Memory, min: 0xC000_0000, max: 0xFEBF_FFFF, translation: 0, length: 0x3EC0_0000 },
            Resource::Range { kind: RangeKind::Memory, min: 0x8_0000_0000, max: 0xF_FFFF_FFFF, translation: 0, length: 0x8_0000_0000 },
            Resource::Range { kind: RangeKind::BusNumbers, min: 0, max: 0xFF, translation: 0, length: 0x100 },
            Resource::Range { kind: RangeKind::Io, min: 0xD00, max: 0xFFFF, translation: 0, length: 0xF300 },
        ]
    );
    assert_eq!(
        get("\\TPD0"),
        vec![
            Resource::I2c { controller: String::from("\\_SB.PCI0.I2C1"), address: 0x2C, ten_bit: false, speed: 400_000 },
            Resource::Gpio { controller: String::from("\\_SB.GPIO"), pins: vec![9], interrupt: true, edge: false, active_low: true, wake: true },
            Resource::Gpio { controller: String::from("\\_SB.GPI0"), pins: vec![0x12, 0x13], interrupt: false, edge: false, active_low: false, wake: false },
        ]
    );
}

#[test]
fn global_lock_taken_once_and_given_back() {
    let mut host = TestHost::default();
    let Some(mut aml) = loaded("lock", &mut host) else { return };
    for (method, times) in [("\\T001", 1), ("\\T002", 2), ("\\T003", 0), ("\\T004", 1)] {
        let before = host.locks;
        aml.evaluate(method, Vec::new(), &mut host).unwrap();
        assert_eq!(host.locks - before, times, "{method}: taken");
        assert_eq!(host.locks, host.unlocks, "{method}: given back");
    }
}
