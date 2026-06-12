use std::collections::HashMap;
use std::sync::atomic::AtomicBool;

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use crate::device::manager::DeviceManager;
use crate::device::types::{Color, DeviceId, EffectConfig};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceRuntimeState {
    pub effect: EffectConfig,
    /// 0..=100
    pub brightness: u8,
    /// zone_id -> per-LED colors, used by EffectConfig::Custom.
    pub custom_colors: HashMap<String, Vec<Color>>,
}

impl Default for DeviceRuntimeState {
    fn default() -> Self {
        DeviceRuntimeState {
            effect: EffectConfig::default(),
            brightness: 80,
            custom_colors: HashMap::new(),
        }
    }
}

pub struct AppState {
    pub manager: Mutex<DeviceManager>,
    pub runtime: Mutex<HashMap<DeviceId, DeviceRuntimeState>>,
    /// Set by mutating commands; the persistence task saves and clears it.
    pub dirty: AtomicBool,
}

impl AppState {
    pub fn new() -> Self {
        AppState {
            manager: Mutex::new(DeviceManager::new()),
            runtime: Mutex::new(HashMap::new()),
            dirty: AtomicBool::new(false),
        }
    }

    /// Ensure every known device has a runtime state entry.
    pub fn seed_runtime(&self) {
        let ids: Vec<DeviceId> = {
            let manager = self.manager.lock();
            manager.infos().into_iter().map(|i| i.id).collect()
        };
        let mut runtime = self.runtime.lock();
        for id in ids {
            runtime.entry(id).or_default();
        }
    }
}
