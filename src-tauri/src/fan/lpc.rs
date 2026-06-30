//! Ring-0 LPC/EC port I/O via PawnIO.
//!
//! Reading the motherboard's Super-I/O / hardware-monitor chip (Nuvoton
//! NCT6687D-R on the MSI Z890) requires privileged x86 `IN`/`OUT` port access,
//! which user-mode code can't do on Windows. We used to do this through
//! WinRing0, but that driver is on Microsoft's vulnerable-driver blocklist
//! (CVE-2020-14979 — arbitrary ring-0 memory R/W) and Defender now quarantines
//! it on sight. We replaced it with **PawnIO** (pawnio.eu, GPL/LGPL): a signed
//! kernel driver that runs sandboxed Pawn bytecode *modules* exposed through a
//! narrow IOCTL surface, so there is no arbitrary-memory hole to blocklist. It
//! is the same driver LibreHardwareMonitor, OpenRGB and FanControl all migrated
//! to in 2025.
//!
//! We talk to the driver device directly (`\\?\GLOBALROOT\Device\PawnIO`) — no
//! `PawnIOLib.dll` dependency — exactly as LHM does: `DeviceIoControl` with a
//! load-binary IOCTL to upload a module, then an execute IOCTL to call its named
//! functions. The module we load is **LpcIO** (from the PawnIO.Modules repo,
//! LGPL-2.1, shipped verbatim in `resources/pawnio/LpcIO.bin`). It exposes:
//! - `ioctl_select_slot(slot)` — pick the SIO config window (0 = 0x2E, 1 = 0x4E)
//! - `ioctl_find_bars()` — probe the LDN base addresses and *allow-list* those
//!   I/O windows for raw port access
//! - `ioctl_pio_inb/outb` — raw byte I/O, but ONLY to the SIO register ports or
//!   a discovered BAR window
//! - `ioctl_superio_inb/inw/outb` — SIO config index/data helpers
//!
//! Loading the driver needs it installed (the user runs PawnIO's signed
//! installer once); when it isn't, `open()` returns `Unavailable` and the fan
//! subsystem reports itself off — never a crash.
//!
//! SAFETY MODEL — two layers, both new vs. WinRing0:
//!  1. **Port allow-list (in-driver).** `ioctl_pio_*` refuses any port outside
//!     the SIO register ports and the BARs `find_bars` discovered, so a stray
//!     write to an arbitrary I/O port — a board-bricking vector — is rejected in
//!     ring 0, not trusted to us.
//!  2. **Cross-process arbitration.** The SIO index/data and EC paged windows are
//!     globally shared hardware; two tools interleaving their index→data
//!     sequences corrupt each other (a suspected cause of the SYS-fan / BIOS
//!     corruption seen with the raw-WinRing0 path). PawnIO's modules document the
//!     `\BaseNamedObjects\Access_ISABUS.HTP.Method` mutant as the arbitration
//!     lock every ISA-bus tool honors; `isa_lock` acquires it so our whole
//!     multi-byte sequences are atomic against MSI Center / HWiNFO / LHM.

#![cfg(windows)]

use std::ffi::c_void;
use std::marker::PhantomData;
use std::ptr;
use std::sync::Arc;

use parking_lot::Mutex;
use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, HANDLE, INVALID_HANDLE_VALUE, WAIT_ABANDONED, WAIT_OBJECT_0,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows_sys::Win32::System::Threading::{
    CreateMutexW, ReleaseMutex, WaitForSingleObject,
};
use windows_sys::Win32::System::IO::DeviceIoControl;

/// Compiled LpcIO PawnIO module (PawnIO.Modules, LGPL-2.1). Uploaded to the
/// driver at `open()`. See `resources/pawnio/COPYING`.
const LPC_IO_BIN: &[u8] = include_bytes!("../../resources/pawnio/LpcIO.bin");

/// PawnIO driver device, in the kernel object namespace so no symlink/letter is
/// needed. Same path LHM opens.
const DEVICE_PATH: &str = r"\\?\GLOBALROOT\Device\PawnIO";

/// PawnIO IOCTL codes (DEVICE_TYPE 41394 << 16, function << 2). Verbatim from
/// LHM's `PawnIo.cs`.
const IOCTL_PIO_LOAD_BINARY: u32 = (41394u32 << 16) | (0x821 << 2);
const IOCTL_PIO_EXECUTE_FN: u32 = (41394u32 << 16) | (0x841 << 2);

