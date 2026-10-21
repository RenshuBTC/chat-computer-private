//! What mice do. So far only their wheels count: turning one scrolls the conversation, as the arrow keys do. (There
//! is no pointer on the screen yet, so moving a mouse or clicking does nothing.) Reports come from USB mice
//! (`usb/device.rs`), read as each mouse's report descriptor says (`hid/`).

use core::sync::atomic::{AtomicI32, Ordering};

/// The wheels' turns not yet taken by the screen, in notches: positive away from you.
static WHEEL: AtomicI32 = AtomicI32::new(0);

/// A mouse's report (from the USB controller's interrupt).
pub fn report(layout: &hid::Pointer, report: &[u8]) {
    // A wheel given as a position (no mouse's is) would say nothing about how far it turned.
    if let Some(turned) = layout.wheel.filter(|w| w.relative).and_then(|w| w.value(report, layout.numbered))
        && turned != 0
    {
        let _ = WHEEL.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |w| Some(w.saturating_add(turned)));
    }
}

/// How far the wheels turned since the last time this was asked, in notches.
pub fn take_wheel() -> i32 {
    WHEEL.swap(0, Ordering::Relaxed)
}
