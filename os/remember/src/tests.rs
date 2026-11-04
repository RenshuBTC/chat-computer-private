extern crate std;

use super::*;
use std::vec::Vec;

/// A file on a "drive" of `sector` bytes a block, with a count of writes, and a way to stop writing partway (as if
/// the PC stopped).
struct File {
    bytes: Vec<u8>,
    sector: usize,
    writes: usize,
    stop_after: Option<usize>,
}

impl Store for File {
    fn sector_size(&self) -> usize {
        self.sector
    }
    fn sectors(&self) -> usize {
        self.bytes.len() / self.sector
    }
    fn read(&mut self, s: usize, buf: &mut [u8]) -> bool {
        buf.copy_from_slice(&self.bytes[s * self.sector..(s + 1) * self.sector]);
        true
    }
    fn write(&mut self, s: usize, buf: &[u8]) -> bool {
        if self.stop_after.is_some_and(|n| self.writes >= n) {
            return false;
        }
        self.writes += 1;
        self.bytes[s * self.sector..(s + 1) * self.sector].copy_from_slice(buf);
        true
    }
    fn flush(&mut self) -> bool {
        true
    }
}

fn data(len: usize, seed: u8) -> Vec<u8> {
    (0..len).map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed)).collect()
}

#[test]
fn what_is_kept_comes_back_newest_first_in_either_half() {
    for sector in [512, 4096] {
        let mut f = File { bytes: blank(0xC0FFEE, 64 * 1024), sector, writes: 0, stop_after: None };
        let m = Memory::new(&f, 0xC0FFEE).unwrap();
        assert_eq!(m.load(&mut f), Ok(None), "a new file holds nothing");
        let one = data(10_000, 1);
        m.save(&mut f, &one).unwrap();
        assert_eq!(m.load(&mut f).unwrap(), Some((0, 1, one.clone())));
        let two = data(20_000, 2);
        m.save(&mut f, &two).unwrap();
        assert_eq!(m.load(&mut f).unwrap(), Some((1, 2, two.clone())));
        let three = data(5, 3);
        m.save(&mut f, &three).unwrap();
        assert_eq!(m.load(&mut f).unwrap(), Some((0, 3, three)));
        // Every chunk still has its tag.
        for (i, c) in f.bytes.chunks_exact(CHUNK).enumerate() {
            assert_eq!(c[..TAG], tag(0xC0FFEE, i as u32), "chunk {i}");
        }
    }
}

#[test]
fn a_write_cut_short_leaves_the_last_whole_copy() {
    let mut f = File { bytes: blank(7, 64 * 1024), sector: 512, writes: 0, stop_after: None };
    let m = Memory::new(&f, 7).unwrap();
    let old = data(3000, 9);
    m.save(&mut f, &old).unwrap();
    // The next save stops after two sectors: its header (written last) never is.
    f.stop_after = Some(f.writes + 2);
    assert_eq!(m.save(&mut f, &data(9000, 4)), Err(Error::Write));
    f.stop_after = None;
    assert_eq!(m.load(&mut f).unwrap(), Some((0, 1, old)));
}

#[test]
fn a_sector_that_isnt_this_files_is_never_written() {
    let mut f = File { bytes: blank(1, 16 * 1024), sector: 512, writes: 0, stop_after: None };
    // Another file's id: nothing may be written, nothing read as kept.
    let m = Memory::new(&f, 2).unwrap();
    assert_eq!(m.save(&mut f, b"hello"), Err(Error::NotOurs));
    assert_eq!(f.writes, 0);
    // A sector out of place (as if the file's blocks were found wrong): refused too.
    let mut g = File { bytes: blank(1, 16 * 1024), sector: 512, writes: 0, stop_after: None };
    g.bytes.copy_within(0..512, 512 * 3);
    let m = Memory::new(&g, 1).unwrap();
    assert_eq!(m.save(&mut g, &data(2000, 1)), Err(Error::NotOurs));
    assert_eq!(g.writes, 0);
}

#[test]
fn too_much_is_refused_and_a_damaged_copy_isnt_believed() {
    let mut f = File { bytes: blank(5, 8 * 1024), sector: 512, writes: 0, stop_after: None };
    let m = Memory::new(&f, 5).unwrap();
    assert_eq!(m.save(&mut f, &data(m.capacity() + 1, 0)), Err(Error::TooBig));
    m.save(&mut f, &data(m.capacity(), 0)).unwrap();
    assert!(m.load(&mut f).unwrap().is_some());
    // A byte of data changed: the copy's hash doesn't match.
    f.bytes[3 * CHUNK + TAG + 10] ^= 1;
    assert_eq!(m.load(&mut f), Ok(None));
    assert_eq!(Memory::new(&File { bytes: blank(5, 512), sector: 512, writes: 0, stop_after: None }, 5).err(), Some(Error::Shape));
}
