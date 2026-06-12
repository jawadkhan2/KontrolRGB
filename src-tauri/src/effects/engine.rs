//! ~30 fps tick loop: computes frames for every device zone, pushes them
//! through the RgbDevice trait, and mirrors them to the frontend at ~15 fps
//! via `device-frame` events.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::device::types::Color;
use crate::state::AppState;

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct FramePayload {
    device_id: String,
    zones: HashMap<String, Vec<Color>>,
}

pub async fn run(app: AppHandle, state: Arc<AppState>) {
    let start = Instant::now();
    let mut interval = tokio::time::interval(Duration::from_millis(33));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    // device_id -> zone_id -> last frame, to skip redundant hardware writes.
    let mut last_frames: HashMap<String, HashMap<String, Vec<Color>>> = HashMap::new();
    let mut tick: u64 = 0;

    loop {
        interval.tick().await;
        tick += 1;
        let t = start.elapsed().as_secs_f32();
        let emit_this_tick = tick % 2 == 0;
        let mut payloads: Vec<FramePayload> = Vec::new();

        {
            let runtime = state.runtime.lock();
            let mut manager = state.manager.lock();

            for device in manager.iter_mut() {
                let info = device.info();
                let Some(rt) = runtime.get(&info.id) else {
                    continue;
                };
                let brightness = rt.brightness as f32 / 100.0;
                let device_last = last_frames.entry(info.id.clone()).or_default();
                let mut payload_zones: HashMap<String, Vec<Color>> = HashMap::new();
                let mut any_changed = false;

                for zone in &info.zones {
                    let mut frame =
                        super::compute_frame(&rt.effect, zone, &rt.custom_colors, t);
                    for c in frame.iter_mut() {
                        *c = c.scale(brightness);
                    }

                    let unchanged = device_last.get(&zone.id) == Some(&frame);
                    if !unchanged {
                        if let Err(e) = device.set_zone_leds(&zone.id, &frame) {
                            eprintln!("set_zone_leds {}/{}: {e}", info.id, zone.id);
                        }
                        device_last.insert(zone.id.clone(), frame.clone());
                        any_changed = true;
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
    }
}
