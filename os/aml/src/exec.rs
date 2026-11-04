//! Running the byte code: a table's code outside its methods as it's loaded, and methods as they're called. The code
//! is read as it runs (no tree is built): each term's opcode says what follows it. Names are looked up as ACPI says
//! (a single segment up through the scopes, others from where they say), and a name that's a method is called with
//! as many arguments as it takes.

use crate::code::{starts_name, Cursor, Name};
use crate::convert::copy;
use crate::field::{Field, FieldAt};
use crate::{Aml, Error, Host, Method, Object, Reference, Region, Space, Value, MOST_DEPTH, ROOT};
use alloc::format;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::cell::RefCell;

/// A method's arguments and locals, its scope, and the objects it made (taken away when it returns).
pub(crate) struct Frame {
    scope: usize,
    args: Vec<Rc<RefCell<Value>>>,
    locals: Vec<Rc<RefCell<Value>>>,
    made: Vec<usize>,
    /// Running a table's code as it's loaded (objects stay), or a method's.
    loading: bool,
}

impl Frame {
    fn new(scope: usize, args: Vec<Value>, loading: bool) -> Frame {
        let mut a: Vec<Rc<RefCell<Value>>> = args.into_iter().map(|v| Rc::new(RefCell::new(v))).collect();
        while a.len() < 7 {
            a.push(Rc::new(RefCell::new(Value::Uninitialized)));
        }
        let locals = (0..8).map(|_| Rc::new(RefCell::new(Value::Uninitialized))).collect();
        Frame { scope, args: a, locals, made: Vec::new(), loading }
    }
}

/// What a term list did: carried on, or returned, broke out of a loop, or continued it.
pub(crate) enum Flow {
    Next,
    Return(Value),
    Break,
    Continue,
}

/// Where a result goes.
enum Target {
    Null,
    Debug,
    Local(usize),
    Arg(usize),
    Node(usize),
    Ref(Reference),
}

/// _OSI's answers: yes to Windows' versions, as Windows itself answers (its firmware is written and tested for that),
/// and to the one feature group string ACPICA, the reference implementation, says yes to by default. (The others, such
/// as "Processor Aggregator Device", are features this system doesn't have.)
const OSI: [&str; 24] = [
    "Windows 2000",
    "Windows 2001",
    "Windows 2001 SP1",
    "Windows 2001.1",
    "Windows 2001 SP2",
    "Windows 2001.1 SP1",
    "Windows 2006",
    "Windows 2006.1",
    "Windows 2006 SP1",
    "Windows 2006 SP2",
    "Windows 2009",
    "Windows 2012",
    "Windows 2013",
    "Windows 2015",
    "Windows 2016",
    "Windows 2017",
    "Windows 2017.2",
    "Windows 2018",
    "Windows 2018.2",
    "Windows 2019",
    "Windows 2020",
    "Windows 2021",
    "Windows 2022",
    "Extended Address Space Descriptor",
];

impl Aml {
    fn step(&mut self) -> Result<(), Error> {
        if self.steps == 0 {
            return Err(Error::TooLong);
        }
        self.steps -= 1;
        Ok(())
    }

    /// Runs a table's code, as it's loaded: its objects go into the namespace.
    pub(crate) fn run_table(&mut self, code: Rc<[u8]>, host: &mut dyn Host) -> Result<(), Error> {
        let end = code.len();
        let mut c = Cursor::new(code, 0, end);
        let mut f = Frame::new(ROOT, Vec::new(), true);
        // An object that can't be made is said, and its code passed over: the rest of the table still loads (term_list).
        self.term_list(&mut c, end, &mut f, host)?;
        Ok(())
    }

    /// The value of an object: a method's result, data, a field's bits.
    pub(crate) fn value_of(&mut self, node: usize, args: Vec<Value>, host: &mut dyn Host) -> Result<Value, Error> {
        match self.nodes[node].object.clone() {
            Object::Method(_) => self.call(node, args, host),
            Object::Osi => Ok(Value::Integer(self.osi(args.first()))),
            Object::Data(v) => Ok(v),
            Object::Field(_) | Object::BufferField { .. } => self.read_field(node, host),
            Object::Alias(n) => self.value_of(n, args, host),
            _ => Ok(Value::Object(node)),
        }
    }

    fn osi(&self, arg: Option<&Value>) -> u64 {
        match arg {
            Some(Value::String(s)) if OSI.contains(&s.as_str()) => self.ones(),
            _ => 0,
        }
    }

    /// Calls a method.
    fn call(&mut self, node: usize, args: Vec<Value>, host: &mut dyn Host) -> Result<Value, Error> {
        let Object::Method(m) = self.nodes[node].object.clone() else { return Err(Error::Type("a method")) };
        if self.depth >= MOST_DEPTH {
            return Err(Error::TooDeep);
        }
        self.depth += 1;
        // (Arguments as they are: a buffer or a package passed is the caller's own, which the method may change.)
        let mut f = Frame::new(node, args, false);
        let mut c = Cursor::new(m.code.clone(), m.start, m.end);
        let r = self.term_list(&mut c, m.end, &mut f, host);
        for n in f.made.iter().rev() {
            self.remove(*n);
        }
        self.depth = self.depth.saturating_sub(1);
        match r? {
            Flow::Return(v) => Ok(v),
            // No value returned: as acpiexec says, nothing (where a caller uses it, zero: see eval).
            _ => Ok(Value::Uninitialized),
        }
    }

    fn term_list(&mut self, c: &mut Cursor, end: usize, f: &mut Frame, host: &mut dyn Host) -> Result<Flow, Error> {
        while c.pos < end {
            let at = c.pos;
            match self.term(c, end, f, host) {
                Ok(Flow::Next) => {}
                Ok(other) => return Ok(other),
                // As a table loads, a term that fails is said and passed over, and the rest of its scope still loads,
                // as ACPICA's parser goes on (a method's code stops at the first error instead).
                Err(e) if f.loading && !matches!(e, Error::TooLong | Error::TooDeep) => {
                    self.problems.push(format!("at {at:#x}: {e:?}"));
                    c.pos = at;
                    if !self.pass(c, f.scope, 0) || c.pos > end || c.pos <= at {
                        return Err(e);
                    }
                }
                Err(e) => return Err(e),
            }
        }
        Ok(Flow::Next)
    }

