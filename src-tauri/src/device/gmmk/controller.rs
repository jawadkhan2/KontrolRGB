//! Raw HID protocol for the Glorious GMMK v1 (Sonix, VID 0x0C45 PID 0x652F).
//!
//! Protocol decoded from USB captures documented by dokutan/rgb_keyboard and
//! Kolossi/GmmkUtil (read as references; this is our own implementation):
//! - Every command is a 64-byte output report with id 0x04 sent to the
//!   vendor HID collection (usage page 0xFF1C — what the official editor
//!   uses), which maps to interrupt-OUT EP 0x03. The firmware acks each
//!   packet with a 64-byte input report on EP 0x82; that ack must be read
//!   after every write (it is the flow control — without it the firmware
//!   drops most of a burst). Feature reports on interface 0 do not work:
//!   only the first packets land, leaving the keyboard dark in an empty
//!   custom profile.
//! - A command burst is framed by a start packet [04 01 00 01] and an end
//!   packet [04 02 00 02].
//! - Per-key color: [04 ck 02 11 03 b5 b6 00 R G B], where (b5, b6) address
//!   the key (profile 1 codes) and ck = b5 + b6 + 0x54 (mod 256).
//! - Settings packets share the shape [04 ck 00 06 01 cmd 00 00 value]:
//!   custom-lighting mode is cmd 0x00 value 0x14 (ck 0x1b), hardware
//!   brightness is cmd 0x01 value 0..=9 (ck 0x08 + value).

use hidapi::{HidApi, HidDevice};

use crate::device::types::Color;
use crate::device::DeviceError;

pub const VID: u16 = 0x0C45;
pub const PID: u16 = 0x652F;

/// Usage page of the GMMK's vendor/config HID collection.
const VENDOR_USAGE_PAGE: u16 = 0xFF1C;

/// How long to wait for the firmware's per-packet ack. Normally it arrives
/// within a few ms; a timeout is not fatal (the references discard the ack
/// contents entirely).
const ACK_TIMEOUT_MS: i32 = 250;

fn comm(e: impl std::fmt::Display) -> DeviceError {
    DeviceError::Comm(e.to_string())
}

/// (b5, b6) wire codes for profile 1, ordered to match
/// `layouts::gmmk_ansi::full_size()` — index i addresses led_index i.
#[rustfmt::skip]
pub const WIRE_CODES: [(u8, u8); 104] = [
    // Row 0: Esc F1-F12 PrtSc ScrLk Pause
    (0x03, 0x00), (0x06, 0x00), (0x09, 0x00), (0x0c, 0x00), (0x0f, 0x00),
    (0x12, 0x00), (0x15, 0x00), (0x18, 0x00), (0x1b, 0x00), (0x1e, 0x00),
    (0x21, 0x00), (0x24, 0x00), (0x27, 0x00), (0x3e, 0x01), (0x41, 0x01),
    (0x44, 0x01),
    // Row 1: ` 1-0 - = Bksp Ins Home PgUp NumLk Np/ Np* Np-
    (0x36, 0x00), (0x39, 0x00), (0x3c, 0x00), (0x3f, 0x00), (0x42, 0x00),
    (0x45, 0x00), (0x48, 0x00), (0x4b, 0x00), (0x4e, 0x00), (0x51, 0x00),
    (0x54, 0x00), (0x57, 0x00), (0x5a, 0x00), (0x26, 0x01), (0x4a, 0x01),
    (0x4d, 0x01), (0x50, 0x01), (0x5d, 0x00), (0x60, 0x00), (0x63, 0x00),
    (0x5c, 0x01),
    // Row 2: Tab Q-P [ ] \ Del End PgDn Np7 Np8 Np9 Np+
    (0x69, 0x00), (0x6c, 0x00), (0x6f, 0x00), (0x72, 0x00), (0x75, 0x00),
    (0x78, 0x00), (0x7b, 0x00), (0x7e, 0x00), (0x81, 0x00), (0x84, 0x00),
    (0x87, 0x00), (0x8a, 0x00), (0x8d, 0x00), (0xc0, 0x00), (0x53, 0x01),
    (0x56, 0x01), (0x59, 0x01), (0x90, 0x00), (0x93, 0x00), (0x96, 0x00),
    (0x5f, 0x01),
    // Row 3: Caps A-L ; ' Enter Np4 Np5 Np6
    (0x9c, 0x00), (0x9f, 0x00), (0xa2, 0x00), (0xa5, 0x00), (0xa8, 0x00),
    (0xab, 0x00), (0xae, 0x00), (0xb1, 0x00), (0xb4, 0x00), (0xb7, 0x00),
    (0xba, 0x00), (0xbd, 0x00), (0xf3, 0x00), (0xc3, 0x00), (0xc6, 0x00),
    (0xc9, 0x00),
    // Row 4: LShift Z-M , . / RShift Up Np1 Np2 Np3 NpEnter
    (0xcf, 0x00), (0xd2, 0x00), (0xd5, 0x00), (0xd8, 0x00), (0xdb, 0x00),
    (0xde, 0x00), (0xe1, 0x00), (0xe4, 0x00), (0xe7, 0x00), (0xea, 0x00),
    (0xed, 0x00), (0xf0, 0x00), (0x20, 0x01), (0xf6, 0x00), (0xf9, 0x00),
    (0xfc, 0x00), (0x2f, 0x01),
    // Row 5: LCtrl Win LAlt Space RAlt Fn Menu RCtrl Left Down Right Np0 Np.
    (0x02, 0x01), (0x05, 0x01), (0x08, 0x01), (0x0b, 0x01), (0x0e, 0x01),
    (0x11, 0x01), (0x14, 0x01), (0x17, 0x01), (0x1a, 0x01), (0x1d, 0x01),
    (0x23, 0x01), (0x29, 0x01), (0x2c, 0x01),
];

