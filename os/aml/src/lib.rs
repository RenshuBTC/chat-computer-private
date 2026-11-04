//! ACPI Machine Language: the byte code a PC's firmware describes its devices and their power with (the ACPI
//! specification, 6.5, chapters 5, 19 and 20). The firmware's DSDT and SSDTs are loaded into a namespace of objects
//! (devices, methods, data, operation regions and their fields, mutexes...), and their methods run when asked: a
//! battery's _BST, a lid's _LID, the _Lxx, _Exx and _Qxx methods behind the firmware's events. Reads and writes of the
//! machine's address spaces (memory, I/O ports, PCI configuration, the embedded controller) go through the `Host`.
//!
//! The firmware is answered as Windows answers it (_OSI says yes to Windows' versions), since that's what PCs'
//! firmware is written and tested for. Integers are 64 bits, or 32 when the DSDT's revision is below 2. Every byte is
//! checked before it's used: a damaged table, or a method that loops forever, gives an error, never a crash or a hang.

#![no_std]

extern crate alloc;

mod code;
mod convert;
mod exec;
mod field;
mod resource;
#[cfg(test)]
mod tests;

use alloc::format;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::cell::RefCell;

pub use convert::eisa_id;
pub use resource::{RangeKind, Resource, resources};

/// The address spaces an operation region can be in.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Space {
    Memory,
    Io,
    Pci,
    EmbeddedControl,
    SmBus,
    Cmos,
    PciBar,
    Ipmi,
    Gpio,
    SerialBus,
    Pcc,
    /// A table's own bytes (DataTableRegion).
    Table,
    Other(u8),
}

impl Space {
    fn of(b: u8) -> Space {
        match b {
            0 => Space::Memory,
            1 => Space::Io,
            2 => Space::Pci,
            3 => Space::EmbeddedControl,
            4 => Space::SmBus,
            5 => Space::Cmos,
            6 => Space::PciBar,
            7 => Space::Ipmi,
            8 => Space::Gpio,
            9 => Space::SerialBus,
            10 => Space::Pcc,
            n => Space::Other(n),
        }
    }

    /// Its number, as _REG is told it.
    pub fn number(self) -> u8 {
        match self {
            Space::Memory => 0,
            Space::Io => 1,
            Space::Pci => 2,
            Space::EmbeddedControl => 3,
            Space::SmBus => 4,
            Space::Cmos => 5,
            Space::PciBar => 6,
            Space::Ipmi => 7,
            Space::Gpio => 8,
            Space::SerialBus => 9,
            Space::Pcc => 10,
            Space::Table => 0x7E,
            Space::Other(n) => n,
        }
    }
}

/// A PCI function's place: segment, bus, device, function.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PciAddress {
    pub segment: u16,
    pub bus: u8,
    pub device: u8,
    pub function: u8,
}