    /// Goes past one term without running it, by the AML grammar alone (a term that failed as its table loaded):
    /// false if the code doesn't make sense there. A name is a method's call if it names a method now (its arguments
    /// then follow), as ACPICA's parser takes it.
    fn pass(&self, c: &mut Cursor, scope: usize, depth: u32) -> bool {
        if depth > 64 {
            return false;
        }
        let Some(op) = c.peek() else { return false };
        if starts_name(op) {
            let Ok(name) = c.name() else { return false };
            let args = match self.resolve(scope, &name).map(|n| &self.nodes[n].object) {
                Some(Object::Method(m)) => m.args as usize,
                _ => 0,
            };
            return (0..args).all(|_| self.pass(c, scope, depth + 1));
        }
        c.pos += 1;
        let arg = |c: &mut Cursor| self.pass(c, scope, depth + 1);
        // A target, or a name to act on: a name there is never a method's call.
        let name_or = |c: &mut Cursor| if c.peek().is_some_and(starts_name) { c.name().is_ok() } else { self.pass(c, scope, depth + 1) };
        let skip = |c: &mut Cursor, n: usize| {
            if c.pos + n > c.end {
                return false;
            }
            c.pos += n;
            true
        };
        let packaged = |c: &mut Cursor| match c.pkg_end() {
            Ok(e) => {
                c.pos = e;
                true
            }
            Err(_) => false,
        };
        match op {
            0x00 | 0x01 | 0xFF | 0x60..=0x6E | 0x9F | 0xA3 | 0xA5 | 0xCC => true,
            0x0A => skip(c, 1),
            0x0B => skip(c, 2),
            0x0C => skip(c, 4),
            0x0E => skip(c, 8),
            0x0D => c.string().is_ok(),
            0x06 => c.name().is_ok() && c.name().is_ok(),
            0x08 => c.name().is_ok() && arg(c),
            0x10 | 0x11 | 0x12 | 0x13 | 0x14 | 0xA0 | 0xA1 | 0xA2 => packaged(c),
            0x15 => c.name().is_ok() && skip(c, 2),
            0x70 | 0x9D => arg(c) && name_or(c),
            0x71 | 0x75 | 0x76 | 0x87 | 0x8E => name_or(c),
            0x72 | 0x73 | 0x74 | 0x77 | 0x79 | 0x7A | 0x7B | 0x7C | 0x7D | 0x7E | 0x7F | 0x84 | 0x85 | 0x88 | 0x9C => {
                arg(c) && arg(c) && name_or(c)
            }
            0x78 => arg(c) && arg(c) && name_or(c) && name_or(c),
            0x80 | 0x81 | 0x82 | 0x96 | 0x97 | 0x98 | 0x99 => arg(c) && name_or(c),
            0x83 | 0x92 | 0xA4 => arg(c),
            0x86 => name_or(c) && arg(c),
            0x89 => arg(c) && skip(c, 1) && arg(c) && skip(c, 1) && arg(c) && arg(c),
            0x8A | 0x8B | 0x8C | 0x8D | 0x8F => arg(c) && arg(c) && c.name().is_ok(),
            0x90 | 0x91 | 0x93 | 0x94 | 0x95 => arg(c) && arg(c),
            0x9E => arg(c) && arg(c) && arg(c) && name_or(c),
            0x5B => {
                let Ok(ext) = c.byte() else { return false };
                match ext {
                    0x30 | 0x31 | 0x33 => true,
                    0x01 => c.name().is_ok() && skip(c, 1),
                    0x02 => c.name().is_ok(),
                    0x12 => name_or(c) && name_or(c),
                    0x13 => arg(c) && arg(c) && arg(c) && c.name().is_ok(),
                    0x1F => (0..6).all(|_| arg(c)),
                    0x20 => name_or(c) && name_or(c),
                    0x21 | 0x22 => arg(c),
                    0x23 => name_or(c) && skip(c, 2),
                    0x24 | 0x26 | 0x27 | 0x2A => name_or(c),
                    0x25 => name_or(c) && arg(c),
                    0x28 | 0x29 => arg(c) && name_or(c),
                    0x32 => skip(c, 5) && arg(c),
                    0x80 => c.name().is_ok() && skip(c, 1) && arg(c) && arg(c),
                    0x81..=0x87 => packaged(c),
                    0x88 => c.name().is_ok() && arg(c) && arg(c) && arg(c),
                    _ => false,
                }
            }
            _ => false,
        }
    }

    /// Runs a term's body (a device's, a scope's) in its own scope.
    fn body(&mut self, c: &mut Cursor, end: usize, scope: usize, f: &mut Frame, host: &mut dyn Host) -> Result<Flow, Error> {
        let saved = f.scope;
        f.scope = scope;
        let r = self.term_list(c, end, f, host);
        f.scope = saved;
        c.pos = end;
        r
    }

    /// Makes a named object (in the current scope, or where its name says), as a method's own if it's a method's.
    fn define(&mut self, f: &mut Frame, name: &Name, object: Object) -> Result<usize, Error> {
        let Some((&last, path)) = name.segs.split_last() else { return Err(Error::Type("a name")) };
        let mut parent = if name.root { ROOT } else { f.scope };
        for _ in 0..name.up {
            parent = self.nodes[parent].parent;
        }
        for &s in path {
            parent = self.child(parent, s).ok_or_else(|| Error::NotFound(name.text()))?;
            if let Object::Alias(t) = self.nodes[parent].object {
                parent = t;
            }
        }
        if let Some(existing) = self.child(parent, last) {
            if matches!(self.nodes[existing].object, Object::External) {
                self.nodes[existing].object = object;
                return Ok(existing);
            }
            return Err(Error::Exists(self.path(existing)));
        }
        let n = self.add(parent, last, object);
        if !f.loading {
            f.made.push(n);
        }
        Ok(n)
    }

    /// Finds a name: a single segment in the current scope or the ones it's in, others where they say.
    pub(crate) fn resolve(&self, scope: usize, name: &Name) -> Option<usize> {
        let mut n = if name.root { ROOT } else { scope };
        for _ in 0..name.up {
            n = self.nodes[n].parent;
        }
        let follow = |n: usize| match self.nodes[n].object {
            Object::Alias(t) => t,
            _ => n,
        };
        if name.searched() {
            let mut s = n;
            for _ in 0..256 {
                if let Some(found) = self.child(s, name.segs[0]) {
                    return Some(follow(found));
                }
                if s == ROOT {
                    return None;
                }
                s = self.nodes[s].parent;
            }
            return None;
        }
        for &seg in &name.segs {
            n = follow(self.child(n, seg)?);
        }
        Some(n)
    }

