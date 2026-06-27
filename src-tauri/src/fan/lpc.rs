//! Ring-0 LPC/EC port I/O via WinRing0.
//!
//! Reading the motherboard's Super-I/O / hardware-monitor chip (Nuvoton
//! NCT6687D-R on the MSI Z890) requires privileged x86 `IN`/`OUT` port access,
//! which user-mode code can't do on Windows. WinRing0 is the de-facto signed
//! kernel driver for this (used by LibreHardwareMonitor, FanControl, HWiNFO):
//! its DLL installs the `.sys` via the Service Control Manager and exposes
//! port I/O through IOCTLs. **Loading the driver needs Administrator.**
//!
//! We load `WinRing0x64.dll` *dynamically* (libloading) rather than link it, so
//! KontrolRGB still launches normally when the DLL/driver is absent or the user
//! isn't elevated — the fan subsystem simply reports itself unavailable. The
//! DLL must live next to the executable; we deliberately do not search PATH or
//! the process working directory for a kernel-driver loader.
//!
//! SAFETY MODEL: Phase 1 uses the SIO/EC protocol writes needed to select
//! registers, but does not write fan-control values. Direct port writes to the
//! wrong I/O port are dangerous; only `nct6687.rs` may call them, and only
//! against the SIO config window or EC data window it has validated.

#![cfg(windows)]

use std::sync::Arc;

use libloading::{Library, Symbol};
use parking_lot::Mutex;

/// Filename of the WinRing0 64-bit DLL we expect alongside the executable.
const DLL_NAME: &str = "WinRing0x64.dll";

#[derive(Debug, thiserror::Error)]
pub enum LpcError {
    #[error("WinRing0 driver not available (missing {DLL_NAME} next to the executable, or not running as Administrator)")]
    Unavailable,
    #[error("WinRing0 reported init status {0}")]
    InitStatus(i32),
    #[error("failed to load {DLL_NAME}: {0}")]
    Load(String),
}

// WinRing0 (OpenLibSys) C ABI. All `stdcall` on win64 is just the default ABI.
type InitializeOls = unsafe extern "C" fn() -> i32;
type DeinitializeOls = unsafe extern "C" fn();
type GetDllStatus = unsafe extern "C" fn() -> i32;
type ReadIoPortByte = unsafe extern "C" fn(port: u16) -> u8;
type WriteIoPortByte = unsafe extern "C" fn(port: u16, value: u8);

/// A live WinRing0 handle. Holds the loaded library plus resolved entry points.
/// All access is serialized through `Lpc` (a single global Mutex) because the
/// SIO/EC index→data protocol is inherently stateful: two interleaved sequences
/// would corrupt each other.
struct Ols {
    // Keep the library alive for as long as the symbols are used. Fields are
    // read via the function pointers below; `_lib` must outlive them.
    _lib: Library,
    read_byte: ReadIoPortByte,
    write_byte: WriteIoPortByte,
    deinit: DeinitializeOls,
}

impl Drop for Ols {
    fn drop(&mut self) {
        // Release the driver handle (does not uninstall the service).
        unsafe { (self.deinit)() };
    }
}

/// Process-wide serialized ring-0 port access.
#[derive(Clone)]
pub struct Lpc {
    inner: Arc<Mutex<Ols>>,
}

impl Lpc {
    /// Load WinRing0 and initialize the driver. `Err(Unavailable)` is the
    /// normal, non-fatal outcome when the DLL is missing or the user isn't an
    /// Administrator — the caller should surface "fan control unavailable",
    /// not treat it as a crash.
    pub fn open() -> Result<Lpc, LpcError> {
        // Resolve the DLL next to the executable only. Letting Windows search
        // PATH/current-directory would make it too easy to load the wrong copy
        // of a kernel-driver shim.
        let candidate = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.join(DLL_NAME)));
        let Some(path) = candidate.filter(|p| p.is_file()) else {
            return Err(LpcError::Unavailable);
        };

        let lib = unsafe { Library::new(&path) }.map_err(|_| LpcError::Unavailable)?;

        unsafe {
            let init: Symbol<InitializeOls> = lib
                .get(b"InitializeOls\0")
                .map_err(|e| LpcError::Load(e.to_string()))?;
            let status: Symbol<GetDllStatus> = lib
                .get(b"GetDllStatus\0")
                .map_err(|e| LpcError::Load(e.to_string()))?;
            let read_byte: Symbol<ReadIoPortByte> = lib
                .get(b"ReadIoPortByte\0")
                .map_err(|e| LpcError::Load(e.to_string()))?;
            let write_byte: Symbol<WriteIoPortByte> = lib
                .get(b"WriteIoPortByte\0")
                .map_err(|e| LpcError::Load(e.to_string()))?;

            // InitializeOls returns nonzero on success; GetDllStatus == 0 (OLS_DLL_NO_ERROR)
            // confirms the driver actually loaded (this is what fails without admin).
            if (init)() == 0 {
                return Err(LpcError::Unavailable);
            }
            let st = (status)();
            if st != 0 {
                return Err(LpcError::InitStatus(st));
            }

            // Detach symbols from the borrow by copying the raw fn pointers; we
            // keep `lib` alive in `Ols` so the pointers stay valid.
            let read_byte = *read_byte;
            let write_byte = *write_byte;
            let deinit: Symbol<DeinitializeOls> = lib
                .get(b"DeinitializeOls\0")
                .map_err(|e| LpcError::Load(e.to_string()))?;
            let deinit = *deinit;

            Ok(Lpc {
                inner: Arc::new(Mutex::new(Ols {
                    _lib: lib,
                    read_byte,
                    write_byte,
                    deinit,
                })),
            })
        }
    }

    /// Read one byte from an I/O port. Always safe (no side effects on the SIO).
    pub fn inb(&self, port: u16) -> u8 {
        let ols = self.inner.lock();
        unsafe { (ols.read_byte)(port) }
    }

    /// Write one byte to an I/O port.
    ///
    /// DANGER: only `nct6687.rs` may call this, and only against ports it has
    /// validated as the SIO config window or the EC data window. A stray write
    /// to an arbitrary port can disturb other hardware. Phase 1 does not write.
    pub fn outb(&self, port: u16, value: u8) {
        let ols = self.inner.lock();
        unsafe { (ols.write_byte)(port, value) }
    }
}
