pub mod engine;

use std::collections::HashMap;
use std::f32::consts::TAU;

use crate::device::types::{Color, EffectConfig, ZoneInfo};

/// Compute the colors for one zone at time `t` (seconds since engine start).
/// Brightness is applied by the caller.
pub fn compute_frame(
    effect: &EffectConfig,
    zone: &ZoneInfo,
    custom_colors: &HashMap<String, Vec<Color>>,
    t: f32,
) -> Vec<Color> {
    let n = zone.led_count as usize;
    match effect {
        EffectConfig::Static { color } => vec![*color; n],
        EffectConfig::Breathing { color, speed } => {
            let level = 0.5 - 0.5 * (t * speed * TAU * 0.5).cos();
            vec![color.scale(level); n]
        }
        EffectConfig::RainbowWave { speed, reverse } => {
            let base = t * speed * 60.0;
            (0..n)
                .map(|i| {
                    let mut pos = led_position(zone, i);
                    if *reverse {
                        pos = 1.0 - pos;
                    }
                    Color::from_hsv(base + pos * 360.0, 1.0, 1.0)
                })
                .collect()
        }
        EffectConfig::ColorCycle { speed } => {
            vec![Color::from_hsv(t * speed * 60.0, 1.0, 1.0); n]
        }
        EffectConfig::Custom => {
            let mut colors = custom_colors.get(&zone.id).cloned().unwrap_or_default();
            colors.resize(n, Color::BLACK);
            colors
        }
    }
}

/// Normalized 0..1 position of an LED within its zone. For keyboard zones the
/// physical key x-coordinate is used so waves sweep left-to-right across the
/// board instead of following LED wire order.
fn led_position(zone: &ZoneInfo, index: usize) -> f32 {
    if let Some(keys) = &zone.keys {
        let max_x = keys
            .iter()
            .map(|k| k.x + k.w)
            .fold(1.0_f32, f32::max);
        keys.iter()
            .find(|k| k.led_index == index as u32)
            .map(|k| (k.x + k.w / 2.0) / max_x)
            .unwrap_or(0.0)
    } else if zone.led_count <= 1 {
        0.0
    } else {
        index as f32 / (zone.led_count - 1) as f32
    }
}