/// What the interpreter asks of the machine.
pub trait Host {
    /// Reads `bits` (8, 16, 32 or 64) at `address` in `space` (memory, I/O ports, the embedded controller...). None
    /// if it can't be read.
    fn read(&mut self, space: Space, address: u64, bits: u8) -> Option<u64>;
    /// Writes `bits` at `address` in `space`. False if it can't be written.
    fn write(&mut self, space: Space, address: u64, bits: u8, value: u64) -> bool;
    /// PCI configuration space: a function's register at `offset`.
    fn read_pci(&mut self, at: PciAddress, offset: u16, bits: u8) -> Option<u64>;
    fn write_pci(&mut self, at: PciAddress, offset: u16, bits: u8, value: u64) -> bool;
    /// Sleep (milliseconds: the processor may do other things) and Stall (microseconds: it waits).
    fn sleep(&mut self, ms: u64);
    fn stall(&mut self, us: u64);
    /// A count of 100-nanosecond units that only goes up (the Timer operator).
    fn timer(&mut self) -> u64;
    /// Notify(object, value): the object's path ("\_SB.PCI0.LPCB.BAT0") and the value.
    fn notify(&mut self, path: &str, value: u64);
    /// Store(x, Debug): what the firmware wrote to its debug object.
    fn debug(&mut self, text: &str);
    /// A table of the firmware's, by its signature and OEM IDs (LoadTable), whole, header included.
    fn table(&mut self, _signature: &[u8; 4], _oem: &[u8], _oem_table: &[u8]) -> Option<Vec<u8>> {
        None
    }
    /// The firmware's global lock (in its FACS table), which its own code (SMM) takes too, for what both use (the
    /// embedded controller, often): taken, within `timeout_ms` (0xFFFF: as long as it takes)? Released.
    fn acquire_global_lock(&mut self, _timeout_ms: u16) -> bool {
        true
    }
    fn release_global_lock(&mut self) {}
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// The byte code doesn't make sense at this offset of a table's code.
    Parse(usize),
    /// No object of this name.
    NotFound(String),
    /// An object defined twice.
    Exists(String),
    /// An operation on the wrong kind of object (what was wanted).
    Type(&'static str),
    /// An index past the end of a buffer, a package or a string, or a field past its region's end.
    Index,
    DivideByZero,
    /// An address space the host can't read or write.
    Region(Space),
    /// A method asked to stop the machine (Fatal): its type, code and argument.
    Fatal(u8, u32, u64),
    /// Methods called within methods too deeply, or expressions nested too deeply.
    TooDeep,
    /// Too many steps for one evaluation (a loop waiting for something that doesn't happen).
    TooLong,
    /// A table whose header doesn't make sense.
    Table,
    /// A method's local or argument read before anything was stored in it.
    Uninitialized,
    /// A method that returned nothing, whose value was wanted.
    NoValue,
}

/// A value: what a method returns, or what an object holds.
#[derive(Clone, Debug)]
pub enum Value {
    Uninitialized,
    Integer(u64),
    String(String),
    Buffer(Rc<RefCell<Vec<u8>>>),
    Package(Rc<RefCell<Vec<Value>>>),
    Reference(Reference),
    /// A namespace object that isn't data: a device, a method, a mutex, a region...
    Object(usize),
}

/// A reference to something: an object (RefOf), a method's local or argument, a package's element or a buffer's
/// byte (Index), or a name in a package that names nothing.
#[derive(Clone, Debug)]
pub enum Reference {
    Node(usize),
    Cell(Rc<RefCell<Value>>),
    Element(Rc<RefCell<Vec<Value>>>, usize),
    Byte(Rc<RefCell<Vec<u8>>>, usize),
    /// A name in a package a table made that names nothing (yet), and the scope it's looked for from: once every
    /// table is loaded it's the object's, or nothing (as ACPICA settles them).
    Unresolved(String, usize),
}

impl Value {
    pub fn buffer(bytes: Vec<u8>) -> Value {
        Value::Buffer(Rc::new(RefCell::new(bytes)))
    }

    pub fn package(items: Vec<Value>) -> Value {
        Value::Package(Rc::new(RefCell::new(items)))
    }

    /// The integer, if it is one.
    pub fn as_integer(&self) -> Option<u64> {
        match self {
            Value::Integer(n) => Some(*n),
            _ => None,
        }
    }

    /// A package's elements (copies), if it is one.
    pub fn elements(&self) -> Option<Vec<Value>> {
        match self {
            Value::Package(p) => Some(p.borrow().clone()),
            _ => None,
        }
    }

    /// A buffer's bytes (a copy), if it is one.
    pub fn bytes(&self) -> Option<Vec<u8>> {
        match self {
            Value::Buffer(b) => Some(b.borrow().clone()),
            _ => None,
        }
    }

