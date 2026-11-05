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

/// The framebuffer as plain numbers, for the mouse pointer (pointer.rs), which draws on it from the USB controller's
/// interrupt: only ever while the chat isn't drawing (the chat draws holding its lock, which keeps interrupts off).
#[derive(Clone, Copy)]
pub struct Raw {
    base: *mut u32,
    pub width: usize,
    pub height: usize,
    stride: usize,
    format: u32,
    masks: [u32; 3],
}

unsafe impl Send for Raw {}

/// The framebuffer's value for a colour (0xRRGGBB).
fn native(format: u32, masks: &[u32; 3], rgb: u32) -> u32 {
    let (r, g, b) = (rgb >> 16 & 0xFF, rgb >> 8 & 0xFF, rgb & 0xFF);
    match format {
        PIXEL_BGR => rgb & 0xFF_FFFF, // blue in the lowest byte: the same layout
        PIXEL_RGB => r | g << 8 | b << 16,
        _ => channel(masks[0], r) | channel(masks[1], g) | channel(masks[2], b),
    }
}

impl Raw {
    /// One pixel, if it's on the screen.
    pub fn put(&self, x: i32, y: i32, rgb: u32) {
        if x < 0 || y < 0 || x as usize >= self.width || y as usize >= self.height {
            return;
        }
        let v = native(self.format, &self.masks, rgb);
        unsafe { self.base.add(y as usize * self.stride + x as usize).write_volatile(v) };
    }

    /// A rectangle put back from `canvas` (the chat's picture of the screen: `canvas_stride` pixels to a row), clipped
    /// to the screen.
    pub fn restore(&self, x: i32, y: i32, w: i32, h: i32, canvas: *const u32, canvas_stride: usize) {
        let (x0, y0) = (x.max(0) as usize, y.max(0) as usize);
        let (x1, y1) = (((x + w).max(0) as usize).min(self.width), ((y + h).max(0) as usize).min(self.height));
        for row in y0..y1 {
            for col in x0..x1 {
                let c = unsafe { *canvas.add(row * canvas_stride + col) };
                let v = native(self.format, &self.masks, c);
                unsafe { self.base.add(row * self.stride + col).write_volatile(v) };
            }
        }
    }
}

impl Screen {
    /// The framebuffer as plain numbers, for the pointer.
    pub fn raw(&self) -> Raw {
        Raw { base: self.base, width: self.width, height: self.height, stride: self.stride, format: self.format, masks: self.masks }
    }

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
        native(self.format, &self.masks, rgb)
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
