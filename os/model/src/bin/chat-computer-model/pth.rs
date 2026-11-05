//! PyTorch's checkpoints (.pth, .pt), read without PyTorch: a zip of a pickle (`data.pkl`) and each tensor's bytes
//! (`data/KEY`). The pickle is read as data: only the opcodes torch.save writes are understood, and only the
//! callables that make dicts and tensors (`collections.OrderedDict`, `torch._utils._rebuild_tensor_v2` and
//! `_rebuild_parameter`, the `torch.*Storage` types) are known; anything else in it is refused, and nothing in it
//! is run.

use std::collections::HashMap;

#[derive(Clone, Debug)]
#[allow(dead_code)] // (Every kind the pickle can hold, read whether it's used or not.)
pub enum Value {
    None,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    Bytes(Vec<u8>),
    Tuple(Vec<Value>),
    List(Vec<Value>),
    Dict(Vec<(Value, Value)>),
    Global(String, String),
    Storage { key: String, kind: String },
    Tensor(Tensor),
    Mark,
}

/// A tensor: where its numbers are (its storage's entry, and its type), and how they're laid out (in elements).
#[derive(Clone, Debug)]
pub struct Tensor {
    pub key: String,
    pub kind: String,
    pub offset: usize,
    pub size: Vec<usize>,
    pub stride: Vec<usize>,
}

impl Tensor {
    pub fn elements(&self) -> usize {
        self.size.iter().product()
    }
}

pub struct Checkpoint<'a> {
    archive: zip::Archive<'a>,
    prefix: String,
    pub root: Value,
}

const MOST: usize = 4 << 30;

impl<'a> Checkpoint<'a> {
    pub fn open(bytes: &'a [u8]) -> Result<Checkpoint<'a>, String> {
        let archive = zip::Archive::open(bytes).map_err(|e| format!("not a PyTorch checkpoint (a zip): {e:?}"))?;
        let pkl = archive
            .entries()
            .iter()
            .find(|e| e.name.ends_with("/data.pkl") || e.name == "data.pkl")
            .ok_or("not a PyTorch checkpoint: no data.pkl")?;
        let prefix = pkl.name[..pkl.name.len() - "data.pkl".len()].to_string();
        let pickle = archive.read(pkl, 64 << 20).map_err(|e| format!("data.pkl: {e:?}"))?;
        let root = unpickle(&pickle)?;
        Ok(Checkpoint { archive, prefix, root })
    }

    /// Its tensors, by name (nested dicts' keys joined with dots).
    pub fn tensors(&self) -> Vec<(String, Tensor)> {
        let mut out = Vec::new();
        walk(&self.root, "", &mut out);
        out
    }

    /// A tensor's numbers, as f32s, in order (row-major).
    pub fn floats(&self, t: &Tensor) -> Result<Vec<f32>, String> {
        let name = format!("{}data/{}", self.prefix, t.key);
        let e = self.archive.find(&name).ok_or(format!("{name}: not in the checkpoint"))?;
        let raw = self.archive.read(e, MOST).map_err(|e| format!("{name}: {e:?}"))?;
        let (width, read): (usize, fn(&[u8]) -> f32) = match t.kind.as_str() {
            "FloatStorage" => (4, |b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])),
            "HalfStorage" => (2, |b| model::quant::f16_to_f32(u16::from_le_bytes([b[0], b[1]]))),
            "BFloat16Storage" => (2, |b| model::quant::bf16_to_f32(u16::from_le_bytes([b[0], b[1]]))),
            other => return Err(format!("{name}: a {other}, not numbers the converter reads")),
        };
        let n = t.elements();
        let mut out = Vec::with_capacity(n);
        let dims = t.size.len();
        let mut index = vec![0usize; dims];
        for _ in 0..n {
            let at = t.offset + index.iter().zip(&t.stride).map(|(i, s)| i * s).sum::<usize>();
            let b = raw.get(at * width..(at + 1) * width).ok_or(format!("{name}: shorter than its tensor"))?;
            out.push(read(b));
            for d in (0..dims).rev() {
                index[d] += 1;
                if index[d] < t.size[d] {
                    break;
                }
                index[d] = 0;
            }
        }
        Ok(out)
    }
}

fn walk(v: &Value, prefix: &str, out: &mut Vec<(String, Tensor)>) {
    match v {
        Value::Dict(items) => {
            for (k, v) in items {
                let key = match k {
                    Value::Str(s) => s.clone(),
                    Value::Int(i) => i.to_string(),
                    _ => continue,
                };
                walk(v, &if prefix.is_empty() { key } else { format!("{prefix}.{key}") }, out);
            }
        }
        Value::Tensor(t) => out.push((prefix.to_string(), t.clone())),
        _ => {}
    }
}