    /// A string's text, if it is one.
    pub fn text(&self) -> Option<&str> {
        match self {
            Value::String(s) => Some(s),
            _ => None,
        }
    }
}

/// An object in the namespace.
#[derive(Clone)]
enum Object {
    /// A scope with nothing of its own: \, \_SB, \_GPE...
    Scope,
    /// Data (Name).
    Data(Value),
    Method(Rc<Method>),
    /// \_OSI, the question about the operating system answered here.
    Osi,
    Device,
    Processor,
    PowerResource,
    ThermalZone,
    Mutex,
    Event(u32),
    Region(Region),
    Field(Rc<field::Field>),
    /// Bits of a buffer: by CreateField (`any`, read as a buffer whatever its size, as ACPICA does), or by
    /// CreateByteField and the like.
    BufferField { buffer: Rc<RefCell<Vec<u8>>>, bit: usize, bits: usize, any: bool },
    Alias(usize),
    External,
}

/// A method: its code (part of a table's), and how many arguments it takes.
struct Method {
    code: Rc<[u8]>,
    start: usize,
    end: usize,
    args: u8,
}

#[derive(Clone)]
struct Region {
    space: Space,
    offset: u64,
    length: u64,
    /// A DataTableRegion's bytes.
    table: Option<Rc<[u8]>>,
    /// Its scope's _REG has been told the space can be used (connect).
    connected: bool,
}

struct Node {
    name: [u8; 4],
    parent: usize,
    children: Vec<usize>,
    object: Object,
}

/// A loaded namespace.
pub struct Aml {
    nodes: Vec<Node>,
    /// Integers are 32 bits (the DSDT's revision is below 2).
    narrow: bool,
    /// Steps and sleeping time left in the current evaluation; how deep method calls and expressions are.
    steps: u64,
    slept_us: u64,
    depth: u32,
    /// The most steps, and sleeping and stalling time, one evaluation gets (`limits`).
    most_steps: u64,
    most_sleep_us: u64,
    /// How many times the firmware's global lock is held (Acquire(\\_GL), fields with the Lock rule).
    pub(crate) global_lock: u32,
    /// When kept: each object evaluated from outside (by path), in order, for comparing with acpiexec's log.
    pub evaluated: Option<Vec<String>>,
    /// What went wrong as tables were loaded (the objects defined after it in its scope may be missing).
    pub problems: Vec<String>,
}

/// The most steps one evaluation takes, and the most time it sleeps or stalls, before it's given up.
const MOST_STEPS: u64 = 64_000_000;
const MOST_SLEEP_US: u64 = 10_000_000;
/// The most methods called within each other, and expressions within each other.
const MOST_DEPTH: u32 = 64;

const ROOT: usize = 0;

fn seg(s: &str) -> [u8; 4] {
    let mut n = [b'_'; 4];
    for (i, b) in s.bytes().take(4).enumerate() {
        n[i] = b;
    }
    n
}

impl Default for Aml {
    fn default() -> Self {
        Self::new()
    }
}

impl Aml {
    /// An empty namespace with what every one has: the root's scopes, and \_OSI, \_OS, \_REV and \_GL.
    pub fn new() -> Aml {
        let mut aml = Aml {
            nodes: vec![Node { name: *b"\\___", parent: ROOT, children: Vec::new(), object: Object::Scope }],
            narrow: false,
            steps: MOST_STEPS,
            slept_us: 0,
            most_steps: MOST_STEPS,
            most_sleep_us: MOST_SLEEP_US,
            global_lock: 0,
            evaluated: None,
            depth: 0,
            problems: Vec::new(),
        };
        // (\_SB and \_TZ are devices, as ACPICA has them: _STA and _INI are looked for in them as in any.)
        for (s, object) in [("_GPE", Object::Scope), ("_PR_", Object::Scope), ("_SB_", Object::Device), ("_SI_", Object::Scope), ("_TZ_", Object::Device)] {
            aml.add(ROOT, seg(s), object);
        }
        aml.add(ROOT, *b"_OSI", Object::Osi);
        aml.add(ROOT, *b"_OS_", Object::Data(Value::String(String::from("Microsoft Windows NT"))));
        aml.add(ROOT, *b"_REV", Object::Data(Value::Integer(2)));
        aml.add(ROOT, *b"_GL_", Object::Mutex);
        aml
    }

    fn add(&mut self, parent: usize, name: [u8; 4], object: Object) -> usize {
        let n = self.nodes.len();
        self.nodes.push(Node { name, parent, children: Vec::new(), object });
        self.nodes[parent].children.push(n);
        n
    }

    fn child(&self, parent: usize, name: [u8; 4]) -> Option<usize> {
        self.nodes[parent].children.iter().copied().find(|&c| self.nodes[c].name == name)
    }

