pub mod engine;

use std::collections::HashMap;
use std::f32::consts::TAU;

use crate::device::types::{Color, EffectConfig, OnboardEffect, OnboardMode, ZoneInfo};

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
            led_positions(zone)
                .into_iter()
                .map(|mut pos| {
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
            let mut colors = vec![Color::BLACK; n];
            if let Some(saved) = custom_colors.get(&zone.id) {
                let count = n.min(saved.len());
                colors[..count].copy_from_slice(&saved[..count]);
            }
            colors
        }
        // The firmware animates onboard effects on the device itself. We can't
        // phase-sync to it, but the user wants the on-screen preview to MOVE so
        // it reads like the real board. `onboard_frame` is a host approximation
        // matching each firmware mode's character (motion/direction/speed/color);
        // timings are calibrated to measured GMMK behavior (see period helpers).
        EffectConfig::Onboard(e) => onboard_frame(e, zone, t),
    }
}

/// Linear interpolation between the speed-0 and speed-4 values for a firmware
/// speed level `s` (0..=4, where higher = faster).
fn lerp_speed(at_0: f32, at_4: f32, s: u8) -> f32 {
    at_0 + (at_4 - at_0) * (s.min(4) as f32 / 4.0)
}

/// Animated host approximation of a firmware onboard effect at time `t`.
/// Output is indexed by led_index (0..led_count), like every other effect.
fn onboard_frame(e: &OnboardEffect, zone: &ZoneInfo, t: f32) -> Vec<Color> {
    let n = zone.led_count as usize;
    match e.mode {
        // Static solid color; the firmware preset doesn't animate.
        OnboardMode::Fixed => vec![e.color; n],

        // Whole board pulses dark -> bright -> dark. Full pulse 3.0s @0 -> 1.2s @4.
        OnboardMode::Breathing => {
            let period = lerp_speed(3.0, 1.2, e.speed).max(0.1);
            let level = 0.5 - 0.5 * (t * TAU / period).cos();
            let base = if e.rainbow {
                // Hue drifts across pulses so a rainbow breathe shows color.
                Color::from_hsv(t * 60.0, 1.0, 1.0)
            } else {
                e.color
            };
            vec![base.scale(level); n]
        }

        // Horizontal sweep. Spectrum == the rainbow form of Wave (matches the
        // firmware mapping in gmmk/controller.rs). Full cross 4.5s @0 -> 1.1s @4.
        OnboardMode::Wave | OnboardMode::Spectrum => {
            let period = lerp_speed(4.5, 1.1, e.speed).max(0.1);
            // Both sub-paths use the SAME (x - phase) convention so rainbow and
            // single-color waves scroll the same way; default is left -> right.
            let dir = if e.reverse { -1.0 } else { 1.0 };
            let phase = dir * t / period;
            let rainbow = e.rainbow || e.mode == OnboardMode::Spectrum;
            led_positions(zone)
                .into_iter()
                .map(|x| {
                    if rainbow {
                        Color::from_hsv((x - phase) * 360.0, 1.0, 1.0)
                    } else {
                        let level = 0.5 + 0.5 * ((x - phase) * TAU).cos();
                        e.color.scale(level)
                    }
                })
                .collect()
        }

        // Rotating band/rainbow around the board center. Full spin 4.0s @0 -> 1.0s @4.
        OnboardMode::Swirl => {
            let period = lerp_speed(4.0, 1.0, e.speed).max(0.1);
            let dir = if e.reverse { -1.0 } else { 1.0 };
            let phase = dir * t * TAU / period;
            key_positions(zone)
                .into_iter()
                .map(|(x, y)| {
                    let angle = (y - 0.5).atan2(x - 0.5);
                    if e.rainbow {
                        Color::from_hsv((angle + phase).to_degrees(), 1.0, 1.0)
                    } else {
                        let level = 0.5 + 0.5 * (angle - phase).cos();
                        e.color.scale(level)
                    }
                })
                .collect()
        }

        // No real keypresses in-app, so simulate: board idles dark, random keys
        // flash and fade over a constant ~1s. Stateless/deterministic in `t`.
        OnboardMode::Reactive => reactive_frame(e, n, t),
    }
}

/// Simulated reactive ripples: deterministic, stateless function of time so a
/// fired LED fades smoothly across ticks instead of flickering. Buckets time
/// into ~0.12s slots; each slot fires a couple of pseudo-random LEDs that then
/// fade out over `FADE` seconds.
fn reactive_frame(e: &OnboardEffect, n: usize, t: f32) -> Vec<Color> {
    const FADE: f32 = 1.0;
    const SLOT: f32 = 0.12;
    if n == 0 {
        return Vec::new();
    }
    let mut levels = vec![0.0_f32; n];
    let first = ((t - FADE) / SLOT).floor() as i64;
    let last = (t / SLOT).floor() as i64;
    for bucket in first..=last {
        // Cheap integer hash of the bucket -> two LED indices that "fire".
        let h = (bucket as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        for k in 0..2u64 {
            let idx = ((h ^ k.wrapping_mul(0xD1B5_4A32_D192_ED03)) as usize) % n;
            let age = t - bucket as f32 * SLOT;
            if (0.0..=FADE).contains(&age) {
                let level = 1.0 - age / FADE;
                if level > levels[idx] {
                    levels[idx] = level;
                }
            }
        }
    }
    levels
        .iter()
        .enumerate()
        .map(|(i, &level)| {
            let base = if e.rainbow {
                // Stable per-key hue so each key keeps its own color as it fades.
                Color::from_hsv((i as f32 * 137.508).rem_euclid(360.0), 1.0, 1.0)
            } else {
                e.color
            };
            base.scale(level)
        })
        .collect()
}

fn led_positions(zone: &ZoneInfo) -> Vec<f32> {
    let n = zone.led_count as usize;
    if let Some(keys) = &zone.keys {
        let max_x = keys.iter().map(|k| k.x + k.w).fold(1.0_f32, f32::max);
        let mut positions = vec![0.0; n];
        for key in keys {
            if let Some(slot) = positions.get_mut(key.led_index as usize) {
                *slot = (key.x + key.w / 2.0) / max_x;
            }
        }
        positions
    } else if zone.led_count <= 1 {
        vec![0.0; n]
    } else {
        (0..n)
            .map(|index| index as f32 / (zone.led_count - 1) as f32)
            .collect()
    }
}

fn key_positions(zone: &ZoneInfo) -> Vec<(f32, f32)> {
    let n = zone.led_count as usize;
    if let Some(keys) = &zone.keys {
        let max_x = keys.iter().map(|k| k.x + k.w).fold(1.0_f32, f32::max);
        let max_y = keys.iter().map(|k| k.y + k.h).fold(1.0_f32, f32::max);
        let mut positions = vec![(0.5, 0.5); n];
        for key in keys {
            if let Some(slot) = positions.get_mut(key.led_index as usize) {
                *slot = ((key.x + key.w / 2.0) / max_x, (key.y + key.h / 2.0) / max_y);
            }
        }
        positions
    } else {
        led_positions(zone).into_iter().map(|x| (x, 0.5)).collect()
    }
}
