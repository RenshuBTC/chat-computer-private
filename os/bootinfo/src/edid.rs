//! The monitor's description of itself (VESA EDID 1.3/1.4): its best resolution and its real size.

pub struct Edid<'a> {
    bytes: &'a [u8],
}

const HEADER: [u8; 8] = [0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00];

/// The first 128-byte block, if it is well formed.
pub fn parse(bytes: &[u8]) -> Option<Edid<'_>> {
    let block = bytes.get(..128)?;
    if block[..8] != HEADER {
        return None;
    }
    let sum = block.iter().fold(0u8, |a, b| a.wrapping_add(*b));
    if sum != 0 {
        return None;
    }
    Some(Edid { bytes: block })
}

impl Edid<'_> {
    /// The first detailed timing: the monitor's preferred (native) resolution.
    pub fn preferred_mode(&self) -> Option<(u32, u32)> {
        let d = &self.bytes[54..72];
        if d[0] == 0 && d[1] == 0 {
            return None; // not a timing (a text descriptor)
        }
        let w = d[2] as u32 | ((d[4] as u32 & 0xF0) << 4);
        let h = d[5] as u32 | ((d[7] as u32 & 0xF0) << 4);
        if w == 0 || h == 0 { None } else { Some((w, h)) }
    }

    /// The picture's size in millimetres: from the detailed timing (precise), or else the rough size
    /// in centimetres from the basic parameters. `None` if the monitor doesn't say (projectors, some TVs).
    pub fn size_mm(&self) -> Option<(u32, u32)> {
        let d = &self.bytes[54..72];
        if d[0] != 0 || d[1] != 0 {
            let w = d[12] as u32 | ((d[14] as u32 & 0xF0) << 4);
            let h = d[13] as u32 | ((d[14] as u32 & 0x0F) << 8);
            if w > 0 && h > 0 {
                return Some((w, h));
            }
        }
        let (w, h) = (self.bytes[21] as u32, self.bytes[22] as u32);
        if w > 0 && h > 0 { Some((w * 10, h * 10)) } else { None }
    }
}