    /// One term: an object's definition, a statement, or an expression whose result isn't wanted.
    fn term(&mut self, c: &mut Cursor, end: usize, f: &mut Frame, host: &mut dyn Host) -> Result<Flow, Error> {
        self.step()?;
        let at = c.pos;
        let op = c.byte()?;
        match op {
            0x06 => {
                let target = c.name()?;
                let alias = c.name()?;
                let node = self.resolve(f.scope, &target).ok_or_else(|| Error::NotFound(target.text()))?;
                self.define(f, &alias, Object::Alias(node))?;
            }
            0x08 => {
                let name = c.name()?;
                let v = self.data(c, f, host)?;
                self.define(f, &name, Object::Data(v))?;
            }
            0x10 => {
                let e = c.pkg_end()?;
                let name = c.name()?;
                let node = self.resolve(f.scope, &name).ok_or_else(|| Error::NotFound(name.text()))?;
                let r = self.body(c, e, node, f, host)?;
                if !matches!(r, Flow::Next) {
                    return Ok(r);
                }
            }
            0x14 => {
                let e = c.pkg_end()?;
                let name = c.name()?;
                let flags = c.byte()?;
                let m = Method { code: c.code.clone(), start: c.pos, end: e, args: flags & 7 };
                c.pos = e;
                self.define(f, &name, Object::Method(Rc::new(m)))?;
            }
            0x15 => {
                let name = c.name()?;
                c.byte()?;
                c.byte()?;
                // (Declared here, defined in another table: made a placeholder if that table isn't loaded yet.)
                if self.resolve(f.scope, &name).is_none() && f.loading {
                    let _ = self.define(f, &name, Object::External);
                }
            }
            0x5B => return self.ext_term(c, at, f, host),
            0x8A..=0x8D | 0x8F => {
                let buffer = self.eval(c, f, host)?;
                let index = self.int(c, f, host)?;
                let name = c.name()?;
                let (bit, bits) = match op {
                    0x8A => (index * 8, 32),
                    0x8B => (index * 8, 16),
                    0x8C => (index * 8, 8),
                    0x8D => (index, 1),
                    _ => (index * 8, 64),
                };
                let b = self.buffer_of(buffer)?;
                let field = self.buffer_field(b, bit, bits, false)?;
                self.define(f, &name, field)?;
            }
            0xA0 => {
                let e = c.pkg_end()?;
                let yes = self.int(c, f, host)? != 0;
                let r = if yes { self.term_list(c, e, f, host)? } else { Flow::Next };
                c.pos = e;
                // An Else after it: run if the If's wasn't.
                if c.pos < end && c.peek() == Some(0xA1) {
                    c.pos += 1;
                    let e2 = c.pkg_end()?;
                    if !yes {
                        let r2 = self.term_list(c, e2, f, host)?;
                        c.pos = e2;
                        return Ok(r2);
                    }
                    c.pos = e2;
                }
                return Ok(r);
            }
            0xA1 => {
                let e = c.pkg_end()?;
                c.pos = e;
            }
            0xA2 => {
                let e = c.pkg_end()?;
                let predicate = c.pos;
                loop {
                    c.pos = predicate;
                    if self.int(c, f, host)? == 0 {
                        break;
                    }
                    match self.term_list(c, e, f, host)? {
                        Flow::Break => break,
                        Flow::Return(v) => {
                            c.pos = e;
                            return Ok(Flow::Return(v));
                        }
                        _ => {}
                    }
                }
                c.pos = e;
            }
            0xA3 | 0xCC => {}
            0xA4 => {
                // Return (a method's call that returns nothing) returns nothing, as ACPICA has it: an error only where
                // the value is then wanted.
                if let Some(v) = self.call_named(c, f, host)? {
                    return Ok(Flow::Return(v));
                }
                let v = self.eval(c, f, host)?;
                return Ok(Flow::Return(v));
            }
            0xA5 => return Ok(Flow::Break),
            0x9F => return Ok(Flow::Continue),
            0x86 => {
                let target = self.target(c, f, host)?;
                let value = self.int(c, f, host)?;
                if let Some(node) = self.target_node(&target) {
                    let path = self.path(node);
                    host.notify(&path, value);
                }
            }
            _ => {
                c.pos = at;
                // A method called for what it does: whatever it returns (or nothing) isn't wanted.
                if self.call_named(c, f, host)?.is_none() {
                    self.eval(c, f, host)?;
                }
            }
        }
        Ok(Flow::Next)
    }

    /// If the code at the cursor is a method's call: the call, and what the method returned (Uninitialized: nothing).
    /// Otherwise None, the cursor where it was.
    fn call_named(&mut self, c: &mut Cursor, f: &mut Frame, host: &mut dyn Host) -> Result<Option<Value>, Error> {
        if !c.peek().is_some_and(starts_name) {
            return Ok(None);
        }
        let at = c.pos;
        let name = c.name()?;
        if let Some(node) = self.resolve(f.scope, &name)
            && let Object::Method(m) = self.nodes[node].object.clone()
        {
            let mut args = Vec::with_capacity(m.args as usize);
            for _ in 0..m.args {
                args.push(self.eval(c, f, host)?);
            }
            return Ok(Some(self.call(node, args, host)?));
        }
        c.pos = at;
        Ok(None)
    }

    /// Terms after the extended opcode prefix (5Bh): objects (regions, fields, devices...) and statements.
    fn ext_term(&mut self, c: &mut Cursor, at: usize, f: &mut Frame, host: &mut dyn Host) -> Result<Flow, Error> {
        let op = c.byte()?;
        match op {
            0x01 => {
                let name = c.name()?;
                c.byte()?;
                self.define(f, &name, Object::Mutex)?;
            }
            0x02 => {
                let name = c.name()?;
                self.define(f, &name, Object::Event(0))?;
            }
            0x80 => {
                let name = c.name()?;
                let space = Space::of(c.byte()?);
                let offset = self.int(c, f, host)?;
                let length = self.int(c, f, host)?;
                self.define(f, &name, Object::Region(Region { space, offset, length, table: None, connected: false }))?;
            }
            0x88 => {
                // DataTableRegion: a table of the firmware's own, read as a region.
                let name = c.name()?;
                let sig = self.text_arg(c, f, host)?;
                let oem = self.text_arg(c, f, host)?;
                let oem_table = self.text_arg(c, f, host)?;
                let mut signature = [0u8; 4];
                for (i, b) in sig.bytes().take(4).enumerate() {
                    signature[i] = b;
                }
                let table = host.table(&signature, oem.as_bytes(), oem_table.as_bytes()).unwrap_or_default();
                let length = table.len() as u64;
                self.define(f, &name, Object::Region(Region { space: Space::Table, offset: 0, length, table: Some(Rc::from(table)), connected: false }))?;
            }
            0x81 => {
                let e = c.pkg_end()?;
                let region = c.name()?;
                let flags = c.byte()?;
                let r = self.resolve(f.scope, &region).ok_or_else(|| Error::NotFound(region.text()));
                match r {
                    Ok(r) => self.fields(c, e, f, flags, || FieldAt::Region(r))?,
                    Err(err) => {
                        c.pos = e;
                        return Err(err);
                    }
                }
            }
            0x86 => {
                let e = c.pkg_end()?;
                let index = c.name()?;
                let data = c.name()?;
                let flags = c.byte()?;
                let (Some(i), Some(d)) = (self.resolve(f.scope, &index), self.resolve(f.scope, &data)) else {
                    c.pos = e;
                    return Err(Error::NotFound(index.text()));
                };
                self.fields(c, e, f, flags, || FieldAt::Index { index: i, data: d })?;
            }
            0x87 => {
                let e = c.pkg_end()?;
                let region = c.name()?;
                let bank = c.name()?;
                let value = self.int(c, f, host)?;
                let flags = c.byte()?;
                let (Some(r), Some(b)) = (self.resolve(f.scope, &region), self.resolve(f.scope, &bank)) else {
                    c.pos = e;
                    return Err(Error::NotFound(region.text()));
                };
                self.fields(c, e, f, flags, || FieldAt::Bank { region: r, bank: b, value })?;
            }
            0x82 | 0x83 | 0x84 | 0x85 => {
                let e = c.pkg_end()?;
                let name = c.name()?;
                let object = match op {
                    0x82 => Object::Device,
                    0x83 => {
                        c.byte()?;
                        c.dword()?;
                        c.byte()?;
                        Object::Processor
                    }
                    0x84 => {
                        c.byte()?;
                        c.word()?;
                        Object::PowerResource
                    }
                    _ => Object::ThermalZone,
                };
                // (The same device defined again, in a second table say, is an error, and its body passed over, as
                // ACPICA has it.)
                let node = match self.define(f, &name, object) {
                    Ok(n) => n,
                    Err(err) => {
                        c.pos = e;
                        return Err(err);
                    }
                };
                let r = self.body(c, e, node, f, host)?;
                if !matches!(r, Flow::Next) {
                    return Ok(r);
                }
            }
            0x13 => {
                let buffer = self.eval(c, f, host)?;
                let bit = self.int(c, f, host)?;
                let bits = self.int(c, f, host)?;
                let name = c.name()?;
                let b = self.buffer_of(buffer)?;
                let field = self.buffer_field(b, bit, bits, true)?;
                self.define(f, &name, field)?;
            }
            0x21 => {
                let us = self.int(c, f, host)?.min(1000);
                self.slept_us += us;
                if self.slept_us > self.most_sleep_us {
                    return Err(Error::TooLong);
                }
                host.stall(us);
            }
            0x22 => {
                let ms = self.int(c, f, host)?.min(2000);
                self.slept_us += ms * 1000;
                if self.slept_us > self.most_sleep_us {
                    return Err(Error::TooLong);
                }
                host.sleep(ms);
            }
            0x24 | 0x26 => {
                let t = self.target(c, f, host)?;
                if let Some(n) = self.target_node(&t)
                    && let Object::Event(count) = &mut self.nodes[n].object
                {
                    *count = if op == 0x24 { count.saturating_add(1) } else { 0 };
                }
            }
            0x27 => {
                let t = self.target(c, f, host)?;
                if self.target_node(&t).is_some() && self.target_node(&t) == self.child(ROOT, *b"_GL_") && self.global_lock > 0 {
                    self.global_lock -= 1;
                    if self.global_lock == 0 {
                        host.release_global_lock();
                    }
                }
            }
            0x32 => {
                let kind = c.byte()?;
                let code = c.dword()?;
                let arg = self.int(c, f, host)?;
                return Err(Error::Fatal(kind, code, arg));
            }
            0x20 => {
                // Load: a table from a region, a field or a buffer.
                let source = self.target(c, f, host)?;
                let handle = self.target(c, f, host)?;
                let table = self.table_from(&source, host)?;
                let loaded = self.load_table(&table, host).is_ok();
                self.settle_packages();
                self.store(&handle, Value::Integer(loaded as u64), f, host)?;
            }
            0x2A => {
                self.target(c, f, host)?;
            }
            _ => {
                c.pos = at;
                self.eval(c, f, host)?;
            }
        }
        Ok(Flow::Next)
    }