    /// Takes an object out of the namespace (one a method made, when it returns), with what's under it.
    fn remove(&mut self, node: usize) {
        let parent = self.nodes[node].parent;
        self.nodes[parent].children.retain(|&c| c != node);
        for c in core::mem::take(&mut self.nodes[node].children) {
            self.nodes[c].parent = c;
        }
        self.nodes[node].object = Object::Scope;
    }

    /// How long one evaluation may go on before it's given up (as `Error::TooLong`): this many steps, and this long
    /// sleeping and stalling (microseconds). By default 64 million steps and 10 seconds: firmware polls hardware in
    /// loops, a few thousand times over, and waits for it, up to a few seconds.
    pub fn limits(&mut self, steps: u64, sleep_us: u64) {
        self.most_steps = steps.max(1);
        self.most_sleep_us = sleep_us;
    }

    /// An object's path, as "\_SB.PCI0.BAT0".
    pub fn path(&self, node: usize) -> String {
        if node == ROOT {
            return String::from("\\");
        }
        let mut parts = Vec::new();
        let mut n = node;
        while n != ROOT && parts.len() < 64 {
            parts.push(String::from_utf8_lossy(&self.nodes[n].name).into_owned());
            if self.nodes[n].parent == n {
                break;
            }
            n = self.nodes[n].parent;
        }
        parts.reverse();
        format!("\\{}", parts.join("."))
    }

    /// The object at an absolute path ("\_SB.PCI0.LPCB", "\_SB_.PCI0"), if there is one.
    pub fn find(&self, path: &str) -> Option<usize> {
        let mut n = ROOT;
        for part in path.trim_start_matches('\\').split('.').filter(|p| !p.is_empty()) {
            if part.len() > 4 {
                return None;
            }
            n = self.child(n, seg(part))?;
            if let Object::Alias(target) = self.nodes[n].object {
                n = target;
            }
        }
        Some(n)
    }

    /// The objects directly under one.
    pub fn children(&self, node: usize) -> Vec<usize> {
        self.nodes.get(node).map(|n| n.children.clone()).unwrap_or_default()
    }

    /// An object's own name ("BAT0").
    pub fn name(&self, node: usize) -> String {
        String::from_utf8_lossy(&self.nodes[node].name).into_owned()
    }

    /// Whether an object is a device (or a processor or thermal zone, which are kept as devices are).
    pub fn is_device(&self, node: usize) -> bool {
        matches!(self.nodes.get(node).map(|n| &n.object), Some(Object::Device | Object::Processor | Object::ThermalZone))
    }

    /// Whether an object is a method.
    pub fn is_method(&self, node: usize) -> bool {
        matches!(self.nodes.get(node).map(|n| &n.object), Some(Object::Method(_)))
    }

    /// How many objects there are (for the log).
    pub fn objects(&self) -> usize {
        self.nodes.len()
    }

    /// Loads a definition block (a DSDT or an SSDT, its header included): its objects go into the namespace, and the
    /// code outside its methods runs. The DSDT goes first: its revision says how wide integers are.
    pub fn load(&mut self, table: &[u8], host: &mut dyn Host) -> Result<(), Error> {
        self.begin();
        let r = self.load_table(table, host);
        self.end(host);
        r
    }

    /// Loads a table (from Load or LoadTable as well), within the evaluation going on.
    pub(crate) fn load_table(&mut self, table: &[u8], host: &mut dyn Host) -> Result<(), Error> {
        if table.len() < 36 {
            return Err(Error::Table);
        }
        let length = u32::from_le_bytes(table[4..8].try_into().unwrap()) as usize;
        if length < 36 || length > table.len() {
            return Err(Error::Table);
        }
        if &table[0..4] == b"DSDT" {
            self.narrow = table[8] < 2;
        }
        let code: Rc<[u8]> = Rc::from(&table[36..length]);
        self.run_table(code, host)
    }

    /// Runs a method (or reads an object) by its absolute path, with these arguments.
    pub fn evaluate(&mut self, path: &str, args: Vec<Value>, host: &mut dyn Host) -> Result<Value, Error> {
        let node = self.find(path).ok_or_else(|| Error::NotFound(String::from(path)))?;
        self.evaluate_node(node, args, host)
    }

