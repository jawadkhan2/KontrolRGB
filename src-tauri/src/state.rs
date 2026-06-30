use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::Arc;
use std::time::Instant;

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use crate::device::manager::DeviceManager;
use crate::device::types::{Color, DeviceId, EffectConfig, OnboardEffect, OnboardMode};

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
    /// User-renamed zones: device_id -> zone_id -> custom name. Overlaid onto
    /// DeviceInfo before it reaches the frontend; persisted in config.
    pub zone_names: Mutex<HashMap<DeviceId, HashMap<String, String>>>,
    /// Active "identify" pulses: device_id -> (zone_id, start time). Transient
    /// (not persisted); the effects engine pulses the zone until it expires.
    pub identify: Mutex<HashMap<DeviceId, (String, Instant)>>,
    /// Set by mutating commands; the persistence task saves and clears it.
    pub dirty: AtomicBool,
    /// True once the user has cleared competing RGB/fan software. The effects
    /// engine and fan write commands gate all hardware writes on this flag.
    /// Shared (`Arc`) with the fan subsystem's control loop, which checks it too.
    pub conflicts_cleared: Arc<AtomicBool>,
    /// Bumped whenever hardware is re-detected so the effects engine knows its
    /// cached "last written" frames belong to old device handles.
    pub device_generation: AtomicU64,
    /// Case-fan control subsystem (NCT6687D-R via ring-0 LPC). Separate from the
    /// RGB device manager: fans aren't RgbDevices and use a different transport.
    pub fan: crate::fan::FanSubsystem,
    /// GPU telemetry (NVML): live temp/clock/fan/power for the GPU page. Lazily
    /// loaded; independent of the GPU's RGB controller. Shared (`Arc`) with the
    /// fan control loop so a fan curve can be driven off the GPU die temp.
    pub gpu_telemetry: Arc<crate::gpu_telemetry::GpuTelemetrySubsystem>,
}

impl AppState {
    pub fn new() -> Self {
        // Shared between the gating checks (effects engine, fan commands) and the
        // fan control loop, so all of them see one source of truth.
        let conflicts_cleared = Arc::new(AtomicBool::new(false));
        let gpu_telemetry = Arc::new(crate::gpu_telemetry::GpuTelemetrySubsystem::new());
        AppState {
            manager: Mutex::new(DeviceManager::new()),
            runtime: Mutex::new(HashMap::new()),
            zone_names: Mutex::new(HashMap::new()),
            identify: Mutex::new(HashMap::new()),
            dirty: AtomicBool::new(false),
            device_generation: AtomicU64::new(0),
            fan: crate::fan::FanSubsystem::new(conflicts_cleared.clone(), gpu_telemetry.clone()),
            gpu_telemetry,
            conflicts_cleared,
        }
    }

    /// Ensure every known device has a runtime state entry. New entries get a
    /// device-appropriate default effect: onboard-only hardware (GMMK) can't
    /// host-stream animation, so it defaults to a firmware effect rather than
    /// the generic (host-animated) default.
    pub fn seed_runtime(&self) {
        let devices: Vec<(DeviceId, bool)> = {
            let manager = self.manager.lock();
            manager
                .infos()
                .into_iter()
                .map(|i| {
                    let onboard_only = i.supported_effects.iter().any(|s| s == "onboard")
                        && !i.supported_effects.iter().any(|s| s == "rainbow_wave");
                    (i.id, onboard_only)
                })
                .collect()
        };
        let mut runtime = self.runtime.lock();
        for (id, onboard_only) in devices {
            runtime.entry(id).or_insert_with(|| {
                if onboard_only {
                    DeviceRuntimeState {
                        effect: EffectConfig::Onboard(OnboardEffect {
                            mode: OnboardMode::Wave,
                            color: Color {
                                r: 120,
                                g: 60,
                                b: 220,
                            },
                            rainbow: true,
                            speed: 2,
                            reverse: false,
                        }),
                        ..Default::default()
                    }
                } else {
                    DeviceRuntimeState::default()
                }
            });
        }
    }
}
