//! Pure, hardware-free core shared by the KontrolRGB desktop app and the
//! kontrolrgb-sim web preview (compiled to WASM there). Anything that touches
//! HID/NvAPI/ring-0 stays in the app; this crate is math + data only, so a
//! frame computed in the browser is bit-identical to one sent to hardware.

pub mod catalog;
pub mod effects;
pub mod fixtures;
pub mod layouts;
pub mod types;

pub use effects::compute_frame;
pub use types::{Color, DeviceInfo, EffectConfig, KeyInfo, OnboardEffect, OnboardMode, ZoneInfo};
