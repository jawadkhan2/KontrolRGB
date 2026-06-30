//! Gigabyte GPU I2C address sweep — find the real RGB controller address on the
//! actual card before trusting the main backend. Self-contained (own minimal
//! NvAPI FFI, only libloading) so it doesn't drag in the Tauri lib, same as
//! hidprobe. The protocol facts mirror src/device/gigabyte_gpu/.
//!
//! NvAPI reaches the GPU's internal I2C on port 1. The RGB Fusion 2 controller
//! answers a write of [0xAB,..] with a 4-byte reply whose first byte is 0xAB.
//! 50-series Gaming OC is expected at 0x71 (confirmed on the 5090 Gaming OC),
//! but the 5080 is unlisted, so we sweep the whole Gigabyte address set.
//!
//! Modes (cargo run --bin gpuprobe -- <mode>):
//!   scan                 list GPUs + PCI ids, sweep addresses, dump handshakes.
//!                        EXPECT: exactly one address replies 0xAB -> that's it.
//!   set <addr> <RRGGBB>  write a static colour to <addr> (hex) on every zone
//!                        index. EXPECT: card lights that colour -> confirmed.
//!   raw  <addr>          write 0xAB to <addr> and print the raw 4-byte reply.

#![allow(non_snake_case)]

use std::ffi::c_void;

use libloading::Library;

const NVAPI_MAX_PHYSICAL_GPUS: usize = 64;

const ID_INITIALIZE: u32 = 0x0150_E828;
const ID_ENUM_PHYSICAL_GPUS: u32 = 0xE5AC_921F;
const ID_GPU_GET_PCI_IDENTIFIERS: u32 = 0x2DDF_B66E;
const ID_I2C_WRITE_EX: u32 = 0x283A_C65A;
const ID_I2C_READ_EX: u32 = 0x4D7B_0709;
// Older non-Ex entry points (no trailing out-param). Some driver branches gate
// the Ex pair but still answer these.
const ID_I2C_WRITE: u32 = 0xE812_EB07;
const ID_I2C_READ: u32 = 0x2FDE_12C5;

/// Map the common negative NvAPI_Status codes to names for diagnostics.
fn nv_status(code: i32) -> &'static str {
    match code {
        0 => "OK",
        -1 => "ERROR",
        -3 => "NO_IMPLEMENTATION",
        -4 => "API_NOT_INITIALIZED",
        -5 => "INVALID_ARGUMENT",
        -6 => "NVIDIA_DEVICE_NOT_FOUND",
        -8 => "INVALID_HANDLE",
        -9 => "INCOMPATIBLE_STRUCT_VERSION",
        -11 => "INVALID_POINTER",
        -100 => "HANDLE_INVALIDATED",
        -104 => "NOT_SUPPORTED",
        -201 => "I2C_NACK_DURING_ADDR / no device",
        -202 => "I2C_NACK_DURING_DATA",
        -203 => "I2C_SPEED_TOO_HIGH",
        _ => "unknown",
    }
}

/// Candidate addresses seen across Gigabyte GPUs in OpenRGB's detector table,
/// plus the immediate neighbourhood, lowest first.
const CANDIDATES: [u8; 14] = [
    0x08, 0x32, 0x50, 0x51, 0x55, 0x62, 0x63, 0x64, 0x65, 0x66, 0x70, 0x71, 0x72, 0x73,
];

#[repr(C)]
struct NvI2cInfoV3 {
    version: u32,
    display_mask: u32,
    is_ddc_port: u8,
    i2c_dev_address: u8,
    i2c_reg_address: *mut u8,
    reg_addr_size: u32,
    data: *mut u8,
    size: u32,
    i2c_speed: u32,
    i2c_speed_khz: u32,
    port_id: u8,
    is_port_id_set: u32,
}

fn i2c_version() -> u32 {
    (3u32 << 16) | (std::mem::size_of::<NvI2cInfoV3>() as u32)
}

