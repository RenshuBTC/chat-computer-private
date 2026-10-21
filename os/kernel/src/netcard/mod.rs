//! Network cards: Chat Computer's own drivers for them. So far: virtio's (virtual machines). The rest of the
//! PC's network chips are named in the log, with their maker's and the chip's numbers, so drivers can be
//! written for exactly those.

mod virtio;

use crate::{idt, pci};
use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

/// What every card's driver does.
pub trait Card: Send {
    /// For the log: what it is, where, and how it interrupts.
    fn describe(&self) -> String;
    fn mac(&self) -> [u8; 6];
    /// Whether a cable (or the virtual one) is plugged in.
    fn link_up(&self) -> bool;
    /// Sends one Ethernet frame (without the checksum the card adds). False: no room just now, or broken.
    fn send(&mut self, frame: &[u8]) -> bool;
    /// Hands each frame that has arrived to `took`. Returns how many.
    fn receive(&mut self, took: &mut dyn FnMut(&[u8])) -> usize;
}

/// Who makes the chips most PCs and laptops have, for the log.
fn maker(vendor: u16) -> &'static str {
    match vendor {
        0x8086 => "Intel",
        0x10EC => "Realtek",
        0x14E4 => "Broadcom",
        0x14C3 => "MediaTek",
        0x168C | 0x17CB => "Qualcomm Atheros",
        0x1969 => "Qualcomm Atheros (Killer)",
        0x11AB | 0x1B4B => "Marvell",
        0x1D6A => "Aquantia",
        0x15B3 => "Mellanox",
        0x1AF4 => "virtio",
        _ => "another maker",
    }
}

/// Starts every card there's a driver for; names the others in the log.
pub fn find() -> Vec<Box<dyn Card>> {
    idt::set_device_handler(idt::NET_VECTOR, interrupt);
    let mut cards: Vec<Box<dyn Card>> = Vec::new();
    for f in pci::of_class(0x02) {
        let kind = match f.subclass {
            0x00 => "Ethernet",
            0x80 => "network (Wi-Fi or other)",
            _ => "network",
        };
        if f.vendor == virtio::VENDOR && virtio::DEVICES.contains(&f.device) {
            match virtio::Virtio::start(&f, idt::NET_VECTOR) {
                Ok(card) => cards.push(Box::new(card)),
                Err(why) => crate::log!("Chat Computer kernel: network card at {}: couldn't start it: {why}", f.at),
            }
        } else {
            let name = format!("{} {kind} chip {:04x}:{:04x}", maker(f.vendor), f.vendor, f.device);
            crate::log!("Chat Computer kernel: network: {name} at {} (no driver yet)", f.at);
        }
    }
    cards
}

/// A card's interrupt: frames arrived. The main loop, woken by it, takes them.
fn interrupt() {}
