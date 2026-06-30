//! Minimal NVML FFI for NVIDIA GPU telemetry (Windows).
//!
//! RGB lives on the card's I2C bus (see `device::gigabyte_gpu`), but live
//! telemetry — temperature, core clock, fan %, power draw — is a different
//! transport: NVIDIA's management library, `nvml.dll`. NVML ships with the
//! driver and exports each call by name (no `QueryInterface` indirection like
//! NvAPI), so we resolve a handful of symbols straight out of the DLL. We load
//! it dynamically (like the WinRing0 fan driver and NvAPI) so the app still
//! starts on a machine with no NVIDIA driver.
//!
//! NVML reports exactly the four values the GPU page wants, already in the units
//! the UI shows: temperature in °C, graphics clock in MHz, fan speed as a
//! percent, and power usage in milliwatts (→ watts). Each getter is read-only.

#![allow(non_snake_case)]

use std::ffi::{c_char, CStr};

use libloading::Library;

/// `nvmlReturn_t::NVML_SUCCESS`.
const NVML_SUCCESS: i32 = 0;
/// `nvmlTemperatureSensors_t::NVML_TEMPERATURE_GPU` (the die sensor).
const NVML_TEMPERATURE_GPU: u32 = 0;
/// `nvmlClockType_t::NVML_CLOCK_GRAPHICS` (the core/shader clock).
const NVML_CLOCK_GRAPHICS: u32 = 0;
/// Plenty for the marketing name ("NVIDIA GeForce RTX 5080").
const NAME_BUF_LEN: usize = 96;

// NVML is a plain C library (Win64 C ABI). `nvmlDevice_t` is an opaque pointer;
// we only ever pass it straight back, so we keep it as `usize` to stay Send.
type InitFn = unsafe extern "C" fn() -> i32;
type ShutdownFn = unsafe extern "C" fn() -> i32;
type GetCountFn = unsafe extern "C" fn(*mut u32) -> i32;
type GetHandleFn = unsafe extern "C" fn(u32, *mut usize) -> i32;
type GetNameFn = unsafe extern "C" fn(usize, *mut c_char, u32) -> i32;
type GetTempFn = unsafe extern "C" fn(usize, u32, *mut u32) -> i32;
type GetClockFn = unsafe extern "C" fn(usize, u32, *mut u32) -> i32;
type GetFanFn = unsafe extern "C" fn(usize, *mut u32) -> i32;
type GetPowerFn = unsafe extern "C" fn(usize, *mut u32) -> i32;

/// A loaded, initialized NVML bound to the first NVIDIA GPU.
pub struct Nvml {
    _lib: Library,
    shutdown: ShutdownFn,
    device: usize,
    /// Card marketing name, queried once at load (it never changes), so the
    /// once-a-second poll doesn't re-FFI and re-allocate a String each tick.
    name: Option<String>,
    get_temp: GetTempFn,
    get_clock: GetClockFn,
    get_fan: GetFanFn,
    get_power: GetPowerFn,
}

// Every field is a `Library` (Send+Sync), a bare fn pointer, or a pointer value
// we only hand back to NVML — never deref ourselves.
unsafe impl Send for Nvml {}
unsafe impl Sync for Nvml {}

