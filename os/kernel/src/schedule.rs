//! Timers, reminders and alarms, as the main Chat Computer has them (its `reminders` tool, `schedule.py`): set for a
//! time from now, or for a time of day when this computer's local time is known (the settings give its offset from
//! UTC). When one is due it rings: a line at the end of the conversation, and the chime (sound/) again every few
//! seconds, a timer's for a minute and an alarm's for ten, until a key is pressed; a reminder rings once. They count
//! the kernel's own time, so the PC's clock being out doesn't move them, and they last until the computer restarts
//! (it starts Windows then).

use crate::sync::SpinLock;
use crate::{chat, clock, sound, time};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use chime::Kind;

/// How long each kind rings unless a key is pressed first, and how often its chime comes, in milliseconds.
fn ring_for(kind: Kind) -> u64 {
    match kind {
        Kind::Reminder => 0,
        Kind::Timer => 60_000,
        Kind::Alarm => 600_000,
    }
}
fn ring_every(kind: Kind) -> u64 {
    match kind {
        Kind::Reminder => 5_000,
        Kind::Timer => 4_000,
        Kind::Alarm => 6_000,
    }
}

/// The most set at once, and the furthest ahead (a week).
const MOST: usize = 50;
const LONGEST_S: f64 = 7.0 * 86_400.0;

struct Item {
    id: u32,
    kind: Kind,
    label: String,
    /// When it rings and when it was set, by the kernel's time since start-up.
    due_ms: u64,
    set_ms: u64,
    /// The time of day it was set for (local seconds since 1970), for a reminder or alarm set that way.
    at: Option<i64>,
}

struct Ringing {
    kind: Kind,
    until_ms: u64,
    next_ms: u64,
}

struct Schedule {
    items: Vec<Item>,
    next_id: u32,
    ringing: Option<Ringing>,
}

static SCHEDULE: SpinLock<Schedule> = SpinLock::new(Schedule { items: Vec::new(), next_id: 1, ringing: None });

/// 600 → "10-minute", 90 → "90-second", 5400 → "90-minute", 7200 → "2-hour" (for "your 10-minute timer").
pub fn say_duration(seconds: u64) -> String {
    let s = seconds;
    if s % 3600 == 0 && s >= 7200 {
        format!("{}-hour", s / 3600)
    } else if s % 60 == 0 && s > 0 {
        format!("{}-minute", s / 60)
    } else if s < 120 {
        format!("{s}-second")
    } else {
        format!("{}-minute {}-second", s / 60, s % 60)
    }
}

/// How long is left, as it's said: "4 minutes 20 seconds", "2 hours 5 minutes", "under a second".
fn say_left(ms: u64) -> String {
    let s = ms.div_ceil(1000);
    let unit = |n: u64, what: &str| format!("{n} {what}{}", if n == 1 { "" } else { "s" });
    match s {
        0 => String::from("under a second"),
        1..=59 => unit(s, "second"),
        60..=3599 if s % 60 == 0 => unit(s / 60, "minute"),
        60..=3599 => format!("{} {}", unit(s / 60, "minute"), unit(s % 60, "second")),
        _ if s / 60 % 60 == 0 => unit(s / 3600, "hour"),
        _ => format!("{} {}", unit(s / 3600, "hour"), unit(s / 60 % 60, "minute")),
    }
}

/// A local time of day as it's said: 18:00 → "6 pm", 07:30 → "7:30 am".
fn say_clock(local: i64) -> String {
    let (_, _, _, h, m) = clock::civil(local);
    let twelve = if h % 12 == 0 { 12 } else { h % 12 };
    let minutes = if m == 0 { String::new() } else { format!(":{m:02}") };
    format!("{twelve}{minutes} {}", if h < 12 { "am" } else { "pm" })
}

