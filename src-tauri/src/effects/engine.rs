//! ~30 fps tick loop: computes frames for every device zone, pushes them
//! through the RgbDevice trait, and mirrors them to the frontend at ~15 fps
//! via `device-frame` events.

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

/// White brightness pulse for the identify animation. `elapsed` is seconds
/// since the pulse started; pulses at ~1.5 Hz, dark -> bright -> dark.
fn identify_color(elapsed: f32) -> Color {
    let s = 0.5 - 0.5 * (elapsed * std::f32::consts::TAU * 1.5).cos();
    let v = (s * 255.0) as u8;
    Color { r: v, g: v, b: v }
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

    // Temporary instrumentation for the "stuck then jump" fan-RGB report. Counts,
    // per 5s window: ticks actually fired (vs the ideal ~150), zone frames written
    // to hardware, and zone frames skipped as unchanged (8-bit dedup). Separates
    // "engine tick starved" (low ticks) from "frame dedup" (high skips, low writes).
    let mut dbg_window = Instant::now();
    let mut dbg_ticks: u64 = 0;
    let mut dbg_writes: u64 = 0;
    let mut dbg_skips: u64 = 0;

    loop {
        interval.tick().await;
        tick += 1;
        dbg_ticks += 1;
        let t = start.elapsed().as_secs_f32();
        let emit_this_tick = tick.is_multiple_of(2);
        let mut payloads: Vec<FramePayload> = Vec::new();
        let device_generation = state.device_generation.load(Ordering::Relaxed);
        let hw_enabled = state.conflicts_cleared.load(Ordering::Relaxed);
        if device_generation != seen_device_generation {
            last_frames.clear();
            last_onboard.clear();
            seen_device_generation = device_generation;
        }

        {
            let runtime = state.runtime.lock();
            let mut identify = state.identify.lock();
            // Drop expired pulses so they stop overriding their zone.
            identify.retain(|_, (_, started)| started.elapsed().as_secs_f32() < IDENTIFY_SECS);
            let mut manager = state.manager.lock();

            for device in manager.iter_mut() {
                let info = device.info();
                let Some(rt) = runtime.get(&info.id) else {
                    continue;
                };
                let brightness = rt.brightness as f32 / 100.0;
                let mut payload_zones: HashMap<String, Vec<Color>> = HashMap::new();

                // Active identify pulse for this device (zone id + its color now).
                let identify_zone: Option<(&str, Color)> =
                    identify.get(&info.id).map(|(z, started)| {
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

                if onboard_hw {
                    last_frames.remove(&info.id);
                    if let EffectConfig::Onboard(e) = &rt.effect {
                        let key = (*e, rt.brightness);
                        if hw_enabled && last_onboard.get(&info.id) != Some(&key) {
                            match device.set_onboard_effect(e, rt.brightness) {
                                Ok(()) => {
                                    last_onboard.insert(info.id.clone(), key);
                                }
                                Err(err) => {
                                    eprintln!("set_onboard_effect {}: {err}", info.id)
                                }
                            }
                        }
                    }
                    if emit_this_tick {
                        for zone in &info.zones {
                            let mut frame =
                                super::compute_frame(&rt.effect, zone, &rt.custom_colors, t);
                            for c in frame.iter_mut() {
                                *c = c.scale(brightness);
                            }
                            payload_zones.insert(zone.id.clone(), frame);
                        }
                        payloads.push(FramePayload {
                            device_id: info.id,
                            zones: payload_zones,
                        });
                    }
                    continue;
                }

                // Host path: per-key streaming (Static/Custom on GMMK; all
                // effects on mock devices, which host-simulate Onboard too).
                last_onboard.remove(&info.id);
                let device_last = last_frames.entry(info.id.clone()).or_default();
                let mut any_changed = false;

                for zone in &info.zones {
                    let mut frame = super::compute_frame(&rt.effect, zone, &rt.custom_colors, t);
                    for c in frame.iter_mut() {
                        *c = c.scale(brightness);
                    }

                    // Identify pulse overrides the zone's frame (full brightness,
                    // ignoring the configured effect) so it's unmistakable.
                    if let Some((zid, color)) = identify_zone {
                        if zid == zone.id {
                            frame.iter_mut().for_each(|c| *c = color);
                        }
                    }

                    let unchanged = device_last.get(&zone.id) == Some(&frame);
                    if !unchanged && hw_enabled {
                        if let Err(e) = device.set_zone_leds(&zone.id, &frame) {
                            eprintln!("set_zone_leds {}/{}: {e}", info.id, zone.id);
                        }
                        device_last.insert(zone.id.clone(), frame.clone());
                        any_changed = true;
                        dbg_writes += 1;
                    } else if hw_enabled {
                        dbg_skips += 1;
                    }
                    if emit_this_tick {
                        payload_zones.insert(zone.id.clone(), frame);
                    }
                }

                // Static/Custom effects idle at zero hardware writes.
                if any_changed {
                    if let Err(e) = device.apply() {
                        eprintln!("apply {}: {e}", info.id);
                    }
                }
                if emit_this_tick {
                    payloads.push(FramePayload {
                        device_id: info.id,
                        zones: payload_zones,
                    });
                }
            }
        }

        for payload in payloads {
            let _ = app.emit("device-frame", payload);
        }

        if dbg_window.elapsed() >= Duration::from_secs(5) {
            let secs = dbg_window.elapsed().as_secs_f32();
            eprintln!(
                "engine health [{:.1}s]: {} ticks ({:.1}/s of ~30 ideal), \
                 {} zone writes, {} zone skips (unchanged/dedup)",
                secs,
                dbg_ticks,
                dbg_ticks as f32 / secs,
                dbg_writes,
                dbg_skips,
            );
            dbg_window = Instant::now();
            dbg_ticks = 0;
            dbg_writes = 0;
            dbg_skips = 0;
        }
    }
}
