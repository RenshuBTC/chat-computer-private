//! Sound through Intel's High Definition Audio (the "High Definition Audio Specification", revision 1.0a): the sound
//! controller of nearly every PC since 2004, Intel's and AMD's chipsets alike, with codecs on its link (Realtek's,
//! Conexant's, Cirrus Logic's...) that make the speakers', headphones' and line out's signals. One driver covers them
//! all: the controller's registers are the specification's, and each codec describes itself (its widgets and how
//! they connect), so the way from a stream to each output is found by following those connections, as Linux's generic
//! driver does.
//!
//! Chat Computer plays short sounds (its chimes): each whole in memory, played once by one output stream, which is
//! stopped after. And it listens: the microphone (a plugged-in one, else the built-in one, else the line in) found the
//! same way, from an input converter (an ADC) back to its pin, and recorded by the first input stream into a ring of
//! memory, which is read as it fills. Commands go to a codec through the command ring (CORB) and its answers come back
//! through the response ring (RIRB), one at a time, waited for. Nothing interrupts the processor.

use crate::dma::Page;
use crate::paging::{self, Cache};
use crate::{cpu, pci, time};
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use bootinfo::PAGE_SIZE;
use core::ptr::{read_volatile, write_volatile};

// The controller's registers.
const GCAP: u64 = 0x00; // 16 bits
const GCTL: u64 = 0x08;
const STATESTS: u64 = 0x0E; // 16
const INTCTL: u64 = 0x20;
const CORBLBASE: u64 = 0x40;
const CORBUBASE: u64 = 0x44;
const CORBWP: u64 = 0x48; // 16
const CORBRP: u64 = 0x4A; // 16
const CORBCTL: u64 = 0x4C; // 8
const CORBSIZE: u64 = 0x4E; // 8
const RIRBLBASE: u64 = 0x50;
const RIRBUBASE: u64 = 0x54;
const RIRBWP: u64 = 0x58; // 16
const RINTCNT: u64 = 0x5A; // 16
const RIRBCTL: u64 = 0x5C; // 8
const RIRBSTS: u64 = 0x5D; // 8
const RIRBSIZE: u64 = 0x5E; // 8
// A stream descriptor's (from 0x80, 0x20 each: the input streams', then the output streams', then the two-way ones).
const SD_CTL: u64 = 0x00; // 24 bits, then the status byte
const SD_LPIB: u64 = 0x04; // where in its ring of memory the stream is (bytes)
const SD_CBL: u64 = 0x08;
const SD_LVI: u64 = 0x0C; // 16
const SD_FMT: u64 = 0x12; // 16
const SD_BDPL: u64 = 0x18;
const SD_BDPU: u64 = 0x1C;
const SD_FIFOS: u64 = 0x10; // 16: how many bytes its FIFO holds, less one

const GCAP_64BIT: u16 = 1 << 0;
const GCTL_RUNNING: u32 = 1 << 0; // CRST: 0 holds the controller (and the link) in reset
const CORBCTL_RUN: u8 = 1 << 1;
const CORBRP_RESET: u16 = 1 << 15;
const RIRBWP_RESET: u16 = 1 << 15;
const RIRBCTL_DMA: u8 = 1 << 1;
/// Counted answers set RIRBSTS's flag (the processor isn't interrupted: GIE stays off). Some controllers (QEMU's) take
/// no more commands until it's cleared.
const RIRBCTL_COUNT: u8 = 1 << 0;
const RIRBSTS_CLEAR: u8 = 0x05;
const SD_RESET: u32 = 1 << 0;
const SD_RUN: u32 = 1 << 1;
const SD_TAG_SHIFT: u32 = 20;

// PCI configuration: Intel's traffic class select, and its no-snoop control; AMD's (ATI's) snoop enable.
const INTEL_TCSEL: u16 = 0x44;
const INTEL_DEVC: u16 = 0x78;
const INTEL_DEVC_NO_SNOOP: u16 = 1 << 11;
const ATI_MISC: u16 = 0x42;
const ATI_SNOOP: u8 = 1 << 1;

// Verbs: 12 bits with an 8-bit payload...
const GET_PARAMETER: u32 = 0xF00;
const SET_CONNECTION_SELECT: u32 = 0x701;
const GET_CONNECTION_LIST: u32 = 0xF02;
const SET_POWER_STATE: u32 = 0x705;
const SET_CHANNEL_STREAM: u32 = 0x706;
const SET_PIN_CONTROL: u32 = 0x707;
const SENSE_PIN: u32 = 0x709;
const GET_PIN_SENSE: u32 = 0xF09;
const SET_EAPD: u32 = 0x70C;
const GET_EAPD: u32 = 0xF0C;
const GET_CONFIG_DEFAULT: u32 = 0xF1C;
// ...or 4 bits with a 16-bit one.
const SET_FORMAT: u32 = 0x2;
const SET_AMP: u32 = 0x3;

// Parameters.
const P_VENDOR: u8 = 0x00;
const P_NODES: u8 = 0x04;
const P_FUNCTION: u8 = 0x05;
const P_WIDGET: u8 = 0x09;
const P_PIN: u8 = 0x0C;
const P_IN_AMP: u8 = 0x0D;
const P_CONNECTIONS: u8 = 0x0E;
const P_OUT_AMP: u8 = 0x12;
const AUDIO_FUNCTION: u32 = 0x01;

