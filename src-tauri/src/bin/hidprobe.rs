//! GMMK v1 protocol probe v4 — built from a VERIFIED USB capture (capA) of the
//! official app, not guesses. capA lit the board solid red via a 0x06 "mode
//! descriptor"; its 0x11 color buffer was stale/empty. We now know the wire
//! format exactly, so this probe drives the keyboard from our own process over
//! the SAME transport the app uses: interrupt-OUT on the vendor collection
//! (usage page 0xFF1C), i.e. hidapi `write()` + draining the per-packet acks.
//!
//! Packet layout (all packets):
//!   [0]=0x04 report id
//!   [1..3]=checksum  (LE16 = sum(bytes[3..]) & 0xFFFF)
//!   [3]=cmd  [4]=len  [5..7]=offset(LE16)  [7]=0x00 pad  [8..8+len]=data
//! Framing: begin = 04 01 00 01, end = 04 02 00 02.
//! Per-key: cmd 0x11, 378-byte / 126-slot RGB buffer sent as 7 chunks of 54.
//!
//! Modes (cargo run --bin hidprobe -- <mode>):
//!   replay [file]      replay capture verbatim (default captures/capA.txt).
//!                      EXPECT: board reproduces capA's state (solid red).
//!                      Proves our process controls the board with no app.
//!   fill RR GG BB [f]  replay capture but swap the 0x11 buffer for a solid
//!                      color (hex). EXPECT: if the board takes that color,
//!                      capA's mode DISPLAYS the per-key buffer -> per-key works.
//!                      If it stays red, the descriptor is a preset that
//!                      ignores the buffer and we must change the mode byte.

use hidapi::{HidApi, HidDevice};
use std::path::Path;
use std::time::Instant;

const VID: u16 = 0x0C45;
const PID: u16 = 0x652F;
const VENDOR_USAGE_PAGE: u16 = 0xFF1C;

/// Build a 64-byte packet with the verified header + checksum.
fn pkt(cmd: u8, offset: u16, data: &[u8]) -> [u8; 64] {
    let mut b = [0u8; 64];
    b[0] = 0x04;
    b[3] = cmd;
    b[4] = data.len() as u8;
    b[5] = (offset & 0xff) as u8;
    b[6] = (offset >> 8) as u8;
    b[7] = 0x00;
    b[8..8 + data.len()].copy_from_slice(data);
    let sum: u32 = b[3..].iter().map(|&x| x as u32).sum();
    let ck = (sum & 0xffff) as u16;
    b[1] = (ck & 0xff) as u8;
    b[2] = (ck >> 8) as u8;
    b
}

/// Write one 64-byte report (interrupt OUT), then NON-BLOCKING drain any acks the
/// firmware has already queued. read_timeout(0) returns immediately (Ok(0) when
/// nothing is waiting), so we consume the ack flow without ever blocking — this
/// is the throughput key. A BLOCKING read per packet (even 50ms) sat the full
/// timeout each time (~50ms/packet => ~5s per full-board frame).
fn send(dev: &HidDevice, buf: &[u8; 64]) {
    if let Err(e) = dev.write(buf) {
        println!("  WRITE ERR: {e}  {:02x?}", &buf[..8]);
        return;
    }
    let mut ack = [0u8; 64];
    while let Ok(n) = dev.read_timeout(&mut ack, 0) {
        if n == 0 {
            break;
        }
    }
}

/// Load capture: one hex string per line -> 64-byte packets.
fn load_capture(path: &str) -> Vec<[u8; 64]> {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {path}: {e}"));
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let bytes: Vec<u8> = (0..line.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&line[i..i + 2], 16).expect("hex"))
            .collect();
        let mut b = [0u8; 64];
        let n = bytes.len().min(64);
        b[..n].copy_from_slice(&bytes[..n]);
        out.push(b);
    }
    out
}

/// 378-byte / 126-slot RGB buffer split into 7 chunks of 54 bytes.
fn color_chunks(r: u8, g: u8, b: u8) -> Vec<[u8; 64]> {
    let mut buf = [0u8; 378];
    for slot in 0..126 {
        buf[slot * 3] = r;
        buf[slot * 3 + 1] = g;
        buf[slot * 3 + 2] = b;
    }
    (0..7)
        .map(|i| {
            let off = i * 54;
            pkt(0x11, off as u16, &buf[off..off + 54])
        })
        .collect()
}

