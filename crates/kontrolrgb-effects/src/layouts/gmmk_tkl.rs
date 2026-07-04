//! TKL (87-key) ANSI layout for the GMMK v1, the actual physical unit.
//!
//! Single source of truth: each key pairs its on-screen geometry (key units,
//! 1U = one key) with its 3-byte hardware wire code, so `led_index` (array
//! position) ↔ geometry ↔ wire code can never drift apart. Wire codes are the
//! dokutan ANSI keycodes, verified on this unit by the hidprobe `cal` landmark
//! test (Esc/Backspace/Enter/Space/Up all lit in the right physical spots).
//!
//! The numpad keys (`Num_*`) of the full 104-key table are intentionally
//! dropped — this board has no numpad.

use crate::types::KeyInfo;

pub struct GmmkKey {
    pub label: &'static str,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    /// Wire code: packet bytes [1], [5], [6] of a per-key color command.
    pub code: [u8; 3],
}

const fn k(label: &'static str, x: f32, y: f32, w: f32, h: f32, code: [u8; 3]) -> GmmkKey {
    GmmkKey {
        label,
        x,
        y,
        w,
        h,
        code,
    }
}

#[rustfmt::skip]
const KEYS: &[GmmkKey] = &[
    // Row 0 — function row
    k("Esc", 0.0, 0.0, 1.0, 1.0, [0x57, 0x03, 0x00]),
    k("F1", 2.0, 0.0, 1.0, 1.0, [0x5a, 0x06, 0x00]), k("F2", 3.0, 0.0, 1.0, 1.0, [0x5d, 0x09, 0x00]),
    k("F3", 4.0, 0.0, 1.0, 1.0, [0x60, 0x0c, 0x00]), k("F4", 5.0, 0.0, 1.0, 1.0, [0x63, 0x0f, 0x00]),
    k("F5", 6.5, 0.0, 1.0, 1.0, [0x66, 0x12, 0x00]), k("F6", 7.5, 0.0, 1.0, 1.0, [0x69, 0x15, 0x00]),
    k("F7", 8.5, 0.0, 1.0, 1.0, [0x6c, 0x18, 0x00]), k("F8", 9.5, 0.0, 1.0, 1.0, [0x6f, 0x1b, 0x00]),
    k("F9", 11.0, 0.0, 1.0, 1.0, [0x72, 0x1e, 0x00]), k("F10", 12.0, 0.0, 1.0, 1.0, [0x75, 0x21, 0x00]),
    k("F11", 13.0, 0.0, 1.0, 1.0, [0x78, 0x24, 0x00]), k("F12", 14.0, 0.0, 1.0, 1.0, [0x7b, 0x27, 0x00]),
    k("PrtSc", 15.25, 0.0, 1.0, 1.0, [0x93, 0x3e, 0x01]), k("ScrLk", 16.25, 0.0, 1.0, 1.0, [0x96, 0x41, 0x01]),
    k("Pause", 17.25, 0.0, 1.0, 1.0, [0x99, 0x44, 0x01]),
    // Row 1 — number row
    k("`", 0.0, 1.5, 1.0, 1.0, [0x8a, 0x36, 0x00]),
    k("1", 1.0, 1.5, 1.0, 1.0, [0x8d, 0x39, 0x00]), k("2", 2.0, 1.5, 1.0, 1.0, [0x90, 0x3c, 0x00]),
    k("3", 3.0, 1.5, 1.0, 1.0, [0x93, 0x3f, 0x00]), k("4", 4.0, 1.5, 1.0, 1.0, [0x96, 0x42, 0x00]),
    k("5", 5.0, 1.5, 1.0, 1.0, [0x99, 0x45, 0x00]), k("6", 6.0, 1.5, 1.0, 1.0, [0x9c, 0x48, 0x00]),
    k("7", 7.0, 1.5, 1.0, 1.0, [0x9f, 0x4b, 0x00]), k("8", 8.0, 1.5, 1.0, 1.0, [0xa2, 0x4e, 0x00]),
    k("9", 9.0, 1.5, 1.0, 1.0, [0xa5, 0x51, 0x00]), k("0", 10.0, 1.5, 1.0, 1.0, [0xa8, 0x54, 0x00]),
    k("-", 11.0, 1.5, 1.0, 1.0, [0xab, 0x57, 0x00]), k("=", 12.0, 1.5, 1.0, 1.0, [0xae, 0x5a, 0x00]),
    k("Bksp", 13.0, 1.5, 2.0, 1.0, [0x7b, 0x26, 0x01]),
    k("Ins", 15.25, 1.5, 1.0, 1.0, [0x9f, 0x4a, 0x01]), k("Home", 16.25, 1.5, 1.0, 1.0, [0xa2, 0x4d, 0x01]),
    k("PgUp", 17.25, 1.5, 1.0, 1.0, [0xa5, 0x50, 0x01]),
    // Row 2 — top alpha row
    k("Tab", 0.0, 2.5, 1.5, 1.0, [0xbd, 0x69, 0x00]),
    k("Q", 1.5, 2.5, 1.0, 1.0, [0xc0, 0x6c, 0x00]), k("W", 2.5, 2.5, 1.0, 1.0, [0xc3, 0x6f, 0x00]),
    k("E", 3.5, 2.5, 1.0, 1.0, [0xc6, 0x72, 0x00]), k("R", 4.5, 2.5, 1.0, 1.0, [0xc9, 0x75, 0x00]),
    k("T", 5.5, 2.5, 1.0, 1.0, [0xcc, 0x78, 0x00]), k("Y", 6.5, 2.5, 1.0, 1.0, [0xcf, 0x7b, 0x00]),
    k("U", 7.5, 2.5, 1.0, 1.0, [0xd2, 0x7e, 0x00]), k("I", 8.5, 2.5, 1.0, 1.0, [0xd5, 0x81, 0x00]),
    k("O", 9.5, 2.5, 1.0, 1.0, [0xd8, 0x84, 0x00]), k("P", 10.5, 2.5, 1.0, 1.0, [0xdb, 0x87, 0x00]),
    k("[", 11.5, 2.5, 1.0, 1.0, [0xde, 0x8a, 0x00]), k("]", 12.5, 2.5, 1.0, 1.0, [0xe1, 0x8d, 0x00]),
    k("\\", 13.5, 2.5, 1.5, 1.0, [0x14, 0xc0, 0x00]),
    k("Del", 15.25, 2.5, 1.0, 1.0, [0xa8, 0x53, 0x01]), k("End", 16.25, 2.5, 1.0, 1.0, [0xab, 0x56, 0x01]),
    k("PgDn", 17.25, 2.5, 1.0, 1.0, [0xae, 0x59, 0x01]),
    // Row 3 — home row
    k("Caps", 0.0, 3.5, 1.75, 1.0, [0xf0, 0x9c, 0x00]),
    k("A", 1.75, 3.5, 1.0, 1.0, [0xf3, 0x9f, 0x00]), k("S", 2.75, 3.5, 1.0, 1.0, [0xf6, 0xa2, 0x00]),
    k("D", 3.75, 3.5, 1.0, 1.0, [0xf9, 0xa5, 0x00]), k("F", 4.75, 3.5, 1.0, 1.0, [0xfc, 0xa8, 0x00]),
    k("G", 5.75, 3.5, 1.0, 1.0, [0xff, 0xab, 0x00]), k("H", 6.75, 3.5, 1.0, 1.0, [0x02, 0xae, 0x00]),
    k("J", 7.75, 3.5, 1.0, 1.0, [0x05, 0xb1, 0x00]), k("K", 8.75, 3.5, 1.0, 1.0, [0x08, 0xb4, 0x00]),
    k("L", 9.75, 3.5, 1.0, 1.0, [0x0b, 0xb7, 0x00]), k(";", 10.75, 3.5, 1.0, 1.0, [0x0e, 0xba, 0x00]),
    k("'", 11.75, 3.5, 1.0, 1.0, [0x11, 0xbd, 0x00]),
    k("Enter", 12.75, 3.5, 2.25, 1.0, [0x47, 0xf3, 0x00]),
    // Row 4 — bottom alpha row
    k("Shift", 0.0, 4.5, 2.25, 1.0, [0x23, 0xcf, 0x00]),
    k("Z", 2.25, 4.5, 1.0, 1.0, [0x26, 0xd2, 0x00]), k("X", 3.25, 4.5, 1.0, 1.0, [0x29, 0xd5, 0x00]),
    k("C", 4.25, 4.5, 1.0, 1.0, [0x2c, 0xd8, 0x00]), k("V", 5.25, 4.5, 1.0, 1.0, [0x2f, 0xdb, 0x00]),
    k("B", 6.25, 4.5, 1.0, 1.0, [0x32, 0xde, 0x00]), k("N", 7.25, 4.5, 1.0, 1.0, [0x35, 0xe1, 0x00]),
    k("M", 8.25, 4.5, 1.0, 1.0, [0x38, 0xe4, 0x00]), k(",", 9.25, 4.5, 1.0, 1.0, [0x3b, 0xe7, 0x00]),
    k(".", 10.25, 4.5, 1.0, 1.0, [0x3e, 0xea, 0x00]), k("/", 11.25, 4.5, 1.0, 1.0, [0x41, 0xed, 0x00]),
    k("Shift", 12.25, 4.5, 2.75, 1.0, [0x44, 0xf0, 0x00]),
    k("Up", 16.25, 4.5, 1.0, 1.0, [0x75, 0x20, 0x01]),
    // Row 5 — modifier row
    k("Ctrl", 0.0, 5.5, 1.25, 1.0, [0x57, 0x02, 0x01]), k("Win", 1.25, 5.5, 1.25, 1.0, [0x5a, 0x05, 0x01]),
    k("Alt", 2.5, 5.5, 1.25, 1.0, [0x5d, 0x08, 0x01]),
    k("Space", 3.75, 5.5, 6.25, 1.0, [0x60, 0x0b, 0x01]),
    k("Alt", 10.0, 5.5, 1.25, 1.0, [0x63, 0x0e, 0x01]), k("Fn", 11.25, 5.5, 1.25, 1.0, [0x66, 0x11, 0x01]),
    k("Menu", 12.5, 5.5, 1.25, 1.0, [0x69, 0x14, 0x01]), k("Ctrl", 13.75, 5.5, 1.25, 1.0, [0x6c, 0x17, 0x01]),
    k("Left", 15.25, 5.5, 1.0, 1.0, [0x6f, 0x1a, 0x01]), k("Down", 16.25, 5.5, 1.0, 1.0, [0x72, 0x1d, 0x01]),
    k("Right", 17.25, 5.5, 1.0, 1.0, [0x78, 0x23, 0x01]),
];

/// Wire codes in led_index order, for the writer thread.
pub fn wire_codes() -> Vec<[u8; 3]> {
    KEYS.iter().map(|k| k.code).collect()
}

/// Frontend geometry (one KeyInfo per key, led_index = position).
pub fn key_infos() -> Vec<KeyInfo> {
    KEYS.iter()
        .enumerate()
        .map(|(i, k)| KeyInfo {
            led_index: i as u32,
            label: k.label.to_string(),
            x: k.x,
            y: k.y,
            w: k.w,
            h: k.h,
        })
        .collect()
}
