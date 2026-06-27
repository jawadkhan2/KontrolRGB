//! MSI MAG Z890 Tomahawk backend (M3) — ARGB fans on the JARGB_V2 headers.
//!
//! The MSI 761 direct protocol is plain HID feature reports (one ~727-byte
//! packet per header per frame), so unlike the GMMK there's no throughput wall
//! and no writer thread: `apply()` writes the headers inline. The board can
//! silently leave direct mode on a failed write, so a failed frame re-sends the
//! 0x50 setup and retries once.
//!
//! Onboard board RGB is out of scope here (still mock); only JARGB_V2_1/2/3
//! are driven, mapped to MSI header indices 0, 1, and 2.

pub mod controller;

use std::collections::HashMap;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use parking_lot::{Condvar, Mutex};

use controller::MsiController;

use super::types::{Color, DeviceInfo, DeviceType, EffectConfig, ZoneInfo};
use super::{DeviceError, RgbDevice};

/// Default LED count per header until the user resizes to the real fan chain.
const DEFAULT_LEDS: u32 = 16;

/// (zone id, display name, MSI header index hdr1).
const HEADERS: [(&str, &str, u8); 3] = [
    ("jargb_v2_1", "JARGB_V2 1", 0),
    ("jargb_v2_2", "JARGB_V2 2", 1),
    ("jargb_v2_3", "JARGB_V2 3", 2),
];

pub struct MsiDevice {
    info: DeviceInfo,
    shared: Arc<Shared>,
    /// zone id -> staged colors (committed to hardware on apply()).
    staged: HashMap<String, Vec<Color>>,
}

struct WorkerState {
    frame: HashMap<String, Vec<Color>>,
    generation: u64,
    stop: bool,
}

struct Shared {
    state: Mutex<WorkerState>,
    cond: Condvar,
}

/// Detect the board. Ok(None) = not present.
pub fn probe() -> Result<Option<MsiDevice>, DeviceError> {
    let Some(ctl) = MsiController::open()? else {
        return Ok(None);
    };
    // Arm direct mode on the JARGB headers before the first frame.
    ctl.send_setup()?;

    let zones: Vec<ZoneInfo> = HEADERS
        .iter()
        .map(|(id, name, _)| ZoneInfo {
            id: id.to_string(),
            name: name.to_string(),
            led_count: DEFAULT_LEDS,
            resizable: true,
            min_leds: 1,
            max_leds: controller::MAX_LEDS as u32,
            keys: None,
        })
        .collect();

    let staged: HashMap<String, Vec<Color>> = zones
        .iter()
        .map(|z| (z.id.clone(), vec![Color::BLACK; z.led_count as usize]))
        .collect();

    let info = DeviceInfo {
        id: format!("msi-{:04x}-{:04x}", controller::VID, controller::PID),
        name: "MSI MAG Z890 Tomahawk".to_string(),
        device_type: DeviceType::Motherboard,
        zones,
        supported_effects: EffectConfig::ALL_KINDS.map(String::from).to_vec(),
    };

    let shared = Arc::new(Shared {
        state: Mutex::new(WorkerState {
            frame: staged.clone(),
            generation: 0,
            stop: false,
        }),
        cond: Condvar::new(),
    });

    let worker_shared = shared.clone();
    thread::Builder::new()
        .name("msi-writer".to_string())
        .spawn(move || worker(ctl, worker_shared))
        .map_err(|e| DeviceError::Comm(e.to_string()))?;

    Ok(Some(MsiDevice {
        info,
        shared,
        staged,
    }))
}

fn worker(ctl: MsiController, shared: Arc<Shared>) {
    let mut seen_generation = 0u64;
    let mut pending_retry = false;

    loop {
        let frame = {
            let mut st = shared.state.lock();
            while !st.stop && st.generation == seen_generation && !pending_retry {
                shared.cond.wait(&mut st);
            }
            if st.stop {
                return;
            }
            seen_generation = st.generation;
            st.frame.clone()
        };

        match write_frame(&ctl, &frame) {
            Ok(()) => pending_retry = false,
            Err(e) => {
                pending_retry = true;
                eprintln!("msi: frame write failed: {e}");
                thread::sleep(Duration::from_millis(500));
            }
        }
    }
}

fn write_frame(
    ctl: &MsiController,
    frame: &HashMap<String, Vec<Color>>,
) -> Result<(), DeviceError> {
    for (id, _, hdr1) in HEADERS {
        if let Some(colors) = frame.get(id) {
            write_header(ctl, hdr1, colors)?;
        }
    }
    Ok(())
}

/// Write a header, re-arming direct mode and retrying once on failure (the
/// board can drop direct mode after an error).
fn write_header(ctl: &MsiController, hdr1: u8, colors: &[Color]) -> Result<(), DeviceError> {
    if ctl.set_header(hdr1, colors).is_ok() {
        return Ok(());
    }
    ctl.send_setup()?;
    ctl.set_header(hdr1, colors)
}

impl RgbDevice for MsiDevice {
    fn id(&self) -> &str {
        &self.info.id
    }

    fn info(&self) -> DeviceInfo {
        self.info.clone()
    }

    fn set_zone_leds(&mut self, zone_id: &str, colors: &[Color]) -> Result<(), DeviceError> {
        let staged = self
            .staged
            .get_mut(zone_id)
            .ok_or_else(|| DeviceError::UnknownZone(zone_id.to_string()))?;
        let n = staged.len().min(colors.len());
        staged[..n].copy_from_slice(&colors[..n]);
        Ok(())
    }

    fn apply(&mut self) -> Result<(), DeviceError> {
        let mut st = self.shared.state.lock();
        st.frame = self.staged.clone();
        st.generation += 1;
        drop(st);
        self.shared.cond.notify_one();
        Ok(())
    }

    fn resize_zone(&mut self, zone_id: &str, led_count: u32) -> Result<(), DeviceError> {
        let zone = self
            .info
            .zones
            .iter_mut()
            .find(|z| z.id == zone_id)
            .ok_or_else(|| DeviceError::UnknownZone(zone_id.to_string()))?;
        if !zone.resizable {
            return Err(DeviceError::NotResizable(zone_id.to_string()));
        }
        if led_count < zone.min_leds || led_count > zone.max_leds {
            return Err(DeviceError::LedCountOutOfRange {
                count: led_count,
                min: zone.min_leds,
                max: zone.max_leds,
            });
        }
        zone.led_count = led_count;
        self.staged
            .insert(zone_id.to_string(), vec![Color::BLACK; led_count as usize]);
        Ok(())
    }
}

impl Drop for MsiDevice {
    fn drop(&mut self) {
        let mut st = self.shared.state.lock();
        st.stop = true;
        drop(st);
        self.shared.cond.notify_one();
    }
}
