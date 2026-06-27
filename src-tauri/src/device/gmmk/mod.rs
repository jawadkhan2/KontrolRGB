//! Glorious GMMK v1 backend (M2).
//!
//! Two control paths, both verified on the unit:
//! - **Per-key (Static/Custom):** the host paints keys, write-once. The MCU
//!   only ingests ~45 reports/sec, so this is for static patterns, not
//!   animation. Order matters: paint keys FIRST, then enter custom mode.
//! - **Onboard effects:** the host sets a firmware mode in one burst and the
//!   MCU animates it itself (the only way to get smooth animation here).
//!
//! A dedicated writer thread owns the HID handle so the effects engine never
//! blocks; it always applies the newest command, dropping stale ones.

pub mod controller;

use std::sync::Arc;
use std::thread;
use std::time::Duration;

use parking_lot::{Condvar, Mutex};

use controller::GmmkController;

use super::layouts::gmmk_tkl;
use super::types::{Color, DeviceInfo, DeviceType, OnboardEffect, ZoneInfo};
use super::{DeviceError, RgbDevice};

const ZONE_ID: &str = "keys";

/// What the writer thread should put on the keyboard.
#[derive(Clone)]
enum Command {
    /// Per-key custom frame (Static/Custom).
    Frame(Vec<Color>),
    /// Firmware onboard effect + UI brightness percent.
    Onboard(OnboardEffect, u8),
}

struct WorkerState {
    command: Command,
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
    /// Staging buffer for set_zone_leds before apply() commits it.
    staged: Vec<Color>,
}

/// Detect the keyboard. Ok(None) = not plugged in.
pub fn probe() -> Result<Option<GmmkDevice>, DeviceError> {
    let Some(ctl) = GmmkController::open()? else {
        return Ok(None);
    };

    let keys = gmmk_tkl::key_infos();
    let codes = gmmk_tkl::wire_codes();
    let led_count = keys.len() as u32;
    let info = DeviceInfo {
        id: format!("gmmk-{:04x}-{:04x}", controller::VID, controller::PID),
        name: "Glorious GMMK (TKL)".to_string(),
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
        // Host write-once (static/custom) + firmware-animated (onboard).
        supported_effects: vec![
            "static".to_string(),
            "custom".to_string(),
            "onboard".to_string(),
        ],
    };

    let shared = Arc::new(Shared {
        state: Mutex::new(WorkerState {
            command: Command::Frame(vec![Color::BLACK; led_count as usize]),
            generation: 0,
            stop: false,
        }),
        cond: Condvar::new(),
    });

    let worker_shared = shared.clone();
    thread::Builder::new()
        .name("gmmk-writer".to_string())
        .spawn(move || worker(ctl, worker_shared, codes))
        .map_err(|e| DeviceError::Comm(e.to_string()))?;

    Ok(Some(GmmkDevice {
        info,
        shared,
        staged: vec![Color::BLACK; led_count as usize],
    }))
}

fn worker(ctl: GmmkController, shared: Arc<Shared>, codes: Vec<[u8; 3]>) {
    let mut last: Option<Vec<Color>> = None;
    // Whether profile 1 is currently in custom (per-key) mode. Applying an
    // onboard effect leaves custom mode, so this resets afterwards.
    let mut custom_inited = false;
    let mut seen_generation = 0u64;
    // A failed command is retried with the latest state (static effects only
    // bump the generation once).
    let mut pending_retry = false;

    loop {
        let command = {
            let mut st = shared.state.lock();
            while !st.stop && st.generation == seen_generation && !pending_retry {
                shared.cond.wait(&mut st);
            }
            if st.stop {
                return;
            }
            seen_generation = st.generation;
            st.command.clone()
        };

        let res = match &command {
            Command::Onboard(effect, brightness) => {
                let r = ctl.set_onboard(effect, *brightness);
                if r.is_ok() {
                    // Firmware now owns the LEDs; force a full repaint + custom
                    // re-entry next time a per-key frame arrives.
                    custom_inited = false;
                    last = None;
                }
                r
            }
            Command::Frame(frame) => (|| {
                if !custom_inited {
                    // Establish custom mode fast: bulk-fill the dominant color
                    // (~9 writes for the whole board) + paint outliers, THEN
                    // enter custom mode (paint-before-mode is required).
                    paint_bulk(&ctl, &codes, frame)?;
                    ctl.set_custom_mode()?;
                    custom_inited = true;
                    last = Some(frame.clone());
                    Ok(())
                } else {
                    match last.as_deref() {
                        Some(prev) => paint_update(&ctl, &codes, frame, prev)?,
                        None => paint_bulk(&ctl, &codes, frame)?,
                    }
                    last = Some(frame.clone());
                    Ok(())
                }
            })(),
        };

        match res {
            Ok(()) => pending_retry = false,
            Err(e) => {
                pending_retry = true;
                eprintln!("gmmk: command failed: {e}");
                custom_inited = false;
                last = None;
                thread::sleep(Duration::from_millis(500));
            }
        }
    }
}

