//! MSI Mystic Light overlapped feature-report probe.
//!
//! Goal: prove (or kill) the real fix for the "fans stutter then jump" bug on the
//! JARGB_V2 headers during a host-streamed hue cycle.
//!
//! Background: the production path streams 727-byte HID *feature* reports through
//! hidapi's `send_feature_report` → `HidD_SetFeature`, which is fully synchronous
//! and gives us no handle on the underlying IRP. When the MCU sporadically NAKs a
//! SET_FEATURE control transfer, Windows blocks that call for the full ~5s HID
//! class-driver timeout (= the visible freeze), then it completes. `CancelSynchronousIo`
//! proved useless: it unblocks a thread waiting in a *synchronous* syscall but
//! cannot cancel an IRP already handed to the HID driver.
//!
//! The only thing that aborts an in-flight HID control transfer is `CancelIoEx`
//! on an **overlapped** handle. hidapi opens the device synchronously, so we have
//! to bypass it: raw `CreateFileW(FILE_FLAG_OVERLAPPED)` + async
//! `DeviceIoControl(IOCTL_HID_SET_FEATURE)`, wait with a short deadline via
//! `GetOverlappedResultEx`, and on timeout `CancelIoEx` + retry.
//!
//! This probe streams a hue cycle to all three JARGB_V2 headers at ~30fps using
//! that transport, cancelling any write that blocks past `--deadline` ms, and
//! prints stall stats every 5s. WATCH THE FANS: if the cycle is smooth (worst
//! case a sub-100ms hiccup instead of a 5s freeze), the transport is the fix.
//!
//! Usage: cargo run --bin msi_ovl -- [seconds=60] [leds=16] [deadline_ms=80] [fps=30]

use std::time::{Duration, Instant};

const VID: u16 = 0x0DB0;
const PID: u16 = 0x0076;
const USAGE_PAGE: u16 = 0xFF00;

const HDR0_JARGB: u8 = 0x04;
const PACKET_LEN: usize = 7 + 240 * 3; // 727: report id + 7-byte header + 720 colors
const HEADERS: [u8; 3] = [0, 1, 2];

/// Device's HID FeatureReportByteLength (from caps). Every feature report must be
/// sent at exactly this length — the HID driver maps an MDL of this size over the
/// buffer, so a shorter buffer faults (ERROR_NOACCESS 998). hidapi pads to this
/// internally; the raw IOCTL path must do it explicitly. Reports shorter than
/// this (SETUP=290, frame=727) are zero-padded; the device routes by report id.
const FEATURE_LEN: usize = 761;

/// Direct-mode setup report (id 0x50) — verbatim from controller.rs. Arms the
/// JARGB headers; resent on a failed frame in case the board drops direct mode.
#[rustfmt::skip]
const SETUP: [u8; 290] = [
    0x50,
    0x09,0xFF,0x00,0x00,0x00,0xFF,0x00,0x00,0x00,0xFF,0xFF,0xFF,0xFF,0x03,0x15,0x78,
    0x09,0xFF,0x00,0x00,0x00,0xFF,0x00,0x00,0x00,0xFF,0xFF,0xFF,0xFF,0x03,0x15,0x78,
    0x09,0xFF,0x00,0x00,0x00,0xFF,0x00,0x00,0x00,0xFF,0xFF,0xFF,0xFF,0x03,0x15,0x78,
    0x09,0xFF,0x00,0x00,0x00,0xFF,0x00,0x00,0x00,0xFF,0xFF,0xFF,0xFF,0x03,0x15,0x78,
    0x09,0xFF,0x00,0x00,0x00,0xFF,0x00,0x00,0x00,0xFF,0xFF,0xFF,0xFF,0x03,0x95,0x1E,
    0x09,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x94,0x1E,
    0x09,0xFF,0x00,0x00,0x00,0xFF,0x00,0x00,0x00,0xFF,0xFF,0xFF,0xFF,0x03,0x95,0x1E,
    0x09,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x94,0x1E,
    0x09,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x94,0x1E,
    0x09,0xFF,0x00,0x00,0x00,0xFF,0x00,0x00,0x00,0xFF,0xFF,0xFF,0xFF,0x03,0x95,0x1E,
    0x09,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x94,0x1E,
    0x09,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x94,0x1E,
    0x09,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x94,0x1E,
    0x09,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x94,0x1E,
    0x09,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x94,0x1E,
    0x09,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x94,0x1E,
    0x09,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x94,0x1E,
    0x09,0xFF,0x00,0x00,0x00,0xFF,0x00,0x00,0x00,0xFF,0xFF,0xFF,0xFF,0x03,0x95,0x1E,
    0x00,
];

