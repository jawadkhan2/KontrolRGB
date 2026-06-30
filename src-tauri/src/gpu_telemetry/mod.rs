//! GPU telemetry subsystem — live temp / clock / fan / power for the GPU page.
//!
//! Separate from the RGB device manager: telemetry isn't an `RgbDevice` and uses
//! NVML (`nvml.dll`) rather than the card's I2C bus. NVML is Windows/Linux only
//! and the app targets Windows, so the loader is gated behind `cfg(windows)`;
//! elsewhere telemetry simply reports unavailable.
//!
//! The frontend polls `gpu_telemetry` roughly once a second. NVML is initialized
//! lazily on the first read and the handle is cached for the process lifetime,
//! so a machine without an NVIDIA driver pays the failed load exactly once.

#[cfg(windows)]
mod nvml;

#[cfg(windows)]
use std::sync::Arc;

use parking_lot::Mutex;
use serde::Serialize;

/// One telemetry snapshot. Every metric is optional so a partial read (a driver
/// that won't report, say, power) still surfaces the values it does have.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GpuTelemetry {
    /// True once NVML is loaded and a GPU is bound.
    pub available: bool,
    /// Card marketing name (e.g. "NVIDIA GeForce RTX 5080").
    pub name: Option<String>,
    /// Die temperature, °C.
    pub temp_c: Option<u32>,
    /// Core (graphics) clock, MHz.
    pub core_clock_mhz: Option<u32>,
    /// Fan speed, percent of maximum.
    pub fan_pct: Option<u32>,
    /// Board power draw, watts.
    pub power_w: Option<f64>,
}

/// Lazily-loaded NVML binding. `Uninit` until the first read; then either the
/// live handle or `Unavailable` (cached so we don't retry the DLL load forever).
enum State {
    Uninit,
    Unavailable,
    #[cfg(windows)]
    Ready(Arc<nvml::Nvml>),
}

pub struct GpuTelemetrySubsystem {
    state: Mutex<State>,
}

impl GpuTelemetrySubsystem {
    pub fn new() -> Self {
        GpuTelemetrySubsystem {
            state: Mutex::new(State::Uninit),
        }
    }

    /// Read a fresh snapshot. Initializes NVML on first call. Cheap enough to
    /// poll once a second; each getter is an independent NVML query so one
    /// failing metric doesn't sink the rest.
    pub fn read(&self) -> GpuTelemetry {
        // Hold the lock only long enough to init and grab a handle; the actual
        // NVML getters then run lock-free, so a slow/stuck FFI call can't block
        // another reader (and `Arc` keeps the handle alive past the unlock).
        #[cfg(windows)]
        let nvml = {
            let mut state = self.state.lock();
            if matches!(*state, State::Uninit) {
                *state = Self::init();
            }
            match &*state {
                State::Ready(nvml) => nvml.clone(),
                _ => return GpuTelemetry::default(),
            }
        };

        #[cfg(windows)]
        return GpuTelemetry {
            available: true,
            name: nvml.name(),
            temp_c: nvml.temp_c(),
            core_clock_mhz: nvml.core_clock_mhz(),
            fan_pct: nvml.fan_pct().map(|p| p.min(100)),
            power_w: nvml.power_w(),
        };

        #[cfg(not(windows))]
        {
            let mut state = self.state.lock();
            if matches!(*state, State::Uninit) {
                *state = Self::init();
            }
            GpuTelemetry::default()
        }
    }

    #[cfg(windows)]
    fn init() -> State {
        match nvml::Nvml::load() {
            Some(nvml) => State::Ready(Arc::new(nvml)),
            None => State::Unavailable,
        }
    }

    #[cfg(not(windows))]
    fn init() -> State {
        State::Unavailable
    }
}

impl Default for GpuTelemetrySubsystem {
    fn default() -> Self {
        Self::new()
    }
}
