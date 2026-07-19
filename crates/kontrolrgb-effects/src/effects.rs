
use std::collections::HashMap;
use std::f32::consts::TAU;

use crate::types::{Color, EffectConfig, OnboardEffect, OnboardMode, ZoneInfo};

/// Compute the colors for one zone at time `t` (seconds since engine start).
/// Brightness is applied by the caller.
pub fn compute_frame(
    effect: &EffectConfig,
    zone: &ZoneInfo,
    custom_colors: &HashMap<String, Vec<Color>>,
    t: f32,
) -> Vec<Color> {
    let n = zone.led_count as usize;

    // Zones with only a handful of LEDs and no key layout (e.g. the GPU logo,
    // which is one uniform light) can't render spatial motion — a comet or ring
    // just samples one dead point and looks broken. Route those spatial effects
    // to a whole-zone color that animates over TIME instead, so the light still
    // pulses/flashes with the effect's character. Effects that already read fine
    // as a single lamp (Fire, Gradient, Twinkle, Breathing, cycles) fall through.
    if zone.keys.is_none() && zone.led_count <= 4 {
        if let Some(frame) = nonspatial_frame(effect, n, t) {
            return frame;
        }
    }

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
        EffectConfig::Meteor {
            color,
            speed,
            reverse,
        } => {
            let dir = if *reverse { -1.0 } else { 1.0 };
            let head = (t * speed * 0.35 * dir).rem_euclid(1.0);
            led_positions(zone)
                .into_iter()
                .map(|pos| {
                    let mut d = pos - head;
                    if d > 0.0 {
                        d -= 1.0; // only trail behind the head, wrapping
                    }
                    let level = (1.0 + d / 0.35).max(0.0); // 1 at head, fades behind
                    color.scale(level * level)
                })
                .collect()
        }
        EffectConfig::Fire { speed } => key_positions(zone)
            .into_iter()
            .enumerate()
            .map(|(i, (_, y))| {
                let flick = 0.6
                    + 0.4
                        * (t * speed * 9.0 + i as f32 * 1.7).sin()
                        * hash01(i as u64 ^ ((t * speed * 12.0) as u64));
                let heat = ((1.0 - y) * flick).clamp(0.0, 1.0);
                let hue = 25.0 + heat * 25.0; // red -> orange -> yellow
                let val = 0.25 + 0.75 * heat;
                Color::from_hsv(hue, 1.0 - heat * 0.4, val)
            })
            .collect(),
        EffectConfig::Twinkle { color, speed } => {
            const SLOT: f32 = 0.18;
            const FADE: f32 = 0.9;
            let ts = t * speed;
            (0..n)
                .map(|i| {
                    let mut best = 0.0_f32;
                    let last = (ts / SLOT).floor() as i64;
                    let steps = (FADE / SLOT).ceil() as i64;
                    for s in 0..=steps {
                        let bucket = last - s;
                        if hash01((bucket as u64).wrapping_mul(0x2545_F491) ^ i as u64) > 0.88 {
                            let age = ts - bucket as f32 * SLOT;
                            best = best.max((1.0 - age / FADE).max(0.0));
                        }
                    }
                    let base = color.scale(0.12);
                    let spark = Color { r: 255, g: 255, b: 255 }.scale(best);
                    Color {
                        r: base.r.max(spark.r),
                        g: base.g.max(spark.g),
                        b: base.b.max(spark.b),
                    }
                })
                .collect()
        }
        EffectConfig::Gradient { color, speed } => {
            // Blend `color` with its hue complement across the zone.
            let (h, s, v) = to_hsv(*color);
            let other = Color::from_hsv(h + 150.0, s, v);
            led_positions(zone)
                .into_iter()
                .map(|pos| {
                    let f = 0.5 + 0.5 * (pos * TAU - t * speed * 0.4).sin();
                    lerp_color(*color, other, f)
                })
                .collect()
        }
        EffectConfig::Plasma { speed } => key_positions(zone)
            .into_iter()
            .map(|(x, y)| {
                let ts = t * speed;
                let v = (x * 6.0 + ts).sin() + (y * 6.0 + ts * 1.3).sin() + ((x + y) * 5.0 + ts * 0.7).sin();
                Color::from_hsv(v * 60.0 + ts * 40.0, 1.0, 1.0)
            })
            .collect(),
        EffectConfig::Larson { color, speed } => {
            let head = 0.5 - 0.5 * (t * speed * 1.4).cos(); // 0..1..0 bounce
            led_positions(zone)
                .into_iter()
                .map(|pos| {
                    let level = (1.0 - (pos - head).abs() / 0.18).max(0.0);
                    color.scale(level * level)
                })
                .collect()
        }
        EffectConfig::TheaterChase { color, speed } => {
            let phase = ((t * speed * 6.0) as i64).rem_euclid(3);
            (0..n)
                .map(|i| {
                    if (i as i64 % 3) == phase {
                        *color
                    } else {
                        color.scale(0.04)
                    }
                })
                .collect()
        }
        EffectConfig::Ripple { speed } => key_positions(zone)
            .into_iter()
            .map(|(x, y)| {
                let (dx, dy) = (x - 0.5, y - 0.5);
                let r = (dx * dx + dy * dy).sqrt();
                let wave = (r * 22.0 - t * speed * 4.0).sin();
                let level = wave.max(0.0) * (1.0 - r).max(0.0);
                Color::from_hsv(200.0 + r * 120.0, 1.0, level)
            })
            .collect(),
        EffectConfig::Aurora { speed } => {
            let ts = t * speed;
            spatial_positions(zone)
                .into_iter()
                .map(|(x, y)| {
                    // Two curtains drifting at different rates. Integer spatial
                    // frequencies (x·τ·k) so the pattern closes seamlessly
                    // around a fan ring, where x wraps 0→1.
                    let c1 = (x * TAU + ts * 0.31).sin();
                    let c2 = (x * TAU * 2.0 - ts * 0.17).sin();
                    let hue = 155.0 + 50.0 * c1 + 30.0 * c2; // teal/green..violet
                    let shimmer = 0.5 + 0.5 * (x * TAU * 3.0 + ts * 0.9).sin();
                    // Curtains hang from the top of the zone / ring.
                    let val = (0.35 + 0.65 * shimmer) * (1.0 - 0.45 * y);
                    Color::from_hsv(hue, 0.9, val.clamp(0.0, 1.0))
                })
                .collect()
        }
        EffectConfig::Vortex {
            color,
            speed,
            reverse,
            arms,
        } => {
            let dir = if *reverse { -1.0 } else { 1.0 };
            let phase = dir * t * speed * 1.8;
            let arms = (*arms).clamp(1, 4) as f32;
            // Angle around the zone: physical ring angle on strips, angle
            // about the layout center on key matrices.
            let angles: Vec<f32> = if zone.keys.is_some() {
                key_positions(zone)
                    .into_iter()
                    .map(|(x, y)| (y - 0.5).atan2(x - 0.5))
                    .collect()
            } else {
                led_positions(zone).into_iter().map(|p| p * TAU).collect()
            };
            angles
                .into_iter()
                .map(|a| {
                    let lobe = 0.5 + 0.5 * (arms * a - phase).cos();
                    let level = lobe * lobe * lobe * lobe; // sharpen into arcs
                    color.scale(level)
                })
                .collect()
        }
        EffectConfig::Heartbeat { color, speed } => {
            // One beat per second at speed 1: strong systolic thump, softer
            // diastolic echo, faint resting glow between beats.
            let ph = (t * speed).rem_euclid(1.0);
            let g = |c: f32, w: f32| (-((ph - c) * (ph - c)) / (w * w)).exp();
            let level = (g(0.12, 0.045) + 0.55 * g(0.32, 0.06) + 0.05).min(1.0);
            vec![color.scale(level); n]
        }
        EffectConfig::Thunderstorm { speed } => {
            let ts = t * speed;
            let (flash, bolt_x) = lightning(ts);
            // Near-dark storm base with a slow cloud roll.
            let breathe = 0.8 + 0.2 * (ts * 0.6).sin();
            let base = Color { r: 8, g: 16, b: 42 }.scale(breathe);
            if zone.keys.is_some() {
                // Bolt lands on a column; the rest of the board sees sky glow.
                key_positions(zone)
                    .into_iter()
                    .map(|(x, _)| {
                        let d = (x - bolt_x) / 0.10;
                        let bolt = (-d * d).exp();
                        let f = flash * (0.25 + 0.75 * bolt);
                        add_sat(base, Color { r: 200, g: 215, b: 255 }.scale(f))
                    })
                    .collect()
            } else {
                // Strips/rings (and the GPU lamp): the whole fixture flashes,
                // like a fan lit by a strike outside the window.
                vec![add_sat(base, Color { r: 200, g: 215, b: 255 }.scale(flash)); n]
            }
        }
        EffectConfig::Sunset { speed } => {
            let ts = t * speed;
            // The sun slowly sinks and rises again; hue shimmers gently along x.
            let drift = 0.10 * (ts * 0.25).sin();
            spatial_positions(zone)
                .into_iter()
                .map(|(x, y)| {
                    let g = (y + drift).clamp(0.0, 1.0); // 0 = high sky, 1 = horizon
                    let hue = 275.0 + g * 115.0 + 6.0 * (x * TAU + ts * 0.4).sin();
                    let val = 0.45 + 0.55 * g;
                    Color::from_hsv(hue, 0.95 - 0.2 * g, val)
                })
                .collect()
        }
        // The firmware animates onboard effects on the device itself. We can't
        // phase-sync to it, but the user wants the on-screen preview to MOVE so
        // it reads like the real board. `onboard_frame` is a host approximation
        // matching each firmware mode's character (motion/direction/speed/color);
        // timings are calibrated to measured GMMK behavior (see period helpers).
        EffectConfig::Onboard(e) => onboard_frame(e, zone, t),
    }
}

