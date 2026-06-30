//! Gigabyte "Gen4" GPU RGB wire protocol (RTX 40/50-series boards).
//!
//! Verified on an RTX 5080 Gaming OC. The controller is an ITE chip on the
//! GPU's I2C bus, reached over NvAPI port 1 (see [`super::nvapi`]). Protocol
//! facts were read from SignalRGB's `Gigabyte_Gen4_GPU` plugin and OpenRGB as
//! documentation only — never linked or copied.
//!
//! There is no mode/speed register: the controller takes a static colour per
//! zone and the host streams frames for any animation. A colour write is one
//! 64-byte I2C block:
//!
//!   `[H0, H1, H2, H3, H4,  R, G, B, 0x00, zoneId]` then zero-padded.
//!
//! The 5080 Gaming OC maps to SignalRGB's "AERO" layout: a single LED on zone 0
//! with the header `[0x16, 0x01, 0x00, 0x06, 0x00]`. The card has no zones
//! beyond 0 — writing colour to other indices (as the older `[0x12, …]` default
//! header did to all six) wedges the controller under streaming. So we write
//! exactly one block, to zone 0. Detection writes the `[0x11, 0x01]` query and
//! reads back a 4-byte reply whose last two bytes are the model id (0x4176).

use std::sync::Arc;

use crate::device::types::Color;
use crate::device::DeviceError;

use super::nvapi::{GpuHandle, NvApi};

/// I2C address of the Gen4 controller on 40/50-series Gigabyte boards.
pub const ADDR: u8 = 0x75;

/// Gigabyte PCI subsystem-vendor id, used to confirm the card before probing.
pub const GIGABYTE_SUB_VEN: u16 = 0x1458;

/// Colour blocks are full 64-byte I2C writes regardless of payload length.
const BLOCK_LEN: usize = 64;

/// The card's only lit zone (AERO "Side logo").
const ZONE: u8 = 0;

/// AERO colour-block header preceding `[R, G, B, 0x00, zoneId]`.
const COLOR_HEADER: [u8; 5] = [0x16, 0x01, 0x00, 0x06, 0x00];

/// Detect-block header; the reply's last two bytes carry the board model id.
const DETECT_HEADER: [u8; 2] = [0x11, 0x01];

/// Gen4 controller model id reported by the RTX 40/50-series Gaming OC boards
/// (verified on the 5080 Gaming OC). Matched in both byte orders so a different
/// chip happening to answer at `0x75` with arbitrary non-zero bytes is rejected.
const MODEL_ID: u16 = 0x4176;

/// Handle to one detected controller: the shared NvAPI binding plus the GPU it
/// lives on.
#[derive(Clone)]
pub struct Controller {
    nvapi: Arc<NvApi>,
    handle: GpuHandle,
}

impl Controller {
    pub fn new(nvapi: Arc<NvApi>, handle: GpuHandle) -> Self {
        Controller { nvapi, handle }
    }

    /// Probe `ADDR`: write the detect block, read 4 bytes, and confirm the
    /// controller reported the expected Gen4 model id. A NACK/error or a
    /// mismatched id means no Gen4 controller here.
    pub fn probe(&self) -> bool {
        let mut query = [0u8; BLOCK_LEN];
        query[..DETECT_HEADER.len()].copy_from_slice(&DETECT_HEADER);
        if self.nvapi.i2c_write_block(self.handle, ADDR, &query).is_err() {
            return false;
        }
        match self.nvapi.i2c_read_block(self.handle, ADDR, 4) {
            // Bytes [2],[3] are the model id. Accept either byte order so we
            // don't depend on the bus endianness, but require the known value.
            Ok(resp) if resp.len() >= 4 => {
                let be = ((resp[2] as u16) << 8) | resp[3] as u16;
                let le = ((resp[3] as u16) << 8) | resp[2] as u16;
                be == MODEL_ID || le == MODEL_ID
            }
            _ => false,
        }
    }

    /// Drive the card's logo to one static `color` — a single 64-byte block.
    pub fn set_color(&self, color: Color) -> Result<(), DeviceError> {
        let mut blk = [0u8; BLOCK_LEN];
        blk[..COLOR_HEADER.len()].copy_from_slice(&COLOR_HEADER);
        blk[5] = color.r;
        blk[6] = color.g;
        blk[7] = color.b;
        blk[8] = 0x00;
        blk[9] = ZONE;
        self.nvapi.i2c_write_block(self.handle, ADDR, &blk)
    }
}