/// Approx HID-write cost of a bulk fill (begin + 7 chunks + end). Used to
/// decide bulk-vs-per-key; the MCU caps at ~45 writes/s, so fewer writes is the
/// whole game.
const BULK_WRITE_COST: usize = 9;

/// The most common color in the frame — the color we bulk-fill, so the fewest
/// keys need an individual per-key override. Empty frame → BLACK.
fn dominant_color(frame: &[Color]) -> Color {
    let mut counts: Vec<(Color, usize)> = Vec::new();
    for &c in frame {
        match counts.iter_mut().find(|(col, _)| *col == c) {
            Some(e) => e.1 += 1,
            None => counts.push((c, 1)),
        }
    }
    counts
        .into_iter()
        .max_by_key(|&(_, n)| n)
        .map_or(Color::BLACK, |(c, _)| c)
}

/// Paint the given key indices in one begin/end burst (no-op if empty).
fn paint_keys(
    ctl: &GmmkController,
    codes: &[[u8; 3]],
    frame: &[Color],
    indices: &[usize],
) -> Result<(), DeviceError> {
    if indices.is_empty() {
        return Ok(());
    }
    ctl.begin()?;
    for &i in indices {
        ctl.set_key(codes[i], frame[i])?;
    }
    ctl.end()
}

/// Render a whole frame the cheap way: bulk-fill the dominant color, then paint
/// only the keys that differ from it. A uniform frame (Static) is ~9 writes; a
/// custom pattern with a dominant background is ~9 + a handful, versus one
/// write per key (~89 for the TKL) — which is what made switching visibly wipe.
fn paint_bulk(ctl: &GmmkController, codes: &[[u8; 3]], frame: &[Color]) -> Result<(), DeviceError> {
    let bg = dominant_color(frame);
    ctl.set_all_bulk(bg)?;
    let n = codes.len().min(frame.len());
    let outliers: Vec<usize> = (0..n).filter(|&i| frame[i] != bg).collect();
    paint_keys(ctl, codes, frame, &outliers)
}

/// Apply a frame while already in custom mode, choosing whichever is fewer HID
/// writes: a per-key delta against `last`, or a fresh bulk fill + outliers.
/// Sparse tweaks stay per-key; a switch that changes most keys goes bulk.
fn paint_update(
    ctl: &GmmkController,
    codes: &[[u8; 3]],
    frame: &[Color],
    last: &[Color],
) -> Result<(), DeviceError> {
    let n = codes.len().min(frame.len());
    let changed: Vec<usize> = (0..n).filter(|&i| last.get(i) != Some(&frame[i])).collect();
    if changed.is_empty() {
        return Ok(());
    }
    let bg = dominant_color(frame);
    let outliers = (0..n).filter(|&i| frame[i] != bg).count();
    if BULK_WRITE_COST + outliers < changed.len() {
        paint_bulk(ctl, codes, frame)
    } else {
        paint_keys(ctl, codes, frame, &changed)
    }
}

impl GmmkDevice {
    fn submit(&self, command: Command) {
        let mut st = self.shared.state.lock();
        st.command = command;
        st.generation += 1;
        drop(st);
        self.shared.cond.notify_one();
    }
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
        let n = self.staged.len().min(colors.len());
        self.staged[..n].copy_from_slice(&colors[..n]);
        Ok(())
    }

    fn apply(&mut self) -> Result<(), DeviceError> {
        self.submit(Command::Frame(self.staged.clone()));
        Ok(())
    }

    fn resize_zone(&mut self, zone_id: &str, _led_count: u32) -> Result<(), DeviceError> {
        Err(DeviceError::NotResizable(zone_id.to_string()))
    }

    fn set_onboard_effect(
        &mut self,
        effect: &OnboardEffect,
        brightness: u8,
    ) -> Result<(), DeviceError> {
        self.submit(Command::Onboard(*effect, brightness));
        Ok(())
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