// Widgets: their kinds (bits 23:20 of their capabilities), and what they have.
const OUTPUT: u32 = 0x0;
const INPUT: u32 = 0x1;
const MIXER: u32 = 0x2;
const SELECTOR: u32 = 0x3;
const PIN: u32 = 0x4;
const W_IN_AMP: u32 = 1 << 1;
const W_OUT_AMP: u32 = 1 << 2;
const W_AMP_OWN: u32 = 1 << 3;
const W_CONNECTIONS: u32 = 1 << 8;
const W_DIGITAL: u32 = 1 << 9;
const W_POWER: u32 = 1 << 10;
// Pins: what they can do, and how they're set.
const PIN_TRIGGER: u32 = 1 << 1;
const PIN_PRESENCE: u32 = 1 << 2;
const PIN_HEADPHONE_DRIVE: u32 = 1 << 3;
const PIN_OUT: u32 = 1 << 4;
const PIN_IN: u32 = 1 << 5;
const PIN_VREF_80: u32 = 1 << 12;
const PIN_EAPD: u32 = 1 << 16;
const PIN_OUT_ENABLE: u32 = 1 << 6;
const PIN_HEADPHONE_ENABLE: u32 = 1 << 7;
const PIN_IN_ENABLE: u32 = 1 << 5;
/// A microphone's bias voltage, 80% (what electret microphones, the usual kind, take).
const PIN_BIAS_80: u32 = 0x4;
const EAPD: u32 = 1 << 1;
// A pin's configuration (set by the PC's maker): how it's connected, and what to.
const CONNECTED_NOTHING: u32 = 1;
const CONNECTED_BUILT_IN: u32 = 2;
const LINE_OUT: u32 = 0x0;
const SPEAKER: u32 = 0x1;
const HEADPHONES: u32 = 0x2;
const LINE_IN: u32 = 0x8;
const MICROPHONE: u32 = 0xA;
// Amplifiers: set both channels, of the output or an input; muted; the gain step that's 0 dB.
const AMP_OUT: u32 = 1 << 15;
const AMP_IN: u32 = 1 << 14;
const AMP_BOTH: u32 = 3 << 12;
const AMP_MUTE: u32 = 1 << 7;

/// 48 kHz, 16 bits, two channels: what every codec plays.
pub const FORMAT: u16 = 0x0011;
pub const RATE: u32 = 48_000;
/// The streams' numbers on the link (any of 1 to 15, each its own): what's played, and what's heard.
const STREAM: u32 = 1;
const IN_STREAM: u32 = 2;
/// How much a microphone's own amplifier (its "boost") adds, where it has one: 20 dB, as Windows has it.
const BOOST_DB: u32 = 20;
/// The ring of memory what's heard goes round, in pages: 2.7 seconds of it (read far more often than that).
const HEARD_PAGES: usize = 128;
/// How long a command's answer may take.
const ANSWER_MS: u64 = 50;
/// Silence after a sound, so that the stream (a ring) doesn't come round to the sound again before it's stopped.
const TAIL_S: u32 = 2;

/// A widget of a codec's audio function.
struct Widget {
    nid: u16,
    caps: u32,
    pin_caps: u32,
    config: u32,
    connections: Vec<u16>,
}

impl Widget {
    fn kind(&self) -> u32 {
        (self.caps >> 20) & 0xF
    }
}

/// An output: a pin (speakers, headphones or a line out), and the way to it from an output converter (a DAC): the
/// widgets on the way, each with the connection it takes, the DAC last.
struct Output {
    pin: u16,
    device: u32,
    built_in: bool,
    path: Vec<(u16, usize)>,
}

impl Output {
    fn dac(&self) -> u16 {
        self.path.last().map_or(0, |p| p.0)
    }
}

/// An input: a pin (a microphone, or a line in), and the way from an input converter (an ADC) to it: the widgets on
/// the way, each with the connection it takes, the ADC first and the pin last.
struct Input {
    pin: u16,
    device: u32,
    built_in: bool,
    path: Vec<(u16, usize)>,
}

impl Input {
    fn adc(&self) -> u16 {
        self.path.first().map_or(0, |p| p.0)
    }
}

/// What's being recorded: the ring of pages the input stream fills, the list of them it reads, and how far into the
/// ring it's been read (bytes).
struct Recording {
    pages: Vec<Page>,
    /// (Kept while it records: the controller reads it.)
    #[allow(dead_code)]
    list: Page,
    bytes: u32,
    read: u32,
    input: usize,
}

/// A sound ready to play: its pages, and the list of them the controller reads (the buffer descriptor list).
pub struct Sound {
    pages: Vec<Page>,
    list: Page,
    bytes: u32,
    /// How long the sound itself lasts (not its silence after).
    pub micros: u64,
}

impl Sound {
    /// Mono 16-bit samples at `RATE` as a sound for both channels, at `volume` percent, in memory the controller
    /// can reach (`low`: below 4 GiB). None when memory runs out.
    pub fn new(samples: &[i16], volume: u32, low: bool) -> Option<Sound> {
        let total = (samples.len() + (RATE * TAIL_S) as usize) * 4;
        let pages_needed = total.div_ceil(PAGE_SIZE as usize).clamp(2, 256);
        let mut pages = Vec::with_capacity(pages_needed);
        for _ in 0..pages_needed {
            pages.push(Page::new(low)?);
        }
        let per_page = PAGE_SIZE as usize / 4;
        for (i, &s) in samples.iter().enumerate().take(pages_needed * per_page) {
            let v = ((s as i32 * volume.min(100) as i32) / 100) as i16 as u16 as u32;
            unsafe { write_volatile(pages[i / per_page].at::<u32>((i % per_page) * 4), v | v << 16) };
        }
        let list = Page::new(low)?;
        for (i, page) in pages.iter().enumerate() {
            unsafe {
                write_volatile(list.at::<u64>(16 * i), page.phys);
                write_volatile(list.at::<u32>(16 * i + 8), PAGE_SIZE as u32);
                write_volatile(list.at::<u32>(16 * i + 12), 0);
            }
        }
        // (A controller set not to snoop the caches reads memory itself.)
        for page in pages.iter().chain(core::iter::once(&list)) {
            for line in (0..PAGE_SIZE).step_by(64) {
                cpu::flush_line(page.at::<u8>(line as usize) as u64);
            }
        }
        cpu::memory_fence();
        let bytes = (pages_needed * PAGE_SIZE as usize) as u32;
        Some(Sound { pages, list, bytes, micros: samples.len() as u64 * 1_000_000 / RATE as u64 })
    }

