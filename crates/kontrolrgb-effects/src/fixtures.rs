//! Device fixtures for the kontrolrgb-sim web preview: the same zones the real
//! app exposes for this rig, minus hardware probing. LED counts on the JARGB
//! headers are defaults — the zones are resizable and the sim UI can adjust
//! them (fans per header × LEDs per fan) without touching this file.

use crate::layouts::gmmk_tkl;
use crate::types::{DeviceInfo, DeviceType, EffectConfig, ZoneInfo};

/// LEDs on one AsiaHorse AMICI ring (sim default; resizable in the UI).
pub const LEDS_PER_FAN: u32 = 12;

/// Matches the real MSI 761-protocol per-header cap closely enough for preview.
const MAX_HEADER_LEDS: u32 = 240;

fn fan_header(id: &str, name: &str, fans: u32) -> ZoneInfo {
    ZoneInfo {
        id: id.to_string(),
        name: name.to_string(),
        led_count: fans * LEDS_PER_FAN,
        resizable: true,
        min_leds: 1,
        max_leds: MAX_HEADER_LEDS,
        keys: None,
    }
}

/// The simulated rig: GMMK TKL keyboard, MSI Z890 (3 JARGB fan headers),
/// Gigabyte RTX 5080 logo. Ids intentionally match the real app's device ids
/// so exported presets line up.
pub fn sim_devices() -> Vec<DeviceInfo> {
    let keys = gmmk_tkl::key_infos();
    let led_count = keys.len() as u32;

    vec![
        DeviceInfo {
            id: "gmmk-0c45-652f".to_string(),
            name: "Glorious GMMK (TKL)".to_string(),
            device_type: DeviceType::Keyboard,
            zones: vec![ZoneInfo {
                id: "keys".to_string(),
                name: "Keys".to_string(),
                led_count,
                resizable: false,
                min_leds: led_count,
                max_leds: led_count,
                keys: Some(keys),
            }],
            // Matches the real probe (device/gmmk/mod.rs): the MCU ingests
            // ~45 HID reports/sec, so the host can only paint static frames;
            // everything animated must be a firmware onboard mode. The sim
            // front-end degrades unsupported kinds exactly like the app does.
            supported_effects: vec![
                "static".to_string(),
                "custom".to_string(),
                "onboard".to_string(),
            ],
        },
        DeviceInfo {
            id: "msi-0db0-0076".to_string(),
            name: "MSI MAG Z890 Tomahawk".to_string(),
            device_type: DeviceType::Motherboard,
            zones: vec![
                // Real rig: 3 side-panel fans (daisy-chained, act as one fan),
                // 3 top fans, 1 rear exhaust. The sim UI resizes these.
                fan_header("jargb_v2_1", "JARGB_V2 1 (side fans)", 3),
                fan_header("jargb_v2_2", "JARGB_V2 2 (top fans)", 3),
                fan_header("jargb_v2_3", "JARGB_V2 3 (rear fan)", 1),
            ],
            supported_effects: EffectConfig::ALL_KINDS.map(String::from).to_vec(),
        },
        DeviceInfo {
            id: "gigabyte-gpu".to_string(),
            name: "Gigabyte RTX 5080 Gaming OC".to_string(),
            device_type: DeviceType::Gpu,
            zones: vec![ZoneInfo {
                id: "logo".to_string(),
                name: "Logo".to_string(),
                led_count: 1,
                resizable: false,
                min_leds: 1,
                max_leds: 1,
                keys: None,
            }],
            supported_effects: EffectConfig::ALL_KINDS.map(String::from).to_vec(),
        },
    ]
}