type QueryInterfaceFn = unsafe extern "C" fn(u32) -> *const c_void;
type InitializeFn = unsafe extern "C" fn() -> i32;
type EnumPhysicalGpusFn = unsafe extern "C" fn(*mut usize, *mut i32) -> i32;
type GetPciIdentifiersFn = unsafe extern "C" fn(usize, *mut u32, *mut u32, *mut u32, *mut u32) -> i32;
type I2cExFn = unsafe extern "C" fn(usize, *mut NvI2cInfoV3, *mut u32) -> i32;
type I2cFn = unsafe extern "C" fn(usize, *mut NvI2cInfoV3) -> i32;

struct NvApi {
    _lib: Library,
    enum_physical_gpus: EnumPhysicalGpusFn,
    get_pci_identifiers: GetPciIdentifiersFn,
    i2c_write_ex: I2cExFn,
    i2c_read_ex: I2cExFn,
    i2c_write: Option<I2cFn>,
    #[allow(dead_code)] // resolved for symmetry; diag only exercises writes
    i2c_read: Option<I2cFn>,
}

impl NvApi {
    fn load() -> NvApi {
        let lib = unsafe { Library::new("nvapi64.dll") }
            .expect("nvapi64.dll not found — is the NVIDIA driver installed?");
        unsafe {
            let query: QueryInterfaceFn = {
                let sym: libloading::Symbol<QueryInterfaceFn> =
                    lib.get(b"nvapi_QueryInterface\0").expect("nvapi_QueryInterface");
                *sym
            };
            let resolve = |id: u32| -> *const c_void {
                let p = query(id);
                assert!(!p.is_null(), "QueryInterface(0x{id:08X}) returned null");
                p
            };
            let initialize: InitializeFn = std::mem::transmute(resolve(ID_INITIALIZE));
            let enum_physical_gpus: EnumPhysicalGpusFn =
                std::mem::transmute(resolve(ID_ENUM_PHYSICAL_GPUS));
            let get_pci_identifiers: GetPciIdentifiersFn =
                std::mem::transmute(resolve(ID_GPU_GET_PCI_IDENTIFIERS));
            let i2c_write_ex: I2cExFn = std::mem::transmute(resolve(ID_I2C_WRITE_EX));
            let i2c_read_ex: I2cExFn = std::mem::transmute(resolve(ID_I2C_READ_EX));

            // Non-Ex variants may not be present on every driver — resolve softly.
            let resolve_opt = |id: u32| -> Option<*const c_void> {
                let p = query(id);
                if p.is_null() {
                    None
                } else {
                    Some(p)
                }
            };
            let i2c_write: Option<I2cFn> = resolve_opt(ID_I2C_WRITE).map(|p| std::mem::transmute(p));
            let i2c_read: Option<I2cFn> = resolve_opt(ID_I2C_READ).map(|p| std::mem::transmute(p));

            let status = initialize();
            assert_eq!(status, 0, "NvAPI_Initialize failed ({status})");

            NvApi {
                _lib: lib,
                enum_physical_gpus,
                get_pci_identifiers,
                i2c_write_ex,
                i2c_read_ex,
                i2c_write,
                i2c_read,
            }
        }
    }

    fn gpus(&self) -> Vec<usize> {
        let mut handles = [0usize; NVAPI_MAX_PHYSICAL_GPUS];
        let mut count: i32 = 0;
        let status = unsafe { (self.enum_physical_gpus)(handles.as_mut_ptr(), &mut count) };
        assert_eq!(status, 0, "NvAPI_EnumPhysicalGPUs failed ({status})");
        let count = count.clamp(0, NVAPI_MAX_PHYSICAL_GPUS as i32) as usize;
        handles[..count].to_vec()
    }

    /// (pci_vendor, pci_device, sub_vendor, sub_device)
    fn pci_ids(&self, handle: usize) -> (u16, u16, u16, u16) {
        let (mut dev, mut sub, mut rev, mut ext) = (0u32, 0u32, 0u32, 0u32);
        let status =
            unsafe { (self.get_pci_identifiers)(handle, &mut dev, &mut sub, &mut rev, &mut ext) };
        if status != 0 {
            return (0, 0, 0, 0);
        }
        (
            (dev & 0xFFFF) as u16,
            (dev >> 16) as u16,
            (sub & 0xFFFF) as u16,
            (sub >> 16) as u16,
        )
    }