/// PawnIO execute ABI: the function name occupies a fixed 32-byte ASCII prefix of
/// the input buffer, followed by the `u64` argument cells.
const FN_NAME_LEN: usize = 32;

/// Global ISA-bus arbitration mutex shared with every other hardware-monitor
/// tool. `\BaseNamedObjects\X` is the `Global\` namespace in Win32 naming.
const ISA_MUTEX_NAME: &str = r"Global\Access_ISABUS.HTP.Method";

/// How long to wait for the ISA mutex before proceeding best-effort. Long enough
/// to outlast another tool's poll, short enough not to stall the UI.
const ISA_LOCK_TIMEOUT_MS: u32 = 200;

#[derive(Debug, thiserror::Error)]
pub enum LpcError {
    #[error("PawnIO driver not available (install it from pawnio.eu and run KontrolRGB as Administrator)")]
    Unavailable,
    #[error("PawnIO rejected the LpcIO module (load IOCTL failed, win32 error {0})")]
    LoadFailed(u32),
}

/// A raw Windows HANDLE that we promise is safe to move/share between threads.
/// Kernel handles are process-global; the only thread-affinity constraint
/// (a mutex must be released by the acquiring thread) is upheld by `isa_lock`
/// returning a guard that releases in the same call frame.
#[derive(Clone, Copy)]
struct WinHandle(HANDLE);
unsafe impl Send for WinHandle {}
unsafe impl Sync for WinHandle {}

/// Owns the PawnIO device handle; closes it on drop. Behind a `Mutex` in `Lpc`
/// so concurrent callers can't interleave `DeviceIoControl`s on it.
struct Device(WinHandle);

impl Drop for Device {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0 .0) };
    }
}

struct Inner {
    dev: Mutex<Device>,
    /// Cross-process ISA-bus arbitration mutex (best-effort; null if unavailable).
    isa: WinHandle,
}

impl Drop for Inner {
    fn drop(&mut self) {
        if !self.isa.0.is_null() {
            unsafe { CloseHandle(self.isa.0) };
        }
    }
}

/// Process-wide serialized ring-0 port access via PawnIO.
#[derive(Clone)]
pub struct Lpc {
    inner: Arc<Inner>,
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

impl Lpc {
    /// Open the PawnIO device and upload the LpcIO module. `Err(Unavailable)` is
    /// the normal, non-fatal outcome when the driver isn't installed or we aren't
    /// elevated — the caller surfaces "fan control unavailable", not a crash.
    pub fn open() -> Result<Lpc, LpcError> {
        let path = wide(DEVICE_PATH);
        // GENERIC_READ | GENERIC_WRITE.
        let handle = unsafe {
            CreateFileW(
                path.as_ptr(),
                0x8000_0000 | 0x4000_0000,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                ptr::null(),
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                ptr::null_mut(),
            )
        };
        if handle == INVALID_HANDLE_VALUE || handle.is_null() {
            return Err(LpcError::Unavailable);
        }

        // Upload the LpcIO module bytecode.
        let ok = unsafe {
            DeviceIoControl(
                handle,
                IOCTL_PIO_LOAD_BINARY,
                LPC_IO_BIN.as_ptr() as *const c_void,
                LPC_IO_BIN.len() as u32,
                ptr::null_mut(),
                0,
                ptr::null_mut(),
                ptr::null_mut(),
            )
        };
        if ok == 0 {
            let err = unsafe { GetLastError() };
            unsafe { CloseHandle(handle) };
            return Err(LpcError::LoadFailed(err));
        }

        // Best-effort open of the shared ISA-bus mutex. CreateMutexW opens it if
        // another tool already created it (ERROR_ALREADY_EXISTS). A null result
        // just means we run without cross-process arbitration.
        let name = wide(ISA_MUTEX_NAME);
        let isa = unsafe { CreateMutexW(ptr::null(), 0, name.as_ptr()) };

        Ok(Lpc {
            inner: Arc::new(Inner {
                dev: Mutex::new(Device(WinHandle(handle))),
                isa: WinHandle(isa),
            }),
        })
    }

