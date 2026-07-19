use serde::{Deserialize, Serialize};

pub type DeviceId = String;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Color {
    pub const BLACK: Color = Color { r: 0, g: 0, b: 0 };

    pub fn scale(self, factor: f32) -> Color {
        let f = factor.clamp(0.0, 1.0);
        Color {
            r: (self.r as f32 * f) as u8,
            g: (self.g as f32 * f) as u8,
            b: (self.b as f32 * f) as u8,
        }
    }

    /// h in degrees [0, 360), s and v in [0, 1].
    pub fn from_hsv(h: f32, s: f32, v: f32) -> Color {
        let h = h.rem_euclid(360.0);
        let c = v * s;
        let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
        let m = v - c;
        let (r, g, b) = match h {
            h if h < 60.0 => (c, x, 0.0),
            h if h < 120.0 => (x, c, 0.0),
            h if h < 180.0 => (0.0, c, x),
            h if h < 240.0 => (0.0, x, c),
            h if h < 300.0 => (x, 0.0, c),
            _ => (c, 0.0, x),
        };
        Color {
            r: ((r + m) * 255.0) as u8,
            g: ((g + m) * 255.0) as u8,
            b: ((b + m) * 255.0) as u8,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceType {
    Keyboard,
    Motherboard,
    Gpu,
}

/// One key of a keyboard matrix zone. Positions/sizes are in key units (1U);
/// the frontend scales to pixels.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyInfo {
    pub led_index: u32,
    pub label: String,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZoneInfo {
    pub id: String,
    pub name: String,
    pub led_count: u32,
    pub resizable: bool,
    pub min_leds: u32,
    pub max_leds: u32,
    /// Some => render as keyboard layout, None => render as LED strip.
    pub keys: Option<Vec<KeyInfo>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceInfo {
    pub id: DeviceId,
    pub name: String,
    pub device_type: DeviceType,
    pub zones: Vec<ZoneInfo>,
    pub supported_effects: Vec<String>,
}

/// A firmware-driven ("onboard") effect: the device MCU animates it itself.
/// Used for devices (e.g. the GMMK v1) whose host write throughput is too low
/// to stream smooth animation — we set the mode once and the hardware runs it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnboardMode {
    Fixed,
    Breathing,
    Wave,
    Spectrum,
    Reactive,
    Swirl,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct OnboardEffect {
    pub mode: OnboardMode,
    /// Base color for single-color modes (ignored when `rainbow`).
    pub color: Color,
    /// Let the firmware cycle hue instead of using `color`.
    pub rainbow: bool,
    /// 0..=4 (0 slowest).
    pub speed: u8,
    /// Reverse animation direction where the mode supports it.
    pub reverse: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EffectConfig {
    Static {
        color: Color,
    },
    Breathing {
        color: Color,
        speed: f32,
    },
    RainbowWave {
        speed: f32,
        reverse: bool,
    },
    ColorCycle {
        speed: f32,
    },
    /// Per-LED colors live in DeviceRuntimeState::custom_colors.
    Custom,
    /// Bright head sweeps across the zone with a fading tail; wraps around.
    Meteor {
        color: Color,
        speed: f32,
        reverse: bool,
    },
    /// Flickering warm gradient (fire); hue is fixed, `speed` sets flicker rate.
    Fire {
        speed: f32,
    },
    /// Random LEDs sparkle bright and fade over a dim base `color`.
    Twinkle {
        color: Color,
        speed: f32,
    },
    /// Static spatial blend between `color` and its complement, slowly drifting.
    Gradient {
        color: Color,
        speed: f32,
    },
    /// Sine-interference plasma across the zone, full-spectrum hue.
    Plasma {
        speed: f32,
    },
    /// Single lit dot sweeps back and forth (KITT) with a soft trail.
    Larson {
        color: Color,
        speed: f32,
    },
    /// Evenly spaced lit segments march along the zone.
    TheaterChase {
        color: Color,
        speed: f32,
    },
    /// Expanding rings radiate from the zone center outward.
    Ripple {
        speed: f32,
    },
    /// Drifting northern-lights curtains in a teal/green/violet band.
    Aurora {
        speed: f32,
    },
    /// Bright arcs orbit the zone center — fan rings spin, keyboards get a
    /// rotating radar sweep.
    Vortex {
        color: Color,
        speed: f32,
        reverse: bool,
        /// Number of evenly spaced arcs (1..=4). Defaults so configs built
        /// from the app's generic color/speed/direction controls stay valid.
        #[serde(default = "default_vortex_arms")]
        arms: u32,
    },
    /// Double-thump cardiac pulse over a faint resting glow.
    Heartbeat {
        color: Color,
        speed: f32,
    },
    /// Near-dark storm blue with deterministic lightning strikes; bolts are
    /// localized on key matrices, whole-ring flashes on LED strips.
    Thunderstorm {
        speed: f32,
    },
    /// Vertical dusk gradient (violet high, gold at the horizon) that slowly
    /// sinks and shimmers. Fan rings map ring height to the gradient.
    Sunset {
        speed: f32,
    },
    /// Firmware-animated effect (hardware runs it; host sets it once).
    Onboard(OnboardEffect),
}

fn default_vortex_arms() -> u32 {
    2
}

impl EffectConfig {
    /// Host-computed effect kinds (used by the mock devices' supported_effects).
    pub const ALL_KINDS: [&'static str; 18] = [
        "static",
        "breathing",
        "rainbow_wave",
        "color_cycle",
        "custom",
        "meteor",
        "fire",
        "twinkle",
        "gradient",
        "plasma",
        "larson",
        "theater_chase",
        "ripple",
        "aurora",
        "vortex",
        "heartbeat",
        "thunderstorm",
        "sunset",
    ];
}

impl Default for EffectConfig {
    fn default() -> Self {
        EffectConfig::RainbowWave {
            speed: 1.0,
            reverse: false,
        }
    }
}
