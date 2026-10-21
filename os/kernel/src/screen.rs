//! The screen: the framebuffer the firmware set up. Chat Computer draws a picture of the whole screen in memory
//! (draw.rs) and copies the parts that changed here, converting its colours (0xRRGGBB) to the framebuffer's
//! pixel layout on the way.

use crate::paging::{self, Cache};
use bootinfo::{Framebuffer, PIXEL_BGR, PIXEL_BITMASK, PIXEL_RGB};

pub struct Screen {
    base: *mut u32,
    pub width: usize,
    pub height: usize,
    stride: usize,
    format: u32,
    masks: [u32; 3],
}

unsafe impl Send for Screen {}

/// An 8-bit channel value placed in `mask`'s bits.
fn channel(mask: u32, level: u32) -> u32 {
    if mask == 0 {
        return 0;
    }
    let shift = mask.trailing_zeros();
    let bits = (mask >> shift).count_ones().min(16);
    let max = (1u32 << bits) - 1;
    ((level * max + 127) / 255) << shift & mask
}

impl Screen {
    pub fn new(fb: &Framebuffer) -> Option<Screen> {
        let bytes = fb.stride as u64 * fb.height as u64 * 4;
        if fb.phys == 0 || fb.width == 0 || fb.height == 0 || fb.stride < fb.width || (fb.size != 0 && fb.size < bytes) {
            return None;
        }
        // Every pixel is 32 bits wide. A format described by masks that all fit in fewer bits has
        // smaller pixels, which this code can't draw (the start-up stage doesn't pick those).
        match fb.format {
            PIXEL_RGB | PIXEL_BGR => {}
            PIXEL_BITMASK if (fb.red_mask | fb.green_mask | fb.blue_mask | fb.reserved_mask).leading_zeros() < 8 => {}
            _ => return None,
        }
        let base = paging::map_device(fb.phys, bytes, Cache::WriteCombining) as *mut u32;
        Some(Screen {
            base,
            width: fb.width as usize,
            height: fb.height as usize,
            stride: fb.stride as usize,
            format: fb.format,
            masks: [fb.red_mask, fb.green_mask, fb.blue_mask],
        })
    }

    /// The framebuffer's value for a colour.
    fn native(&self, rgb: u32) -> u32 {
        let (r, g, b) = (rgb >> 16 & 0xFF, rgb >> 8 & 0xFF, rgb & 0xFF);
        match self.format {
            PIXEL_BGR => rgb & 0xFF_FFFF, // blue in the lowest byte: the same layout
            PIXEL_RGB => r | g << 8 | b << 16,
            _ => channel(self.masks[0], r) | channel(self.masks[1], g) | channel(self.masks[2], b),
        }
    }

    /// Copies a `w` by `h` rectangle of colours (`src`, rows `src_stride` apart, its top left at (`x`, `y`) of
    /// the screen as well), clipped to the screen.
    pub fn copy(&mut self, x: usize, y: usize, w: usize, h: usize, src: &[u32], src_stride: usize) {
        let (x1, y1) = ((x + w).min(self.width), (y + h).min(self.height));
        if x >= x1 || y >= y1 {
            return;
        }
        for row in y..y1 {
            let from = &src[row * src_stride + x..row * src_stride + x1];
            let line = unsafe { self.base.add(row * self.stride + x) };
            if self.format == PIXEL_BGR {
                unsafe { core::ptr::copy_nonoverlapping(from.as_ptr(), line, from.len()) };
            } else {
                for (i, &c) in from.iter().enumerate() {
                    unsafe { line.add(i).write(self.native(c)) };
                }
            }
        }
    }
}