    /// A table's bytes for Load: from an operation region (its header says how long), a field or a buffer.
    fn table_from(&mut self, source: &Target, host: &mut dyn Host) -> Result<Vec<u8>, Error> {
        let node = self.target_node(source).ok_or(Error::Type("a region, a field or a buffer"))?;
        match self.nodes[node].object.clone() {
            Object::Region(r) => {
                let byte = |aml: &mut Aml, host: &mut dyn Host, i: u64| -> Result<u8, Error> {
                    if r.space == Space::Pci {
                        return Err(Error::Region(Space::Pci));
                    }
                    let _ = aml;
                    host.read(r.space, r.offset + i, 8).map(|b| b as u8).ok_or(Error::Region(r.space))
                };
                let mut head = Vec::new();
                for i in 0..8 {
                    head.push(byte(self, host, i)?);
                }
                let length = u32::from_le_bytes(head[4..8].try_into().unwrap()) as u64;
                if !(36..=(1 << 20)).contains(&length) || length > r.length {
                    return Err(Error::Table);
                }
                let mut table = head;
                for i in 8..length {
                    table.push(byte(self, host, i)?);
                }
                Ok(table)
            }
            _ => {
                let v = self.value_of(node, Vec::new(), host)?;
                self.to_buffer(&v)
            }
        }
    }

    /// A field list: named fields of so many bits, reserved bits, and changes of access type.
    fn fields(&mut self, c: &mut Cursor, end: usize, f: &mut Frame, flags: u8, at: impl Fn() -> FieldAt) -> Result<(), Error> {
        let mut access = flags & 0x0F;
        let update = (flags >> 5) & 3;
        let lock = flags & 0x10 != 0;
        let mut bit: u64 = 0;
        while c.pos < end {
            match c.peek() {
                Some(0x00) => {
                    c.pos += 1;
                    bit += c.pkg_length()? as u64;
                }
                Some(0x01) => {
                    c.pos += 1;
                    access = c.byte()? & 0x0F;
                    c.byte()?;
                }
                Some(0x02) => {
                    // A connection (GPIO, serial bus): not used here.
                    c.pos += 1;
                    if c.peek().is_some_and(starts_name) {
                        c.name()?;
                    } else if c.peek() == Some(0x11) {
                        c.pos += 1;
                        let e = c.pkg_end()?;
                        c.pos = e;
                    } else {
                        return Err(Error::Parse(c.pos));
                    }
                }
                Some(0x03) => {
                    c.pos += 1;
                    access = c.byte()? & 0x0F;
                    c.byte()?;
                    c.byte()?;
                }
                _ => {
                    let seg = c.seg()?;
                    let bits = c.pkg_length()? as u64;
                    let name = Name { root: false, up: 0, segs: vec![seg] };
                    let field = Field { at: at(), bit, bits, access, update, lock };
                    self.define(f, &name, Object::Field(Rc::new(field)))?;
                    bit += bits;
                }
            }
        }
        c.pos = end;
        Ok(())
    }

    /// Data for Name: a constant, a string, a buffer, a package, or a reference to a named object.
    fn data(&mut self, c: &mut Cursor, f: &mut Frame, host: &mut dyn Host) -> Result<Value, Error> {
        if c.peek().is_some_and(starts_name) {
            let name = c.name()?;
            return Ok(match self.resolve(f.scope, &name) {
                Some(n) => Value::Reference(Reference::Node(n)),
                None => Value::Reference(Reference::Unresolved(name.text(), f.scope)),
            });
        }
        self.eval(c, f, host)
    }

    /// A term's value, as an integer.
    fn int(&mut self, c: &mut Cursor, f: &mut Frame, host: &mut dyn Host) -> Result<u64, Error> {
        let v = self.eval(c, f, host)?;
        self.to_integer(&v)
    }

    fn text_arg(&mut self, c: &mut Cursor, f: &mut Frame, host: &mut dyn Host) -> Result<String, Error> {
        let v = self.eval(c, f, host)?;
        self.to_text(&v)
    }

    /// The buffer a value is, or refers to (for buffer fields, which share it).
    fn buffer_of(&self, v: Value) -> Result<Rc<RefCell<Vec<u8>>>, Error> {
        match v {
            Value::Buffer(b) => Ok(b),
            Value::Reference(r) => self.buffer_of(self.deref(&r)?),
            _ => Err(Error::Type("a buffer")),
        }
    }

    /// A term's value (an expression, a constant, a name's value or a method's result).
    pub(crate) fn eval(&mut self, c: &mut Cursor, f: &mut Frame, host: &mut dyn Host) -> Result<Value, Error> {
        self.step()?;
        if self.depth > MOST_DEPTH * 4 {
            return Err(Error::TooDeep);
        }
        self.depth += 1;
        let r = self.eval_inner(c, f, host);
        self.depth = self.depth.saturating_sub(1);
        r
    }