    /// Runs a method (or reads an object).
    pub fn evaluate_node(&mut self, node: usize, args: Vec<Value>, host: &mut dyn Host) -> Result<Value, Error> {
        if self.evaluated.is_some() {
            let path = self.path(node);
            self.evaluated.as_mut().unwrap().push(path);
        }
        self.begin();
        let r = self.value_of(node, args, host);
        self.end(host);
        r
    }

    /// Runs `name` under `node` if there's one ("_STA" under a device), else None.
    pub fn evaluate_child(&mut self, node: usize, name: &str, args: Vec<Value>, host: &mut dyn Host) -> Option<Result<Value, Error>> {
        let n = self.child(node, seg(name))?;
        Some(self.evaluate_node(n, args, host))
    }

    /// An integer `name` under `node` gives (an integer, or a method that returns one).
    pub fn integer_child(&mut self, node: usize, name: &str, host: &mut dyn Host) -> Option<u64> {
        self.begin();
        let r = self.child_integer(node, name, host);
        self.end(host);
        r
    }

    /// `name` under `node`'s value, within the evaluation going on (none started, none ended).
    pub(crate) fn child_value(&mut self, node: usize, name: &str, host: &mut dyn Host) -> Option<Result<Value, Error>> {
        let n = self.child(node, seg(name))?;
        Some(self.value_of(n, Vec::new(), host))
    }

    pub(crate) fn child_integer(&mut self, node: usize, name: &str, host: &mut dyn Host) -> Option<u64> {
        match self.child_value(node, name, host)? {
            Ok(v) => self.integer(&v).ok(),
            Err(_) => None,
        }
    }

    fn begin(&mut self) {
        self.steps = self.most_steps;
        self.slept_us = 0;
        self.depth = 0;
    }

    /// After an evaluation: the global lock given back if a method left it held (it failed, or forgot).
    fn end(&mut self, host: &mut dyn Host) {
        self.depth = 0;
        if self.global_lock > 0 {
            self.global_lock = 0;
            host.release_global_lock();
        }
    }

    /// The devices whose hardware ID (_HID) or compatible IDs (_CID) are `id` ("PNP0C0A", "ACPI0003").
    pub fn devices(&mut self, id: &str, host: &mut dyn Host) -> Vec<usize> {
        let mut out = Vec::new();
        for node in 0..self.nodes.len() {
            if !self.is_device(node) || !self.alive(node) {
                continue;
            }
            if self.ids(node, host).iter().any(|i| i == id) {
                out.push(node);
            }
        }
        out
    }

    /// A device's IDs: its _HID and its _CIDs, as strings ("PNP0C0A").
    pub fn ids(&mut self, node: usize, host: &mut dyn Host) -> Vec<String> {
        self.begin();
        let r = self.child_ids(node, host);
        self.end(host);
        r
    }

    pub(crate) fn child_ids(&mut self, node: usize, host: &mut dyn Host) -> Vec<String> {
        let mut out = Vec::new();
        for name in ["_HID", "_CID"] {
            let Some(Ok(v)) = self.child_value(node, name, host) else { continue };
            let items = match &v {
                Value::Package(p) => p.borrow().clone(),
                other => vec![other.clone()],
            };
            for item in items {
                match item {
                    Value::Integer(n) => out.push(eisa_id(n as u32)),
                    Value::String(s) => out.push(s),
                    _ => {}
                }
            }
        }
        out
    }

    /// Whether an object is still in the namespace (not one a method made, gone since).
    fn alive(&self, node: usize) -> bool {
        let mut n = node;
        for _ in 0..256 {
            if n == ROOT {
                return true;
            }
            let p = self.nodes[n].parent;
            if p == n || !self.nodes[p].children.contains(&n) {
                return false;
            }
            n = p;
        }
        false
    }

    /// A device's status (_STA), as ACPI defines its bits: present (1), enabled (2), shown (4), functioning (8), a
    /// battery present (16). Without _STA, all but the battery's.
    pub fn status(&mut self, node: usize, host: &mut dyn Host) -> u64 {
        match self.evaluate_child(node, "_STA", Vec::new(), host) {
            None => 0x0F,
            Some(Ok(v)) => self.integer(&v).unwrap_or(0),
            Some(Err(_)) => 0,
        }
    }