/// The pickle's value (protocols 2 to 4, as torch.save writes them).
pub fn unpickle(p: &[u8]) -> Result<Value, String> {
    let mut stack: Vec<Value> = Vec::new();
    let mut memo: HashMap<u32, Value> = HashMap::new();
    let mut i = 0usize;
    let bad = |what: &str| format!("the checkpoint's pickle: {what}");
    let take = |i: &mut usize, n: usize| -> Result<&[u8], String> {
        let s = p.get(*i..*i + n).ok_or("the checkpoint's pickle ends early")?;
        *i += n;
        Ok(s)
    };
    let u32_at = |b: &[u8]| u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
    let pop_mark = |stack: &mut Vec<Value>| -> Result<Vec<Value>, String> {
        let at = stack.iter().rposition(|v| matches!(v, Value::Mark)).ok_or("a mark missing")?;
        let items = stack.split_off(at + 1);
        stack.pop();
        Ok(items)
    };
    loop {
        let op = *p.get(i).ok_or(bad("no STOP"))?;
        i += 1;
        match op {
            0x80 => {
                take(&mut i, 1)?;
            }
            0x95 => {
                take(&mut i, 8)?;
            }
            b'.' => return stack.pop().ok_or(bad("nothing at its end")),
            b'}' => stack.push(Value::Dict(Vec::new())),
            b']' => stack.push(Value::List(Vec::new())),
            b')' => stack.push(Value::Tuple(Vec::new())),
            b'(' => stack.push(Value::Mark),
            b'N' => stack.push(Value::None),
            0x88 => stack.push(Value::Bool(true)),
            0x89 => stack.push(Value::Bool(false)),
            b'X' => {
                let n = u32_at(take(&mut i, 4)?) as usize;
                stack.push(Value::Str(String::from_utf8_lossy(take(&mut i, n)?).into_owned()));
            }
            0x8c => {
                let n = take(&mut i, 1)?[0] as usize;
                stack.push(Value::Str(String::from_utf8_lossy(take(&mut i, n)?).into_owned()));
            }
            b'T' | b'B' => {
                let n = u32_at(take(&mut i, 4)?) as usize;
                stack.push(Value::Bytes(take(&mut i, n)?.to_vec()));
            }
            b'U' | b'C' => {
                let n = take(&mut i, 1)?[0] as usize;
                stack.push(Value::Bytes(take(&mut i, n)?.to_vec()));
            }
            b'J' => {
                let b = take(&mut i, 4)?;
                stack.push(Value::Int(i32::from_le_bytes([b[0], b[1], b[2], b[3]]) as i64));
            }
            b'K' => stack.push(Value::Int(take(&mut i, 1)?[0] as i64)),
            b'M' => {
                let b = take(&mut i, 2)?;
                stack.push(Value::Int(u16::from_le_bytes([b[0], b[1]]) as i64));
            }
            0x8a => {
                let n = take(&mut i, 1)?[0] as usize;
                let b = take(&mut i, n)?;
                if n > 8 {
                    return Err(bad("a number too big"));
                }
                let mut v: i64 = 0;
                for (k, &byte) in b.iter().enumerate() {
                    v |= (byte as i64) << (8 * k);
                }
                if n > 0 && n < 8 && b[n - 1] & 0x80 != 0 {
                    v -= 1i64 << (8 * n);
                }
                stack.push(Value::Int(v));
            }
            b'G' => {
                let b = take(&mut i, 8)?;
                stack.push(Value::Float(f64::from_be_bytes(b.try_into().unwrap())));
            }
            b'c' => {
                let mut line = || -> Result<String, String> {
                    let end = p[i..].iter().position(|&b| b == b'\n').ok_or(bad("a global without its end"))?;
                    let s = String::from_utf8_lossy(&p[i..i + end]).into_owned();
                    i += end + 1;
                    Ok(s)
                };
                let (module, name) = (line()?, line()?);
                stack.push(Value::Global(module, name));
            }
            0x93 => {
                let name = stack.pop().ok_or(bad("STACK_GLOBAL without a name"))?;
                let module = stack.pop().ok_or(bad("STACK_GLOBAL without a module"))?;
                match (module, name) {
                    (Value::Str(m), Value::Str(n)) => stack.push(Value::Global(m, n)),
                    _ => return Err(bad("STACK_GLOBAL of something else")),
                }
            }
            b'q' => {
                let k = take(&mut i, 1)?[0] as u32;
                memo.insert(k, stack.last().cloned().ok_or(bad("BINPUT of nothing"))?);
            }
            b'r' => {
                let k = u32_at(take(&mut i, 4)?);
                memo.insert(k, stack.last().cloned().ok_or(bad("LONG_BINPUT of nothing"))?);
            }
            0x94 => {
                let k = memo.len() as u32;
                memo.insert(k, stack.last().cloned().ok_or(bad("MEMOIZE of nothing"))?);
            }
            b'h' => {
                let k = take(&mut i, 1)?[0] as u32;
                stack.push(memo.get(&k).cloned().ok_or(bad("BINGET of what wasn't kept"))?);
            }
            b'j' => {
                let k = u32_at(take(&mut i, 4)?);
                stack.push(memo.get(&k).cloned().ok_or(bad("LONG_BINGET of what wasn't kept"))?);
            }
            b't' => {
                let items = pop_mark(&mut stack)?;
                stack.push(Value::Tuple(items));
            }
            0x85..=0x87 => {
                let n = (op - 0x84) as usize;
                if stack.len() < n {
                    return Err(bad("a tuple of too little"));
                }
                let items = stack.split_off(stack.len() - n);
                stack.push(Value::Tuple(items));
            }
            b'Q' => {
                let pid = stack.pop().ok_or(bad("BINPERSID of nothing"))?;
                // ('storage', torch.FloatStorage, key, location, numel)
                let Value::Tuple(parts) = pid else { return Err(bad("a persistent id that isn't a tuple")) };
                match parts.as_slice() {
                    [Value::Str(tag), Value::Global(m, kind), Value::Str(key), ..] if tag == "storage" && m == "torch" => {
                        stack.push(Value::Storage { key: key.clone(), kind: kind.clone() });
                    }
                    _ => return Err(bad("a persistent id that isn't a storage")),
                }
            }
            b'R' | 0x81 => {
                let args = stack.pop().ok_or(bad("REDUCE without its arguments"))?;
                let f = stack.pop().ok_or(bad("REDUCE without what's called"))?;
                let Value::Tuple(args) = args else { return Err(bad("REDUCE's arguments aren't a tuple")) };
                let v = match f {
                    Value::Global(m, n) if m == "collections" && n == "OrderedDict" => Value::Dict(Vec::new()),
                    Value::Global(m, n) if m == "torch._utils" && n == "_rebuild_tensor_v2" => {
                        let ints = |v: &Value| -> Option<Vec<usize>> {
                            match v {
                                Value::Tuple(xs) | Value::List(xs) => xs
                                    .iter()
                                    .map(|x| if let Value::Int(i) = x { usize::try_from(*i).ok() } else { None })
                                    .collect(),
                                _ => None,
                            }
                        };
                        match args.as_slice() {
                            [Value::Storage { key, kind }, Value::Int(offset), size, stride, ..] => Value::Tensor(Tensor {
                                key: key.clone(),
                                kind: kind.clone(),
                                offset: usize::try_from(*offset).map_err(|_| bad("a negative offset"))?,
                                size: ints(size).ok_or(bad("a tensor's size"))?,
                                stride: ints(stride).ok_or(bad("a tensor's stride"))?,
                            }),
                            _ => return Err(bad("a tensor rebuilt from something else")),
                        }
                    }
                    Value::Global(m, n) if m == "torch._utils" && n == "_rebuild_parameter" => {
                        args.into_iter().next().ok_or(bad("a parameter of nothing"))?
                    }
                    Value::Global(m, n) => return Err(bad(&format!("{m}.{n}, which the converter doesn't call"))),
                    _ => return Err(bad("REDUCE of something not a global")),
                };
                stack.push(v);
            }
            b'b' => {
                // (An object's state, set after it's made: an OrderedDict's _metadata. Not needed.)
                stack.pop().ok_or(bad("BUILD without its state"))?;
            }
            b's' => {
                let v = stack.pop().ok_or(bad("SETITEM without its value"))?;
                let k = stack.pop().ok_or(bad("SETITEM without its key"))?;
                match stack.last_mut() {
                    Some(Value::Dict(items)) => items.push((k, v)),
                    _ => return Err(bad("SETITEM on something not a dict")),
                }
            }
            b'u' => {
                let items = pop_mark(&mut stack)?;
                match stack.last_mut() {
                    Some(Value::Dict(d)) => {
                        let mut it = items.into_iter();
                        while let (Some(k), Some(v)) = (it.next(), it.next()) {
                            d.push((k, v));
                        }
                    }
                    _ => return Err(bad("SETITEMS on something not a dict")),
                }
            }
            b'a' => {
                let v = stack.pop().ok_or(bad("APPEND of nothing"))?;
                match stack.last_mut() {
                    Some(Value::List(l)) => l.push(v),
                    _ => return Err(bad("APPEND to something not a list")),
                }
            }
            b'e' => {
                let items = pop_mark(&mut stack)?;
                match stack.last_mut() {
                    Some(Value::List(l)) => l.extend(items),
                    _ => return Err(bad("APPENDS to something not a list")),
                }
            }
            other => return Err(bad(&format!("opcode {other:#04x}, which the converter doesn't read"))),
        }
    }
}