    /// Invoke one PawnIO module function. `input`/`out_len` are counts of `u64`
    /// cells. Returns the cells the driver wrote back.
    fn execute(&self, name: &str, input: &[u64], out_len: usize) -> Result<Vec<u64>, LpcError> {
        let mut in_buf = vec![0u8; FN_NAME_LEN + input.len() * 8];
        let n = name.len().min(FN_NAME_LEN - 1);
        in_buf[..n].copy_from_slice(&name.as_bytes()[..n]);
        for (i, v) in input.iter().enumerate() {
            let o = FN_NAME_LEN + i * 8;
            in_buf[o..o + 8].copy_from_slice(&v.to_le_bytes());
        }

        let mut out_buf = vec![0u8; out_len * 8];
        let mut returned: u32 = 0;

        let dev = self.inner.dev.lock();
        let ok = unsafe {
            DeviceIoControl(
                dev.0 .0,
                IOCTL_PIO_EXECUTE_FN,
                in_buf.as_ptr() as *const c_void,
                in_buf.len() as u32,
                if out_len == 0 {
                    ptr::null_mut()
                } else {
                    out_buf.as_mut_ptr() as *mut c_void
                },
                out_buf.len() as u32,
                &mut returned,
                ptr::null_mut(),
            )
        };
        drop(dev);

        if ok == 0 {
            return Err(LpcError::LoadFailed(unsafe { GetLastError() }));
        }
        let cells = (returned as usize) / 8;
        let mut out = Vec::with_capacity(cells);
        for i in 0..cells {
            let o = i * 8;
            out.push(u64::from_le_bytes(out_buf[o..o + 8].try_into().unwrap()));
        }
        Ok(out)
    }

    /// Select the SIO config window: slot 0 = ports 0x2E/0x2F, slot 1 = 0x4E/0x4F.
    /// Must be called before any port I/O on that slot (the module gates I/O on a
    /// selected register port). Resets the BAR allow-list.
    pub fn select_slot(&self, slot: u8) -> Result<(), LpcError> {
        self.execute("ioctl_select_slot", &[slot as u64], 0).map(|_| ())
    }

    /// Probe the SIO logical devices and allow-list their decoded I/O base
    /// windows (BARs), enabling `inb`/`outb` against the EC window. Call once,
    /// while the chip is unlocked (config mode), before touching the EC.
    pub fn find_bars(&self) -> Result<(), LpcError> {
        self.execute("ioctl_find_bars", &[], 0).map(|_| ())
    }

    /// Read one byte from an I/O port (must be an allow-listed SIO/BAR port).
    /// Infallible by contract — a driver/permission failure degrades to 0.
    pub fn inb(&self, port: u16) -> u8 {
        self.execute("ioctl_pio_inb", &[port as u64], 1)
            .ok()
            .and_then(|v| v.first().copied())
            .unwrap_or(0) as u8
    }

    /// Write one byte to an I/O port.
    ///
    /// The driver refuses ports outside the SIO register ports and discovered
    /// BARs, so only the validated SIO config window and EC data window are
    /// reachable — a stray write elsewhere is rejected in ring 0.
    pub fn outb(&self, port: u16, value: u8) {
        let _ = self.execute("ioctl_pio_outb", &[port as u64, value as u64], 0);
    }

    /// Acquire the global ISA-bus arbitration mutex for the returned guard's
    /// lifetime, so a multi-byte SIO/EC sequence is atomic against other tools.
    /// Recursive (a thread may nest these). On timeout it proceeds best-effort
    /// rather than block the caller.
    pub fn isa_lock(&self) -> IsaGuard<'_> {
        let h = self.inner.isa.0;
        let acquired = if h.is_null() {
            false
        } else {
            matches!(
                unsafe { WaitForSingleObject(h, ISA_LOCK_TIMEOUT_MS) },
                WAIT_OBJECT_0 | WAIT_ABANDONED
            )
        };
        IsaGuard {
            handle: h,
            acquired,
            _p: PhantomData,
        }
    }
}

/// Holds the ISA-bus mutex; releases it on drop (same thread that acquired it).
pub struct IsaGuard<'a> {
    handle: HANDLE,
    acquired: bool,
    _p: PhantomData<&'a Lpc>,
}

impl Drop for IsaGuard<'_> {
    fn drop(&mut self) {
        if self.acquired {
            unsafe { ReleaseMutex(self.handle) };
        }
    }
}
