//! Raw HID protocol for the Glorious GMMK v1 (Sonix, VID 0x0C45 PID 0x652F).
//!
//! Verified on the real unit (2026-06-17/18) via `src/bin/hidprobe.rs`:
//! - Every command is a 64-byte output report (id 0x04) on the vendor HID
//!   collection (usage page 0xFF1C). The firmware acks each packet on an input
//!   report; we drain acks NON-BLOCKING after each write (`read_timeout(0)`
//!   loop). A blocking per-packet read sat the full timeout (~50 ms/pkt → ~5 s
//!   per frame) — the non-blocking drain is the throughput key.
//! - A burst is framed by start [04 01 00 01] and end [04 02 00 02].
//! - Per-key color (custom mode): [04 c0 02 11 03 c1 c2 00 R G B], where
//!   [c0,c1,c2] is the key's wire code (c0 == (c1+c2+0x54)&0xff).
//! - Settings (mode/brightness/speed/direction/rainbow/color) are cmd 0x06
//!   packets built by `pkt()`; the checksum is the LE16 sum of bytes[3..].
//! - Custom (per-key) flow: PAINT every key first, THEN set custom mode 0x14
//!   (order matters — mode-first left the board dark). Do NOT write hardware
//!   brightness 9 — it blacks the board out.

use hidapi::{HidApi, HidDevice};

use crate::device::types::{Color, OnboardEffect, OnboardMode};
use crate::device::DeviceError;

pub const VID: u16 = 0x0C45;
pub const PID: u16 = 0x652F;

/// Usage page of the GMMK's vendor/config HID collection.
const VENDOR_USAGE_PAGE: u16 = 0xFF1C;

fn comm(e: impl std::fmt::Display) -> DeviceError {
    DeviceError::Comm(e.to_string())
}

/// Build a 64-byte settings packet: `04 [cksum_lo cksum_hi] cmd len off_lo
/// off_hi 00 data...`, checksum = LE16 sum of bytes[3..] (verified).
fn pkt(cmd: u8, offset: u16, data: &[u8]) -> [u8; 64] {
    let mut b = [0u8; 64];
    b[0] = 0x04;
    b[3] = cmd;
    b[4] = data.len() as u8;
    b[5] = (offset & 0xff) as u8;
    b[6] = (offset >> 8) as u8;
    b[8..8 + data.len()].copy_from_slice(data);
    let sum: u32 = b[3..].iter().map(|&x| x as u32).sum();
    let ck = (sum & 0xffff) as u16;
    b[1] = (ck & 0xff) as u8;
    b[2] = (ck >> 8) as u8;
    b
}

/// Map an `OnboardMode` to its firmware mode byte and whether rainbow is forced
/// on (Spectrum = the rainbow form of the wave mode).
fn mode_bytes(mode: OnboardMode) -> (u8, bool) {
    match mode {
        OnboardMode::Fixed => (0x06, false),
        OnboardMode::Breathing => (0x05, false),
        OnboardMode::Wave => (0x01, false), // horizontal_wave
        // Spectrum = the rainbow form of horizontal_wave. Mode 0x06 (fixed) is a
        // STATIC solid-color preset that ignores the rainbow byte, so it showed
        // nothing animated — the wave mode with rainbow forced on is what the
        // protocol research (hidprobe MODES) documents as "spectrum".
        OnboardMode::Spectrum => (0x01, true),
        OnboardMode::Reactive => (0x07, false), // reactive_single
        OnboardMode::Swirl => (0x0b, false),
    }
}

/// Map a UI brightness percent (0..=100) to a firmware brightness LEVEL.
/// Verified on this unit (full sweep): the firmware has 5 levels, 0..=4, where
/// 0=off and 4=brightest. Values >=5 black the board out / hang the ack, so we
/// clamp hard into 0..=4 and never send more.
fn fw_brightness(pct: u8) -> u8 {
    match pct {
        0 => 0,      // off
        1..=25 => 1, // dim
        26..=50 => 2,
        51..=75 => 3,
        _ => 4, // brightest
    }
}

pub struct GmmkController {
    dev: HidDevice,
}

