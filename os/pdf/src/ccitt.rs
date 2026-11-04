//! CCITT's fax compression (ITU-T T.4 and T.6), as PDF's CCITTFaxDecode filter has it (ISO 32000-1, 7.4.6): Group 4
//! (each line coded by how it differs from the one above), Group 3's one-dimensional lines (runs of white and black)
//! and its mixed lines, with or without the end-of-line codes and their fill. What many office scanners write a
//! black-and-white page in.
//!
//! The result is PDF's: a bit a pixel, each line its own bytes, a black pixel 0 unless BlackIs1 says 1. A damaged
//! line ends the picture there (the lines decoded so far are kept, the rest white).

use alloc::vec;
use alloc::vec::Vec;

/// The filter's parameters (its DecodeParms).
#[derive(Clone, Copy, Debug)]
pub struct Params {
    /// Below 0: Group 4; 0: Group 3's one-dimensional lines; above: Group 3's mixed lines.
    pub k: i64,
    pub columns: usize,
    /// How many lines, if it says (0: until its end). (End-of-line codes are passed over wherever they are, whether
    /// or not its EndOfLine says they're there.)
    pub rows: usize,
    pub byte_align: bool,
    pub black_is_1: bool,
}

/// What the codes for runs mean: (bits, code, run).
const WHITE: &[(u8, u16, u16)] = &[
    (8, 0b00110101, 0), (6, 0b000111, 1), (4, 0b0111, 2), (4, 0b1000, 3), (4, 0b1011, 4), (4, 0b1100, 5),
    (4, 0b1110, 6), (4, 0b1111, 7), (5, 0b10011, 8), (5, 0b10100, 9), (5, 0b00111, 10), (5, 0b01000, 11),
    (6, 0b001000, 12), (6, 0b000011, 13), (6, 0b110100, 14), (6, 0b110101, 15), (6, 0b101010, 16),
    (6, 0b101011, 17), (7, 0b0100111, 18), (7, 0b0001100, 19), (7, 0b0001000, 20), (7, 0b0010111, 21),
    (7, 0b0000011, 22), (7, 0b0000100, 23), (7, 0b0101000, 24), (7, 0b0101011, 25), (7, 0b0010011, 26),
    (7, 0b0100100, 27), (7, 0b0011000, 28), (8, 0b00000010, 29), (8, 0b00000011, 30), (8, 0b00011010, 31),
    (8, 0b00011011, 32), (8, 0b00010010, 33), (8, 0b00010011, 34), (8, 0b00010100, 35), (8, 0b00010101, 36),
    (8, 0b00010110, 37), (8, 0b00010111, 38), (8, 0b00101000, 39), (8, 0b00101001, 40), (8, 0b00101010, 41),
    (8, 0b00101011, 42), (8, 0b00101100, 43), (8, 0b00101101, 44), (8, 0b00000100, 45), (8, 0b00000101, 46),
    (8, 0b00001010, 47), (8, 0b00001011, 48), (8, 0b01010010, 49), (8, 0b01010011, 50), (8, 0b01010100, 51),
    (8, 0b01010101, 52), (8, 0b00100100, 53), (8, 0b00100101, 54), (8, 0b01011000, 55), (8, 0b01011001, 56),
    (8, 0b01011010, 57), (8, 0b01011011, 58), (8, 0b01001010, 59), (8, 0b01001011, 60), (8, 0b00110010, 61),
    (8, 0b00110011, 62), (8, 0b00110100, 63),
    (5, 0b11011, 64), (5, 0b10010, 128), (6, 0b010111, 192), (7, 0b0110111, 256), (8, 0b00110110, 320),
    (8, 0b00110111, 384), (8, 0b01100100, 448), (8, 0b01100101, 512), (8, 0b01101000, 576), (8, 0b01100111, 640),
    (9, 0b011001100, 704), (9, 0b011001101, 768), (9, 0b011010010, 832), (9, 0b011010011, 896),
    (9, 0b011010100, 960), (9, 0b011010101, 1024), (9, 0b011010110, 1088), (9, 0b011010111, 1152),
    (9, 0b011011000, 1216), (9, 0b011011001, 1280), (9, 0b011011010, 1344), (9, 0b011011011, 1408),
    (9, 0b010011000, 1472), (9, 0b010011001, 1536), (9, 0b010011010, 1600), (6, 0b011000, 1664),
    (9, 0b010011011, 1728),
];