    /// Initializes the namespace once every table is loaded (and the embedded controller's space connected), as
    /// ACPICA does: \\_INI (which Windows runs) and \\_SB._INI first; then `spaces` connected (the whole namespace's
    /// regions in them told by _REG, as ACPICA does for PCI configuration space, which it always handles); then the
    /// devices, in the namespace's order, each device (or processor or thermal zone) with an _INI under it asked its
    /// _STA: if it's present its _INI runs; if it's neither present nor functioning what's under it is left alone.
    /// Returns how many devices were found present.
    pub fn initialize(&mut self, spaces: &[Space], host: &mut dyn Host) -> usize {
        self.settle_packages();
        let mut has_ini = vec![false; self.nodes.len()];
        for n in 0..self.nodes.len() {
            if self.nodes[n].name != *b"_INI" || !self.alive(n) {
                continue;
            }
            let mut p = self.nodes[n].parent;
            if !self.is_device(p) {
                continue;
            }
            loop {
                has_ini[p] = true;
                if p == ROOT {
                    break;
                }
                p = self.nodes[p].parent;
            }
        }
        let sb = self.child(ROOT, seg("_SB_"));
        for scope in [Some(ROOT), sb].into_iter().flatten() {
            if let Some(Err(e)) = self.evaluate_child(scope, "_INI", Vec::new(), host) {
                self.problems.push(format!("{}._INI: {e:?}", if scope == ROOT { String::new() } else { self.path(scope) }));
            }
        }
        for &space in spaces {
            self.connect_under(ROOT, space, host);
        }
        let mut present = 0;
        let mut stack: Vec<usize> = self.children(ROOT).into_iter().rev().collect();
        let mut seen = 0;
        while let Some(node) = stack.pop() {
            seen += 1;
            if seen > 1_000_000 {
                break;
            }
            if !self.is_device(node) {
                stack.extend(self.children(node).into_iter().rev());
                continue;
            }
            if !has_ini.get(node).copied().unwrap_or(false) {
                continue;
            }
            let sta = match self.sta(node, host) {
                Ok(sta) => sta,
                // (A failing _STA: no _INI, but what's under the device is still looked at.)
                Err(_) => {
                    stack.extend(self.children(node).into_iter().rev());
                    continue;
                }
            };
            if sta & 1 == 0 {
                if sta & 8 != 0 {
                    stack.extend(self.children(node).into_iter().rev());
                }
                continue;
            }
            present += 1;
            if Some(node) != sb {
                if let Some(Err(e)) = self.evaluate_child(node, "_INI", Vec::new(), host) {
                    self.problems.push(format!("{}._INI: {e:?}", self.path(node)));
                }
            }
            stack.extend(self.children(node).into_iter().rev());
        }
        present
    }

    /// A device's _STA, strictly: an integer, or all bits set when there's none.
    fn sta(&mut self, node: usize, host: &mut dyn Host) -> Result<u64, Error> {
        match self.evaluate_child(node, "_STA", Vec::new(), host) {
            None => Ok(u32::MAX as u64),
            Some(Ok(Value::Integer(n))) => Ok(n),
            Some(Ok(_)) => Err(Error::Type("an integer")),
            Some(Err(e)) => Err(e),
        }
    }

    /// A device's _HID (None: it has none), as a string.
    fn hid(&mut self, node: usize, host: &mut dyn Host) -> Option<Result<String, Error>> {
        Some(match self.evaluate_child(node, "_HID", Vec::new(), host)? {
            Ok(Value::Integer(n)) => Ok(eisa_id(n as u32)),
            Ok(Value::String(s)) => Ok(s),
            Ok(_) => Err(Error::Type("an integer or a string")),
            Err(e) => Err(e),
        })
    }