fn begin(dev: &HidDevice) {
    send(dev, &pkt(0x01, 0, &[]));
}
fn end(dev: &HidDevice) {
    send(dev, &pkt(0x02, 0, &[]));
}

/// Send the 7-chunk color buffer inside a begin/end burst.
fn send_buffer(dev: &HidDevice, chunks: &[[u8; 64]]) {
    begin(dev);
    for c in chunks {
        send(dev, c);
    }
    end(dev);
}

/// Per-key keycodes (ANSI), from dokutan/rgb_keyboard keycodes_ansi, keyed by
/// name so we know the physical identity of every key. [c0,c1,c2] -> packet
/// bytes [1],[5],[6]. c0 == (c1+c2+0x54)&0xff. Num_* are numpad (NOT on a TKL).
#[rustfmt::skip]
const NAMED: [(&str, [u8; 3]); 104] = [
    ("Esc",[0x57,0x03,0x00]),("F1",[0x5a,0x06,0x00]),("F2",[0x5d,0x09,0x00]),("F3",[0x60,0x0c,0x00]),
    ("F4",[0x63,0x0f,0x00]),("F5",[0x66,0x12,0x00]),("F6",[0x69,0x15,0x00]),("F7",[0x6c,0x18,0x00]),
    ("F8",[0x6f,0x1b,0x00]),("F9",[0x72,0x1e,0x00]),("F10",[0x75,0x21,0x00]),("F11",[0x78,0x24,0x00]),
    ("F12",[0x7b,0x27,0x00]),("PrtSc",[0x93,0x3e,0x01]),("ScrLk",[0x96,0x41,0x01]),("Pause",[0x99,0x44,0x01]),
    ("Tilde",[0x8a,0x36,0x00]),("1",[0x8d,0x39,0x00]),("2",[0x90,0x3c,0x00]),("3",[0x93,0x3f,0x00]),
    ("4",[0x96,0x42,0x00]),("5",[0x99,0x45,0x00]),("6",[0x9c,0x48,0x00]),("7",[0x9f,0x4b,0x00]),
    ("8",[0xa2,0x4e,0x00]),("9",[0xa5,0x51,0x00]),("0",[0xa8,0x54,0x00]),("Minus",[0xab,0x57,0x00]),
    ("Equals",[0xae,0x5a,0x00]),("Backspace",[0x7b,0x26,0x01]),("Insert",[0x9f,0x4a,0x01]),("Home",[0xa2,0x4d,0x01]),
    ("PgUp",[0xa5,0x50,0x01]),("Delete",[0xa8,0x53,0x01]),("End",[0xab,0x56,0x01]),("PgDn",[0xae,0x59,0x01]),
    ("Tab",[0xbd,0x69,0x00]),("q",[0xc0,0x6c,0x00]),("w",[0xc3,0x6f,0x00]),("e",[0xc6,0x72,0x00]),
    ("r",[0xc9,0x75,0x00]),("t",[0xcc,0x78,0x00]),("y",[0xcf,0x7b,0x00]),("u",[0xd2,0x7e,0x00]),
    ("i",[0xd5,0x81,0x00]),("o",[0xd8,0x84,0x00]),("p",[0xdb,0x87,0x00]),("Bracket_l",[0xde,0x8a,0x00]),
    ("Bracket_r",[0xe1,0x8d,0x00]),("Backslash",[0x14,0xc0,0x00]),("Up",[0x75,0x20,0x01]),("Left",[0x6f,0x1a,0x01]),
    ("Down",[0x72,0x1d,0x01]),("Right",[0x78,0x23,0x01]),("Caps_Lock",[0xf0,0x9c,0x00]),("a",[0xf3,0x9f,0x00]),
    ("s",[0xf6,0xa2,0x00]),("d",[0xf9,0xa5,0x00]),("f",[0xfc,0xa8,0x00]),("g",[0xff,0xab,0x00]),
    ("h",[0x02,0xae,0x00]),("j",[0x05,0xb1,0x00]),("k",[0x08,0xb4,0x00]),("l",[0x0b,0xb7,0x00]),
    ("Semicolon",[0x0e,0xba,0x00]),("Apostrophe",[0x11,0xbd,0x00]),("Return",[0x47,0xf3,0x00]),("Shift_l",[0x23,0xcf,0x00]),
    ("z",[0x26,0xd2,0x00]),("x",[0x29,0xd5,0x00]),("c",[0x2c,0xd8,0x00]),("v",[0x2f,0xdb,0x00]),
    ("b",[0x32,0xde,0x00]),("n",[0x35,0xe1,0x00]),("m",[0x38,0xe4,0x00]),("Comma",[0x3b,0xe7,0x00]),
    ("Period",[0x3e,0xea,0x00]),("Slash",[0x41,0xed,0x00]),("Shift_r",[0x44,0xf0,0x00]),("Ctrl_l",[0x57,0x02,0x01]),
    ("Super_l",[0x5a,0x05,0x01]),("Alt_l",[0x5d,0x08,0x01]),("Space",[0x60,0x0b,0x01]),("Alt_r",[0x63,0x0e,0x01]),
    ("Fn",[0x66,0x11,0x01]),("Menu",[0x69,0x14,0x01]),("Ctrl_r",[0x6c,0x17,0x01]),("Num_Lock",[0xb1,0x5d,0x00]),
    ("Num_Slash",[0xb4,0x60,0x00]),("Num_Asterisk",[0xb7,0x63,0x00]),("Num_Minus",[0xb1,0x5c,0x01]),("Num_7",[0xe4,0x90,0x00]),
    ("Num_8",[0xe7,0x93,0x00]),("Num_9",[0xea,0x96,0x00]),("Num_Plus",[0xb4,0x5f,0x01]),("Num_4",[0x17,0xc3,0x00]),
    ("Num_5",[0x1a,0xc6,0x00]),("Num_6",[0x1d,0xc9,0x00]),("Num_1",[0x4a,0xf6,0x00]),("Num_2",[0x4d,0xf9,0x00]),
    ("Num_3",[0x50,0xfc,0x00]),("Num_0",[0x7e,0x29,0x01]),("Num_Period",[0x81,0x2c,0x01]),("Num_Return",[0x84,0x2f,0x01]),
];