    fn entries(&self) -> u16 {
        self.pages.len() as u16
    }
}

pub struct Hda {
    regs: u64,
    /// Whether it can reach memory above 4 GiB.
    pub wide: bool,
    /// The command ring (at 0) and the response ring (at 2048).
    rings: Page,
    corb_entries: u16,
    rirb_entries: u16,
    corb_wp: u16,
    rirb_rp: u16,
    /// The first output stream's registers, and the first input stream's (0: none).
    stream: u64,
    in_stream: u64,
    codec: u8,
    /// The codec's name, as its maker's ID says.
    pub name: String,
    afg: u16,
    widgets: Vec<Widget>,
    /// The audio function's own amplifier capabilities (for widgets without their own).
    out_amp: u32,
    in_amp: u32,
    outputs: Vec<Output>,
    inputs: Vec<Input>,
    /// When the sound playing now ends (time::now_us), if one is.
    until: Option<u64>,
    recording: Option<Recording>,
}

unsafe impl Send for Hda {}

impl Hda {
    fn r8(&self, reg: u64) -> u8 {
        unsafe { read_volatile((self.regs + reg) as *const u8) }
    }
    fn w8(&self, reg: u64, v: u8) {
        unsafe { write_volatile((self.regs + reg) as *mut u8, v) }
    }
    fn r16(&self, reg: u64) -> u16 {
        unsafe { read_volatile((self.regs + reg) as *const u16) }
    }
    fn w16(&self, reg: u64, v: u16) {
        unsafe { write_volatile((self.regs + reg) as *mut u16, v) }
    }
    fn r32(&self, reg: u64) -> u32 {
        unsafe { read_volatile((self.regs + reg) as *const u32) }
    }
    fn w32(&self, reg: u64, v: u32) {
        unsafe { write_volatile((self.regs + reg) as *mut u32, v) }
    }

    fn wait(&self, ms: u64, done: impl Fn(&Self) -> bool) -> bool {
        let end = time::now_us() + ms * 1000;
        while time::now_us() < end {
            if done(self) {
                return true;
            }
            time::delay_us(20);
        }
        done(self)
    }

    /// Starts the controller at `f`, finds its codecs, and the first with an audio function that has outputs (a
    /// graphics card's controller has only its screens' digital ones, which aren't used).
    pub fn start(f: &pci::Function) -> Result<Hda, String> {
        f.at.power_on();
        let bar = f.at.memory_bar(0).ok_or_else(|| String::from("its registers aren't in memory"))?;
        if bar.size < 0x100 {
            return Err(String::from("its registers are too few"));
        }
        let regs = paging::map_device(bar.phys, bar.size.min(0x4000), Cache::Uncached);
        f.at.enable(true);
        snoop(f);
        let rings = Page::new(true).ok_or_else(|| String::from("out of memory"))?;
        let mut d = Hda {
            regs,
            wide: false,
            rings,
            corb_entries: 256,
            rirb_entries: 256,
            corb_wp: 0,
            rirb_rp: 0,
            stream: 0,
            in_stream: 0,
            codec: 0,
            name: String::new(),
            afg: 0,
            widgets: Vec::new(),
            out_amp: 0,
            in_amp: 0,
            outputs: Vec::new(),
            inputs: Vec::new(),
            until: None,
            recording: None,
        };
        let gcap = d.r16(GCAP);
        d.wide = gcap & GCAP_64BIT != 0;
        let (outputs, inputs) = ((gcap >> 12) & 0xF, (gcap >> 8) & 0xF);
        if outputs == 0 {
            return Err(String::from("it has no output streams"));
        }
        d.stream = 0x80 + 0x20 * inputs as u64;
        d.in_stream = if inputs > 0 { 0x80 } else { 0 };

        // Everything stopped (what the firmware left running), and the controller reset and started again.
        d.w32(INTCTL, 0);
        d.w8(CORBCTL, 0);
        d.w8(RIRBCTL, 0);
        for i in 0..(inputs + outputs + ((gcap >> 3) & 0x1F)) as u64 {
            let sd = 0x80 + 0x20 * i;
            d.w32(sd + SD_CTL, d.r32(sd + SD_CTL) & !SD_RUN & 0x00FF_FFFF);
        }
        d.wait(10, |d| d.r8(CORBCTL) & CORBCTL_RUN == 0 && d.r8(RIRBCTL) & RIRBCTL_DMA == 0);
        d.w32(GCTL, d.r32(GCTL) & !GCTL_RUNNING);
        if !d.wait(100, |d| d.r32(GCTL) & GCTL_RUNNING == 0) {
            return Err(String::from("it didn't go into reset"));
        }
        time::delay_us(200);
        d.w32(GCTL, d.r32(GCTL) | GCTL_RUNNING);
        if !d.wait(100, |d| d.r32(GCTL) & GCTL_RUNNING != 0) {
            return Err(String::from("it didn't come out of reset"));
        }
        // Codecs say they're there within 25 frames of the link (521 µs).
        time::delay_ms(1);
        d.wait(100, |d| d.r16(STATESTS) & 0x7FFF != 0);
        let present = d.r16(STATESTS) & 0x7FFF;
        d.w16(STATESTS, present);
        if present == 0 {
            return Err(String::from("no codec answers on its link"));
        }
        d.start_rings()?;

        let mut why = String::from("no codec has outputs");
        for codec in (0..15u8).filter(|c| present & 1 << c != 0) {
            match d.read_codec(codec) {
                Ok(()) => return Ok(d),
                Err(e) => why = format!("codec {codec}: {e}"),
            }
        }
        Err(why)
    }