impl GmmkController {
    /// Open the vendor HID collection (usage page 0xFF1C), falling back to
    /// interface 1 if the usage page isn't reported. Ok(None) = not present.
    pub fn open() -> Result<Option<Self>, DeviceError> {
        let api = HidApi::new().map_err(comm)?;
        let gmmk = |d: &&hidapi::DeviceInfo| d.vendor_id() == VID && d.product_id() == PID;
        let Some(info) = api
            .device_list()
            .find(|d| gmmk(d) && d.usage_page() == VENDOR_USAGE_PAGE)
            .or_else(|| {
                api.device_list()
                    .find(|d| gmmk(d) && d.interface_number() == 1)
            })
        else {
            return Ok(None);
        };
        let dev = api.open_path(info.path()).map_err(comm)?;
        Ok(Some(GmmkController { dev }))
    }

    /// Write one 64-byte report, then drain any queued acks WITHOUT blocking.
    fn cmd(&self, packet: &[u8; 64]) -> Result<(), DeviceError> {
        self.dev.write(packet).map_err(comm)?;
        let mut ack = [0u8; 64];
        while let Ok(n) = self.dev.read_timeout(&mut ack, 0) {
            if n == 0 {
                break;
            }
        }
        Ok(())
    }

    pub fn begin(&self) -> Result<(), DeviceError> {
        self.cmd(&pkt(0x01, 0, &[]))
    }

    pub fn end(&self) -> Result<(), DeviceError> {
        self.cmd(&pkt(0x02, 0, &[]))
    }

    /// Switch profile 1 to the custom (per-key) lighting mode. Call AFTER
    /// painting keys, in its own begin()/end() burst.
    pub fn set_custom_mode(&self) -> Result<(), DeviceError> {
        self.begin()?;
        self.cmd(&pkt(0x06, 0x00, &[0x14]))?;
        self.end()
    }

    /// Fill the WHOLE board with one color via the firmware's bulk 0x11 color
    /// buffer: `begin` + 7 chunks + `end` ≈ 9 writes for all keys, versus one
    /// write per key. The buffer is 126 slots × 3 bytes = 378 bytes, sent as 7
    /// `pkt(0x11, off, &[54 bytes])` chunks. A SOLID fill is slot-order
    /// independent, so it needs no slot→key calibration. Verified working in
    /// custom mode (2026-06-17). Call inside/at entry to custom mode; on entry
    /// paint BEFORE set_custom_mode().
    pub fn set_all_bulk(&self, color: Color) -> Result<(), DeviceError> {
        const SLOTS: usize = 126;
        const CHUNK: usize = 54; // 18 LEDs × 3 bytes per 0x11 packet
        let mut buf = [0u8; SLOTS * 3];
        for slot in 0..SLOTS {
            buf[slot * 3] = color.r;
            buf[slot * 3 + 1] = color.g;
            buf[slot * 3 + 2] = color.b;
        }
        self.begin()?;
        let mut off = 0;
        while off < buf.len() {
            self.cmd(&pkt(0x11, off as u16, &buf[off..off + CHUNK]))?;
            off += CHUNK;
        }
        self.end()
    }

    /// Set one key's color (custom mode). Call inside a begin()/end() burst.
    pub fn set_key(&self, code: [u8; 3], color: Color) -> Result<(), DeviceError> {
        let p = [
            0x04, code[0], 0x02, 0x11, 0x03, code[1], code[2], 0x00, color.r, color.g, color.b, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        ];
        self.cmd(&p)
    }

    /// Apply a firmware onboard effect in one burst. `brightness_pct` is the
    /// UI 0..=100 value, mapped to the safe firmware level.
    pub fn set_onboard(
        &self,
        effect: &OnboardEffect,
        brightness_pct: u8,
    ) -> Result<(), DeviceError> {
        let (mode, force_rainbow) = mode_bytes(effect.mode);
        let rainbow = (effect.rainbow || force_rainbow) as u8;
        let speed = effect.speed.min(4);
        let dir = if effect.reverse { 0xff } else { 0x00 };
        self.begin()?;
        self.cmd(&pkt(0x06, 0x00, &[mode]))?;
        self.cmd(&pkt(0x06, 0x01, &[fw_brightness(brightness_pct)]))?;
        self.cmd(&pkt(0x06, 0x02, &[0x04 - speed]))?;
        self.cmd(&pkt(0x06, 0x03, &[dir]))?;
        self.cmd(&pkt(0x06, 0x04, &[rainbow]))?;
        self.cmd(&pkt(
            0x06,
            0x05,
            &[effect.color.r, effect.color.g, effect.color.b],
        ))?;
        self.end()
    }
}