impl Nvml {
    /// Load `nvml.dll`, initialize, and bind GPU 0. Returns `None` when the DLL
    /// is absent (no NVIDIA driver) or no GPU is present, so the caller can
    /// report telemetry as unavailable rather than failing.
    pub fn load() -> Option<Nvml> {
        // System32 `nvml.dll` is the standard name; older drivers shipped it as
        // `nvidia-ml.dll`. Try both.
        let lib = unsafe { Library::new("nvml.dll") }
            .or_else(|_| unsafe { Library::new("nvidia-ml.dll") })
            .ok()?;

        // SAFETY: each symbol below is an NVML export resolved by its documented
        // name, and the fn signatures match NVML's public C ABI.
        unsafe {
            let init = *lib.get::<InitFn>(b"nvmlInit_v2\0").ok()?;
            let shutdown = *lib.get::<ShutdownFn>(b"nvmlShutdown\0").ok()?;
            let get_count = *lib.get::<GetCountFn>(b"nvmlDeviceGetCount_v2\0").ok()?;
            let get_handle = *lib
                .get::<GetHandleFn>(b"nvmlDeviceGetHandleByIndex_v2\0")
                .ok()?;
            let get_name = *lib.get::<GetNameFn>(b"nvmlDeviceGetName\0").ok()?;
            let get_temp = *lib.get::<GetTempFn>(b"nvmlDeviceGetTemperature\0").ok()?;
            let get_clock = *lib.get::<GetClockFn>(b"nvmlDeviceGetClockInfo\0").ok()?;
            let get_fan = *lib.get::<GetFanFn>(b"nvmlDeviceGetFanSpeed\0").ok()?;
            let get_power = *lib.get::<GetPowerFn>(b"nvmlDeviceGetPowerUsage\0").ok()?;

            if init() != NVML_SUCCESS {
                return None;
            }

            // From here on a failure must still call shutdown to balance init.
            let mut count: u32 = 0;
            if get_count(&mut count) != NVML_SUCCESS || count == 0 {
                shutdown();
                return None;
            }

            // GPU 0: NVML only enumerates NVIDIA cards, so index 0 is the card.
            let mut device: usize = 0;
            if get_handle(0, &mut device) != NVML_SUCCESS {
                shutdown();
                return None;
            }

            // Query the (immutable) card name once, here, rather than every poll.
            let mut buf = [0 as c_char; NAME_BUF_LEN];
            // SAFETY: buf is NAME_BUF_LEN long; NVML writes a NUL-terminated string.
            let name = if get_name(device, buf.as_mut_ptr(), NAME_BUF_LEN as u32) == NVML_SUCCESS {
                // SAFETY: NVML NUL-terminates within the buffer on success.
                Some(CStr::from_ptr(buf.as_ptr()).to_string_lossy().into_owned())
            } else {
                None
            };

            Some(Nvml {
                _lib: lib,
                shutdown,
                device,
                name,
                get_temp,
                get_clock,
                get_fan,
                get_power,
            })
        }
    }

    pub fn name(&self) -> Option<String> {
        self.name.clone()
    }

    /// Die temperature in °C.
    pub fn temp_c(&self) -> Option<u32> {
        let mut out: u32 = 0;
        // SAFETY: single u32 out-param per NVML's contract.
        let status = unsafe { (self.get_temp)(self.device, NVML_TEMPERATURE_GPU, &mut out) };
        (status == NVML_SUCCESS).then_some(out)
    }

    /// Current graphics (core) clock in MHz.
    pub fn core_clock_mhz(&self) -> Option<u32> {
        let mut out: u32 = 0;
        // SAFETY: single u32 out-param per NVML's contract.
        let status = unsafe { (self.get_clock)(self.device, NVML_CLOCK_GRAPHICS, &mut out) };
        (status == NVML_SUCCESS).then_some(out)
    }

    /// Fan speed as a percent of its maximum (0..=100).
    pub fn fan_pct(&self) -> Option<u32> {
        let mut out: u32 = 0;
        // SAFETY: single u32 out-param per NVML's contract.
        let status = unsafe { (self.get_fan)(self.device, &mut out) };
        (status == NVML_SUCCESS).then_some(out)
    }

    /// Board power draw in watts (NVML reports milliwatts).
    pub fn power_w(&self) -> Option<f64> {
        let mut milliwatts: u32 = 0;
        // SAFETY: single u32 out-param per NVML's contract.
        let status = unsafe { (self.get_power)(self.device, &mut milliwatts) };
        (status == NVML_SUCCESS).then_some(milliwatts as f64 / 1000.0)
    }
}

impl Drop for Nvml {
    fn drop(&mut self) {
        // Balance nvmlInit_v2 so the driver refcount stays correct.
        // SAFETY: resolved NVML export, no args.
        unsafe {
            (self.shutdown)();
        }
    }
}