    fn info(&self, port: u8, addr: u8, buf: &mut [u8]) -> NvI2cInfoV3 {
        NvI2cInfoV3 {
            version: i2c_version(),
            display_mask: 0,
            is_ddc_port: 0,
            i2c_dev_address: addr << 1,
            i2c_reg_address: std::ptr::null_mut(),
            reg_addr_size: 0,
            data: buf.as_mut_ptr(),
            size: buf.len() as u32,
            i2c_speed: 0xFFFF,
            i2c_speed_khz: 0,
            port_id: port,
            is_port_id_set: 1,
        }
    }

    fn write(&self, handle: usize, port: u8, addr: u8, data: &[u8]) -> Result<(), i32> {
        let mut buf = data.to_vec();
        let mut info = self.info(port, addr, &mut buf);
        let mut unknown: u32 = 0;
        let status = unsafe { (self.i2c_write_ex)(handle, &mut info, &mut unknown) };
        if status == 0 {
            Ok(())
        } else {
            Err(status)
        }
    }

    fn read(&self, handle: usize, port: u8, addr: u8, len: usize) -> Result<Vec<u8>, i32> {
        let mut buf = vec![0u8; len];
        let mut info = self.info(port, addr, &mut buf);
        let mut unknown: u32 = 0;
        let status = unsafe { (self.i2c_read_ex)(handle, &mut info, &mut unknown) };
        if status == 0 {
            let n = (info.size as usize).min(buf.len());
            buf.truncate(n);
            Ok(buf)
        } else {
            Err(status)
        }
    }

    /// Raw status of a single Ex write (for diagnostics).
    fn write_status_ex(&self, handle: usize, port: u8, addr: u8, data: &[u8]) -> i32 {
        let mut buf = data.to_vec();
        let mut info = self.info(port, addr, &mut buf);
        let mut unknown: u32 = 0;
        unsafe { (self.i2c_write_ex)(handle, &mut info, &mut unknown) }
    }

    /// Raw status of a single non-Ex write (None if entry point absent).
    fn write_status_plain(&self, handle: usize, port: u8, addr: u8, data: &[u8]) -> Option<i32> {
        let f = self.i2c_write?;
        let mut buf = data.to_vec();
        let mut info = self.info(port, addr, &mut buf);
        Some(unsafe { f(handle, &mut info) })
    }

    /// Write 0xAB, read 4 bytes. Returns the reply (or the failing NvAPI status).
    fn handshake(&self, handle: usize, port: u8, addr: u8) -> Result<Vec<u8>, i32> {
        self.write(handle, port, addr, &[0xAB, 0, 0, 0, 0, 0, 0, 0])?;
        self.read(handle, port, addr, 4)
    }

    /// Write a static colour to every zone index (0..5), mirroring the backend.
    fn set_color(&self, handle: usize, port: u8, addr: u8, r: u8, g: u8, b: u8) -> Result<(), i32> {
        for zone in 0u8..5 {
            // mode packet: static (0x01), full brightness 0x63, normal speed.
            self.write(handle, port, addr, &[0x88, 0x01, 0x02, 0x63, 0x00, zone + 1, 0, 0])?;
            let color_pkt: [u8; 8] = match zone {
                0 | 1 => [0xB0, 0x01, r, g, b, r, g, b],
                2 => [0xB1, 0x01, r, g, b, 0, 0, 0],
                _ => [0x40, r, g, b, zone + 1, 0, 0, 0],
            };
            self.write(handle, port, addr, &color_pkt)?;
        }
        Ok(())
    }

