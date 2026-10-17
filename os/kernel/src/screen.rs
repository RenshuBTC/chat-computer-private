//! The screen: the framebuffer the firmware set up, drawn on directly. ClaudeOS draws black, white,
//! and the greys between them that smooth the edges of letters. Nothing else.

use crate::paging::{self, Cache};
use bootinfo::{Framebuffer, PIXEL_BGR, PIXEL_BITMASK, PIXEL_RGB};

pub struct Screen {
    base: *mut u32,
    pub width: usize,
    pub height: usize,
    stride: usize,
    /// The pixel value for each grey level, 0 = black to 255 = white.
    grey: [u32; 256],
}

unsafe impl Send for Screen {}

fn channel(mask: u32, level: u8) -> u32 {
    if mask == 0 {
        return 0;
    }
    let shift = mask.trailing_zeros();
    let bits = (mask >> shift).count_ones().min(16);
    let max = (1u32 << bits) - 1;
    ((level as u32 * max + 127) / 255) << shift & mask
}

impl Screen {
    pub fn new(fb: &Framebuffer) -> Option<Screen> {
        if fb.phys == 0 || fb.width == 0 || fb.height == 0 || fb.stride < fb.width {
            return None;
        }
        let mut grey = [0u32; 256];
        for (level, v) in grey.iter_mut().enumerate() {
            let l = level as u32;
            *v = match fb.format {
                PIXEL_RGB | PIXEL_BGR => l << 16 | l << 8 | l,
                PIXEL_BITMASK => {
                    channel(fb.red_mask, level as u8) | channel(fb.green_mask, level as u8) | channel(fb.blue_mask, level as u8)
                }
                _ => return None,
            };
        }
        let bytes = fb.stride as u64 * fb.height as u64 * 4;
        let base = paging::map_device(fb.phys, bytes, Cache::WriteCombining) as *mut u32;
        Some(Screen { base, width: fb.width as usize, height: fb.height as usize, stride: fb.stride as usize, grey })
    }

    /// Fills a rectangle with one grey level (clipped to the screen).
    pub fn fill(&mut self, x: usize, y: usize, w: usize, h: usize, level: u8) {
        let v = self.grey[level as usize];
        for row in y.min(self.height)..(y + h).min(self.height) {
            let line = unsafe { self.base.add(row * self.stride) };
            for col in x.min(self.width)..(x + w).min(self.width) {
                unsafe { line.add(col).write(v) };
            }
        }
    }

    /// Draws ink coverage (0 = paper, 255 = full ink) as black on white, with its top-left corner at
    /// (`x`, `y`), clipped to the screen.
    pub fn draw_ink(&mut self, x: isize, y: isize, w: usize, h: usize, ink: &[u8]) {
        for row in 0..h {
            let sy = y + row as isize;
            if sy < 0 || sy >= self.height as isize {
                continue;
            }
            let line = unsafe { self.base.add(sy as usize * self.stride) };
            for col in 0..w {
                let sx = x + col as isize;
                if sx < 0 || sx >= self.width as isize {
                    continue;
                }
                let v = self.grey[255 - ink[row * w + col] as usize];
                unsafe { line.add(sx as usize).write(v) };
            }
        }
    }
}
