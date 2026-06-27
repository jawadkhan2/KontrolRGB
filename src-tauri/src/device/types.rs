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
#[derive(Debug, Clone, Serialize)]
pub struct KeyInfo {
    pub led_index: u32,
    pub label: String,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

#[derive(Debug, Clone, Serialize)]
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

#[derive(Debug, Clone, Serialize)]
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
    /// Firmware-animated effect (hardware runs it; host sets it once).
    Onboard(OnboardEffect),
}

impl EffectConfig {
    /// Host-computed effect kinds (used by the mock devices' supported_effects).
    pub const ALL_KINDS: [&'static str; 5] = [
        "static",
        "breathing",
        "rainbow_wave",
        "color_cycle",
        "custom",
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