/// Look up a key's 3-byte code by name.
fn code_of(name: &str) -> [u8; 3] {
    NAMED
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, c)| *c)
        .unwrap_or([0, 0, 0])
}

/// One per-key packet (dokutan custom format): 04 c0 02 11 03 c1 c2 00 R G B.
fn key_pkt(code: [u8; 3], r: u8, g: u8, b: u8) -> [u8; 64] {
    let mut p = [0u8; 64];
    p[0] = 0x04;
    p[1] = code[0];
    p[2] = 0x02;
    p[3] = 0x11;
    p[4] = 0x03;
    p[5] = code[1];
    p[6] = code[2];
    p[7] = 0x00;
    p[8] = r;
    p[9] = g;
    p[10] = b;
    p
}

/// dokutan custom flow, in dokutan's EXACT order (rgb_keyboard.cpp):
///   1. write_custom  — paint each key with an individual per-key packet
///   2. write_mode     — set mode = custom (0x14)
///   3. write_brightness
///      Each step is its own begin/.../end burst. Order matters: paint BEFORE
///      switching to custom mode (our earlier mode-first order left the board dark).
fn direct(dev: &HidDevice, r: u8, g: u8, b: u8) {
    paint_solid(dev, r, g, b);
    apply_custom_mode(dev);
}

/// Paint every key one solid color via individual packets (one begin/end burst).
/// Does NOT set mode — caller switches to custom mode (once) separately. This is
/// the per-frame unit of work for the individual-packet animation path.
fn paint_solid(dev: &HidDevice, r: u8, g: u8, b: u8) {
    begin(dev);
    for (_, code) in NAMED.iter() {
        send(dev, &key_pkt(*code, r, g, b));
    }
    end(dev);
}

/// Paint a per-key set (name -> RGB); every other key is set to off (000000).
/// Then switch to custom mode so the painted buffer is displayed.
fn paint_named(dev: &HidDevice, keys: &[(&str, [u8; 3])]) {
    begin(dev);
    for (name, _) in NAMED.iter() {
        let rgb = keys
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, c)| *c)
            .unwrap_or([0, 0, 0]);
        send(dev, &key_pkt(code_of(name), rgb[0], rgb[1], rgb[2]));
    }
    end(dev);
    apply_custom_mode(dev);
}

/// write_mode: custom ([1]=0x1b, [8]=0x14 == pkt(0x06,0,&[0x14])). Brightness
/// write intentionally OMITTED — brightness 9 (max) blacked the board out.
fn apply_custom_mode(dev: &HidDevice) {
    begin(dev);
    send(dev, &pkt(0x06, 0x00, &[0x14]));
    end(dev);
}