/// "18:00", "6pm", "6:30 pm", "07.15" or "7" as hours and minutes.
pub fn parse_clock(text: &str) -> Option<(u32, u32)> {
    let t = text.trim().to_lowercase();
    let (t, half) = if let Some(rest) = t.strip_suffix("pm").or_else(|| t.strip_suffix("p.m.")) {
        (rest.trim(), Some(true))
    } else if let Some(rest) = t.strip_suffix("am").or_else(|| t.strip_suffix("a.m.")) {
        (rest.trim(), Some(false))
    } else {
        (t.as_str(), None)
    };
    let (h, m) = match t.split_once([':', '.']) {
        Some((h, m)) if m.len() == 2 => (h.parse::<u32>().ok()?, m.parse::<u32>().ok()?),
        None => (t.parse::<u32>().ok()?, 0),
        _ => return None,
    };
    let h = match half {
        Some(pm) if (1..=12).contains(&h) => (h % 12) + if pm { 12 } else { 0 },
        Some(_) => return None,
        None => h,
    };
    (h < 24 && m < 60).then_some((h, m))
}

/// What one is, as it's said: "a 10-minute timer for tea (rings in 4 minutes 20 seconds)", "a reminder at 6 pm:
/// call Mum", "an alarm at 7 am".
fn describe(item: &Item, now_ms: u64) -> String {
    let left = say_left(item.due_ms.saturating_sub(now_ms));
    match item.kind {
        Kind::Timer => {
            let length = say_duration((item.due_ms - item.set_ms).div_ceil(1000));
            let label = if item.label.is_empty() { String::new() } else { format!(" for {}", item.label) };
            format!("a {length} timer{label} (rings in {left})")
        }
        kind => {
            let what = if kind == Kind::Alarm { "an alarm" } else { "a reminder" };
            let when = match item.at {
                Some(at) => format!("at {}", say_clock(at)),
                None => format!("in {left}"),
            };
            let label = if item.label.is_empty() { String::new() } else { format!(": {}", item.label) };
            format!("{what} {when}{label}")
        }
    }
}

/// What's said when it rings: "Your 10-minute timer is done: tea.", "Reminder: call Mum.", "It's 7 am."
fn ring_text(item: &Item) -> String {
    let label = item.label.trim_end_matches('.');
    match item.kind {
        Kind::Timer => {
            let length = say_duration((item.due_ms - item.set_ms).div_ceil(1000));
            if label.is_empty() { format!("Your {length} timer is done.") } else { format!("Your {length} timer is done: {label}.") }
        }
        Kind::Reminder if label.is_empty() => String::from("Here's your reminder."),
        Kind::Reminder => format!("Reminder: {label}."),
        Kind::Alarm => {
            let at = item.at.map(say_clock).unwrap_or_else(|| String::from("time"));
            if label.is_empty() { format!("It's {at}.") } else { format!("It's {at}: {label}.") }
        }
    }
}

fn add(kind: Kind, label: &str, in_ms: u64, at: Option<i64>) -> Result<String, String> {
    let now = time::uptime_ms();
    let mut s = SCHEDULE.lock();
    if s.items.len() >= MOST {
        return Err(format!("{MOST} are set already: cancel some first."));
    }
    let id = s.next_id;
    s.next_id += 1;
    let item = Item { id, kind, label: String::from(label.trim()), due_ms: now + in_ms, set_ms: now, at };
    let said = format!("Set {}. [{id}]", describe(&item, now));
    s.items.push(item);
    Ok(said)
}

/// A timer `seconds` long.
pub fn timer(seconds: f64, label: &str) -> Result<String, String> {
    if !(seconds >= 1.0 && seconds <= LONGEST_S) {
        return Err(String::from("A timer can be from 1 second to a week long."));
    }
    let ms = (seconds * 1000.0) as u64;
    add(Kind::Timer, label, ms / 1000 * 1000, None)
}

/// A reminder (or alarm) `in_minutes` from now, or at time of day `at` (the next time it comes).
pub fn reminder(alarm: bool, label: &str, in_minutes: Option<f64>, at: Option<&str>) -> Result<String, String> {
    let kind = if alarm { Kind::Alarm } else { Kind::Reminder };
    if let Some(m) = in_minutes {
        if !(m > 0.0 && m * 60.0 <= LONGEST_S) {
            return Err(String::from("in_minutes must be more than 0 and at most a week."));
        }
        return add(kind, label, (m * 60_000.0) as u64, None);
    }
    let Some(at) = at.filter(|a| !a.trim().is_empty()) else {
        return Err(String::from("Say when: in_minutes, or at (a time of day like 18:00 or 6pm)."));
    };
    let Some((h, m)) = parse_clock(at) else {
        return Err(format!("{at:?} isn't a time of day (like 18:00 or 6pm)."));
    };
    let Some(local) = clock::local_now() else {
        return Err(String::from(
            "This computer's local time isn't known to Chat Computer's own system (its clock's time zone wasn't set up), so \
             it can't ring at a time of day: set it for a number of minutes from now instead.",
        ));
    };
    let today = local - local.rem_euclid(86_400);
    let mut target = today + (h * 3600 + m * 60) as i64;
    if target <= local {
        target += 86_400;
    }
    add(kind, label, (target - local) as u64 * 1000, Some(target))
}