    fn eval_inner(&mut self, c: &mut Cursor, f: &mut Frame, host: &mut dyn Host) -> Result<Value, Error> {
        let at = c.pos;
        let op = c.byte()?;
        let v = match op {
            0x00 => Value::Integer(0),
            0x01 => Value::Integer(1),
            0xFF => Value::Integer(self.ones()),
            0x0A => Value::Integer(c.byte()? as u64),
            0x0B => Value::Integer(c.word()? as u64),
            0x0C => Value::Integer(c.dword()? as u64),
            0x0E => Value::Integer(self.trim(c.qword()?)),
            0x0D => Value::String(c.string()?),
            0x11 => {
                let e = c.pkg_end()?;
                let size = self.int(c, f, host)? as usize;
                if size > 1 << 20 {
                    return Err(Error::Index);
                }
                // (A damaged size can run past the buffer's end.)
                let mut b = c.code.get(c.pos..e).ok_or(Error::Parse(at))?.to_vec();
                c.pos = e;
                if b.len() < size {
                    b.resize(size, 0);
                }
                Value::buffer(b)
            }
            0x12 | 0x13 => {
                let e = c.pkg_end()?;
                let count = if op == 0x12 { c.byte()? as usize } else { self.int(c, f, host)? as usize };
                if count > 1 << 16 {
                    return Err(Error::Index);
                }
                let mut items = Vec::new();
                while c.pos < e {
                    let item = if c.peek().is_some_and(starts_name) {
                        // A name in a package: a reference to the object, not its value.
                        let name = c.name()?;
                        match self.resolve(f.scope, &name) {
                            Some(n) => Value::Reference(Reference::Node(n)),
                            // (As a table loads, settled once every table is; as a method runs, nothing.)
                            None if f.loading => Value::Reference(Reference::Unresolved(name.text(), f.scope)),
                            None => Value::Uninitialized,
                        }
                    } else {
                        self.eval(c, f, host)?
                    };
                    items.push(item);
                }
                c.pos = e;
                items.truncate(count.max(if op == 0x13 { 0 } else { count }));
                while items.len() < count {
                    items.push(Value::Uninitialized);
                }
                Value::package(items)
            }
            // (Read before anything was stored in it: an error, as ACPICA has it.)
            0x60..=0x6E => {
                let cell = if op < 0x68 { &f.locals[(op - 0x60) as usize] } else { &f.args[(op - 0x68) as usize] };
                match &*cell.borrow() {
                    Value::Uninitialized => return Err(Error::Uninitialized),
                    v => v.clone(),
                }
            }
            0x70 => {
                let v = self.eval(c, f, host)?;
                let t = self.target(c, f, host)?;
                self.store(&t, v.clone(), f, host)?;
                v
            }
            0x71 => {
                let t = self.target(c, f, host)?;
                Value::Reference(self.reference(&t, f)?)
            }
            0x72 | 0x74 | 0x77 | 0x79 | 0x7A | 0x7B | 0x7C | 0x7D | 0x7E | 0x7F | 0x85 => {
                let a = self.int(c, f, host)?;
                let b = self.int(c, f, host)?;
                let n = match op {
                    0x72 => a.wrapping_add(b),
                    0x74 => a.wrapping_sub(b),
                    0x77 => a.wrapping_mul(b),
                    0x79 => {
                        if b >= 64 { 0 } else { a << b }
                    }
                    0x7A => {
                        if b >= 64 { 0 } else { a >> b }
                    }
                    0x7B => a & b,
                    0x7C => !(a & b),
                    0x7D => a | b,
                    0x7E => !(a | b),
                    0x7F => a ^ b,
                    _ => {
                        if b == 0 {
                            return Err(Error::DivideByZero);
                        }
                        a % b
                    }
                };
                let n = Value::Integer(self.trim(n));
                let t = self.target(c, f, host)?;
                self.store(&t, n.clone(), f, host)?;
                n
            }
            0x73 => {
                let a = self.eval(c, f, host)?;
                let b = self.eval(c, f, host)?;
                let v = self.concat(&a, &b)?;
                let t = self.target(c, f, host)?;
                self.store(&t, v.clone(), f, host)?;
                v
            }
            0x84 => {
                // ConcatenateResTemplate: two resource templates, their end tags dropped, one end tag after.
                let a = self.eval(c, f, host)?;
                let b = self.eval(c, f, host)?;
                let mut x = self.to_buffer(&a)?;
                let y = self.to_buffer(&b)?;
                let strip = |v: &mut Vec<u8>| {
                    if v.len() >= 2 && v[v.len() - 2] == 0x79 {
                        v.truncate(v.len() - 2);
                    }
                };
                strip(&mut x);
                let mut y2 = y.clone();
                strip(&mut y2);
                x.extend_from_slice(&y2);
                x.extend_from_slice(&[0x79, 0]);
                let v = Value::buffer(x);
                let t = self.target(c, f, host)?;
                self.store(&t, v.clone(), f, host)?;
                v
            }
            0x75 | 0x76 => {
                let t = self.target(c, f, host)?;
                let old = self.target_value(&t, f, host)?;
                let n = self.to_integer(&old)?;
                let n = Value::Integer(self.trim(if op == 0x75 { n.wrapping_add(1) } else { n.wrapping_sub(1) }));
                self.store(&t, n.clone(), f, host)?;
                n
            }
            0x78 => {
                let a = self.int(c, f, host)?;
                let b = self.int(c, f, host)?;
                if b == 0 {
                    return Err(Error::DivideByZero);
                }
                let remainder = self.target(c, f, host)?;
                let quotient = self.target(c, f, host)?;
                self.store(&remainder, Value::Integer(a % b), f, host)?;
                self.store(&quotient, Value::Integer(a / b), f, host)?;
                Value::Integer(a / b)
            }
            0x80 => {
                let a = self.int(c, f, host)?;
                let n = Value::Integer(self.trim(!a));
                let t = self.target(c, f, host)?;
                self.store(&t, n.clone(), f, host)?;
                n
            }
            0x81 | 0x82 => {
                let a = self.int(c, f, host)?;
                let n = if a == 0 {
                    0
                } else if op == 0x81 {
                    64 - a.leading_zeros() as u64
                } else {
                    a.trailing_zeros() as u64 + 1
                };
                let t = self.target(c, f, host)?;
                self.store(&t, Value::Integer(n), f, host)?;
                Value::Integer(n)
            }
            0x83 => {
                let v = self.eval(c, f, host)?;
                match v {
                    Value::Reference(r) => self.deref_value(&r, host)?,
                    Value::String(name) => {
                        let n = self.find_from(f.scope, &name).ok_or(Error::NotFound(name))?;
                        self.value_of(n, Vec::new(), host)?
                    }
                    _ => return Err(Error::Type("a reference")),
                }
            }
            0x87 => {
                let t = self.target(c, f, host)?;
                let v = self.target_value(&t, f, host)?;
                let v = match v {
                    Value::Reference(r) => self.deref(&r)?,
                    v => v,
                };
                Value::Integer(match v {
                    Value::String(s) => s.len() as u64,
                    Value::Buffer(b) => b.borrow().len() as u64,
                    Value::Package(p) => p.borrow().len() as u64,
                    _ => return Err(Error::Type("a string, a buffer or a package")),
                })
            }
            0x88 => {
                let v = self.eval(c, f, host)?;
                let i = self.int(c, f, host)? as usize;
                let v = match v {
                    Value::Reference(r) => self.deref(&r)?,
                    v => v,
                };
                let r = match v {
                    Value::Package(p) => {
                        if i >= p.borrow().len() {
                            return Err(Error::Index);
                        }
                        Reference::Element(p, i)
                    }
                    Value::Buffer(b) => {
                        if i >= b.borrow().len() {
                            return Err(Error::Index);
                        }
                        Reference::Byte(b, i)
                    }
                    Value::String(s) => {
                        let byte = *s.as_bytes().get(i).ok_or(Error::Index)?;
                        Reference::Byte(Rc::new(RefCell::new(vec![byte])), 0)
                    }
                    _ => return Err(Error::Type("a package, a buffer or a string")),
                };
                let v = Value::Reference(r);
                let t = self.target(c, f, host)?;
                self.store(&t, v.clone(), f, host)?;
                v
            }
            0x89 => {
                let p = self.eval(c, f, host)?;
                let op1 = c.byte()?;
                let a = self.eval(c, f, host)?;
                let op2 = c.byte()?;
                let b = self.eval(c, f, host)?;
                let start = self.int(c, f, host)? as usize;
                let items = match p {
                    Value::Package(p) => p.borrow().clone(),
                    Value::Reference(r) => match self.deref(&r)? {
                        Value::Package(p) => p.borrow().clone(),
                        _ => return Err(Error::Type("a package")),
                    },
                    _ => return Err(Error::Type("a package")),
                };
                let mut found = self.ones();
                for (i, item) in items.iter().enumerate().skip(start) {
                    if self.matches(item, op1, &a)? && self.matches(item, op2, &b)? {
                        found = i as u64;
                        break;
                    }
                }
                Value::Integer(found)
            }
            0x8E => {
                let t = self.target_or_unresolved(c, f)?;
                let v = match t {
                    Some(t) => self.target_type(&t, f),
                    None => 0,
                };
                Value::Integer(v)
            }
            0x90 | 0x91 => {
                let a = self.int(c, f, host)?;
                let b = self.int(c, f, host)?;
                let yes = if op == 0x90 { a != 0 && b != 0 } else { a != 0 || b != 0 };
                self.boolean(yes)
            }
            0x92 => match c.peek() {
                Some(0x93 | 0x94 | 0x95) => {
                    let inner = c.byte()?;
                    let a = self.eval(c, f, host)?;
                    let b = self.eval(c, f, host)?;
                    let o = self.compare(&a, &b)?;
                    self.boolean(match inner {
                        0x93 => o != core::cmp::Ordering::Equal,
                        0x94 => o != core::cmp::Ordering::Greater,
                        _ => o != core::cmp::Ordering::Less,
                    })
                }
                _ => {
                    let a = self.int(c, f, host)?;
                    self.boolean(a == 0)
                }
            },
            0x93 | 0x94 | 0x95 => {
                let a = self.eval(c, f, host)?;
                let b = self.eval(c, f, host)?;
                let o = self.compare(&a, &b)?;
                self.boolean(match op {
                    0x93 => o == core::cmp::Ordering::Equal,
                    0x94 => o == core::cmp::Ordering::Greater,
                    _ => o == core::cmp::Ordering::Less,
                })
            }
            0x96 => {
                let a = self.eval(c, f, host)?;
                let v = Value::buffer(self.to_buffer(&a)?);
                let t = self.target(c, f, host)?;
                self.store(&t, v.clone(), f, host)?;
                v
            }
            0x97 | 0x98 => {
                let a = self.eval(c, f, host)?;
                let s = if op == 0x97 { self.decimal_string(&a)? } else { self.hex_string(&a)? };
                let v = Value::String(s);
                let t = self.target(c, f, host)?;
                self.store(&t, v.clone(), f, host)?;
                v
            }
            0x99 => {
                let a = self.eval(c, f, host)?;
                let v = Value::Integer(self.explicit_integer(&a)?);
                let t = self.target(c, f, host)?;
                self.store(&t, v.clone(), f, host)?;
                v
            }
            0x9C => {
                let a = self.eval(c, f, host)?;
                let length = self.int(c, f, host)?;
                let b = self.to_buffer(&a)?;
                let s: String = b.iter().take(length.min(b.len() as u64) as usize).take_while(|&&x| x != 0).map(|&x| x as char).collect();
                let v = Value::String(s);
                let t = self.target(c, f, host)?;
                self.store(&t, v.clone(), f, host)?;
                v
            }
            0x9D => {
                let v = self.eval(c, f, host)?;
                let t = self.target(c, f, host)?;
                self.copy_object(&t, copy(&v), f, host)?;
                v
            }
            0x9E => {
                let a = self.eval(c, f, host)?;
                let index = self.int(c, f, host)? as usize;
                let length = self.int(c, f, host)? as usize;
                let a = match a {
                    Value::Reference(r) => self.deref(&r)?,
                    a => a,
                };
                let v = match a {
                    Value::String(s) => {
                        let b = s.as_bytes();
                        let from = index.min(b.len());
                        let to = from.saturating_add(length).min(b.len());
                        Value::String(String::from_utf8_lossy(&b[from..to]).into_owned())
                    }
                    other => {
                        let b = self.to_buffer(&other)?;
                        let from = index.min(b.len());
                        let to = from.saturating_add(length).min(b.len());
                        Value::buffer(b[from..to].to_vec())
                    }
                };
                let t = self.target(c, f, host)?;
                self.store(&t, v.clone(), f, host)?;
                v
            }
            0x5B => {
                let ext = c.byte()?;
                match ext {
                    0x12 => {
                        let t = self.target_or_unresolved(c, f)?;
                        let target = self.target(c, f, host)?;
                        match t {
                            Some(t) => {
                                let r = self.reference(&t, f)?;
                                self.store(&target, Value::Reference(r), f, host)?;
                                Value::Integer(self.ones())
                            }
                            None => Value::Integer(0),
                        }
                    }
                    0x23 => {
                        // Acquire: a mutex is the method's at once (one processor runs AML here); the global lock
                        // is the firmware's too, so the host takes it (or times out: Ones).
                        let t = self.target(c, f, host)?;
                        let timeout = c.word()?;
                        if self.target_node(&t).is_some() && self.target_node(&t) == self.child(ROOT, *b"_GL_") {
                            if self.global_lock == 0 && !host.acquire_global_lock(timeout) {
                                Value::Integer(self.ones())
                            } else {
                                self.global_lock += 1;
                                Value::Integer(0)
                            }
                        } else {
                            Value::Integer(0)
                        }
                    }
                    0x25 => {
                        let t = self.target(c, f, host)?;
                        self.int(c, f, host)?;
                        let mut signalled = false;
                        if let Some(n) = self.target_node(&t)
                            && let Object::Event(count) = &mut self.nodes[n].object
                            && *count > 0
                        {
                            *count -= 1;
                            signalled = true;
                        }
                        Value::Integer(if signalled { 0 } else { self.ones() })
                    }
                    0x28 => {
                        let a = self.int(c, f, host)?;
                        let mut n = 0u64;
                        for i in (0..16).rev() {
                            n = n * 10 + ((a >> (4 * i)) & 0xF);
                        }
                        let v = Value::Integer(self.trim(n));
                        let t = self.target(c, f, host)?;
                        self.store(&t, v.clone(), f, host)?;
                        v
                    }
                    0x29 => {
                        let mut a = self.int(c, f, host)?;
                        let mut n = 0u64;
                        let mut shift = 0;
                        while a > 0 && shift < 64 {
                            n |= (a % 10) << shift;
                            a /= 10;
                            shift += 4;
                        }
                        let v = Value::Integer(n);
                        let t = self.target(c, f, host)?;
                        self.store(&t, v.clone(), f, host)?;
                        v
                    }
                    0x30 => Value::Integer(2),
                    0x33 => Value::Integer(host.timer()),
                    0x1F => {
                        let sig = self.text_arg(c, f, host)?;
                        let oem = self.text_arg(c, f, host)?;
                        let oem_table = self.text_arg(c, f, host)?;
                        let _root = self.eval(c, f, host)?;
                        let _param = self.eval(c, f, host)?;
                        let _data = self.eval(c, f, host)?;
                        let mut signature = [0u8; 4];
                        for (i, b) in sig.bytes().take(4).enumerate() {
                            signature[i] = b;
                        }
                        match host.table(&signature, oem.as_bytes(), oem_table.as_bytes()) {
                            Some(t) => {
                                let loaded = self.load_table(&t, host).is_ok();
                                self.settle_packages();
                                Value::Integer(loaded as u64)
                            }
                            None => Value::Integer(0),
                        }
                    }
                    _ => return Err(Error::Parse(at)),
                }
            }
            b if starts_name(b) => {
                c.pos = at;
                let name = c.name()?;
                let Some(node) = self.resolve(f.scope, &name) else { return Err(Error::NotFound(name.text())) };
                match self.nodes[node].object.clone() {
                    Object::Method(m) => {
                        let mut args = Vec::with_capacity(m.args as usize);
                        for _ in 0..m.args {
                            args.push(self.eval(c, f, host)?);
                        }
                        // A method that returns nothing, used as a value: an error, as ACPICA (and so Linux) has it.
                        match self.call(node, args, host)? {
                            Value::Uninitialized => return Err(Error::NoValue),
                            v => v,
                        }
                    }
                    Object::Osi => {
                        let arg = self.eval(c, f, host)?;
                        Value::Integer(self.osi(Some(&arg)))
                    }
                    _ => self.value_of(node, Vec::new(), host)?,
                }
            }
            _ => return Err(Error::Parse(at)),
        };
        Ok(v)
    }