/// Onboard firmware effect modes (dokutan write_mode table, profile 1).
/// Settings packets = pkt(0x06, selector, &[value]); mode is selector 0.
/// Only `custom` (0x14) is verified on this unit so far — this command exists
/// to verify the rest. The curated UI set is marked with a leading '*'.
#[rustfmt::skip]
const MODES: [(&str, u8); 20] = [
    ("fixed", 0x06),            // * single static color
    ("breathing", 0x05),        // * single-color breathe
    ("breathing_color", 0x04),  //   multi-color breathe
    ("horizontal_wave", 0x01),  // * wave (use rainbow=1 for spectrum)
    ("vertical_wave", 0x0c),
    ("diagonal_wave", 0x10),
    ("swirl", 0x0b),            // * swirl
    ("vortex", 0x0e),
    ("rain", 0x0f),
    ("ripple", 0x12),
    ("waterfall", 0x0a),
    ("sine", 0x0d),
    ("pulse", 0x02),
    ("hurricane", 0x03),
    ("reactive_single", 0x07),  // * react on keypress
    ("reactive_ripple", 0x08),
    ("reactive_horizontal", 0x09),
    ("reactive_color", 0x11),
    ("custom", 0x14),           //   per-key (verified)
    ("off", 0x13),              //   all LEDs off (recovery)
];

fn mode_byte(name: &str) -> Option<u8> {
    MODES.iter().find(|(n, _)| *n == name).map(|(_, v)| *v)
}

/// Apply one onboard firmware effect in a single begin/end burst.
/// brightness 0..=9 (START mid, NOT 9 — 9 has blacked the board before),
/// speed 0..=4, rainbow 0/1, dir 0/1.
#[allow(clippy::too_many_arguments)]
fn onboard(dev: &HidDevice, mode: u8, r: u8, g: u8, b: u8, bri: u8, spd: u8, rainbow: u8, dir: u8) {
    let bri = bri.min(9);
    let spd = spd.min(4);
    let dir = if dir != 0 { 0xff } else { 0x00 };
    begin(dev);
    send(dev, &pkt(0x06, 0x00, &[mode]));
    send(dev, &pkt(0x06, 0x01, &[bri]));
    send(dev, &pkt(0x06, 0x02, &[0x04 - spd]));
    send(dev, &pkt(0x06, 0x03, &[dir]));
    send(dev, &pkt(0x06, 0x04, &[rainbow.min(1)]));
    send(dev, &pkt(0x06, 0x05, &[r, g, b]));
    end(dev);
}

fn open_vendor(api: &HidApi) -> HidDevice {
    let info = api
        .device_list()
        .find(|d| {
            d.vendor_id() == VID && d.product_id() == PID && d.usage_page() == VENDOR_USAGE_PAGE
        })
        .expect("GMMK vendor collection (usage page 0xFF1C) not found");
    println!("opening {}", info.path().to_string_lossy());
    api.open_path(info.path()).expect("open")
}

