//! The date and time. The start-up stage read the PC's clock; from then on the kernel's own timer counts on.
//!
//! The clock keeps UTC on some PCs and local time on others (Windows keeps local time), and the firmware rarely
//! says which. The Windows setup writes the offset into the settings; without it the time is taken as UTC, give
//! or take 14 hours (the most any time zone is from UTC), and only the date is said (each message the model's
//! given starts with when it was sent; reminders at a time of day need the offset).

use crate::sync::SpinLock;
use crate::time;
use alloc::format;
use alloc::string::String;

/// EFI_TIME's "time zone not known".
const UNSPECIFIED: i32 = 2047;
/// What a clock may have drifted since it was last set, on top of its time zone.
const DRIFT: i64 = 10 * 60;
/// 20 October 2026, when this was written: no clock that says it's earlier can be right. (A date written here,
/// not the build's, so every build of this source comes out the same.)
const WRITTEN: i64 = 1_792_454_400;

struct Clock {
    /// Seconds since 1970 (UTC) when the start-up stage read the clock: when the kernel's uptime was 0, near
    /// enough (the kernel's timer starts a moment later).
    utc: i64,
    /// How far off that may be, in seconds.
    error: i64,
    /// The local time's offset from UTC, in minutes, if the settings gave it (Windows' time zone when Chat Computer
    /// was set up).
    offset: Option<i32>,
}

static CLOCK: SpinLock<Option<Clock>> = SpinLock::new(None);

/// Takes the clock's reading (seconds since 1970 as if UTC; 0 if it couldn't be read), the time zone the firmware
/// gave and the offset in the settings. Returns what it made of them, for the log.
pub fn init(raw: i64, zone: i32, offset_minutes: Option<i32>) -> String {
    if raw == 0 {
        return String::from("the PC's clock couldn't be read");
    }
    let (utc, error, how) = match (offset_minutes, zone) {
        // (A summer-time change since the setup ran leaves it an hour out.)
        (Some(m), _) => (raw - m as i64 * 60, 3600 + DRIFT, "local time, offset from the settings"),
        (None, 0) => (raw, DRIFT, "UTC, the firmware says"),
        (None, UNSPECIFIED) => (raw, 14 * 3600 + DRIFT, "time zone unknown"),
        // A zone the firmware gives, but the specification's versions disagree on which way it goes.
        (None, z) => (raw, (z.unsigned_abs() as i64).min(1440) * 60 + DRIFT, "time zone from the firmware"),
    };
    *CLOCK.lock() = Some(Clock { utc, error, offset: offset_minutes });
    let behind = if utc + error < WRITTEN { " (behind: earlier than this kernel was written)" } else { "" };
    format!("{} ({how}){behind}", text(utc, error))
}

/// Seconds since 1970 (UTC), if the clock could be read.
pub fn now() -> Option<i64> {
    CLOCK.lock().as_ref().map(|c| c.utc + (time::uptime_ms() / 1000) as i64)
}

/// How far `now` may be off, in seconds.
pub fn error() -> i64 {
    CLOCK.lock().as_ref().map_or(0, |c| c.error)
}

/// (year, month, day, hour, minute) for seconds since 1970 (Howard Hinnant's civil_from_days).
pub fn civil(t: i64) -> (i64, u32, u32, u32, u32) {
    let days = t.div_euclid(86_400);
    let secs = t.rem_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = yoe + era * 400 + if month <= 2 { 1 } else { 0 };
    (year, month, day, (secs / 3600) as u32, (secs / 60 % 60) as u32)
}

const MONTHS: [&str; 12] =
    ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];

/// "21 October 2026, 08:15 UTC", or only the day when the time could be hours out.
pub fn text(t: i64, error: i64) -> String {
    let (y, mo, d, h, mi) = civil(t);
    let month = MONTHS[(mo as usize).clamp(1, 12) - 1];
    if error > 3 * 3600 { format!("{d} {month} {y}") } else { format!("{d} {month} {y}, {h:02}:{mi:02} UTC") }
}

/// The local time's offset from UTC in minutes, if known.
fn offset() -> Option<i32> {
    CLOCK.lock().as_ref().and_then(|c| c.offset)
}

/// The local time now (seconds since 1970, as if the local time were UTC), when the settings gave its offset and the
/// clock is close enough to trust for a time of day (a reminder at 6 pm).
pub fn local_now() -> Option<i64> {
    let (t, m) = (now()?, offset()?);
    (error() <= 3 * 3600).then_some(t + m as i64 * 60)
}

/// The date (and time, when it's known well enough) now, for people: in local time when the settings gave the
/// time zone, else in UTC.
pub fn now_text() -> Option<String> {
    let (t, error) = (now()?, error());
    match offset() {
        Some(m) if error <= 3 * 3600 => {
            let (y, mo, d, h, mi) = civil(t + m as i64 * 60);
            let month = MONTHS[(mo as usize).clamp(1, 12) - 1];
            let sign = if m < 0 { '-' } else { '+' };
            let (oh, om) = (m.unsigned_abs() / 60, m.unsigned_abs() % 60);
            Some(format!("{d} {month} {y}, {h:02}:{mi:02} local time (UTC{sign}{oh}:{om:02})"))
        }
        _ => Some(text(t, error)),
    }
}

/// When `t` (seconds since 1970) was, short, for the note on each message the model's given: "Tue 3 Nov 2026 19:02",
/// in local time when the settings gave the time zone, else in UTC (said so); the day alone when the clock may be
/// hours out.
pub fn short_text(t: i64) -> String {
    const DAYS: [&str; 7] = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"]; // (1 January 1970 was a Thursday)
    let (local, zone) = match offset() {
        Some(m) => (t + m as i64 * 60, ""),
        None => (t, " UTC"),
    };
    let (y, mo, d, h, mi) = civil(local);
    let day = DAYS[local.div_euclid(86_400).rem_euclid(7) as usize];
    let month = &MONTHS[(mo as usize).clamp(1, 12) - 1][..3];
    if error() > 3 * 3600 { format!("{day} {d} {month} {y}") } else { format!("{day} {d} {month} {y} {h:02}:{mi:02}{zone}") }
}