/// Everything set, soonest first.
pub fn list() -> String {
    let now = time::uptime_ms();
    let s = SCHEDULE.lock();
    let mut items: Vec<&Item> = s.items.iter().collect();
    items.sort_by_key(|i| i.due_ms);
    let lines: Vec<String> = items.iter().map(|i| format!("[{}] {}", i.id, describe(i, now))).collect();
    if lines.is_empty() { String::from("Nothing is set.") } else { lines.join("\n") }
}

/// Cancels one by its id, or all ("all"), or every timer, reminder or alarm ("timers", "reminders", "alarms").
pub fn cancel(which: &str) -> Result<String, String> {
    let now = time::uptime_ms();
    let w = which.trim().trim_start_matches('[').trim_end_matches(']').to_lowercase();
    let mut s = SCHEDULE.lock();
    let gone: Vec<Item> = {
        let pick = |i: &Item| match w.as_str() {
            "all" => true,
            "timers" | "timer" => i.kind == Kind::Timer,
            "reminders" | "reminder" => i.kind == Kind::Reminder,
            "alarms" | "alarm" => i.kind == Kind::Alarm,
            id => id.parse::<u32>().ok() == Some(i.id),
        };
        let (gone, kept): (Vec<Item>, Vec<Item>) = s.items.drain(..).partition(|i| pick(i));
        s.items = kept;
        gone
    };
    if gone.is_empty() {
        return Err(String::from("Nothing like that is set. (list shows what is.)"));
    }
    let said: Vec<String> = gone.iter().map(|i| describe(i, now)).collect();
    Ok(format!("Cancelled {}.", said.join("; ")))
}

/// Silences one that's ringing.
pub fn stop() -> String {
    if SCHEDULE.lock().ringing.take().is_some() {
        sound::stop();
        String::from("Stopped.")
    } else {
        String::from("Nothing is ringing.")
    }
}

/// A key was pressed: one that's ringing stops (the key is typed as well).
pub fn key() {
    if SCHEDULE.lock().ringing.take().is_some() {
        sound::stop();
    }
}

/// Rings what's due, and rings again what's ringing (after every wake).
pub fn work() {
    let now = time::uptime_ms();
    let (due, again) = {
        let mut s = SCHEDULE.lock();
        let first = s.items.iter().enumerate().filter(|(_, i)| i.due_ms <= now).min_by_key(|(_, i)| i.due_ms).map(|(k, _)| k);
        let due = first.map(|k| s.items.remove(k));
        let mut again = None;
        if let Some(item) = &due {
            s.ringing = Some(Ringing { kind: item.kind, until_ms: now + ring_for(item.kind), next_ms: now + ring_every(item.kind) });
        } else if let Some(r) = s.ringing.as_ref().filter(|r| now >= r.next_ms).map(|r| (r.kind, r.until_ms)) {
            if now >= r.1 {
                s.ringing = None;
            } else {
                s.ringing = Some(Ringing { kind: r.0, until_ms: r.1, next_ms: now + ring_every(r.0) });
                again = Some(r.0);
            }
        }
        (due, again)
    };
    if let Some(item) = due {
        let text = ring_text(&item);
        crate::log!("Chat Computer kernel: rings: {text}");
        chat::note(text);
        sound::ring(item.kind);
    } else if let Some(kind) = again {
        sound::ring(kind);
    }
}

/// What's set, for computer_status.
pub fn status() -> String {
    let n = SCHEDULE.lock().items.len();
    match n {
        0 => String::from("Timers and reminders: none set"),
        1 => String::from("Timers and reminders: 1 set (the reminders tool lists it)"),
        n => format!("Timers and reminders: {n} set (the reminders tool lists them)"),
    }
}
