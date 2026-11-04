//! A USB network adapter (or a phone sharing its connection), as a network card: the USB side (usb/net.rs) keeps
//! the frames that arrive and sends the frames given to it; this passes them between it and the network stack.

use super::Card;
use crate::usb;
use alloc::format;
use alloc::string::String;

pub struct UsbNet {
    hc: usize,
    slot: u8,
    id: u64,
    mac: [u8; 6],
    describe: String,
    link: bool,
    /// Whether it's still plugged in.
    present: bool,
}

impl UsbNet {
    pub fn new(a: &usb::NetAdapter) -> UsbNet {
        UsbNet {
            hc: a.hc,
            slot: a.slot,
            id: a.id,
            mac: a.mac,
            describe: format!("USB network adapter \"{}\" ({}, {}), address {}", a.name, a.kind, a.speed, net::Mac(a.mac)),
            link: true,
            present: true,
        }
    }
}

impl Card for UsbNet {
    fn describe(&self) -> String {
        self.describe.clone()
    }

    fn mac(&self) -> [u8; 6] {
        self.mac
    }

    fn link_up(&self) -> bool {
        self.present && self.link
    }

    fn send(&mut self, frame: &[u8]) -> bool {
        match usb::net_send(self.hc, self.slot, self.id, frame) {
            Some(sent) => sent,
            None => {
                self.present = false;
                false
            }
        }
    }

    fn receive(&mut self, took: &mut dyn FnMut(&[u8])) -> usize {
        match usb::net_take(self.hc, self.slot, self.id) {
            Some((frames, link)) => {
                self.link = link;
                for f in &frames {
                    took(f);
                }
                frames.len()
            }
            None => {
                self.present = false;
                0
            }
        }
    }

    fn present(&self) -> bool {
        self.present
    }
}
