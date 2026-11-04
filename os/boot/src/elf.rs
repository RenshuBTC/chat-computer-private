//! Reads the kernel, which is carried inside the start-up stage as a 64-bit ELF executable.

pub struct Elf<'a> {
    data: &'a [u8],
    pub entry: u64,
    phoff: usize,
    phentsize: usize,
    phnum: usize,
}

/// One part of the kernel to copy into memory (an ELF PT_LOAD program header).
pub struct Load {
    pub vaddr: u64,
    pub memsz: u64,
    pub offset: u64,
    pub filesz: u64,
    pub flags: u32,
}

pub const PF_X: u32 = 1;
pub const PF_W: u32 = 2;

/// A part's end is rounded up to a whole page, which must not wrap around the address space.
const PAGE: u64 = 4096;

fn u16_at(d: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_le_bytes(d.get(o..o + 2)?.try_into().ok()?))
}
fn u32_at(d: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_le_bytes(d.get(o..o + 4)?.try_into().ok()?))
}
fn u64_at(d: &[u8], o: usize) -> Option<u64> {
    Some(u64::from_le_bytes(d.get(o..o + 8)?.try_into().ok()?))
}

impl<'a> Elf<'a> {
    pub fn parse(data: &'a [u8]) -> Result<Elf<'a>, &'static str> {
        const BAD: &str = "the kernel inside the start-up stage is damaged";
        if data.get(..4) != Some(b"\x7fELF".as_slice()) {
            return Err(BAD);
        }
        // 64-bit, little-endian, an executable, for x86-64.
        if data[4] != 2 || data[5] != 1 || u16_at(data, 16) != Some(2) || u16_at(data, 18) != Some(62) {
            return Err("the kernel inside the start-up stage isn't a 64-bit x86 executable");
        }
        let entry = u64_at(data, 24).ok_or(BAD)?;
        let phoff = u64_at(data, 32).ok_or(BAD)? as usize;
        let phentsize = u16_at(data, 54).ok_or(BAD)? as usize;
        let phnum = u16_at(data, 56).ok_or(BAD)? as usize;
        if phentsize < 56 || phoff.checked_add(phentsize * phnum).is_none_or(|end| end > data.len()) {
            return Err(BAD);
        }
        Ok(Elf { data, entry, phoff, phentsize, phnum })
    }

    pub fn loads(&self) -> impl Iterator<Item = Result<Load, &'static str>> + '_ {
        (0..self.phnum).filter_map(move |i| {
            let p = self.phoff + i * self.phentsize;
            if u32_at(self.data, p) != Some(1) {
                return None; // not PT_LOAD
            }
            let load = (|| {
                Some(Load {
                    flags: u32_at(self.data, p + 4)?,
                    offset: u64_at(self.data, p + 8)?,
                    vaddr: u64_at(self.data, p + 16)?,
                    filesz: u64_at(self.data, p + 32)?,
                    memsz: u64_at(self.data, p + 40)?,
                })
            })();
            let fits = |l: &Load| {
                l.filesz <= l.memsz
                    && l.offset.checked_add(l.filesz).is_some_and(|end| end <= self.data.len() as u64)
                    && l.vaddr.checked_add(l.memsz).is_some_and(|end| end.checked_add(PAGE).is_some())
            };
            Some(match load {
                Some(l) if fits(&l) => Ok(l),
                _ => Err("the kernel inside the start-up stage is damaged"),
            })
        })
    }

    pub fn bytes(&self, load: &Load) -> &'a [u8] {
        &self.data[load.offset as usize..(load.offset + load.filesz) as usize]
    }
}