    /// A device's _CIDs (None: it has none), as strings.
    fn cids(&mut self, node: usize, host: &mut dyn Host) -> Option<Result<Vec<String>, Error>> {
        let v = match self.evaluate_child(node, "_CID", Vec::new(), host)? {
            Ok(v) => v,
            Err(e) => return Some(Err(e)),
        };
        let items = match &v {
            Value::Package(p) => p.borrow().clone(),
            other => vec![other.clone()],
        };
        let mut out = Vec::new();
        for item in items {
            match item {
                Value::Integer(n) => out.push(eisa_id(n as u32)),
                Value::String(s) => out.push(s),
                _ => return Some(Err(Error::Type("an integer or a string"))),
            }
        }
        Some(Ok(out))
    }

    /// The devices whose _HID or a _CID is `id`, as ACPICA finds them for drivers (AcpiGetDevices): in the
    /// namespace's order, only those whose _STA says they're present or functioning; one whose _HID, _CID or _STA
    /// fails isn't, nor is anything under it, nor anything under one that matched but is neither present nor
    /// functioning.
    pub fn present_devices(&mut self, id: &str, host: &mut dyn Host) -> Vec<usize> {
        self.find_devices(id, usize::MAX, host)
    }

    /// The same, stopping at the `most`th found; an empty `id` is any device's.
    pub fn find_devices(&mut self, id: &str, most: usize, host: &mut dyn Host) -> Vec<usize> {
        let mut out = Vec::new();
        let mut stack: Vec<usize> = self.children(ROOT).into_iter().rev().collect();
        let mut seen = 0;
        while let Some(node) = stack.pop() {
            seen += 1;
            if seen > 1_000_000 {
                break;
            }
            if !matches!(self.nodes[node].object, Object::Device) {
                stack.extend(self.children(node).into_iter().rev());
                continue;
            }
            let matched = if id.is_empty() {
                true
            } else {
                match self.hid(node, host) {
                None => false,
                Some(Err(_)) => continue,
                Some(Ok(h)) if h == id => true,
                Some(Ok(_)) => match self.cids(node, host) {
                    None => false,
                    Some(Err(_)) => continue,
                    Some(Ok(list)) => list.iter().any(|c| c == id),
                },
                }
            };
            if matched {
                match self.sta(node, host) {
                    Ok(sta) if sta & 0x09 != 0 => out.push(node),
                    _ => continue,
                }
                if out.len() >= most {
                    break;
                }
            }
            stack.extend(self.children(node).into_iter().rev());
        }
        out
    }

    /// Tells the firmware an address space can now be used (a driver for it is ready), throughout the namespace.
    pub fn connect(&mut self, space: Space, host: &mut dyn Host) {
        self.connect_under(ROOT, space, host);
    }

    /// Tells the firmware an address space can now be used under `node` (the embedded controller's, under its device),
    /// as ACPICA does: each operation region in it there, in the namespace's order, has its scope's _REG(space, 1)
    /// run, once a region; and for the embedded controller's space (and GPIO's) a _REG directly under `node` is run
    /// even with no region of the space beside it, as firmware expects. Never for memory or I/O ports, which are
    /// always there.
    pub fn connect_under(&mut self, node: usize, space: Space, host: &mut dyn Host) {
        if matches!(space, Space::Memory | Space::Io | Space::Table) {
            return;
        }
        let mut regions = Vec::new();
        let mut stack: Vec<usize> = self.children(node).into_iter().rev().collect();
        while let Some(n) = stack.pop() {
            if regions.len() > 100_000 {
                break;
            }
            if let Object::Region(r) = &self.nodes[n].object {
                if r.space == space {
                    regions.push(n);
                }
            }
            stack.extend(self.children(n).into_iter().rev());
        }
        let args = || vec![Value::Integer(space.number() as u64), Value::Integer(1)];
        for region in regions {
            let parent = self.nodes[region].parent;
            let Some(reg) = self.child(parent, seg("_REG")).filter(|&r| self.is_method(r)) else { continue };
            match &mut self.nodes[region].object {
                Object::Region(r) if !r.connected => r.connected = true,
                _ => continue,
            }
            if let Err(e) = self.evaluate_node(reg, args(), host) {
                self.problems.push(format!("{}._REG: {e:?}", self.path(parent)));
            }
        }
        if matches!(space, Space::EmbeddedControl | Space::Gpio) {
            let beside = self.children(node).iter().any(|&c| matches!(&self.nodes[c].object, Object::Region(r) if r.space == space));
            if let Some(reg) = self.child(node, seg("_REG")).filter(|&r| self.is_method(r) && !beside) {
                if let Err(e) = self.evaluate_node(reg, args(), host) {
                    self.problems.push(format!("{}._REG: {e:?}", self.path(node)));
                }
            }
        }
    }