    /// Gen4 (Blackwell / 50-series) detect: 64-byte [0x11,0x01] block, read 4.
    /// Reply bytes [2],[3] are the card model id (e.g. 0x41 0x76 = Gaming OC).
    fn gen4_detect(&self, handle: usize, port: u8, addr: u8) -> Option<Vec<u8>> {
        let mut blk = vec![0u8; 64];
        blk[0] = 0x11;
        blk[1] = 0x01;
        self.write(handle, port, addr, &blk).ok()?;
        self.read(handle, port, addr, 4).ok()
    }

    /// Gen4 colour write: one 64-byte block per zone. Header [0x12,01,01,06,0A]
    /// then [R,G,B,0x00,zoneId], zero-padded to 64. No mode set-up needed.
    fn set_color_gen4(&self, handle: usize, port: u8, addr: u8, r: u8, g: u8, b: u8) {
        for zone in 0u8..6 {
            let mut blk = vec![0u8; 64];
            blk[..10].copy_from_slice(&[0x12, 0x01, 0x01, 0x06, 0x0A, r, g, b, 0x00, zone]);
            let _ = self.write(handle, port, addr, &blk);
            std::thread::sleep(std::time::Duration::from_millis(9));
        }
    }

    /// setMode per the SignalRGB "Master" plugin: [0x88, mode, 0x06, 0x63, 0x08,
    /// zone] padded to 8. Flag byte 0x08 + speed 0x06 are the values that plugin
    /// uses; our earlier probe sent 0x00/0x02 which the controller ignored.
    fn set_mode_master(&self, handle: usize, port: u8, addr: u8, mode: u8, zone: u8) -> i32 {
        self.write_status_ex(handle, port, addr, &[0x88, mode, 0x06, 0x63, 0x08, zone, 0, 0])
    }

    /// Faithful replay of the SignalRGB Master-protocol static-colour path.
    /// Init: set static mode on zones 0..8 (takes the card out of its onboard
    /// preset). Then drive colour two ways so one of them lands:
    ///   - per-LED zone packets [0xB0+zone*4+i, 0x01, R,G,B,R,G,B]
    ///   - standard packets: setMode(static, led+1) then [0x40, R,G,B, led]
    fn set_color_master(&self, handle: usize, port: u8, addr: u8, r: u8, g: u8, b: u8) {
        for zone in 0u8..8 {
            self.set_mode_master(handle, port, addr, 0x01, zone);
        }
        // Per-LED zone packets (zones 0..3, 4 packets each = up to 8 LEDs/zone).
        for zone in 0u8..4 {
            for i in 0u8..4 {
                let idx = 0xB0u8.wrapping_add(zone * 4).wrapping_add(i);
                self.write_status_ex(handle, port, addr, &[idx, 0x01, r, g, b, r, g, b]);
            }
        }
        // Standard single-colour packets for leds 1..8.
        for led in 1u8..=8 {
            self.set_mode_master(handle, port, addr, 0x01, led + 1);
            self.write_status_ex(handle, port, addr, &[0x40, r, g, b, led, 0, 0, 0]);
        }
    }
}