    /// The command and response rings, as big as the controller has them, running.
    fn start_rings(&mut self) -> Result<(), String> {
        let base = self.rings.phys;
        let (corb_size, entries) = ring_size(self.r8(CORBSIZE));
        self.corb_entries = entries;
        self.w8(CORBSIZE, (self.r8(CORBSIZE) & !3) | corb_size);
        self.w32(CORBLBASE, base as u32);
        self.w32(CORBUBASE, (base >> 32) as u32);
        // Its read pointer reset (some controllers never show the reset bit set: that's not waited for long).
        self.w16(CORBRP, CORBRP_RESET);
        self.wait(1, |d| d.r16(CORBRP) & CORBRP_RESET != 0);
        self.w16(CORBRP, 0);
        self.wait(1, |d| d.r16(CORBRP) & CORBRP_RESET == 0);
        self.w16(CORBWP, 0);
        self.corb_wp = 0;

        let (rirb_size, entries) = ring_size(self.r8(RIRBSIZE));
        self.rirb_entries = entries;
        self.w8(RIRBSIZE, (self.r8(RIRBSIZE) & !3) | rirb_size);
        self.w32(RIRBLBASE, (base + 2048) as u32);
        self.w32(RIRBUBASE, ((base + 2048) >> 32) as u32);
        self.w16(RIRBWP, RIRBWP_RESET);
        self.rirb_rp = 0;
        self.w16(RINTCNT, 1);
        self.w8(RIRBSTS, RIRBSTS_CLEAR);
        self.w8(RIRBCTL, RIRBCTL_DMA | RIRBCTL_COUNT);
        self.w8(CORBCTL, CORBCTL_RUN);
        if !self.wait(10, |d| d.r8(CORBCTL) & CORBCTL_RUN != 0) {
            return Err(String::from("its command ring didn't start"));
        }
        Ok(())
    }

    /// Sends a command to node `nid` of codec `codec`, and waits for its answer.
    fn command(&mut self, codec: u8, nid: u16, verb: u32) -> Option<u32> {
        let entry = (codec as u32) << 28 | ((nid as u32) & 0xFF) << 20 | (verb & 0xF_FFFF);
        self.corb_wp = (self.corb_wp + 1) % self.corb_entries;
        let slot = self.rings.at::<u32>(4 * self.corb_wp as usize);
        unsafe { write_volatile(slot, entry) };
        cpu::flush_line(slot as u64);
        cpu::memory_fence();
        self.w16(CORBWP, self.corb_wp);
        let end = time::now_us() + ANSWER_MS * 1000;
        loop {
            let wp = self.r16(RIRBWP) & 0xFF;
            while self.rirb_rp != wp % self.rirb_entries {
                self.rirb_rp = (self.rirb_rp + 1) % self.rirb_entries;
                let at = self.rings.at::<u32>(2048 + 8 * self.rirb_rp as usize);
                cpu::flush_line(at as u64);
                cpu::memory_fence();
                let (answer, extra) = unsafe { (read_volatile(at), read_volatile(at.add(1))) };
                self.w8(RIRBSTS, RIRBSTS_CLEAR);
                // (An unsolicited one, a jack plugged in say, isn't the answer.)
                if extra & 1 << 4 == 0 && (extra & 0xF) as u8 == codec {
                    return Some(answer);
                }
            }
            if time::now_us() >= end {
                self.w8(RIRBSTS, RIRBSTS_CLEAR);
                return None;
            }
            time::delay_us(10);
        }
    }

    fn verb(&mut self, nid: u16, verb: u32, payload: u32) -> Option<u32> {
        self.command(self.codec, nid, verb << 8 | (payload & 0xFF))
    }

    fn verb16(&mut self, nid: u16, verb: u32, payload: u32) -> Option<u32> {
        self.command(self.codec, nid, verb << 16 | (payload & 0xFFFF))
    }

    fn parameter(&mut self, nid: u16, p: u8) -> u32 {
        self.verb(nid, GET_PARAMETER, p as u32).unwrap_or(0)
    }

    /// Codec `codec`: its audio function's widgets, and its outputs with the way to each.
    fn read_codec(&mut self, codec: u8) -> Result<(), String> {
        self.codec = codec;
        let id = self.verb(0, GET_PARAMETER, P_VENDOR as u32).ok_or_else(|| String::from("it doesn't answer"))?;
        self.name = codec_name(id);
        let nodes = self.parameter(0, P_NODES);
        let (first, count) = ((nodes >> 16) & 0xFF, nodes & 0xFF);
        let afg = (first..first + count).find(|&n| self.parameter(n as u16, P_FUNCTION) & 0xFF == AUDIO_FUNCTION);
        let afg = afg.ok_or_else(|| format!("{} has no audio function", self.name))? as u16;
        self.afg = afg;
        self.verb(afg, SET_POWER_STATE, 0);
        time::delay_ms(10);
        self.out_amp = self.parameter(afg, P_OUT_AMP);
        self.in_amp = self.parameter(afg, P_IN_AMP);
        let nodes = self.parameter(afg, P_NODES);
        let (first, count) = ((nodes >> 16) & 0xFF, nodes & 0xFF);
        self.widgets.clear();
        for nid in first..first + count {
            let nid = nid as u16;
            let caps = self.parameter(nid, P_WIDGET);
            let mut w = Widget { nid, caps, pin_caps: 0, config: 0, connections: Vec::new() };
            if w.kind() == PIN {
                w.pin_caps = self.parameter(nid, P_PIN);
                w.config = self.verb(nid, GET_CONFIG_DEFAULT, 0).unwrap_or(0);
            }
            if caps & W_CONNECTIONS != 0 {
                w.connections = self.connections(nid);
            }
            self.widgets.push(w);
        }
        // The outputs: pins that can drive one, the maker says are connected to speakers, headphones or a line out,
        // and that a DAC reaches. (Digital ones, S/PDIF and the screens' HDMI, aren't used.)
        self.outputs.clear();
        let pins: Vec<(u16, u32, bool)> = self
            .widgets
            .iter()
            .filter(|w| w.kind() == PIN && w.pin_caps & PIN_OUT != 0 && w.caps & W_DIGITAL == 0)
            .filter(|w| w.config >> 30 != CONNECTED_NOTHING && matches!((w.config >> 20) & 0xF, LINE_OUT | SPEAKER | HEADPHONES))
            .map(|w| (w.nid, (w.config >> 20) & 0xF, w.config >> 30 == CONNECTED_BUILT_IN))
            .collect();
        for (pin, device, built_in) in pins {
            let mut seen = vec![pin];
            if let Some(path) = self.path_to_dac(pin, 0, &mut seen) {
                self.outputs.push(Output { pin, device, built_in, path });
            }
        }
        if self.outputs.is_empty() {
            return Err(format!("{} has no outputs Chat Computer can use", self.name));
        }
        // The inputs: pins that can take a signal in, the maker says are connected to a microphone or a line in, and
        // that an ADC reaches.
        self.inputs.clear();
        let pins: Vec<(u16, u32, bool)> = self
            .widgets
            .iter()
            .filter(|w| w.kind() == PIN && w.pin_caps & PIN_IN != 0 && w.caps & W_DIGITAL == 0)
            .filter(|w| w.config >> 30 != CONNECTED_NOTHING && matches!((w.config >> 20) & 0xF, MICROPHONE | LINE_IN))
            .map(|w| (w.nid, (w.config >> 20) & 0xF, w.config >> 30 == CONNECTED_BUILT_IN))
            .collect();
        let adcs: Vec<u16> = self.widgets.iter().filter(|w| w.kind() == INPUT && w.caps & W_DIGITAL == 0).map(|w| w.nid).collect();
        for (pin, device, built_in) in pins {
            for &adc in &adcs {
                let mut seen = vec![adc];
                if let Some(path) = self.path_to_pin(adc, pin, 0, &mut seen) {
                    self.inputs.push(Input { pin, device, built_in, path });
                    break;
                }
            }
        }
        Ok(())
    }