/// Whole-zone animated color for the spatial motion effects, used on zones that
/// have no useful spatial resolution (few LEDs, no key layout — e.g. the GPU
/// logo). Returns `None` for effects that already render acceptably per-LED on a
/// single lamp (Fire/Gradient/Twinkle/Static/Breathing/cycles), so those keep
/// their normal path. Each arm samples the effect's motion at the zone center so
/// the single light still flashes/pulses in the effect's character.
fn nonspatial_frame(effect: &EffectConfig, n: usize, t: f32) -> Option<Vec<Color>> {
    let color = match effect {
        // Comet: one bright flash per sweep as the head passes, then fades.
        EffectConfig::Meteor {
            color,
            speed,
            reverse,
        } => {
            let dir = if *reverse { -1.0 } else { 1.0 };
            let head = (t * speed * 0.35 * dir).rem_euclid(1.0);
            let level = (1.0 - head / 0.35).max(0.0);
            color.scale(level * level)
        }
        // Bounce: the dot crosses center twice per cycle -> double pulse.
        EffectConfig::Larson { color, speed } => {
            let head = 0.5 - 0.5 * (t * speed * 1.4).cos();
            let level = (1.0 - (0.5 - head).abs() / 0.18).max(0.0);
            color.scale(level * level)
        }
        // March collapses to a blink (~1/3 duty) on a single light.
        EffectConfig::TheaterChase { color, speed } => {
            let phase = ((t * speed * 6.0) as i64).rem_euclid(3);
            if phase == 0 {
                *color
            } else {
                color.scale(0.04)
            }
        }
        // Rings collapse to a breathing pulse sampled at the center.
        EffectConfig::Ripple { speed } => {
            let level = (t * speed * 4.0).sin().max(0.0);
            Color::from_hsv(210.0, 1.0, level)
        }
        // Plasma collapses to a smooth full-spectrum hue drift.
        EffectConfig::Plasma { speed } => {
            let ts = t * speed;
            let v = ts.sin() + (ts * 1.3).sin() + (ts * 0.7).sin();
            Color::from_hsv(v * 60.0 + ts * 40.0, 1.0, 1.0)
        }
        // Sunset collapses to the palette swept over time: the lamp slowly
        // travels sky -> horizon and back instead of pinning to one stop.
        EffectConfig::Sunset { speed } => {
            let g = 0.5 - 0.5 * (t * speed * 0.25).cos();
            Color::from_hsv(275.0 + g * 115.0, 0.95 - 0.2 * g, 0.45 + 0.55 * g)
        }
        _ => return None,
    };
    Some(vec![color; n])
}