fn main() {
    let api = HidApi::new().expect("hidapi init");
    for d in api
        .device_list()
        .filter(|d| d.vendor_id() == VID && d.product_id() == PID)
    {
        println!(
            "found: iface {} usage_page {:#06x} usage {:#04x}",
            d.interface_number(),
            d.usage_page(),
            d.usage()
        );
    }

    let args: Vec<String> = std::env::args().skip(1).collect();
    let mode = args.first().map(String::as_str).unwrap_or("replay");
    let dev = open_vendor(&api);

    match mode {
        "replay" => {
            let file = args
                .get(1)
                .map(String::as_str)
                .unwrap_or("captures/capA.txt");
            let file = if Path::new(file).exists() {
                file.to_string()
            } else {
                format!("../{file}") // when run from src-tauri/
            };
            let pkts = load_capture(&file);
            println!("== replay {} ({} packets) verbatim ==", file, pkts.len());
            println!("EXPECT: board reproduces capA (solid red). Proves transport.");
            for p in &pkts {
                send(&dev, p);
            }
            println!("done — did the board change to capA's state (red)?");
        }
        "fill" => {
            let r = u8::from_str_radix(args.get(1).map(String::as_str).unwrap_or("00"), 16)
                .unwrap_or(0);
            let g = u8::from_str_radix(args.get(2).map(String::as_str).unwrap_or("ff"), 16)
                .unwrap_or(0);
            let b = u8::from_str_radix(args.get(3).map(String::as_str).unwrap_or("00"), 16)
                .unwrap_or(0);
            let file = args
                .get(4)
                .map(String::as_str)
                .unwrap_or("captures/capA.txt");
            let file = if Path::new(file).exists() {
                file.to_string()
            } else {
                format!("../{file}")
            };
            let pkts = load_capture(&file);
            let chunks = color_chunks(r, g, b);
            println!(
                "== fill: replay {} but 0x11 buffer = {:02x}{:02x}{:02x} ==",
                file, r, g, b
            );
            println!("EXPECT: board takes that color => capA's mode shows the buffer (per-key!).");
            println!("        stays red => preset ignores buffer; must change mode byte.");
            let mut ci = 0;
            for p in &pkts {
                if p[3] == 0x11 {
                    // Replace with our color chunk at the same offset.
                    if ci < chunks.len() {
                        send(&dev, &chunks[ci]);
                        ci += 1;
                    }
                } else {
                    send(&dev, p);
                }
            }
            println!("done — what color is the board now?");
        }
        "direct" => {
            // direct [RR GG BB]   default green. Per-key (individual packets).
            let r = u8::from_str_radix(args.get(1).map(String::as_str).unwrap_or("00"), 16)
                .unwrap_or(0);
            let g = u8::from_str_radix(args.get(2).map(String::as_str).unwrap_or("ff"), 16)
                .unwrap_or(0);
            let b = u8::from_str_radix(args.get(3).map(String::as_str).unwrap_or("00"), 16)
                .unwrap_or(0);
            println!("== direct (per-key packets): paint all keys {r:02x}{g:02x}{b:02x} ==");
            println!("EXPECT: whole board lights that color => custom per-key works!");
            direct(&dev, r, g, b);
            println!("done — what color/effect is the board now?");
        }
        "cal" => {
            // Light 5 unambiguous landmark keys in distinct colors, rest off.
            // Confirms per-key ADDRESSING (not just "all keys take one color").
            let landmarks: [(&str, [u8; 3]); 5] = [
                ("Esc", [0xff, 0x00, 0x00]),       // top-left corner: RED
                ("Backspace", [0x00, 0xff, 0x00]), // top-right alpha: GREEN
                ("Return", [0x00, 0x00, 0xff]),    // mid-right: BLUE
                ("Space", [0xff, 0xff, 0xff]),     // bottom center: WHITE
                ("Up", [0xff, 0xff, 0x00]),        // arrow cluster: YELLOW
            ];
            println!("== cal: landmark addressing test ==");
            println!("EXPECT (TKL): Esc=RED, Backspace=GREEN, Enter=BLUE, Space=WHITE, Up-arrow=YELLOW; all else OFF.");
            paint_named(&dev, &landmarks);
            println!("done — report each lit key's PHYSICAL position + color.");
        }
        "bench" => {
            // Stream N full-board frames (individual-packet path) and time them.
            // Measures the real fps ceiling for whole-board per-key updates.
            let frames: u32 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(60);
            println!("== bench: {frames} full-board frames (individual per-key path) ==");
            paint_solid(&dev, 0xff, 0x00, 0x00);
            apply_custom_mode(&dev); // enter custom mode once
            let colors = [
                (0xff, 0x00, 0x00),
                (0x00, 0xff, 0x00),
                (0x00, 0x00, 0xff),
                (0xff, 0xff, 0x00),
                (0x00, 0xff, 0xff),
                (0xff, 0x00, 0xff),
            ];
            let t0 = Instant::now();
            for f in 0..frames {
                let (r, g, b) = colors[f as usize % colors.len()];
                paint_solid(&dev, r, g, b);
            }
            let dt = t0.elapsed().as_secs_f64();
            println!(
                "{frames} frames in {:.1} ms => {:.2} ms/frame, {:.1} fps (full board, {} keys)",
                dt * 1000.0,
                dt * 1000.0 / frames as f64,
                frames as f64 / dt,
                NAMED.len()
            );
        }
        "benchwrite" => {
            // Pure write latency: N raw writes, NO ack reads. Isolates whether the
            // ~24ms/packet cost is hidapi WriteFile or our read-drain.
            let n: u32 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(200);
            let chunk = color_chunks(0xff, 0x00, 0x00)[0];
            let t0 = Instant::now();
            for _ in 0..n {
                let _ = dev.write(&chunk);
            }
            let dt = t0.elapsed().as_secs_f64();
            println!(
                "benchwrite: {n} raw writes (no read) in {:.1} ms => {:.2} ms/write",
                dt * 1000.0,
                dt * 1000.0 / n as f64
            );
        }
        "benchbulk" => {
            // Stream N full-board frames via the 7-packet bulk buffer and time them.
            // This is the high-fps full-board path.
            let frames: u32 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(120);
            println!("== benchbulk: {frames} full-board frames (7-packet bulk buffer) ==");
            send_buffer(&dev, &color_chunks(0xff, 0x00, 0x00));
            apply_custom_mode(&dev); // enter custom mode once
            let colors = [
                (0xff, 0x00, 0x00),
                (0x00, 0xff, 0x00),
                (0x00, 0x00, 0xff),
                (0xff, 0xff, 0x00),
                (0x00, 0xff, 0xff),
                (0xff, 0x00, 0xff),
            ];
            let t0 = Instant::now();
            for f in 0..frames {
                let (r, g, b) = colors[f as usize % colors.len()];
                send_buffer(&dev, &color_chunks(r, g, b));
            }
            let dt = t0.elapsed().as_secs_f64();
            println!(
                "{frames} frames in {:.1} ms => {:.2} ms/frame, {:.1} fps (bulk, whole board)",
                dt * 1000.0,
                dt * 1000.0 / frames as f64,
                frames as f64 / dt
            );
        }
        "bulk" => {
            // Re-test the 7-packet whole-board 0x11 buffer in CUSTOM mode now that
            // order/brightness are fixed. If the board takes this color, the bulk
            // path drives custom display => the high-fps highway (~7 pkts/frame).
            let r = u8::from_str_radix(args.get(1).map(String::as_str).unwrap_or("00"), 16)
                .unwrap_or(0);
            let g = u8::from_str_radix(args.get(2).map(String::as_str).unwrap_or("ff"), 16)
                .unwrap_or(0);
            let b = u8::from_str_radix(args.get(3).map(String::as_str).unwrap_or("ff"), 16)
                .unwrap_or(0);
            println!("== bulk: 7-chunk 0x11 buffer = {r:02x}{g:02x}{b:02x}, then custom mode ==");
            println!("EXPECT: board takes that color => bulk buffer works in custom mode (high-fps path).");
            send_buffer(&dev, &color_chunks(r, g, b));
            apply_custom_mode(&dev);
            println!("done — what color is the board? (any keys lit at all?)");
        }
        "mode" => {
            // mode <name> [RRGGBB] [bri 0-9] [spd 0-4] [rainbow 0/1] [dir 0/1]
            // Sends ONE onboard firmware effect burst. The MCU then animates it
            // itself (no host streaming). Verifies the dokutan mode bytes on this
            // unit. `mode list` prints the table; `mode off` / `mode custom` recover.
            let name = args.get(1).map(String::as_str).unwrap_or("list");
            if name == "list" {
                println!("modes (* = curated UI set):");
                for (n, v) in MODES.iter() {
                    println!("  {n:<22} 0x{v:02x}");
                }
                println!("usage: mode <name> [RRGGBB] [bri 0-9] [spd 0-4] [rainbow 0/1] [dir 0/1]");
                return;
            }
            let Some(mb) = mode_byte(name) else {
                println!("unknown mode '{name}'. run `mode list`.");
                return;
            };
            let hex = args.get(2).map(String::as_str).unwrap_or("ffffff");
            let r = u8::from_str_radix(hex.get(0..2).unwrap_or("ff"), 16).unwrap_or(0xff);
            let g = u8::from_str_radix(hex.get(2..4).unwrap_or("ff"), 16).unwrap_or(0xff);
            let b = u8::from_str_radix(hex.get(4..6).unwrap_or("ff"), 16).unwrap_or(0xff);
            let bri: u8 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(4);
            let spd: u8 = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(2);
            let rainbow: u8 = args.get(5).and_then(|s| s.parse().ok()).unwrap_or(0);
            let dir: u8 = args.get(6).and_then(|s| s.parse().ok()).unwrap_or(0);
            println!("== mode {name} (0x{mb:02x}) color={r:02x}{g:02x}{b:02x} bri={bri} spd={spd} rainbow={rainbow} dir={dir} ==");
            println!("EXPECT: keyboard runs the '{name}' firmware animation by itself.");
            onboard(&dev, mb, r, g, b, bri, spd, rainbow, dir);
            println!(
                "done — does the board animate '{name}' correctly? (note brightness behavior)"
            );
        }
        other => {
            println!("unknown mode '{other}'. use: replay | fill RR GG BB | direct RR GG BB | cal | bench [frames] | bulk RR GG BB | mode <name> ...");
        }
    }
}
