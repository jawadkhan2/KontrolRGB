pub mod gmmk;
pub mod layouts;
pub mod manager;
pub mod mock;
pub mod msi;
pub mod types;

use types::{Color, DeviceInfo, OnboardEffect};

#[derive(Debug, thiserror::Error)]
pub enum DeviceError {
    #[error("unknown zone: {0}")]
    UnknownZone(String),
    #[error("zone {0} is not resizable")]
    NotResizable(String),
    #[error("led count {count} out of range {min}..={max}")]
    LedCountOutOfRange { count: u32, min: u32, max: u32 },
    #[error("communication error: {0}")]
    Comm(String),
    #[error("operation not supported by this device")]
    Unsupported,
}

/// The contract every RGB backend implements (mock now; GMMK / MSI Mystic
/// Light / Gigabyte GPU later). The effects engine is the only writer.
pub trait RgbDevice: Send + Sync {
    fn id(&self) -> &str;
    fn info(&self) -> DeviceInfo;
    /// Stage colors for one zone (already brightness-scaled by the engine).
    fn set_zone_leds(&mut self, zone_id: &str, colors: &[Color]) -> Result<(), DeviceError>;
    /// Flush the staged frame to hardware.
    fn apply(&mut self) -> Result<(), DeviceError>;
    /// Resize a resizable zone (ARGB headers).
    fn resize_zone(&mut self, zone_id: &str, led_count: u32) -> Result<(), DeviceError>;

    /// Apply a firmware-driven onboard effect (the device animates it itself).
    /// `brightness` is the UI 0..=100 value. Default: unsupported — only
    /// hardware with onboard effects (GMMK) overrides this.
    fn set_onboard_effect(
        &mut self,
        _effect: &OnboardEffect,
        _brightness: u8,
    ) -> Result<(), DeviceError> {
        Err(DeviceError::Unsupported)
    }
}