    /// The way from node `from` (an ADC, or a mixer or selector on the way) to the pin `pin` through mixers and
    /// selectors (at most four), as (node, the connection it takes) pairs, the pin last.
    fn path_to_pin(&self, from: u16, pin: u16, depth: u32, seen: &mut Vec<u16>) -> Option<Vec<(u16, usize)>> {
        let w = self.widget(from)?;
        if let Some(i) = w.connections.iter().position(|&n| n == pin) {
            return Some(vec![(from, i), (pin, 0)]);
        }
        if depth >= 4 {
            return None;
        }
        for (i, &next) in w.connections.iter().enumerate() {
            let Some(n) = self.widget(next) else { continue };
            if !matches!(n.kind(), MIXER | SELECTOR) || seen.contains(&next) {
                continue;
            }
            seen.push(next);
            if let Some(mut rest) = self.path_to_pin(next, pin, depth + 1, seen) {
                rest.insert(0, (from, i));
                return Some(rest);
            }
        }
        None
    }

    /// The nodes node `nid` takes its input from, in order (ranges written out).
    fn connections(&mut self, nid: u16) -> Vec<u16> {
        let p = self.parameter(nid, P_CONNECTIONS);
        let (long, n) = (p & 0x80 != 0, (p & 0x7F) as usize);
        let (per, bits, range) = if long { (2, 16, 0x8000u32) } else { (4, 8, 0x80u32) };
        let mut list: Vec<u16> = Vec::new();
        let mut i = 0;
        while i < n {
            let r = self.verb(nid, GET_CONNECTION_LIST, i as u32).unwrap_or(0);
            for k in 0..per {
                if i + k >= n {
                    break;
                }
                let e = (r >> (bits * k)) & ((1 << bits) - 1);
                let id = (e & !range) as u16;
                match list.last() {
                    Some(&last) if e & range != 0 && id > last && id - last < 256 => list.extend(last + 1..=id),
                    _ => list.push(id),
                }
            }
            i += per;
        }
        list
    }

    fn widget(&self, nid: u16) -> Option<&Widget> {
        self.widgets.iter().find(|w| w.nid == nid)
    }

    /// The way from node `from` to a DAC through mixers and selectors (at most four), as (node, the connection it
    /// takes) pairs, the DAC last.
    fn path_to_dac(&self, from: u16, depth: u32, seen: &mut Vec<u16>) -> Option<Vec<(u16, usize)>> {
        let w = self.widget(from)?;
        // A DAC straight on first, then through what's between.
        for (i, &next) in w.connections.iter().enumerate() {
            if let Some(n) = self.widget(next)
                && n.kind() == OUTPUT
                && n.caps & W_DIGITAL == 0
            {
                return Some(vec![(from, i), (next, 0)]);
            }
        }
        if depth >= 4 {
            return None;
        }
        for (i, &next) in w.connections.iter().enumerate() {
            let Some(n) = self.widget(next) else { continue };
            if !matches!(n.kind(), MIXER | SELECTOR) || seen.contains(&next) {
                continue;
            }
            seen.push(next);
            if let Some(mut rest) = self.path_to_dac(next, depth + 1, seen) {
                rest.insert(0, (from, i));
                return Some(rest);
            }
        }
        None
    }

    /// What it plays on and hears from, for people: "Realtek ALC897: speakers, headphones; hears a built-in
    /// microphone, a microphone jack".
    /// Whether it has something to listen to (a microphone or a line in), and an input stream to record it with.
    pub fn has_input(&self) -> bool {
        self.in_stream != 0 && !self.inputs.is_empty()
    }

    pub fn describe(&self) -> String {
        let mut kinds: Vec<&str> = Vec::new();
        for o in &self.outputs {
            let k = match o.device {
                SPEAKER => "speakers",
                HEADPHONES => "headphones",
                _ => "line out",
            };
            if !kinds.contains(&k) {
                kinds.push(k);
            }
        }
        let mut heard: Vec<&str> = Vec::new();
        for i in &self.inputs {
            let k = input_kind(i);
            if !heard.contains(&k) {
                heard.push(k);
            }
        }
        let hears = if heard.is_empty() { String::new() } else { format!("; hears {}", heard.join(", ")) };
        format!("{}: {}{hears}", self.name, kinds.join(", "))
    }