    fn boolean(&self, yes: bool) -> Value {
        Value::Integer(if yes { self.ones() } else { 0 })
    }

    /// What a reference refers to, its value (a field read; a method isn't called, as ACPICA doesn't).
    fn deref_value(&mut self, r: &Reference, host: &mut dyn Host) -> Result<Value, Error> {
        match r {
            Reference::Node(n) if matches!(self.nodes[*n].object, Object::Method(_) | Object::Osi) => Err(Error::Type("data, not a method")),
            Reference::Node(n) => self.value_of(*n, Vec::new(), host),
            other => self.deref(other),
        }
    }

    /// A name given as a string (DerefOf of a path): from the current scope.
    pub(crate) fn find_from(&self, scope: usize, text: &str) -> Option<usize> {
        let root = text.starts_with('\\');
        let up = text.trim_start_matches('\\').chars().take_while(|&c| c == '^').count();
        let rest = text.trim_start_matches('\\').trim_start_matches('^');
        let mut segs = Vec::new();
        for part in rest.split('.').filter(|p| !p.is_empty()) {
            if part.len() > 4 {
                return None;
            }
            let mut s = [b'_'; 4];
            for (i, b) in part.bytes().enumerate() {
                s[i] = b.to_ascii_uppercase();
            }
            segs.push(s);
        }
        self.resolve(scope, &Name { root, up, segs })
    }

