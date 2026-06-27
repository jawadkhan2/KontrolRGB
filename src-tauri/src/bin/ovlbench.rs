//! Overlapped (async) HID write throughput experiment.
//!
//! hidapi's synchronous `write()` costs ~22ms/packet on this GMMK v1 (measured),
//! which caps host-streamed RGB at ~6-7fps full-board. That 22ms could be either
//! (a) hidapi's WriteFile waiting for each transfer to fully complete before
//! returning, or (b) the keyboard MCU genuinely taking ~22ms to ingest a report.
//!
//! This probe bypasses hidapi: it opens the vendor HID collection with raw Win32
//! `CreateFileW(..FILE_FLAG_OVERLAPPED..)` and issues writes with a sliding window
//! of `depth` concurrent overlapped I/Os. If the device can pipeline, throughput
//! rises sharply with depth (=> case a, host 30fps is viable). If ms/write stays
//! ~22ms regardless of depth, the MCU is the bottleneck (=> case b, not viable).
//!
//! Usage: cargo run --bin ovlbench -- [N writes] [depth]   (defaults 200 32)
//! depth 1 reproduces the synchronous baseline.

use hidapi::HidApi;
use std::time::Instant;

const VID: u16 = 0x0C45;
const PID: u16 = 0x652F;
const VENDOR_USAGE_PAGE: u16 = 0xFF1C;

/// Build a 64-byte packet with the verified header + checksum (same as hidprobe).
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

#[cfg(windows)]
fn run(path_wide: &[u16], packet: &[u8; 64], n: usize, depth: usize) {
    use windows_sys::Win32::Devices::HumanInterfaceDevice::{
        HidD_FreePreparsedData, HidD_GetPreparsedData, HidP_GetCaps, HIDP_CAPS,
    };
    use windows_sys::Win32::Foundation::{
        CloseHandle, GetLastError, ERROR_IO_PENDING, HANDLE, INVALID_HANDLE_VALUE,
    };
    use windows_sys::Win32::Storage::FileSystem::{CreateFileW, WriteFile};
    use windows_sys::Win32::System::Threading::{CreateEventW, ResetEvent};
    use windows_sys::Win32::System::IO::{GetOverlappedResult, OVERLAPPED};

    const GENERIC_WRITE: u32 = 0x4000_0000;
    const GENERIC_READ: u32 = 0x8000_0000;
    const FILE_SHARE_RW: u32 = 0x0000_0003;
    const OPEN_EXISTING: u32 = 3;
    const FILE_FLAG_OVERLAPPED: u32 = 0x4000_0000;

    unsafe {
        let h = CreateFileW(
            path_wide.as_ptr(),
            GENERIC_WRITE | GENERIC_READ,
            FILE_SHARE_RW,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_OVERLAPPED,
            std::ptr::null_mut(),
        );
        if h == INVALID_HANDLE_VALUE {
            println!("CreateFileW failed: err {}", GetLastError());
            return;
        }

        // Query the output report length so WriteFile gets the size it expects.
        let mut out_len: u32 = 64;
        let mut pp: isize = 0;
        if HidD_GetPreparsedData(h, &mut pp) != 0 {
            let mut caps: HIDP_CAPS = std::mem::zeroed();
            if HidP_GetCaps(pp, &mut caps) == 0x0011_0000 {
                out_len = caps.OutputReportByteLength as u32;
            }
            HidD_FreePreparsedData(pp);
        }
        println!("opened raw handle; OutputReportByteLength = {out_len}");

        // Per-slot buffers (must stay alive for the duration of each I/O).
        let mut bufs: Vec<Vec<u8>> = (0..depth)
            .map(|_| {
                let mut v = vec![0u8; out_len as usize];
                let copy = (out_len as usize).min(64);
                v[..copy].copy_from_slice(&packet[..copy]);
                v
            })
            .collect();
        let mut ovls: Vec<OVERLAPPED> = (0..depth).map(|_| std::mem::zeroed()).collect();
        let events: Vec<HANDLE> = (0..depth)
            .map(|_| CreateEventW(std::ptr::null(), 1, 0, std::ptr::null()))
            .collect();
        let mut inflight = vec![false; depth];

        let t0 = Instant::now();
        for i in 0..n {
            let s = i % depth;
            if inflight[s] {
                let mut got: u32 = 0;
                GetOverlappedResult(h, &ovls[s], &mut got, 1); // wait = TRUE
                inflight[s] = false;
            }
            ovls[s] = std::mem::zeroed();
            ovls[s].hEvent = events[s];
            ResetEvent(events[s]);
            let mut written: u32 = 0;
            let ok = WriteFile(h, bufs[s].as_ptr(), out_len, &mut written, &mut ovls[s]);
            if ok == 0 {
                let err = GetLastError();
                if err == ERROR_IO_PENDING {
                    inflight[s] = true;
                } else {
                    println!("WriteFile err {err} at i={i}");
                    break;
                }
            }
        }
        // Drain any still in flight.
        for s in 0..depth {
            if inflight[s] {
                let mut got: u32 = 0;
                GetOverlappedResult(h, &ovls[s], &mut got, 1);
            }
        }
        let dt = t0.elapsed().as_secs_f64();
        println!(
            "ovl depth={depth}: {n} writes in {:.1} ms => {:.2} ms/write, {:.0} writes/s",
            dt * 1000.0,
            dt * 1000.0 / n as f64,
            n as f64 / dt
        );

        for e in events {
            CloseHandle(e);
        }
        let _ = &mut bufs;
        CloseHandle(h);
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let n: usize = args.first().and_then(|s| s.parse().ok()).unwrap_or(200);
    let depth: usize = args
        .get(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(32)
        .max(1);

    let api = HidApi::new().expect("hidapi init");
    let info = api
        .device_list()
        .find(|d| {
            d.vendor_id() == VID && d.product_id() == PID && d.usage_page() == VENDOR_USAGE_PAGE
        })
        .expect("GMMK vendor collection (usage page 0xFF1C) not found");
    let path = info.path().to_string_lossy().to_string();
    println!("device path: {path}");

    // A valid red bulk-buffer chunk (content irrelevant to throughput, but valid).
    let mut data = [0u8; 54];
    for i in 0..18 {
        data[i * 3] = 0xff;
    }
    let packet = pkt(0x11, 0, &data);

    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        let wide: Vec<u16> = std::ffi::OsStr::new(&path)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        println!("== overlapped write bench: N={n}, depth={depth} ==");
        run(&wide, &packet, n, depth);
    }
    #[cfg(not(windows))]
    {
        let _ = (&packet, n, depth);
        println!("ovlbench is Windows-only");
    }
}