    /// For the log: each output, and the way to it.
    pub fn paths(&self) -> String {
        let mut out = String::new();
        for o in &self.outputs {
            let way: Vec<String> = o.path.iter().map(|(n, _)| format!("{n:#x}")).collect();
            let what = match o.device {
                SPEAKER => "speakers",
                HEADPHONES => "headphones",
                _ => "line out",
            };
            out += &format!("{}{what} {}", if out.is_empty() { "" } else { "; " }, way.join(" < "));
        }
        for i in &self.inputs {
            let way: Vec<String> = i.path.iter().map(|(n, _)| format!("{n:#x}")).collect();
            out += &format!("{}{} {}", if out.is_empty() { "" } else { "; " }, input_kind(i), way.join(" < "));
        }
        out
    }

    /// The amplifier capabilities of widget `w` (its own, or its audio function's).
    fn amp_caps(&mut self, nid: u16, caps: u32, output: bool) -> u32 {
        if caps & W_AMP_OWN != 0 {
            self.parameter(nid, if output { P_OUT_AMP } else { P_IN_AMP })
        } else if output {
            self.out_amp
        } else {
            self.in_amp
        }
    }

    /// Sets an amplifier of `nid` (its output's, or input `input`'s), both channels, to 0 dB (or muted).
    fn amp(&mut self, nid: u16, caps: u32, input: Option<usize>, mute: bool) {
        let amp = self.amp_caps(nid, caps, input.is_none());
        let zero_db = amp & 0x7F;
        let which = match input {
            None => AMP_OUT,
            Some(i) => AMP_IN | ((i as u32 & 0xF) << 8),
        };
        self.verb16(nid, SET_AMP, which | AMP_BOTH | if mute { AMP_MUTE } else { zero_db });
    }

    /// Whether headphones are plugged into one of the headphone jacks (that can tell).
    fn headphones_in(&mut self) -> bool {
        let jacks: Vec<(u16, u32)> = self
            .outputs
            .iter()
            .filter(|o| o.device == HEADPHONES && !o.built_in)
            .filter_map(|o| self.widget(o.pin).map(|w| (o.pin, w.pin_caps)))
            .collect();
        jacks.into_iter().any(|(pin, caps)| self.plugged(pin, caps))
    }

    /// Whether something's plugged into the jack at `pin` (false if it can't tell).
    fn plugged(&mut self, pin: u16, caps: u32) -> bool {
        if caps & PIN_PRESENCE == 0 {
            return false;
        }
        if caps & PIN_TRIGGER != 0 {
            self.verb(pin, SENSE_PIN, 0);
            time::delay_us(100);
        }
        self.verb(pin, GET_PIN_SENSE, 0).unwrap_or(0) & 1 << 31 != 0
    }

    /// Every output set up to play the stream: the ways to them powered, connected and unmuted, their pins on (the
    /// built-in speakers off while headphones are plugged in, as on any computer), their external amplifiers on.
    fn set_outputs(&mut self) {
        let headphones = self.headphones_in();
        let afg = self.afg;
        self.verb(afg, SET_POWER_STATE, 0);
        for k in 0..self.outputs.len() {
            let (pin, device, built_in, path) = {
                let o = &self.outputs[k];
                (o.pin, o.device, o.built_in, o.path.clone())
            };
            let on = !(device == SPEAKER && built_in && headphones);
            for (j, &(nid, input)) in path.iter().enumerate() {
                let Some((caps, kind, inputs)) = self.widget(nid).map(|w| (w.caps, w.kind(), w.connections.len())) else { continue };
                if caps & W_POWER != 0 {
                    self.verb(nid, SET_POWER_STATE, 0);
                }
                let last = j + 1 == path.len();
                if !last && kind != MIXER && inputs > 1 {
                    self.verb(nid, SET_CONNECTION_SELECT, input as u32);
                }
                if !last && kind == MIXER && caps & W_IN_AMP != 0 {
                    // Only the DAC's way in heard (not the microphone's loop back to the speakers, say).
                    for i in 0..inputs {
                        self.amp(nid, caps, Some(i), i != input);
                    }
                }
                if !last && kind == PIN && caps & W_IN_AMP != 0 {
                    self.amp(nid, caps, Some(0), true);
                }
                if caps & W_OUT_AMP != 0 {
                    self.amp(nid, caps, None, !on && kind == PIN);
                }
            }
            let Some(pin_caps) = self.widget(pin).map(|w| w.pin_caps) else { continue };
            let mut control = if on { PIN_OUT_ENABLE } else { 0 };
            if on && device == HEADPHONES && pin_caps & PIN_HEADPHONE_DRIVE != 0 {
                control |= PIN_HEADPHONE_ENABLE;
            }
            self.verb(pin, SET_PIN_CONTROL, control);
            if pin_caps & PIN_EAPD != 0 {
                let eapd = self.verb(pin, GET_EAPD, 0).unwrap_or(0) & 0xFF;
                self.verb(pin, SET_EAPD, if on { eapd | EAPD } else { eapd & !EAPD });
            }
        }
        // Each DAC on a way plays the stream, both its channels.
        let mut dacs: Vec<u16> = self.outputs.iter().map(|o| o.dac()).collect();
        dacs.sort_unstable();
        dacs.dedup();
        for dac in dacs {
            self.verb16(dac, SET_FORMAT, FORMAT as u32);
            self.verb(dac, SET_CHANNEL_STREAM, STREAM << 4);
        }
    }

    fn sd_ctl(&self) -> u32 {
        self.ctl_of(self.stream)
    }

    fn ctl_of(&self, sd: u64) -> u32 {
        self.r32(sd + SD_CTL) & 0x00FF_FFFF
    }

    /// The output stream stopped and reset.
    fn reset_stream(&mut self) {
        self.reset(self.stream);
    }

