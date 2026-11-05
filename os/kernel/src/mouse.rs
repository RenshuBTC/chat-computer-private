//! What mice do: moving one moves the pointer on the screen, its left button clicks (the send button, while an
//! answer comes its stop), and its wheel scrolls the conversation, as the arrow keys do. Reports come from USB mice
//! (`usb/device.rs`), read as each mouse's report descriptor says (`hid/`), in the USB controller's interrupt; the
//! screen takes what's added up here from its main loop.

use core::sync::atomic::{AtomicI32, AtomicU32, Ordering};

/// The wheels' turns not yet taken by the screen, in notches: positive away from you.
static WHEEL: AtomicI32 = AtomicI32::new(0);
/// The movement not yet taken (in the mouse's own counts: right and down positive), the buttons held now, and those
/// pressed since the screen last asked (bit 0: the left one), so a quick click isn't missed.
static DX: AtomicI32 = AtomicI32::new(0);
static DY: AtomicI32 = AtomicI32::new(0);
static HELD: AtomicU32 = AtomicU32::new(0);
static PRESSED: AtomicU32 = AtomicU32::new(0);

/// A mouse's report (from the USB controller's interrupt).
pub fn report(layout: &hid::Pointer, report: &[u8]) {
    // A wheel given as a position (no mouse's is) would say nothing about how far it turned.
    if let Some(turned) = layout.wheel.filter(|w| w.relative).and_then(|w| w.value(report, layout.numbered))
        && turned != 0
    {
        let _ = WHEEL.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |w| Some(w.saturating_add(turned)));
    }
    // Movement (a mouse's is a change since its last report; a tablet's positions aren't read yet).
    let moved = |f: Option<hid::Field>| f.filter(|f| f.relative).and_then(|f| f.value(report, layout.numbered));
    if let Some(dx) = moved(layout.x).filter(|v| *v != 0) {
        let _ = DX.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |v| Some(v.saturating_add(dx)));
    }
    if let Some(dy) = moved(layout.y).filter(|v| *v != 0) {
        let _ = DY.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |v| Some(v.saturating_add(dy)));
    }
    if let Some(down) = layout.buttons_in(report) {
        let was = HELD.swap(down, Ordering::Relaxed);
        PRESSED.fetch_or(down & !was, Ordering::Relaxed);
    }
}

/// How far the wheels turned since the last time this was asked, in notches.
pub fn take_wheel() -> i32 {
    WHEEL.swap(0, Ordering::Relaxed)
}

/// The movement since the last time this was asked (right and down, in the mice's counts), and the buttons pressed
/// meanwhile.
pub fn take_motion() -> (i32, i32, u32) {
    (DX.swap(0, Ordering::Relaxed), DY.swap(0, Ordering::Relaxed), PRESSED.swap(0, Ordering::Relaxed))
}