    /// Two values compared as the first one's type: integers, strings or buffers (byte by byte, the shorter first if
    /// it's the other's start).
    fn compare(&self, a: &Value, b: &Value) -> Result<core::cmp::Ordering, Error> {
        let a = match a {
            Value::Reference(r) => self.deref(r)?,
            a => a.clone(),
        };
        Ok(match &a {
            Value::String(s) => {
                let t = self.to_text(b)?;
                s.as_bytes().cmp(t.as_bytes())
            }
            Value::Buffer(x) => {
                let y = self.to_buffer(b)?;
                x.borrow().as_slice().cmp(y.as_slice())
            }
            other => {
                let x = self.to_integer(other)?;
                let y = self.to_integer(b)?;
                x.cmp(&y)
            }
        })
    }

    /// Concatenate: the result is the first operand's type (two integers make a buffer).
    fn concat(&self, a: &Value, b: &Value) -> Result<Value, Error> {
        let a = match a {
            Value::Reference(r) => self.deref(r)?,
            a => a.clone(),
        };
        Ok(match &a {
            Value::String(s) => Value::String(format!("{s}{}", self.to_text(b)?)),
            Value::Integer(_) => {
                let mut x = self.to_buffer(&a)?;
                let y = Value::Integer(self.to_integer(b)?);
                x.extend_from_slice(&self.to_buffer(&y)?);
                Value::buffer(x)
            }
            _ => {
                let mut x = self.to_buffer(&a)?;
                x.extend_from_slice(&self.to_buffer(b)?);
                Value::buffer(x)
            }
        })
    }

    /// Match's comparisons: 0 true, 1 equal, 2 less or equal, 3 less, 4 greater or equal, 5 greater (the package's
    /// element compared to the operand).
    fn matches(&self, item: &Value, op: u8, operand: &Value) -> Result<bool, Error> {
        if op == 0 {
            return Ok(true);
        }
        let item = match item {
            Value::Reference(r) => self.deref(r)?,
            i => i.clone(),
        };
        if !matches!(item, Value::Integer(_) | Value::String(_) | Value::Buffer(_)) {
            return Ok(false);
        }
        let o = self.compare(&item, operand)?;
        Ok(match op {
            1 => o.is_eq(),
            2 => o.is_le(),
            3 => o.is_lt(),
            4 => o.is_ge(),
            5 => o.is_gt(),
            _ => false,
        })
    }