    /// The stream at `sd` stopped and reset.
    fn reset(&mut self, sd: u64) {
        self.w32(sd + SD_CTL, self.ctl_of(sd) & !SD_RUN);
        self.wait(5, |d| d.ctl_of(sd) & SD_RUN == 0);
        self.w32(sd + SD_CTL, self.ctl_of(sd) | SD_RESET);
        self.wait(5, |d| d.ctl_of(sd) & SD_RESET != 0);
        self.w32(sd + SD_CTL, self.ctl_of(sd) & !SD_RESET);
        self.wait(5, |d| d.ctl_of(sd) & SD_RESET == 0);
    }

    /// Plays `sound` from its start (whatever was playing stops).
    pub fn play(&mut self, sound: &Sound) {
        self.reset_stream();
        self.set_outputs();
        let sd = self.stream;
        self.w32(sd + SD_CBL, sound.bytes);
        self.w16(sd + SD_LVI, sound.entries() - 1);
        self.w16(sd + SD_FMT, FORMAT);
        self.w32(sd + SD_BDPL, sound.list.phys as u32);
        self.w32(sd + SD_BDPU, (sound.list.phys >> 32) as u32);
        let ctl = (self.sd_ctl() & !(0xF << SD_TAG_SHIFT)) | STREAM << SD_TAG_SHIFT;
        self.w32(sd + SD_CTL, ctl);
        self.w32(sd + SD_CTL, ctl | SD_RUN);
        self.until = Some(time::now_us() + sound.micros + 60_000);
    }

    /// Stops the stream once the sound has played.
    pub fn work(&mut self) {
        if self.until.is_some_and(|t| time::now_us() >= t) {
            self.until = None;
            self.reset_stream();
        }
    }

    /// Stops what's playing, at once.
    pub fn stop(&mut self) {
        if self.until.take().is_some() {
            self.reset_stream();
        }
    }

    /// Which input to listen to: a microphone plugged into a jack that can tell it's there, else a built-in
    /// microphone, else a microphone jack that can't tell, else a line in.
    fn chosen_input(&mut self) -> Option<usize> {
        for k in 0..self.inputs.len() {
            let (pin, device, built_in) = (self.inputs[k].pin, self.inputs[k].device, self.inputs[k].built_in);
            let caps = self.widget(pin).map_or(0, |w| w.pin_caps);
            if device == MICROPHONE && !built_in && self.plugged(pin, caps) {
                return Some(k);
            }
        }
        let jack_that_cant_tell = |i: &Input, caps: u32| i.device == MICROPHONE && caps & PIN_PRESENCE == 0;
        self.inputs
            .iter()
            .position(|i| i.device == MICROPHONE && i.built_in)
            .or_else(|| self.inputs.iter().position(|i| jack_that_cant_tell(i, self.widget(i.pin).map_or(0, |w| w.pin_caps))))
            .or_else(|| self.inputs.iter().position(|i| i.device == LINE_IN))
    }

    /// Input `k`'s way, from its pin to its ADC, set up to record: powered, connected and unmuted at 0 dB (a mixer's
    /// other inputs muted), the pin taking a signal in (a microphone given its bias, and its own amplifier's 20 dB),
    /// the ADC making the input stream, both channels.
    fn set_input(&mut self, k: usize) {
        let afg = self.afg;
        self.verb(afg, SET_POWER_STATE, 0);
        let (pin, device, path) = (self.inputs[k].pin, self.inputs[k].device, self.inputs[k].path.clone());
        for &(nid, input) in &path[..path.len() - 1] {
            let Some((caps, kind, inputs)) = self.widget(nid).map(|w| (w.caps, w.kind(), w.connections.len())) else { continue };
            if caps & W_POWER != 0 {
                self.verb(nid, SET_POWER_STATE, 0);
            }
            if kind != MIXER && inputs > 1 {
                self.verb(nid, SET_CONNECTION_SELECT, input as u32);
            }
            if caps & W_IN_AMP != 0 {
                if kind == MIXER {
                    for i in 0..inputs {
                        self.amp(nid, caps, Some(i), i != input);
                    }
                } else {
                    self.amp(nid, caps, Some(input), false);
                }
            }
            if caps & W_OUT_AMP != 0 {
                self.amp(nid, caps, None, false);
            }
        }
        let Some((caps, pin_caps)) = self.widget(pin).map(|w| (w.caps, w.pin_caps)) else { return };
        if caps & W_POWER != 0 {
            self.verb(pin, SET_POWER_STATE, 0);
        }
        let mut control = PIN_IN_ENABLE;
        if device == MICROPHONE && pin_caps & PIN_VREF_80 != 0 {
            control |= PIN_BIAS_80;
        }
        self.verb(pin, SET_PIN_CONTROL, control);
        if caps & W_IN_AMP != 0 {
            self.boost(pin, caps, if device == MICROPHONE { BOOST_DB } else { 0 });
        }
        let adc = self.inputs[k].adc();
        self.verb16(adc, SET_FORMAT, FORMAT as u32);
        self.verb(adc, SET_CHANNEL_STREAM, IN_STREAM << 4);
    }

    /// The input amplifier of `nid` (both channels) set `db` above 0 dB, as near as its steps go.
    fn boost(&mut self, nid: u16, caps: u32, db: u32) {
        let amp = self.amp_caps(nid, caps, false);
        // (Its 0 dB step, its last step, and each step's size in quarters of a dB.)
        let (zero, last, size) = (amp & 0x7F, (amp >> 8) & 0x7F, ((amp >> 16) & 0x7F) + 1);
        let step = (zero + (db * 4 + size / 2) / size).min(last.max(zero));
        self.verb16(nid, SET_AMP, AMP_IN | AMP_BOTH | step);
    }