/// HSV hue (0..360) at full sat/val -> (r,g,b). Cheap, no float libs.
fn hue_rgb(h: f32) -> (u8, u8, u8) {
    let h = h.rem_euclid(360.0) / 60.0;
    let x = (1.0 - (h % 2.0 - 1.0).abs()) * 255.0;
    let x = x as u8;
    match h as u32 {
        0 => (255, x, 0),
        1 => (x, 255, 0),
        2 => (0, 255, x),
        3 => (0, x, 255),
        4 => (x, 0, 255),
        _ => (255, 0, x),
    }
}

/// Build a 727-byte per-header feature report: solid `(r,g,b)` across `leds` LEDs.
fn header_packet(hdr1: u8, leds: usize, r: u8, g: u8, b: u8) -> [u8; PACKET_LEN] {
    let n = leds.min(240);
    let mut buf = [0u8; PACKET_LEN];
    buf[0] = 0x51;
    buf[1] = 0x09;
    buf[2] = HDR0_JARGB;
    buf[3] = hdr1;
    buf[6] = n as u8;
    for i in 0..n {
        let o = 7 + i * 3;
        buf[o] = r;
        buf[o + 1] = g;
        buf[o + 2] = b;
    }
    buf
}

#[cfg(windows)]
fn run(path_wide: &[u16], secs: u64, leds: usize, deadline_ms: u32, fps: u32) {
    use windows_sys::Win32::Foundation::{
        CloseHandle, GetLastError, ERROR_IO_PENDING, INVALID_HANDLE_VALUE, WAIT_TIMEOUT,
    };
    use windows_sys::Win32::Storage::FileSystem::{CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE};
    use windows_sys::Win32::System::IO::{
        CancelIoEx, DeviceIoControl, GetOverlappedResult, GetOverlappedResultEx, OVERLAPPED,
    };

    const GENERIC_WRITE: u32 = 0x4000_0000;
    const GENERIC_READ: u32 = 0x8000_0000;
    const OPEN_EXISTING: u32 = 3;
    const FILE_FLAG_OVERLAPPED: u32 = 0x4000_0000;
    // IOCTL_HID_SET_FEATURE = CTL_CODE(FILE_DEVICE_KEYBOARD(0xb), 100, METHOD_OUT_DIRECT(2), ANY)
    const IOCTL_HID_SET_FEATURE: u32 = 0x000B_0192;

    unsafe {
        let h = CreateFileW(
            path_wide.as_ptr(),
            GENERIC_WRITE | GENERIC_READ,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_OVERLAPPED,
            std::ptr::null_mut(),
        );
        if h == INVALID_HANDLE_VALUE {
            println!("CreateFileW failed: err {}", GetLastError());
            return;
        }
        println!("opened raw overlapped handle");

        // Dump the device's report lengths so we can confirm 727/290 are right.
        {
            use windows_sys::Win32::Devices::HumanInterfaceDevice::{
                HidD_FreePreparsedData, HidD_GetPreparsedData, HidP_GetCaps, HIDP_CAPS,
            };
            let mut pp: isize = 0;
            if HidD_GetPreparsedData(h, &mut pp) != 0 {
                let mut caps: HIDP_CAPS = std::mem::zeroed();
                if HidP_GetCaps(pp, &mut caps) == 0x0011_0000 {
                    println!(
                        "caps: input={} output={} feature={} (we send setup=290, frame={})",
                        caps.InputReportByteLength,
                        caps.OutputReportByteLength,
                        caps.FeatureReportByteLength,
                        PACKET_LEN
                    );
                }
                HidD_FreePreparsedData(pp);
            }
        }

        // One overlapped write of `buf`, waiting up to `deadline_ms`. On timeout,
        // CancelIoEx the IRP and drain it. Returns (ok, blocked_ms, cancelled).
        let set_feature = |buf: &[u8]| -> (bool, u128, bool) {
            let t0 = Instant::now();
            // Pad to the device's FeatureReportByteLength (the driver maps an MDL
            // of exactly this size; a shorter buffer faults with 998).
            let mut padded = [0u8; FEATURE_LEN];
            let n = buf.len().min(FEATURE_LEN);
            padded[..n].copy_from_slice(&buf[..n]);
            let buf: &[u8] = &padded;
            let mut ovl: OVERLAPPED = std::mem::zeroed();
            let mut bytes: u32 = 0;
            // METHOD_OUT_DIRECT SET_FEATURE: the report buffer is the *output*
            // buffer (the driver maps it via MDL and reads it to send to the
            // device). This mirrors hid.dll's HidD_SetFeature, which calls
            // DeviceIoControl(h, IOCTL_HID_SET_FEATURE, NULL, 0, buf, len, ...).
            let ok = DeviceIoControl(
                h,
                IOCTL_HID_SET_FEATURE,
                std::ptr::null(),
                0,
                buf.as_ptr() as *mut _,
                buf.len() as u32,
                &mut bytes,
                &mut ovl,
            );
            if ok != 0 {
                return (true, t0.elapsed().as_millis(), false); // completed synchronously
            }
            let err = GetLastError();
            if err != ERROR_IO_PENDING {
                eprintln!("DeviceIoControl(SET_FEATURE) failed immediately: err {err}");
                return (false, t0.elapsed().as_millis(), false);
            }
            // Pending: wait with deadline.
            let wr = GetOverlappedResultEx(h, &ovl, &mut bytes, deadline_ms, 0);
            if wr != 0 {
                return (true, t0.elapsed().as_millis(), false);
            }
            if GetLastError() == WAIT_TIMEOUT {
                // The stuck transfer — abort the IRP and drain it.
                CancelIoEx(h, &ovl);
                let mut drained: u32 = 0;
                GetOverlappedResult(h, &ovl, &mut drained, 1); // wait for the cancel to settle
                return (false, t0.elapsed().as_millis(), true);
            }
            (false, t0.elapsed().as_millis(), false)
        };

        // Arm direct mode once (blocking is fine — one-shot).
        let (ok, _, _) = set_feature(&SETUP);
        println!("setup arm: {}", if ok { "ok" } else { "FAILED" });

        let frame_dt = Duration::from_secs_f64(1.0 / fps as f64);
        let end = Instant::now() + Duration::from_secs(secs);
        let mut window = Instant::now();
        let (mut frames, mut writes, mut stalls, mut cancels, mut rearms) = (0u64, 0u64, 0u64, 0u64, 0u64);
        let mut worst_block: u128 = 0;
        let mut hue: f32 = 0.0;

        while Instant::now() < end {
            let frame_start = Instant::now();
            let (r, g, b) = hue_rgb(hue);
            hue = (hue + 2.0) % 360.0; // ~6s per full cycle at 30fps

            for &hdr in &HEADERS {
                let pkt = header_packet(hdr, leds, r, g, b);
                let (mut ok, blocked, cancelled) = set_feature(&pkt);
                writes += 1;
                worst_block = worst_block.max(blocked);
                if cancelled {
                    cancels += 1;
                }
                if blocked >= deadline_ms as u128 {
                    stalls += 1;
                }
                // Recovery: re-arm direct mode and retry this header once.
                if !ok {
                    let _ = set_feature(&SETUP);
                    rearms += 1;
                    let (ok2, _, _) = set_feature(&pkt);
                    ok = ok2;
                    let _ = ok;
                }
            }
            frames += 1;

            if window.elapsed() >= Duration::from_secs(5) {
                let s = window.elapsed().as_secs_f32();
                println!(
                    "[{:.1}s] {} frames ({:.1}/s), {} writes, {} stalls(>{}ms), {} cancels, {} re-arms, worst block {}ms",
                    s, frames, frames as f32 / s, writes, stalls, deadline_ms, cancels, rearms, worst_block
                );
                frames = 0; writes = 0; stalls = 0; cancels = 0; rearms = 0; worst_block = 0;
                window = Instant::now();
            }

            // Pace to target fps.
            let spent = frame_start.elapsed();
            if spent < frame_dt {
                std::thread::sleep(frame_dt - spent);
            }
        }

        CloseHandle(h);
        println!("done.");
    }
}

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let secs: u64 = a.first().and_then(|s| s.parse().ok()).unwrap_or(60);
    let leds: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(16);
    let deadline_ms: u32 = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(80);
    let fps: u32 = a.get(3).and_then(|s| s.parse().ok()).unwrap_or(30).max(1);

    let api = hidapi::HidApi::new().expect("hidapi init");
    let msi = |d: &&hidapi::DeviceInfo| d.vendor_id() == VID && d.product_id() == PID;
    let info = api
        .device_list()
        .find(|d| msi(d) && d.usage_page() == USAGE_PAGE)
        .or_else(|| api.device_list().find(msi))
        .expect("MSI Mystic Light (VID 0x0DB0/PID 0x0076) not found");
    let path = info.path().to_string_lossy().to_string();
    println!("device path: {path}");
    println!("== msi overlapped probe: {secs}s, {leds} leds, {deadline_ms}ms deadline, {fps}fps ==");
    println!("WATCH THE FANS — report whether the hue cycle is smooth.");

    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        let wide: Vec<u16> = std::ffi::OsStr::new(&path)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        run(&wide, secs, leds, deadline_ms, fps);
    }
    #[cfg(not(windows))]
    {
        let _ = (secs, leds, deadline_ms, fps);
        println!("windows-only");
    }
}
