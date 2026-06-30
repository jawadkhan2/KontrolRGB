//! Gigabyte RTX 5080 Gaming OC backend (M4) — GPU RGB over NvAPI I2C.
//!
//! The card's RGB controller is a Gigabyte "Gen4" ITE chip on the GPU's
//! internal I2C bus, reachable only through NVIDIA's `nvapi64.dll` (port 1,
//! address 0x75). Detection enumerates the physical GPUs, keeps the
//! Gigabyte-subvendor ones, and probes 0x75 for the controller's model-id reply.
//!
//! There is no per-LED strip here: the controller lights the card's logo as a
//! single colour and has no firmware animation modes, so we expose one zone and
//! let the effects engine stream frames. An NvAPI I2C round-trip is slow enough
//! that doing it inline on `apply()` would stall the engine's shared device
//! lock, so a writer thread owns the I2C and coalesces to the latest colour —
//! `apply()` only stages and signals.

pub mod controller;
pub mod nvapi;

use std::sync::Arc;
use std::thread;

use parking_lot::{Condvar, Mutex};

use controller::Controller;

use super::types::{Color, DeviceInfo, DeviceType, EffectConfig, ZoneInfo};
use super::{DeviceError, RgbDevice};

/// The single uniform RGB zone the controller exposes.
const ZONE_ID: &str = "logo";

pub struct GigabyteGpuDevice {
    info: DeviceInfo,
    shared: Arc<Shared>,
    /// Colour staged by `set_zone_leds`, pushed to the writer on `apply()`.
    staged: Color,
}

struct WorkerState {
    color: Color,
    generation: u64,
    stop: bool,
}

struct Shared {
    state: Mutex<WorkerState>,
    cond: Condvar,
}

/// Detect the card. `Ok(None)` = no Gigabyte NVIDIA GPU with an RGB controller
/// (caller falls back to the mock).
pub fn probe() -> Result<Option<GigabyteGpuDevice>, DeviceError> {
    let Some(nvapi) = nvapi::NvApi::load()? else {
        return Ok(None); // no NVIDIA driver present
    };

    for handle in nvapi.enum_physical_gpus()? {
        // Only poke I2C on Gigabyte boards — avoid touching unrelated buses.
        match nvapi.pci_identifiers(handle) {
            Ok((_, _, sub_ven, _)) if sub_ven == controller::GIGABYTE_SUB_VEN => {}
            _ => continue,
        }

        let ctl = Controller::new(nvapi.clone(), handle);
        if !ctl.probe() {
            continue;
        }

        let info = DeviceInfo {
            id: "gigabyte-gpu".to_string(),
            name: "Gigabyte RTX 5080 Gaming OC".to_string(),
            device_type: DeviceType::Gpu,
            zones: vec![ZoneInfo {
                id: ZONE_ID.to_string(),
                name: "Logo".to_string(),
                led_count: 1,
                resizable: false,
                min_leds: 1,
                max_leds: 1,
                keys: None,
            }],
            supported_effects: EffectConfig::ALL_KINDS.map(String::from).to_vec(),
        };

        let shared = Arc::new(Shared {
            state: Mutex::new(WorkerState {
                color: Color::BLACK,
                generation: 0,
                stop: false,
            }),
            cond: Condvar::new(),
        });

        let worker_shared = shared.clone();
        thread::Builder::new()
            .name("gigabyte-gpu-writer".to_string())
            .spawn(move || worker(ctl, worker_shared))
            .map_err(|e| DeviceError::Comm(e.to_string()))?;

        return Ok(Some(GigabyteGpuDevice {
            info,
            shared,
            staged: Color::BLACK,
        }));
    }

    Ok(None)
}

/// Writer thread: sleeps until `apply()` bumps the generation, then pushes the
/// latest colour. Because it reads the newest colour each wake, frames produced
/// while a write is in flight collapse into one — the I2C self-throttles to
/// whatever the controller can take instead of being streamed at 30 fps.
fn worker(ctl: Controller, shared: Arc<Shared>) {
    let mut seen_generation = 0u64;

    loop {
        let color = {
            let mut st = shared.state.lock();
            while !st.stop && st.generation == seen_generation {
                shared.cond.wait(&mut st);
            }
            if st.stop {
                return;
            }
            seen_generation = st.generation;
            st.color
        };

        if let Err(e) = ctl.set_color(color) {
            eprintln!("gigabyte-gpu: colour write failed: {e}");
        }
    }
}

impl RgbDevice for GigabyteGpuDevice {
    fn id(&self) -> &str {
        &self.info.id
    }

    fn info(&self) -> DeviceInfo {
        self.info.clone()
    }

    fn set_zone_leds(&mut self, zone_id: &str, colors: &[Color]) -> Result<(), DeviceError> {
        if zone_id != ZONE_ID {
            return Err(DeviceError::UnknownZone(zone_id.to_string()));
        }
        // Uniform zone: the first LED's colour drives the whole card. Brightness
        // is already baked in by the effects engine.
        self.staged = colors.first().copied().unwrap_or(Color::BLACK);
        Ok(())
    }

    fn apply(&mut self) -> Result<(), DeviceError> {
        let mut st = self.shared.state.lock();
        st.color = self.staged;
        st.generation += 1;
        drop(st);
        self.shared.cond.notify_one();
        Ok(())
    }

    fn resize_zone(&mut self, zone_id: &str, _led_count: u32) -> Result<(), DeviceError> {
        Err(DeviceError::NotResizable(zone_id.to_string()))
    }
}

impl Drop for GigabyteGpuDevice {
    fn drop(&mut self) {
        let mut st = self.shared.state.lock();
        st.stop = true;
        drop(st);
        self.shared.cond.notify_one();
    }
}