const BLACK: &[(u8, u16, u16)] = &[
    (10, 0b0000110111, 0), (3, 0b010, 1), (2, 0b11, 2), (2, 0b10, 3), (3, 0b011, 4), (4, 0b0011, 5), (4, 0b0010, 6),
    (5, 0b00011, 7), (6, 0b000101, 8), (6, 0b000100, 9), (7, 0b0000100, 10), (7, 0b0000101, 11), (7, 0b0000111, 12),
    (8, 0b00000100, 13), (8, 0b00000111, 14), (9, 0b000011000, 15), (10, 0b0000010111, 16), (10, 0b0000011000, 17),
    (10, 0b0000001000, 18), (11, 0b00001100111, 19), (11, 0b00001101000, 20), (11, 0b00001101100, 21),
    (11, 0b00000110111, 22), (11, 0b00000101000, 23), (11, 0b00000010111, 24), (11, 0b00000011000, 25),
    (12, 0b000011001010, 26), (12, 0b000011001011, 27), (12, 0b000011001100, 28), (12, 0b000011001101, 29),
    (12, 0b000001101000, 30), (12, 0b000001101001, 31), (12, 0b000001101010, 32), (12, 0b000001101011, 33),
    (12, 0b000011010010, 34), (12, 0b000011010011, 35), (12, 0b000011010100, 36), (12, 0b000011010101, 37),
    (12, 0b000011010110, 38), (12, 0b000011010111, 39), (12, 0b000001101100, 40), (12, 0b000001101101, 41),
    (12, 0b000011011010, 42), (12, 0b000011011011, 43), (12, 0b000001010100, 44), (12, 0b000001010101, 45),
    (12, 0b000001010110, 46), (12, 0b000001010111, 47), (12, 0b000001100100, 48), (12, 0b000001100101, 49),
    (12, 0b000001010010, 50), (12, 0b000001010011, 51), (12, 0b000000100100, 52), (12, 0b000000110111, 53),
    (12, 0b000000111000, 54), (12, 0b000000100111, 55), (12, 0b000000101000, 56), (12, 0b000001011000, 57),
    (12, 0b000001011001, 58), (12, 0b000000101011, 59), (12, 0b000000101100, 60), (12, 0b000001011010, 61),
    (12, 0b000001100110, 62), (12, 0b000001100111, 63),
    (10, 0b0000001111, 64), (12, 0b000011001000, 128), (12, 0b000011001001, 192), (12, 0b000001011011, 256),
    (12, 0b000000110011, 320), (12, 0b000000110100, 384), (12, 0b000000110101, 448), (13, 0b0000001101100, 512),
    (13, 0b0000001101101, 576), (13, 0b0000001001010, 640), (13, 0b0000001001011, 704), (13, 0b0000001001100, 768),
    (13, 0b0000001001101, 832), (13, 0b0000001110010, 896), (13, 0b0000001110011, 960), (13, 0b0000001110100, 1024),
    (13, 0b0000001110101, 1088), (13, 0b0000001110110, 1152), (13, 0b0000001110111, 1216),
    (13, 0b0000001010010, 1280), (13, 0b0000001010011, 1344), (13, 0b0000001010100, 1408),
    (13, 0b0000001010101, 1472), (13, 0b0000001011010, 1536), (13, 0b0000001011011, 1600),
    (13, 0b0000001100100, 1664), (13, 0b0000001100101, 1728),
];

/// The makeup codes for the longest runs, white's and black's alike.
const LONGER: &[(u8, u16, u16)] = &[
    (11, 0b00000001000, 1792), (11, 0b00000001100, 1856), (11, 0b00000001101, 1920), (12, 0b000000010010, 1984),
    (12, 0b000000010011, 2048), (12, 0b000000010100, 2112), (12, 0b000000010101, 2176), (12, 0b000000010110, 2240),
    (12, 0b000000010111, 2304), (12, 0b000000011100, 2368), (12, 0b000000011101, 2432), (12, 0b000000011110, 2496),
    (12, 0b000000011111, 2560),
];

