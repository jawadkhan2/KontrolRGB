//! ~30 fps tick loop: computes frames for every device zone, pushes them
//! through the RgbDevice trait, and mirrors them to the frontend at ~15 fps
//! via `device-frame` events.
//!
//! Efficiency rules:
//! - preview frames are only emitted while the window is visible (a hidden
//!   webview has nobody to show them to, and each one is a JSON serialise +
//!   IPC hop);
//! - with the window hidden and every effect time-invariant (Static, Custom,
//!   firmware Onboard) the loop only does real work every `IDLE_EVERY` ticks;
//! - device info is borrowed, never cloned per tick.
//!
//! Lock order (crate-wide): manager → runtime → identify → zone_names → stale.

use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::device::types::{Color, EffectConfig, OnboardEffect};
use crate::state::AppState;

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct FramePayload {
    device_id: String,
    zones: HashMap<String, Vec<Color>>,
}

/// How long an identify pulse runs before it expires and the zone returns to
/// its configured effect.
const IDENTIFY_SECS: f32 = 4.0;

/// Hidden window + only static effects: do a pass every this many ticks
/// (~4 Hz) instead of every tick. Still fast enough to react to a resync.
const IDLE_EVERY: u64 = 8;

/// White brightness pulse for the identify animation. `elapsed` is seconds
/// since the pulse started; pulses at ~1.5 Hz, dark -> bright -> dark.
fn identify_color(elapsed: f32) -> Color {
    let s = 0.5 - 0.5 * (elapsed * std::f32::consts::TAU * 1.5).cos();
    let v = (s * 255.0) as u8;
    Color { r: v, g: v, b: v }
}

fn time_invariant(effect: &EffectConfig) -> bool {
    matches!(
        effect,
        EffectConfig::Static { .. } | EffectConfig::Custom | EffectConfig::Onboard(_)
    )
}

