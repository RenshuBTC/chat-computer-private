//! The mouse pointer: moved and drawn straight onto the screen as each of the mouse's reports comes, in the USB
//! controller's interrupt, so it keeps up with the mouse (a thousand reports a second, for some) whatever the rest of
//! Chat Computer is busy with (its model working, say). It's drawn over the screen and never into the picture of the
//! screen the chat keeps (draw.rs's canvas), which is where what was under it comes back from when it moves on; when
//! the chat copies part of that picture to the screen over the pointer, it's drawn again (`over`).
//!
//! It moves as far as the mouse counts, one pixel a count (Windows' own speed, without its "enhance pointer
//! precision"), a little more only when the mouse is moved fast.

use crate::screen::Raw;
use crate::sync::SpinLock;

/// The arrow, row by row: X its outline, o its inside (as Windows' standard pointer, 12 × 19), in pixels of `scale`.
const ARROW: [&str; 19] = [
    "X           ",
    "XX          ",
    "XoX         ",
    "XooX        ",
    "XoooX       ",
    "XooooX      ",
    "XoooooX     ",
    "XooooooX    ",
    "XoooooooX   ",
    "XooooooooX  ",
    "XoooooooooX ",
    "XooooooooooX",
    "XooooooXXXXX",
    "XoooXooX    ",
    "XooX XooX   ",
    "XoX  XooX   ",
    "XX    XooX  ",
    "      XooX  ",
    "       XX   ",
];

struct State {
    screen: Option<Raw>,
    /// The chat's picture of the screen (its pixels, as many to a row as the screen is wide), where what's under the
    /// pointer comes back from.
    canvas: *const u32,
    /// Where the arrow's tip is (in pixels), and whether the mouse has moved yet (it isn't shown till then).
    x: f32,
    y: f32,
    shown: bool,
    /// The arrow's top left as drawn, if it is.
    drawn: Option<(i32, i32)>,
    scale: i32,
    /// It moved since the chat last asked; where the left button was last pressed, not yet taken.
    moved: bool,
    press: Option<(f32, f32)>,
}

unsafe impl Send for State {}

static STATE: SpinLock<State> = SpinLock::new(State {
    screen: None,
    canvas: core::ptr::null(),
    x: 0.0,
    y: 0.0,
    shown: false,
    drawn: None,
    scale: 1,
    moved: false,
    press: None,
});

/// Where it draws: the screen, the chat's picture of it (`canvas`: its pixels, which stay where they are), and the
/// arrow's size (1, 2...: pixels of the arrow's drawing).
pub fn init(screen: Raw, canvas: *const u32, scale: i32) {
    let mut s = STATE.lock();
    s.x = screen.width as f32 / 2.0;
    s.y = screen.height as f32 / 2.0;
    s.screen = Some(screen);
    s.canvas = canvas;
    s.scale = scale.max(1);
}

/// The mouse moved `dx`, `dy` counts (one report's: right and down positive).
pub fn moved(dx: i32, dy: i32) {
    if dx == 0 && dy == 0 {
        return;
    }
    let mut s = STATE.lock();
    let Some(screen) = s.screen else { return };
    // One pixel a count; past 24 counts in a report (a fast flick), a little more, up to half as much again.
    let speed = |d: i32| -> f32 {
        let v = d as f32;
        let fast = ((v.abs() - 24.0) / 48.0).clamp(0.0, 0.5);
        v * (1.0 + fast)
    };
    s.x = (s.x + speed(dx)).clamp(0.0, screen.width as f32 - 1.0);
    s.y = (s.y + speed(dy)).clamp(0.0, screen.height as f32 - 1.0);
    s.shown = true;
    s.moved = true;
    redraw(&mut s);
}

/// The left button was pressed (from the same interrupt, after the report's movement): where, for the click.
pub fn pressed() {
    let mut s = STATE.lock();
    s.shown = true;
    s.press = Some((s.x, s.y));
    redraw(&mut s);
}

/// What happened since the main loop last asked: where the tip is now (if it moved, or was pressed), and where the
/// left button was pressed (if it was).
pub fn take() -> (Option<(f32, f32)>, Option<(f32, f32)>) {
    let mut s = STATE.lock();
    let press = s.press.take();
    let moved = core::mem::replace(&mut s.moved, false);
    ((moved || press.is_some()).then_some((s.x, s.y)), press)
}

/// The arrow's part of the screen at its tip (x, y): (x, y, width, height).
fn rect_at(x: i32, y: i32, scale: i32) -> (i32, i32, i32, i32) {
    (x, y, 12 * scale, 19 * scale)
}

/// The chat copied (x, y, w, h) of its picture to the screen: the arrow, if it was there, is drawn again. (Not if
/// this lock is held: the screen being drawn after a stop in the middle of drawing the arrow.)
pub fn over(x: i32, y: i32, w: i32, h: i32) {
    let Some(mut s) = STATE.try_lock() else { return };
    let Some((dx, dy)) = s.drawn else { return };
    let (ax, ay, aw, ah) = rect_at(dx, dy, s.scale);
    if ax < x + w && x < ax + aw && ay < y + h && y < ay + ah {
        s.drawn = None;
        redraw(&mut s);
    }
}

/// Puts back what was under the arrow, and draws it where it is now.
fn redraw(s: &mut State) {
    let Some(screen) = s.screen else { return };
    if let Some((x, y)) = s.drawn.take()
        && !s.canvas.is_null()
    {
        let (x, y, w, h) = rect_at(x, y, s.scale);
        // (What the chat's picture has there.)
        screen.restore(x, y, w, h, s.canvas, screen.width);
    }
    if !s.shown {
        return;
    }
    let (x, y) = (s.x as i32, s.y as i32);
    let scale = s.scale;
    for (row, line) in ARROW.iter().enumerate() {
        for (col, c) in line.bytes().enumerate() {
            let rgb = match c {
                b'X' => 0x000000,
                b'o' => 0xFFFFFF,
                _ => continue,
            };
            for py in 0..scale {
                for px in 0..scale {
                    screen.put(x + col as i32 * scale + px, y + row as i32 * scale + py, rgb);
                }
            }
        }
    }
    s.drawn = Some((x, y));
}