    /// Starts listening: the input chosen set up, and the first input stream recording it into a ring of memory, 48
    /// kHz in 16 bits, both channels. What it listens to ("a built-in microphone"), or why it can't.
    pub fn listen(&mut self) -> Result<String, String> {
        self.stop_listening();
        if self.in_stream == 0 {
            return Err(String::from("its controller has no input streams"));
        }
        let input = self.chosen_input().ok_or_else(|| String::from("it has no microphone or line in"))?;
        let low = !self.wide;
        let mut pages = Vec::with_capacity(HEARD_PAGES);
        for _ in 0..HEARD_PAGES {
            pages.push(Page::new(low).ok_or_else(|| String::from("out of memory"))?);
        }
        let list = Page::new(low).ok_or_else(|| String::from("out of memory"))?;
        for (i, page) in pages.iter().enumerate() {
            unsafe {
                write_volatile(list.at::<u64>(16 * i), page.phys);
                write_volatile(list.at::<u32>(16 * i + 8), PAGE_SIZE as u32);
                write_volatile(list.at::<u32>(16 * i + 12), 0);
            }
        }
        for line in (0..PAGE_SIZE).step_by(64) {
            cpu::flush_line(list.at::<u8>(line as usize) as u64);
        }
        cpu::memory_fence();
        let sd = self.in_stream;
        self.reset(sd);
        self.set_input(input);
        let bytes = (HEARD_PAGES * PAGE_SIZE as usize) as u32;
        self.w32(sd + SD_CBL, bytes);
        self.w16(sd + SD_LVI, HEARD_PAGES as u16 - 1);
        self.w16(sd + SD_FMT, FORMAT);
        self.w32(sd + SD_BDPL, list.phys as u32);
        self.w32(sd + SD_BDPU, (list.phys >> 32) as u32);
        let ctl = (self.ctl_of(sd) & !(0xF << SD_TAG_SHIFT)) | IN_STREAM << SD_TAG_SHIFT;
        self.w32(sd + SD_CTL, ctl);
        self.w32(sd + SD_CTL, ctl | SD_RUN);
        let what = input_kind(&self.inputs[input]);
        self.recording = Some(Recording { pages, list, bytes, read: 0, input });
        Ok(String::from(what))
    }

    /// What's been heard since last asked (each sample the mean of its two channels), appended to `out`: as far as
    /// the stream has written it to memory (where it is on the link, less what its FIFO still holds).
    pub fn heard(&mut self, out: &mut Vec<i16>) {
        let sd = self.in_stream;
        let (position, fifo) = (self.r32(sd + SD_LPIB), self.r16(sd + SD_FIFOS) as u32 + 1);
        let Some(r) = &mut self.recording else { return };
        let ready = ((position % r.bytes + r.bytes - r.read) % r.bytes).saturating_sub(fifo.div_ceil(4) * 4) & !3;
        let page_size = PAGE_SIZE as u32;
        for _ in 0..ready / 4 {
            let page = &r.pages[(r.read / page_size) as usize];
            let at = (r.read % page_size) as usize;
            if at % 64 == 0 {
                // (Read from memory, which the controller wrote, not from a line the caches kept from before.)
                cpu::flush_line(page.at::<u8>(at) as u64);
            }
            let v = unsafe { read_volatile(page.at::<u32>(at)) };
            out.push(((v as u16 as i16 as i32 + (v >> 16) as u16 as i16 as i32) / 2) as i16);
            r.read = (r.read + 4) % r.bytes;
        }
    }

    /// Stops listening.
    pub fn stop_listening(&mut self) {
        if let Some(r) = self.recording.take() {
            self.reset(self.in_stream);
            let pin = self.inputs[r.input].pin;
            self.verb(pin, SET_PIN_CONTROL, 0);
            drop(r);
        }
    }

}

/// An input as people call it.
fn input_kind(i: &Input) -> &'static str {
    match (i.device, i.built_in) {
        (MICROPHONE, true) => "a built-in microphone",
        (MICROPHONE, false) => "a microphone jack",
        _ => "a line in",
    }
}

/// The ring size to choose from a ring's size register (the biggest it can be): its setting, and its entries.
fn ring_size(reg: u8) -> (u8, u16) {
    let can = reg >> 4;
    if can & 4 != 0 {
        (2, 256)
    } else if can & 2 != 0 {
        (1, 16)
    } else {
        (0, 2)
    }
}

/// Makes sure the controller's memory reads and writes go through the processor's caches, as Linux does: Intel's
/// controllers can be set not to (no snoop), and AMD's need it switched on. And Intel's traffic class 0 (some codecs
/// crackle otherwise).
fn snoop(f: &pci::Function) {
    match f.vendor {
        0x8086 => {
            f.at.write8(INTEL_TCSEL, f.at.read8(INTEL_TCSEL) & !0x07);
            let devc = f.at.read16(INTEL_DEVC);
            if devc & INTEL_DEVC_NO_SNOOP != 0 {
                f.at.write16(INTEL_DEVC, devc & !INTEL_DEVC_NO_SNOOP);
            }
        }
        0x1002 | 0x1022 => f.at.write8(ATI_MISC, f.at.read8(ATI_MISC) | ATI_SNOOP),
        _ => {}
    }
}

/// A codec's name from its vendor and device ID.
fn codec_name(id: u32) -> String {
    let (vendor, device) = ((id >> 16) as u16, id as u16);
    match vendor {
        0x10EC => format!("Realtek ALC{device:x}"),
        0x14F1 => format!("Conexant CX{device:x}"),
        0x1106 => format!("VIA VT{device:04x}"),
        0x1013 => format!("Cirrus Logic CS{device:x}"),
        0x111D | 0x92C1 => format!("IDT 92HD{device:x}"),
        0x8384 => format!("SigmaTel STAC{device:x}"),
        0x11D4 => format!("Analog Devices AD{device:x}"),
        0x1AF4 => String::from("QEMU's codec"),
        0x8086 => String::from("Intel's HDMI codec"),
        0x10DE => String::from("NVIDIA's HDMI codec"),
        0x1002 => String::from("AMD's HDMI codec"),
        _ => format!("codec {id:08x}"),
    }
}