pub async fn run(app: AppHandle, state: Arc<AppState>) {
    let start = Instant::now();
    let mut interval = tokio::time::interval(Duration::from_millis(33));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    // device_id -> zone_id -> last frame, to skip redundant hardware writes.
    let mut last_frames: HashMap<String, HashMap<String, Vec<Color>>> = HashMap::new();
    // device_id -> last onboard effect applied to firmware, to apply only on change.
    let mut last_onboard: HashMap<String, (OnboardEffect, u8)> = HashMap::new();
    let mut seen_device_generation = state.device_generation.load(Ordering::Relaxed);
    let mut tick: u64 = 0;

    loop {
        interval.tick().await;
        tick += 1;
        let ui_visible = state.ui_visible.load(Ordering::Relaxed);
        let device_generation = state.device_generation.load(Ordering::Relaxed);
        let generation_changed = device_generation != seen_device_generation;

        if !ui_visible && !generation_changed && !tick.is_multiple_of(IDLE_EVERY) {
            let idle = {
                let runtime = state.runtime.lock();
                runtime.values().all(|rt| time_invariant(&rt.effect))
            } && state.identify.lock().is_empty();
            if idle {
                continue;
            }
        }

        let t = start.elapsed().as_secs_f32();
        let emit_this_tick = ui_visible && tick.is_multiple_of(2);
        let mut payloads: Vec<FramePayload> = Vec::new();
        let hw_enabled = state.conflicts_cleared.load(Ordering::Relaxed);
        if generation_changed {
            last_frames.clear();
            last_onboard.clear();
            seen_device_generation = device_generation;
        }

        {
            let mut manager = state.manager.lock();
            let runtime = state.runtime.lock();
            let mut identify = state.identify.lock();
            // Drop expired pulses so they stop overriding their zone.
            identify.retain(|_, (_, started)| started.elapsed().as_secs_f32() < IDENTIFY_SECS);

            for device in manager.iter_mut() {
                // --- Phase 1: compute, borrowing the device's info. ---
                let info = device.info();
                let Some(rt) = runtime.get(&info.id) else {
                    continue;
                };
                let device_id = info.id.clone();
                let brightness = rt.brightness as f32 / 100.0;

                // Active identify pulse for this device (zone id + its color now).
                let identify_zone: Option<(&str, Color)> =
                    identify.get(&device_id).map(|(z, started)| {
                        (z.as_str(), identify_color(started.elapsed().as_secs_f32()))
                    });

                // Firmware-animated effect on hardware that supports it: set the
                // mode ONCE when it changes; never stream per-key frames. We
                // still emit a host preview approximation so the UI animates.
                // An active identify pulse forces the host path so we can stream
                // the pulse frames, even on otherwise firmware-animated hardware.
                let onboard_hw = matches!(rt.effect, EffectConfig::Onboard(_))
                    && info.supported_effects.iter().any(|s| s == "onboard")
                    && identify_zone.is_none();

                let frames: Vec<(String, Vec<Color>)> = if onboard_hw && !emit_this_tick {
                    Vec::new()
                } else {
                    info.zones
                        .iter()
                        .map(|zone| {
                            let mut frame =
                                super::compute_frame(&rt.effect, zone, &rt.custom_colors, t);
                            for c in frame.iter_mut() {
                                *c = c.scale(brightness);
                            }
                            // Identify pulse overrides the zone's frame (full
                            // brightness, ignoring the configured effect).
                            if let Some((zid, color)) = identify_zone {
                                if zid == zone.id {
                                    frame.iter_mut().for_each(|c| *c = color);
                                }
                            }
                            (zone.id.clone(), frame)
                        })
                        .collect()
                };

                // --- Phase 2: write (needs &mut device). ---
                if onboard_hw {
                    last_frames.remove(&device_id);
                    if let EffectConfig::Onboard(e) = &rt.effect {
                        let key = (*e, rt.brightness);
                        if hw_enabled && last_onboard.get(&device_id) != Some(&key) {
                            match device.set_onboard_effect(e, rt.brightness) {
                                Ok(()) => {
                                    last_onboard.insert(device_id.clone(), key);
                                }
                                Err(err) => {
                                    crate::klog!("set_onboard_effect {device_id}: {err}")
                                }
                            }
                        }
                    }
                    if emit_this_tick {
                        payloads.push(FramePayload {
                            device_id,
                            zones: frames.into_iter().collect(),
                        });
                    }
                    continue;
                }

                // Host path: per-key streaming (Static/Custom on GMMK; all
                // effects on mock devices, which host-simulate Onboard too).
                last_onboard.remove(&device_id);
                let device_last = last_frames.entry(device_id.clone()).or_default();
                let mut any_changed = false;

                for (zone_id, frame) in &frames {
                    let unchanged = device_last.get(zone_id) == Some(frame);
                    if !unchanged && hw_enabled {
                        // Cache only on success — a failed write must be retried
                        // next tick, not remembered as if it reached the hardware.
                        match device.set_zone_leds(zone_id, frame) {
                            Ok(()) => {
                                device_last.insert(zone_id.clone(), frame.clone());
                                any_changed = true;
                            }
                            Err(e) => {
                                crate::klog!("set_zone_leds {device_id}/{zone_id}: {e}")
                            }
                        }
                    }
                }

                // Static/Custom effects idle at zero hardware writes (the MSI
                // writer's own heartbeat keeps the board honest).
                if any_changed {
                    if let Err(e) = device.apply() {
                        crate::klog!("apply {device_id}: {e}");
                        // Forget this device's cached frames so the whole frame
                        // is re-staged and re-applied next tick.
                        last_frames.remove(&device_id);
                    }
                }
                if emit_this_tick {
                    payloads.push(FramePayload {
                        device_id,
                        zones: frames.into_iter().collect(),
                    });
                }
            }
        }

        for payload in payloads {
            let _ = app.emit("device-frame", payload);
        }
    }
}
