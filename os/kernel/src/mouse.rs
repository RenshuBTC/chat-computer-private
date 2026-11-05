//! What mice do: moving one moves the pointer on the screen (pointer.rs, which moves and draws it as each report
//! comes), its left button clicks (the send button, while an answer comes its stop), and its wheel scrolls the
//! conversation, as the arrow keys do. Reports come from USB mice (`usb/device.rs`), read as each mouse's report
//! descriptor says (`hid/`), in the USB controller's interrupt; the screen takes the wheel's turns added up here, and
//! the pointer's moves and clicks, from its main loop.

use core::sync::atomic::{AtomicI32, AtomicU32, Ordering};

/// The wheels' turns not yet taken by the screen, in notches: positive away from you.
static WHEEL: AtomicI32 = AtomicI32::new(0);
/// The buttons held now (bit 0: the left one).
static HELD: AtomicU32 = AtomicU32::new(0);

/// A mouse's report (from the USB controller's interrupt).
pub fn report(layout: &hid::Pointer, report: &[u8]) {
    // A wheel given as a position (no mouse's is) would say nothing about how far it turned.
    if let Some(turned) = layout.wheel.filter(|w| w.relative).and_then(|w| w.value(report, layout.numbered))
        && turned != 0
    {
        let _ = WHEEL.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |w| Some(w.saturating_add(turned)));
    }
    // Movement (a mouse's is a change since its last report; a tablet's positions aren't read yet): the pointer moves
    // now, before a press in the same report, which is where it then is.
    let moved = |f: Option<hid::Field>| f.filter(|f| f.relative).and_then(|f| f.value(report, layout.numbered)).unwrap_or(0);
    let (dx, dy) = (moved(layout.x), moved(layout.y));
    if dx != 0 || dy != 0 {
        crate::pointer::moved(dx, dy);
    }
    if let Some(down) = layout.buttons_in(report) {
        let was = HELD.swap(down, Ordering::Relaxed);
        if down & !was & 1 != 0 {
            crate::pointer::pressed();
        }
    }
}

/// How far the wheels turned since the last time this was asked, in notches.
pub fn take_wheel() -> i32 {
    WHEEL.swap(0, Ordering::Relaxed)
}