fn hex_byte(s: &str) -> u8 {
    let s = s.trim_start_matches("0x");
    u8::from_str_radix(s, 16).unwrap_or_else(|_| panic!("bad hex byte: {s}"))
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mode = args.first().map(String::as_str).unwrap_or("scan");
    let nvapi = NvApi::load();
    let gpus = nvapi.gpus();
    println!("NvAPI loaded — {} physical GPU(s)", gpus.len());

    match mode {
        // Quick pass: candidate addresses on port 1 only (the documented port).
        "scan" => {
            for (i, &h) in gpus.iter().enumerate() {
                print_gpu(&nvapi, i, h);
                println!("  port 1, {} candidate addresses...", CANDIDATES.len());
                let mut any = false;
                for &addr in &CANDIDATES {
                    if let Ok(resp) = nvapi.handshake(h, 1, addr) {
                        any = true;
                        report_hit(1, addr, &resp);
                    }
                }
                if !any {
                    println!("  nothing answered on port 1. Run `matrix` to sweep all ports.");
                }
            }
        }
        // Exhaustive: every port 0..8 x full 7-bit address range. Detects on the
        // WRITE ACK (status 0) — a live device ACKs its address even before any
        // read, so this separates "nothing here" (NACK) from "bus gated" (every
        // address fails identically). Prints a per-port status histogram so a
        // global gate is visible.
        "matrix" => {
            for (i, &h) in gpus.iter().enumerate() {
                print_gpu(&nvapi, i, h);
                let mut acks: Vec<(u8, u8)> = Vec::new();
                for port in 0u8..8 {
                    let mut hist: std::collections::BTreeMap<i32, u32> = Default::default();
                    for addr in 0x08u8..0x78 {
                        let st = nvapi.write_status_ex(h, port, addr, &[0xAB, 0, 0, 0, 0, 0, 0, 0]);
                        *hist.entry(st).or_default() += 1;
                        if st == 0 {
                            acks.push((port, addr));
                            // ACK => a device is here; try the handshake read.
                            match nvapi.handshake(h, port, addr) {
                                Ok(resp) => report_hit(port, addr, &resp),
                                Err(s) => println!(
                                    "    port {port}, 0x{addr:02X}: write ACK but read failed ({s} {})",
                                    nv_status(s)
                                ),
                            }
                        }
                    }
                    let summary: Vec<String> = hist
                        .iter()
                        .map(|(st, n)| format!("{st}({}) x{n}", nv_status(*st)))
                        .collect();
                    println!("  port {port}: {}", summary.join(", "));
                }
                println!();
                match acks.as_slice() {
                    [] => println!(
                        "  NO address ACKed on any port. Every write was refused identically =>\n  NvAPI I2C is gated for this card/driver (not an address problem). See diag."
                    ),
                    hits => {
                        println!("  {} address(es) ACKed:", hits.len());
                        for (p, a) in hits {
                            println!("    port {p}, 0x{a:02X}  (try: set {a:02x} ff0000 {p})");
                        }
                    }
                }
            }
        }
        // Surface the exact NvAPI status for one I2C write so we know WHY the
        // sweep is silent (struct-version mismatch vs. NACK vs. gated/unsupported).
        "diag" => {
            let port: u8 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(1);
            let addr = args.get(2).map(|s| hex_byte(s)).unwrap_or(0x71);
            let h = gpus.first().copied().expect("no GPU");
            println!(
                "NvI2cInfoV3: sizeof={} version={:#08x}",
                std::mem::size_of::<NvI2cInfoV3>(),
                i2c_version()
            );
            println!(
                "non-Ex entry points present: write={} read={}",
                nvapi.i2c_write.is_some(),
                nvapi.i2c_read.is_some()
            );
            let probe = [0xABu8, 0, 0, 0, 0, 0, 0, 0];
            let ex = nvapi.write_status_ex(h, port, addr, &probe);
            println!("I2CWriteEx port {port} 0x{addr:02X}: status {ex} ({})", nv_status(ex));
            match nvapi.write_status_plain(h, port, addr, &probe) {
                Some(s) => println!("I2CWrite   port {port} 0x{addr:02X}: status {s} ({})", nv_status(s)),
                None => println!("I2CWrite   (non-Ex): entry point not found at this offset"),
            }
            println!(
                "\nReading: INCOMPATIBLE_STRUCT_VERSION => struct/version wrong; NACK/no device =>\nwrong addr/port; NOT_SUPPORTED/NO_IMPLEMENTATION => driver gates NvAPI I2C (needs\nanother path); INVALID_ARGUMENT => call shape wrong."
            );
        }
        // Faithful SignalRGB Master-protocol static colour (flag 0x08 init +
        // per-LED + standard paths). Use this when `set` ACKs but doesn't light.
        "master" => {
            let addr = hex_byte(args.get(1).expect("usage: master <addr> <RRGGBB> [port]"));
            let hex = args.get(2).map(String::as_str).unwrap_or("00ff00");
            let r = hex_byte(hex.get(0..2).unwrap_or("00"));
            let g = hex_byte(hex.get(2..4).unwrap_or("ff"));
            let b = hex_byte(hex.get(4..6).unwrap_or("00"));
            let port: u8 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(1);
            let h = gpus.first().copied().expect("no GPU");
            println!("master-protocol set port {port}, 0x{addr:02X} -> {r:02x}{g:02x}{b:02x}");
            nvapi.set_color_master(h, port, addr, r, g, b);
            println!("done — did the card change to {r:02x}{g:02x}{b:02x}?");
        }
        // The real 50-series (Blackwell) protocol: 64-byte color blocks.
        "gen4" => {
            let addr = hex_byte(args.get(1).map(String::as_str).unwrap_or("75"));
            let hex = args.get(2).map(String::as_str).unwrap_or("00ff00");
            let r = hex_byte(hex.get(0..2).unwrap_or("00"));
            let g = hex_byte(hex.get(2..4).unwrap_or("ff"));
            let b = hex_byte(hex.get(4..6).unwrap_or("00"));
            let port: u8 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(1);
            let h = gpus.first().copied().expect("no GPU");
            match nvapi.gen4_detect(h, port, addr) {
                Some(resp) => println!(
                    "gen4 detect 0x{addr:02X}: {:02X?}  (model id = {:02X}{:02X})",
                    resp,
                    resp.get(2).copied().unwrap_or(0),
                    resp.get(3).copied().unwrap_or(0)
                ),
                None => println!("gen4 detect 0x{addr:02X}: read failed (write may still work)"),
            }
            println!("gen4 set port {port}, 0x{addr:02X} -> {r:02x}{g:02x}{b:02x} (64-byte blocks, zones 0..5)");
            nvapi.set_color_gen4(h, port, addr, r, g, b);
            println!("done — did the card change to {r:02x}{g:02x}{b:02x}?");
        }
        "raw" => {
            let addr = hex_byte(args.get(1).expect("usage: raw <addr> [port]"));
            let port: u8 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(1);
            let h = gpus.first().copied().expect("no GPU");
            match nvapi.handshake(h, port, addr) {
                Ok(resp) => println!("port {port}, 0x{addr:02X} reply: {:02X?}", resp),
                Err(s) => println!("port {port}, 0x{addr:02X}: NvAPI error {s}"),
            }
        }
        "set" => {
            let addr = hex_byte(args.get(1).expect("usage: set <addr> <RRGGBB> [port]"));
            let hex = args.get(2).map(String::as_str).unwrap_or("ff0000");
            let r = hex_byte(hex.get(0..2).unwrap_or("ff"));
            let g = hex_byte(hex.get(2..4).unwrap_or("00"));
            let b = hex_byte(hex.get(4..6).unwrap_or("00"));
            let port: u8 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(1);
            let h = gpus.first().copied().expect("no GPU");
            println!("setting port {port}, 0x{addr:02X} -> {r:02x}{g:02x}{b:02x} (static, all zones)");
            match nvapi.set_color(h, port, addr, r, g, b) {
                Ok(()) => println!("done — did the card light up {r:02x}{g:02x}{b:02x}?"),
                Err(s) => println!("NvAPI error {s} — wrong port/address or no controller here"),
            }
        }
        other => {
            println!("unknown mode '{other}'. use: scan | matrix | diag [port] [addr] | raw <addr> [port] | set <addr> <RRGGBB> [port]");
        }
    }
}

fn print_gpu(nvapi: &NvApi, i: usize, h: usize) {
    let (ven, dev, sub_ven, sub_dev) = nvapi.pci_ids(h);
    let tag = if sub_ven == 0x1458 { "  <- GIGABYTE" } else { "" };
    println!(
        "\nGPU {i}: vendor={ven:#06x} device={dev:#06x} subvendor={sub_ven:#06x} subdevice={sub_dev:#06x}{tag}"
    );
}

fn report_hit(port: u8, addr: u8, resp: &[u8]) {
    let mark = if resp.first() == Some(&0xAB) {
        "  <== 0xAB HANDSHAKE"
    } else {
        ""
    };
    println!("    port {port}, 0x{addr:02X}: reply {:02X?}{mark}", resp);
}
