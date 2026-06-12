//! Glorious GMMK v1 backend (M2).
//!
//! The wire protocol is slow (one 64-byte packet + ack per key, ~200-300 ms
//! for a full 104-key frame), so a dedicated writer thread owns the HID
//! handle: the effects engine stages frames without blocking, and the worker
//! always writes the newest frame, dropping stale intermediates.

pub mod controller;

use std::sync::Arc;
use std::thread;
use std::time::Duration;

use parking_lot::{Condvar, Mutex};

use controller::GmmkController;

use super::layouts::gmmk_ansi;
use super::types::{Color, DeviceInfo, DeviceType, EffectConfig, ZoneInfo};
use super::{DeviceError, RgbDevice};

const ZONE_ID: &str = "keys";

struct WorkerState {
    desired: Vec<Color>,
    generation: u64,
    stop: bool,
}

struct Shared {
    state: Mutex<WorkerState>,
    cond: Condvar,
}

pub struct GmmkDevice {
    info: DeviceInfo,
    shared: Arc<Shared>,
}

/// Detect the keyboard. Ok(None) = not plugged in.
pub fn probe() -> Result<Option<GmmkDevice>, DeviceError> {
    let Some(ctl) = GmmkController::open()? else {
        return Ok(None);
    };

    let keys = gmmk_ansi::full_size();
    let led_count = keys.len() as u32;
    let info = DeviceInfo {
        id: format!("gmmk-{:04x}-{:04x}", controller::VID, controller::PID),
        name: "Glorious GMMK".to_string(),
        device_type: DeviceType::Keyboard,
        zones: vec![ZoneInfo {
            id: ZONE_ID.to_string(),
            name: "Keys".to_string(),
            led_count,
            resizable: false,
            min_leds: led_count,
            max_leds: led_count,
            keys: Some(keys),
        }],
        supported_effects: EffectConfig::ALL_KINDS.map(String::from).to_vec(),
    };

    let shared = Arc::new(Shared {
        state: Mutex::new(WorkerState {
            desired: vec![Color::BLACK; led_count as usize],
            generation: 0,
            stop: false,
        }),
        cond: Condvar::new(),
    });

    let worker_shared = shared.clone();
    thread::Builder::new()
        .name("gmmk-writer".to_string())
        .spawn(move || worker(ctl, worker_shared))
        .map_err(|e| DeviceError::Comm(e.to_string()))?;

    Ok(Some(GmmkDevice { info, shared }))
}

fn worker(ctl: GmmkController, shared: Arc<Shared>) {
    // Put profile 1 into custom per-key mode at max hardware brightness;
    // dimming is done in software by the effects engine. The first writes
    // after open occasionally time out (seen right after a replug), so
    // initialization is retried before each frame until it sticks.
    let init = |ctl: &GmmkController| -> Result<(), DeviceError> {
        ctl.set_active_profile_1()?;
        ctl.begin()?;
        ctl.set_custom_mode()?;
        ctl.set_hw_brightness(9)?;
        ctl.end()?;
        eprintln!("gmmk: initialized (profile 1, custom mode)");
        Ok(())
    };
    let mut initialized = false;

    let mut last: Option<Vec<Color>> = None;
    let mut seen_generation = 0u64;
    // A failed frame is retried with the latest desired state instead of
    // waiting for the next generation bump — static effects only bump once.
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
            st.desired.clone()
        };

        let res = (|| {
            if !initialized {
                init(&ctl)?;
                initialized = true;
            }
            write_frame(&ctl, &frame, last.as_deref())
        })();
        match res {
            Ok(()) => {
                last = Some(frame);
                pending_retry = false;
            }
            Err(e) => {
                pending_retry = true;
                eprintln!("gmmk: write failed: {e}");
                // Forget what we think is on the keyboard, redo init, and
                // back off a bit (covers transient errors, e.g. replug).
                initialized = false;
                last = None;
                thread::sleep(Duration::from_millis(500));
            }
        }
    }
}

fn write_frame(
    ctl: &GmmkController,
    frame: &[Color],
    last: Option<&[Color]>,
) -> Result<(), DeviceError> {
    let changed: Vec<usize> = (0..frame.len())
        .filter(|&i| last.map_or(true, |l| l[i] != frame[i]))
        .collect();
    if changed.is_empty() {
        return Ok(());
    }
    ctl.begin()?;
    for i in changed {
        ctl.set_key(i, frame[i])?;
    }
    ctl.end()
}

impl RgbDevice for GmmkDevice {
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
        let mut st = self.shared.state.lock();
        let n = st.desired.len().min(colors.len());
        st.desired[..n].copy_from_slice(&colors[..n]);
        Ok(())
    }

    fn apply(&mut self) -> Result<(), DeviceError> {
        let mut st = self.shared.state.lock();
        st.generation += 1;
        drop(st);
        self.shared.cond.notify_one();
        Ok(())
    }

    fn resize_zone(&mut self, zone_id: &str, _led_count: u32) -> Result<(), DeviceError> {
        Err(DeviceError::NotResizable(zone_id.to_string()))
    }
}

impl Drop for GmmkDevice {
    fn drop(&mut self) {
        let mut st = self.shared.state.lock();
        st.stop = true;
        drop(st);
        self.shared.cond.notify_one();
    }
}