    /// A target: a name, a local, an argument, Debug, a reference (DerefOf, Index, RefOf), or none.
    fn target(&mut self, c: &mut Cursor, f: &mut Frame, host: &mut dyn Host) -> Result<Target, Error> {
        let at = c.pos;
        let op = c.peek().ok_or(Error::Parse(at))?;
        Ok(match op {
            0x00 => {
                c.pos += 1;
                Target::Null
            }
            0x60..=0x67 => {
                c.pos += 1;
                Target::Local((op - 0x60) as usize)
            }
            0x68..=0x6E => {
                c.pos += 1;
                Target::Arg((op - 0x68) as usize)
            }
            0x5B if c.code.get(c.pos + 1) == Some(&0x31) => {
                c.pos += 2;
                Target::Debug
            }
            b if starts_name(b) => {
                let name = c.name()?;
                let node = self.resolve(f.scope, &name).ok_or_else(|| Error::NotFound(name.text()))?;
                if let Object::Method(m) = self.nodes[node].object.clone() {
                    // A method that returns a reference.
                    let mut args = Vec::new();
                    for _ in 0..m.args {
                        args.push(self.eval(c, f, host)?);
                    }
                    match self.call(node, args, host)? {
                        Value::Reference(r) => Target::Ref(r),
                        _ => return Err(Error::Type("a reference")),
                    }
                } else {
                    Target::Node(node)
                }
            }
            _ => match self.eval(c, f, host)? {
                Value::Reference(r) => Target::Ref(r),
                Value::String(name) => {
                    // DerefOf a path, as a target.
                    let n = self.find_from(f.scope, &name).ok_or(Error::NotFound(name))?;
                    Target::Node(n)
                }
                _ => return Err(Error::Type("a target")),
            },
        })
    }

    /// A target that may name nothing (CondRefOf, ObjectType): None if it doesn't.
    fn target_or_unresolved(&mut self, c: &mut Cursor, f: &mut Frame) -> Result<Option<Target>, Error> {
        match c.peek() {
            Some(b) if starts_name(b) => {
                let name = c.name()?;
                Ok(self.resolve(f.scope, &name).map(Target::Node))
            }
            Some(0x60..=0x67) => {
                let op = c.byte()?;
                Ok(Some(Target::Local((op - 0x60) as usize)))
            }
            Some(0x68..=0x6E) => {
                let op = c.byte()?;
                Ok(Some(Target::Arg((op - 0x68) as usize)))
            }
            _ => Err(Error::Parse(c.pos)),
        }
    }

    fn target_node(&self, t: &Target) -> Option<usize> {
        match t {
            Target::Node(n) => Some(*n),
            Target::Ref(Reference::Node(n)) => Some(*n),
            _ => None,
        }
    }

    fn target_type(&self, t: &Target, f: &Frame) -> u64 {
        match t {
            Target::Node(n) => self.type_of(&Value::Object(*n)),
            Target::Local(i) => self.type_of(&f.locals[*i].borrow()),
            Target::Arg(i) => self.type_of(&f.args[*i].borrow()),
            Target::Ref(r) => self.type_of(&Value::Reference(r.clone())),
            Target::Debug => 16,
            Target::Null => 0,
        }
    }

    /// A reference to a target (RefOf, CondRefOf).
    fn reference(&self, t: &Target, f: &Frame) -> Result<Reference, Error> {
        Ok(match t {
            Target::Node(n) => Reference::Node(*n),
            Target::Local(i) => Reference::Cell(f.locals[*i].clone()),
            Target::Arg(i) => match &*f.args[*i].borrow() {
                Value::Reference(r) => r.clone(),
                _ => Reference::Cell(f.args[*i].clone()),
            },
            Target::Ref(r) => r.clone(),
            _ => return Err(Error::Type("something a reference can be to")),
        })
    }

    /// A target's value (for Increment, Decrement, SizeOf).
    fn target_value(&mut self, t: &Target, f: &Frame, host: &mut dyn Host) -> Result<Value, Error> {
        match t {
            Target::Node(n) => self.value_of(*n, Vec::new(), host),
            Target::Local(i) => Ok(f.locals[*i].borrow().clone()),
            Target::Arg(i) => {
                let v = f.args[*i].borrow().clone();
                match v {
                    Value::Reference(r) => self.deref_value(&r, host),
                    v => Ok(v),
                }
            }
            Target::Ref(r) => self.deref_value(r, host),
            _ => Err(Error::Type("a value")),
        }
    }

    /// Stores a value in a target, as Store does: a copy, converted to a named object's type; through a reference an
    /// argument holds; into a field's bits.
    fn store(&mut self, t: &Target, v: Value, f: &mut Frame, host: &mut dyn Host) -> Result<(), Error> {
        let v = match v {
            // A reference is stored as a reference; what a field reads is stored as its value.
            Value::Object(n) if matches!(self.nodes[n].object, Object::Field(_) | Object::BufferField { .. }) => self.read_field(n, host)?,
            v => v,
        };
        match t {
            Target::Null => Ok(()),
            Target::Debug => {
                let text = self.debug_text(&v);
                host.debug(&text);
                Ok(())
            }
            Target::Local(i) => {
                *f.locals[*i].borrow_mut() = copy(&v);
                Ok(())
            }
            Target::Arg(i) => {
                let held = f.args[*i].borrow().clone();
                match held {
                    Value::Reference(r) => self.store_ref(&r, v, host),
                    _ => {
                        *f.args[*i].borrow_mut() = copy(&v);
                        Ok(())
                    }
                }
            }
            Target::Node(n) => self.store_node(*n, &v, host),
            Target::Ref(r) => self.store_ref(r, v, host),
        }
    }

    fn store_ref(&mut self, r: &Reference, v: Value, host: &mut dyn Host) -> Result<(), Error> {
        match r {
            Reference::Node(n) => self.store_node(*n, &v, host),
            Reference::Cell(cell) => {
                *cell.borrow_mut() = copy(&v);
                Ok(())
            }
            Reference::Element(p, i) => {
                let mut p = p.borrow_mut();
                let slot = p.get_mut(*i).ok_or(Error::Index)?;
                *slot = copy(&v);
                Ok(())
            }
            Reference::Byte(b, i) => {
                let n = self.to_integer(&v)?;
                let mut b = b.borrow_mut();
                let slot = b.get_mut(*i).ok_or(Error::Index)?;
                *slot = n as u8;
                Ok(())
            }
            Reference::Unresolved(name, _) => Err(Error::NotFound(name.clone())),
        }
    }

    /// CopyObject: the target replaced by the value, whatever its type was.
    fn copy_object(&mut self, t: &Target, v: Value, f: &mut Frame, host: &mut dyn Host) -> Result<(), Error> {
        match t {
            Target::Node(n) => {
                if matches!(self.nodes[*n].object, Object::Data(_)) {
                    self.nodes[*n].object = Object::Data(v);
                    Ok(())
                } else {
                    self.store_node(*n, &v, host)
                }
            }
            Target::Arg(i) => {
                *f.args[*i].borrow_mut() = v;
                Ok(())
            }
            other => self.store(other, v, f, host),
        }
    }

    /// What Store(x, Debug) shows.
    fn debug_text(&self, v: &Value) -> String {
        match v {
            Value::Integer(n) => format!("{n:#x}"),
            Value::String(s) => s.clone(),
            Value::Buffer(b) => format!("{:02x?}", b.borrow()),
            Value::Package(p) => {
                let items: Vec<String> = p.borrow().iter().map(|i| self.debug_text(i)).collect();
                format!("{{{}}}", items.join(", "))
            }
            Value::Reference(Reference::Node(n)) | Value::Object(n) => self.path(*n),
            Value::Reference(_) => String::from("(a reference)"),
            Value::Uninitialized => String::from("(nothing)"),
        }
    }
}
