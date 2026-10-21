//! The chimes of Chat Computer's timers, reminders and alarms: short bell-like tones, made here (no sound files), the
//! same as the main Chat Computer's (`chatcomputer/voice/chime.py`): each note a tone with its second and third
//! harmonics, rising in 5 ms (no click) and dying away, the notes 70 ms apart.
//!
//! Made with sine recurrences (each sample from the two before), so it takes little arithmetic: the kernel does its
//! floating point in software.
#![cfg_attr(not(test), no_std)]

extern crate alloc;

use alloc::vec::Vec;

/// What rings.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    /// Two soft notes.
    Reminder,
    /// Two brighter ones.
    Timer,
    /// Three rising.
    Alarm,
}

impl Kind {
    /// Its notes, in hertz.
    pub fn notes(self) -> &'static [f64] {
        match self {
            Kind::Reminder => &[784.0, 1046.5],
            Kind::Timer => &[1046.5, 1318.5],
            Kind::Alarm => &[880.0, 1108.7, 1318.5],
        }
    }
}

/// How long each note lasts, and the silence after it, in seconds.
const NOTE_S: f64 = 0.42;
const GAP_S: f64 = 0.07;
/// How fast a note dies away (e^(-4.5 t)), and how long it takes to rise.
const DECAY: f64 = 4.5;
const ATTACK_S: f64 = 0.005;
/// The harmonics' strengths, and what the three add up to at most (so the loudest sample is `loud`).
const SECOND: f64 = 0.35;
const THIRD: f64 = 0.12;
const PEAK: f64 = 1.47;

/// The chime, as 16-bit samples at `rate` per second, its loudest `loud` (0 to 1) of full scale. The main Chat
/// Computer's is 0.32.
pub fn chime(kind: Kind, rate: u32, loud: f64) -> Vec<i16> {
    let rate_f = rate as f64;
    let note = (NOTE_S * rate_f) as usize;
    let gap = (GAP_S * rate_f) as usize;
    let mut out = Vec::with_capacity(kind.notes().len() * (note + gap));
    for &f in kind.notes() {
        bell(&mut out, f, rate_f, note, loud);
        out.resize(out.len() + gap, 0);
    }
    out
}

/// One note: `n` samples of a bell at `freq` hertz.
fn bell(out: &mut Vec<i16>, freq: f64, rate: f64, n: usize, loud: f64) {
    let mut tones = [Sine::new(freq / rate), Sine::new(2.0 * freq / rate), Sine::new(3.0 * freq / rate)];
    let fade = exp(-DECAY / rate);
    let mut level = loud / PEAK;
    let attack = ATTACK_S * rate;
    for i in 0..n {
        let tone = tones[0].next() + SECOND * tones[1].next() + THIRD * tones[2].next();
        let rise = if (i as f64) < attack { i as f64 / attack } else { 1.0 };
        let v = level * rise * tone * 32767.0;
        out.push(round(v).clamp(-32767.0, 32767.0) as i16);
        level *= fade;
    }
}

/// sin(2π k f), k = 0, 1, 2…, for `f` cycles a sample: each value from the two before it (sin((k+1)w) =
/// 2 cos(w) sin(kw) − sin((k−1)w)).
struct Sine {
    twice_cos: f64,
    last: f64,
    before: f64,
}

impl Sine {
    fn new(f: f64) -> Sine {
        let w = 2.0 * core::f64::consts::PI * f;
        // (The one before the first, sin(−w), so that the first is sin(0).)
        Sine { twice_cos: 2.0 * cos(w), last: -sin(w), before: -sin(2.0 * w) }
    }

    fn next(&mut self) -> f64 {
        let v = self.twice_cos * self.last - self.before;
        self.before = self.last;
        self.last = v;
        v
    }
}

fn round(x: f64) -> f64 {
    if x >= 0.0 { (x + 0.5) as i64 as f64 } else { -((-x + 0.5) as i64 as f64) }
}

/// sin x, for |x| up to a few π: reduced to −π…π, then the series to x^17 (ample: under 1e-12).
fn sin(x: f64) -> f64 {
    let two_pi = 2.0 * core::f64::consts::PI;
    let mut x = x - two_pi * round(x / two_pi);
    if x > core::f64::consts::FRAC_PI_2 {
        x = core::f64::consts::PI - x;
    } else if x < -core::f64::consts::FRAC_PI_2 {
        x = -core::f64::consts::PI - x;
    }
    let x2 = x * x;
    let (mut term, mut sum) = (x, x);
    for k in 1..9 {
        term *= -x2 / ((2 * k) as f64 * (2 * k + 1) as f64);
        sum += term;
    }
    sum
}

fn cos(x: f64) -> f64 {
    sin(x + core::f64::consts::FRAC_PI_2)
}

/// e^x for small |x| (a sample's fade): the series to x^8.
fn exp(x: f64) -> f64 {
    let (mut term, mut sum) = (1.0, 1.0);
    for k in 1..9 {
        term *= x / k as f64;
        sum += term;
    }
    sum
}

#[cfg(test)]
mod tests;