/// Deterministic lightning state at storm-time `ts`: (flash level 0..1, bolt
/// x-center 0..1). Time is bucketed into ~1.4s slots; a slot may fire one
/// strike (with a per-slot jitter and column) whose flash flickers as it
/// decays, so consecutive ticks agree on what's mid-flash.
fn lightning(ts: f32) -> (f32, f32) {
    const SLOT: f32 = 1.4;
    const DUR: f32 = 0.5;
    let mut flash = 0.0_f32;
    let mut bolt_x = 0.5_f32;
    let last = (ts / SLOT).floor() as i64;
    for s in 0..=1 {
        let bucket = last - s;
        let b = bucket as u64;
        if hash01(b.wrapping_mul(0x1234_5677)) > 0.45 {
            let jitter = hash01(b.wrapping_mul(0x9E37_79B9)) * (SLOT - DUR);
            let age = ts - (bucket as f32 * SLOT + jitter);
            if (0.0..=DUR).contains(&age) {
                let decay = 1.0 - age / DUR;
                // 14 Hz flicker so the strike stutters like a real strobe.
                let f = decay * decay * (0.55 + 0.45 * (age * 88.0).sin().abs());
                if f > flash {
                    flash = f;
                    bolt_x = hash01(b.wrapping_mul(0xDEAD_BEE7));
                }
            }
        }
    }
    (flash, bolt_x)
}