    /// The general-purpose events the firmware has methods for, by number: \\_GPE's _Lxx (level-triggered: true) and
    /// _Exx (edge-triggered: false) methods, and the method.
    pub fn gpe_methods(&self) -> Vec<(u32, bool, usize)> {
        let mut out = Vec::new();
        let Some(gpe) = self.child(ROOT, seg("_GPE")) else { return out };
        for c in self.children(gpe) {
            let name = self.nodes[c].name;
            if name[0] != b'_' || !matches!(name[1], b'L' | b'E') || !self.is_method(c) {
                continue;
            }
            let hex = |b: u8| (b as char).to_digit(16);
            if let (Some(hi), Some(lo)) = (hex(name[2]), hex(name[3])) {
                out.push((hi * 16 + lo, name[1] == b'L', c));
            }
        }
        out
    }

    /// The general-purpose events present devices wake the PC with (their _PRW's first element, when it's a number):
    /// not ones to run methods for while it's on, as ACPICA leaves them.
    pub fn wake_gpes(&mut self, host: &mut dyn Host) -> Vec<u32> {
        let mut out = Vec::new();
        let devices: Vec<usize> = (0..self.nodes.len()).filter(|&n| self.is_device(n) && self.child(n, seg("_PRW")).is_some() && self.alive(n)).collect();
        for d in devices {
            if self.sta(d, host).map_or(true, |s| s & 1 == 0) {
                continue;
            }
            let Some(Ok(Value::Package(p))) = self.evaluate_child(d, "_PRW", Vec::new(), host) else { continue };
            let first = p.borrow().first().cloned();
            if let Some(Value::Integer(n)) = first {
                if n < 256 && !out.contains(&(n as u32)) {
                    out.push(n as u32);
                }
            }
        }
        out
    }

    /// The names in the tables' packages that named nothing as the tables loaded (forward references, or another
    /// table's objects), settled once every table is: each the object it names now, or nothing (an empty element), as
    /// ACPICA settles them.
    pub(crate) fn settle_packages(&mut self) {
        fn settle(aml: &Aml, v: &Value, depth: u32) {
            let Value::Package(p) = v else { return };
            if depth > 32 {
                return;
            }
            let mut items = p.borrow_mut();
            for item in items.iter_mut() {
                if let Value::Reference(Reference::Unresolved(name, scope)) = item {
                    *item = match aml.find_from(*scope, name) {
                        Some(n) => Value::Reference(Reference::Node(n)),
                        None => Value::Uninitialized,
                    };
                }
            }
            let nested: Vec<Value> = items.iter().filter(|i| matches!(i, Value::Package(_))).cloned().collect();
            drop(items);
            for n in nested {
                settle(aml, &n, depth + 1);
            }
        }
        for node in 0..self.nodes.len() {
            if let Object::Data(v) = &self.nodes[node].object {
                settle(self, v, 0);
            }
        }
    }

    /// The thermal zones (ThermalZone objects), wherever they are.
    pub fn thermal_zones(&self) -> Vec<usize> {
        (0..self.nodes.len()).filter(|&n| matches!(self.nodes[n].object, Object::ThermalZone) && self.alive(n)).collect()
    }

    /// The address spaces the namespace has operation regions in (each once).
    pub fn region_spaces(&self) -> Vec<Space> {
        let mut out: Vec<Space> = Vec::new();
        for n in &self.nodes {
            if let Object::Region(r) = &n.object {
                if !out.contains(&r.space) {
                    out.push(r.space);
                }
            }
        }
        out
    }

    /// The integer a value is (or converts to).
    pub fn integer(&self, v: &Value) -> Result<u64, Error> {
        self.to_integer(v)
    }
}
