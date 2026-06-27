//! Mock devices mirroring the real hardware so the frontend and effects
//! engine can be built and exercised before any protocol work.

use std::collections::HashMap;

use super::layouts::gmmk_ansi;
use super::types::{Color, DeviceInfo, DeviceType, EffectConfig, ZoneInfo};
use super::{DeviceError, RgbDevice};

pub struct MockDevice {
    info: DeviceInfo,
    staged: HashMap<String, Vec<Color>>,
}

impl MockDevice {
    fn new(info: DeviceInfo) -> Self {
        let staged = info
            .zones
            .iter()
            .map(|z| (z.id.clone(), vec![Color::BLACK; z.led_count as usize]))
            .collect();
        MockDevice { info, staged }
    }
}

impl RgbDevice for MockDevice {
    fn id(&self) -> &str {
        &self.info.id
    }

    fn info(&self) -> DeviceInfo {
        self.info.clone()
    }

    fn set_zone_leds(&mut self, zone_id: &str, colors: &[Color]) -> Result<(), DeviceError> {
        let staged = self
            .staged
            .get_mut(zone_id)
            .ok_or_else(|| DeviceError::UnknownZone(zone_id.to_string()))?;
        let n = staged.len().min(colors.len());
        staged[..n].copy_from_slice(&colors[..n]);
        Ok(())
    }

    fn apply(&mut self) -> Result<(), DeviceError> {
        // No hardware behind the mock.
        Ok(())
    }

    fn resize_zone(&mut self, zone_id: &str, led_count: u32) -> Result<(), DeviceError> {
        let zone = self
            .info
            .zones
            .iter_mut()
            .find(|z| z.id == zone_id)
            .ok_or_else(|| DeviceError::UnknownZone(zone_id.to_string()))?;
        if !zone.resizable {
            return Err(DeviceError::NotResizable(zone_id.to_string()));
        }
        if led_count < zone.min_leds || led_count > zone.max_leds {
            return Err(DeviceError::LedCountOutOfRange {
                count: led_count,
                min: zone.min_leds,
                max: zone.max_leds,
            });
        }
        zone.led_count = led_count;
        self.staged
            .insert(zone_id.to_string(), vec![Color::BLACK; led_count as usize]);
        Ok(())
    }
}

fn fixed_zone(id: &str, name: &str, led_count: u32) -> ZoneInfo {
    ZoneInfo {
        id: id.to_string(),
        name: name.to_string(),
        led_count,
        resizable: false,
        min_leds: led_count,
        max_leds: led_count,
        keys: None,
    }
}

fn argb_header(id: &str, name: &str, default_leds: u32) -> ZoneInfo {
    ZoneInfo {
        id: id.to_string(),
        name: name.to_string(),
        led_count: default_leds,
        resizable: true,
        min_leds: 1,
        max_leds: 240,
        keys: None,
    }
}

pub fn mock_gmmk() -> MockDevice {
    let keys = gmmk_ansi::full_size();
    MockDevice::new(DeviceInfo {
        id: "mock-gmmk".to_string(),
        name: "Glorious GMMK".to_string(),
        device_type: DeviceType::Keyboard,
        zones: vec![ZoneInfo {
            id: "keys".to_string(),
            name: "Keys".to_string(),
            led_count: keys.len() as u32,
            resizable: false,
            min_leds: keys.len() as u32,
            max_leds: keys.len() as u32,
            keys: Some(keys),
        }],
        supported_effects: EffectConfig::ALL_KINDS.map(String::from).to_vec(),
    })
}

pub fn mock_msi_z890() -> MockDevice {
    MockDevice::new(DeviceInfo {
        id: "mock-msi-z890".to_string(),
        name: "MSI MAG Z890 Tomahawk".to_string(),
        device_type: DeviceType::Motherboard,
        zones: vec![
            fixed_zone("onboard", "Onboard LEDs", 8),
            argb_header("jargb_v2_1", "JARGB_V2 1", 30),
            argb_header("jargb_v2_2", "JARGB_V2 2", 30),
            argb_header("jargb_v2_3", "JARGB_V2 3", 30),
        ],
        supported_effects: EffectConfig::ALL_KINDS.map(String::from).to_vec(),
    })
}

pub fn mock_rtx5080() -> MockDevice {
    MockDevice::new(DeviceInfo {
        id: "mock-rtx5080".to_string(),
        name: "Gigabyte RTX 5080 Gaming OC".to_string(),
        device_type: DeviceType::Gpu,
        zones: vec![
            fixed_zone("logo", "Logo", 1),
            fixed_zone("side", "Side Bar", 6),
        ],
        supported_effects: EffectConfig::ALL_KINDS.map(String::from).to_vec(),
    })
}