/// Component-wise saturating add (used to layer a flash over a base color).
fn add_sat(a: Color, b: Color) -> Color {
    Color {
        r: a.r.saturating_add(b.r),
        g: a.g.saturating_add(b.g),
        b: a.b.saturating_add(b.b),
    }
}

/// Cheap deterministic hash of `n` into [0, 1). Used by stateless effects
/// (Twinkle, Fire) so a given LED/time slot fires consistently across ticks.
fn hash01(n: u64) -> f32 {
    let mut x = n.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    x ^= x >> 27;
    x = x.wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^= x >> 31;
    (x >> 40) as f32 / (1u64 << 24) as f32
}

/// RGB -> (hue degrees, saturation, value), the inverse of `Color::from_hsv`.
fn to_hsv(c: Color) -> (f32, f32, f32) {
    let (r, g, b) = (c.r as f32 / 255.0, c.g as f32 / 255.0, c.b as f32 / 255.0);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let d = max - min;
    let h = if d == 0.0 {
        0.0
    } else if max == r {
        60.0 * (((g - b) / d).rem_euclid(6.0))
    } else if max == g {
        60.0 * ((b - r) / d + 2.0)
    } else {
        60.0 * ((r - g) / d + 4.0)
    };
    let s = if max == 0.0 { 0.0 } else { d / max };
    (h, s, max)
}

/// Component-wise linear blend `a`->`b` at fraction `f` (0..1).
fn lerp_color(a: Color, b: Color, f: f32) -> Color {
    let f = f.clamp(0.0, 1.0);
    let mix = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * f) as u8;
    Color {
        r: mix(a.r, b.r),
        g: mix(a.g, b.g),
        b: mix(a.b, b.b),
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

/// (x, y) per LED like `key_positions`, but strips derive y from the LED's
/// physical height on a fan ring: LED 0 sits at the top of the ring (see the
/// sim's FanChain: slot angle = pos·τ + π/2), so y = (1 − cos(pos·τ))/2 with
/// 0 = top — matching the key-matrix convention. Lets vertical effects
/// (Aurora, Sunset) track real fan orientation instead of a flat 0.5.
fn spatial_positions(zone: &ZoneInfo) -> Vec<(f32, f32)> {
    if zone.keys.is_some() {
        key_positions(zone)
    } else {
        led_positions(zone)
            .into_iter()
            .map(|p| (p, 0.5 - 0.5 * (p * TAU).cos()))
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