pub struct GmmkController {
    dev: HidDevice,
}

impl GmmkController {
    /// Open the vendor HID collection (usage page 0xFF1C), falling back to
    /// interface 1 if the usage page isn't reported.
    /// Ok(None) = keyboard not present.
    pub fn open() -> Result<Option<Self>, DeviceError> {
        let api = HidApi::new().map_err(comm)?;
        let gmmk =
            |d: &&hidapi::DeviceInfo| d.vendor_id() == VID && d.product_id() == PID;
        let Some(info) = api
            .device_list()
            .find(|d| gmmk(d) && d.usage_page() == VENDOR_USAGE_PAGE)
            .or_else(|| {
                api.device_list().find(|d| gmmk(d) && d.interface_number() == 1)
            })
        else {
            return Ok(None);
        };
        let dev = api.open_path(info.path()).map_err(comm)?;
        Ok(Some(GmmkController { dev }))
    }

    fn cmd(&self, payload: &[u8]) -> Result<(), DeviceError> {
        let mut buf = [0u8; 64];
        buf[..payload.len()].copy_from_slice(payload);
        self.dev.write(&buf).map_err(comm)?;
        // Consume the firmware's ack before the next packet (see module doc).
        let mut ack = [0u8; 64];
        self.dev.read_timeout(&mut ack, ACK_TIMEOUT_MS).map_err(comm)?;
        Ok(())
    }

    pub fn begin(&self) -> Result<(), DeviceError> {
        self.cmd(&[0x04, 0x01, 0x00, 0x01])
    }

    pub fn end(&self) -> Result<(), DeviceError> {
        self.cmd(&[0x04, 0x02, 0x00, 0x02])
    }

    /// Make profile 1 the active profile. All our other packets address
    /// profile 1, so this guards against the keyboard sitting on profile
    /// 2/3 (Fn shortcuts), where our writes would be invisible. Packet from
    /// dokutan's _data_profile table (byte 18 = profile index, byte 1 =
    /// 0xe0 + index); sent outside begin()/end(), as the references do.
    pub fn set_active_profile_1(&self) -> Result<(), DeviceError> {
        self.cmd(&[
            0x04, 0xe0, 0x03, 0x04, 0x2c, 0x00, 0x00, 0x00, 0x55, 0xaa, 0xff,
            0x02, 0x45, 0x0c, 0x2f, 0x65, 0x03, 0x01, 0x00, 0x08, 0x00, 0x00,
            0x00, 0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x08, 0x07, 0x09,
            0x0b, 0x0a, 0x0c, 0x0d, 0x0f, 0x0e, 0x10, 0x12, 0x11, 0x14,
        ])
    }

    /// Switch profile 1 to the custom (per-key) lighting mode.
    pub fn set_custom_mode(&self) -> Result<(), DeviceError> {
        self.cmd(&[0x04, 0x1b, 0x00, 0x06, 0x01, 0x00, 0x00, 0x00, 0x14])
    }

    /// Hardware brightness 0..=9 for profile 1. We pin it to max and do
    /// brightness in software (the effects engine scales RGB).
    pub fn set_hw_brightness(&self, brightness: u8) -> Result<(), DeviceError> {
        let b = brightness.min(9);
        self.cmd(&[0x04, 0x08 + b, 0x00, 0x06, 0x01, 0x01, 0x00, 0x00, b])
    }

    /// Set one key's color (profile 1). Must be called inside begin()/end().
    pub fn set_key(&self, led_index: usize, color: Color) -> Result<(), DeviceError> {
        let (b5, b6) = WIRE_CODES[led_index];
        let ck = b5.wrapping_add(b6).wrapping_add(0x54);
        self.cmd(&[
            0x04, ck, 0x02, 0x11, 0x03, b5, b6, 0x00, color.r, color.g, color.b,
        ])
    }
}