/// The most bits a run's code has, and a table of every code by its first that many bits: its length and run (a
/// length of 0: not a code).
const PEEK: u32 = 13;
type Table = Vec<(u8, u16)>;

fn table(codes: &[&[(u8, u16, u16)]]) -> Table {
    let mut t = vec![(0u8, 0u16); 1 << PEEK];
    for set in codes {
        for &(bits, code, run) in set.iter() {
            let shift = PEEK - bits as u32;
            let first = (code as usize) << shift;
            for e in &mut t[first..first + (1 << shift)] {
                *e = (bits, run);
            }
        }
    }
    t
}

/// A two-dimensional line's codes for each change: pass, horizontal, vertical (by how far), or an end of line.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Mode {
    Pass,
    Horizontal,
    Vertical(isize),
    EndOfLine,
}

struct Bits<'a> {
    data: &'a [u8],
    at: usize,
}

impl Bits<'_> {
    /// The next `n` bits (at most 16), as a number, zeros past the end.
    fn peek(&self, n: u32) -> u32 {
        let mut v = 0u32;
        for i in 0..n as usize {
            let bit = self.at + i;
            let b = self.data.get(bit / 8).map_or(0, |&b| (b >> (7 - bit % 8)) & 1);
            v = (v << 1) | b as u32;
        }
        v
    }

    fn take(&mut self, n: u32) {
        self.at += n as usize;
    }

    fn ended(&self) -> bool {
        self.at >= self.data.len() * 8
    }

    fn align(&mut self) {
        self.at = self.at.div_ceil(8) * 8;
    }

    /// Fill (zeros) and an end-of-line code, passed over if they're next: whether there was one.
    fn end_of_line(&mut self) -> bool {
        while !self.ended() && self.peek(12) == 0 {
            self.take(1);
        }
        if self.peek(12) == 1 {
            self.take(12);
            return true;
        }
        false
    }
}

struct Decoder<'a> {
    bits: Bits<'a>,
    columns: usize,
    white: Table,
    black: Table,
}

impl Decoder<'_> {
    /// A run of white or of black: its makeup codes, then the code that ends it.
    fn run(&mut self, white: bool) -> Option<usize> {
        let mut total = 0usize;
        loop {
            let (bits, run) = (if white { &self.white } else { &self.black })[self.bits.peek(PEEK) as usize];
            if bits == 0 || self.bits.ended() {
                return None;
            }
            self.bits.take(bits as u32);
            total += run as usize;
            if run < 64 {
                return Some(total);
            }
            if total > self.columns + 2560 {
                return None;
            }
        }
    }

    fn mode(&mut self) -> Option<Mode> {
        if self.bits.ended() {
            return None;
        }
        let v = self.bits.peek(7);
        let (bits, mode) = if v >> 6 == 1 {
            (1, Mode::Vertical(0))
        } else if v >> 4 == 0b011 {
            (3, Mode::Vertical(1))
        } else if v >> 4 == 0b010 {
            (3, Mode::Vertical(-1))
        } else if v >> 4 == 0b001 {
            (3, Mode::Horizontal)
        } else if v >> 3 == 0b0001 {
            (4, Mode::Pass)
        } else if v >> 1 == 0b000011 {
            (6, Mode::Vertical(2))
        } else if v >> 1 == 0b000010 {
            (6, Mode::Vertical(-2))
        } else if v == 0b0000011 {
            (7, Mode::Vertical(3))
        } else if v == 0b0000010 {
            (7, Mode::Vertical(-3))
        } else if self.bits.peek(12) == 1 {
            (12, Mode::EndOfLine)
        } else {
            // (The extensions, such as uncompressed mode: not in PDFs that scanners write.)
            return None;
        };
        self.bits.take(bits);
        Some(mode)
    }

    /// A line of runs: where each run ends (the colour changing there, white first).
    fn line_1d(&mut self, out: &mut Vec<usize>) -> Option<()> {
        out.clear();
        let (mut at, mut white) = (0usize, true);
        while at < self.columns {
            at = (at + self.run(white)?).min(self.columns);
            out.push(at);
            white = !white;
        }
        Some(())
    }

    /// A line coded by how it differs from the one above (its changes, `above`): its changes.
    fn line_2d(&mut self, above: &[usize], out: &mut Vec<usize>) -> Option<bool> {
        out.clear();
        let cols = self.columns;
        let (mut a0, mut white) = (-1isize, true);
        let mut from = 0usize;
        while a0 < cols as isize {
            // b1: the first change above, to the right of a0, to the other colour (to black, an even one, from
            // white); b2: the change after it.
            let mut i = from;
            while i < above.len() && above[i] as isize <= a0 {
                i += 1;
            }
            from = i;
            if (i % 2 == 0) != white {
                i += 1;
            }
            let b1 = above.get(i).copied().unwrap_or(cols).min(cols);
            let b2 = above.get(i + 1).copied().unwrap_or(cols).min(cols);
            match self.mode()? {
                Mode::Pass => a0 = b2 as isize,
                Mode::Horizontal => {
                    let start = a0.max(0) as usize;
                    let a1 = (start + self.run(white)?).min(cols);
                    let a2 = (a1 + self.run(!white)?).min(cols);
                    out.push(a1);
                    out.push(a2);
                    a0 = a2 as isize;
                }
                Mode::Vertical(d) => {
                    // (Kept within the line and past a0, as other readers keep it, for coders that stray.)
                    let a1 = (b1 as isize + d).clamp(a0.max(0), cols as isize);
                    out.push(a1 as usize);
                    a0 = a1;
                    white = !white;
                }
                // (In a line's middle: the end of the picture, or a damaged line.)
                Mode::EndOfLine => return Some(false),
            }
        }
        Some(true)
    }
}

/// The picture's lines (`rows` of them, if the parameters don't say: the picture's height), as PDF's filter gives
/// them. None if not even a line could be read.
pub fn decode(data: &[u8], p: &Params, rows: usize) -> Option<Vec<u8>> {
    let columns = p.columns.clamp(1, 1 << 16);
    let rows = if p.rows > 0 { p.rows } else { rows }.clamp(1, 1 << 16);
    let mut d = Decoder { bits: Bits { data, at: 0 }, columns, white: table(&[WHITE, LONGER]), black: table(&[BLACK, LONGER]) };
    let stride = columns.div_ceil(8);
    // (White, as the filter has it.)
    let mut out = vec![if p.black_is_1 { 0u8 } else { 0xFF }; stride * rows];
    let mut above: Vec<usize> = Vec::new();
    let mut line: Vec<usize> = Vec::new();
    let mut done = 0;
    while done < rows {
        let ok = if p.k < 0 {
            if p.byte_align {
                d.bits.align();
            }
            match d.line_2d(&above, &mut line) {
                Some(true) => true,
                _ => break,
            }
        } else {
            if p.byte_align {
                // (Each line's code then ends on a byte's end: the fill before it is passed over with it.)
                d.bits.end_of_line();
            } else if d.bits.end_of_line() && d.bits.peek(12) == 1 {
                // (Two in a row: the end of the picture.)
                break;
            }
            let one = p.k == 0 || {
                let tag = d.bits.peek(1) == 1;
                d.bits.take(1);
                tag
            };
            if one { d.line_1d(&mut line).is_some() } else { matches!(d.line_2d(&above, &mut line), Some(true)) }
        };
        if !ok || d.bits.at > data.len() * 8 {
            break;
        }
        // The line's black runs, as bits.
        let row = &mut out[done * stride..(done + 1) * stride];
        let mut start = 0usize;
        let mut white = true;
        for &change in &line {
            let end = change.min(columns);
            if !white {
                for x in start..end {
                    if p.black_is_1 {
                        row[x / 8] |= 0x80 >> (x % 8);
                    } else {
                        row[x / 8] &= !(0x80 >> (x % 8));
                    }
                }
            }
            start = end;
            white = !white;
        }
        core::mem::swap(&mut above, &mut line);
        done += 1;
    }
    (done > 0).then_some(out)
}
