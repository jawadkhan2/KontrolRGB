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
    /// Saved config for devices NOT present this session (unplugged, or absent
    /// at boot). Held so a rescan can restore them and a save in the meantime
    /// doesn't erase them; drained into the live state by `reapply_after_rescan`
    /// as devices reappear.
    pub stale_device_configs: Mutex<HashMap<DeviceId, crate::persistence::DeviceConfig>>,
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
    /// Conflict-guard settings (auto-enforce etc.); persisted in config.json.
    pub guard: Mutex<crate::process_guard::GuardConfig>,
    /// Last known resizable-zone LED counts per device id. Lets a save keep a
    /// device's counts while it is temporarily absent (mock in its slot), so a
    /// missed probe can no longer wipe them from config.json.
    pub known_zone_counts: Mutex<HashMap<DeviceId, HashMap<String, u32>>>,
    /// Whether the main window is showing. The effects engine skips preview
    /// emission (and idles harder) while it's hidden in the tray.
    pub ui_visible: AtomicBool,
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
            stale_device_configs: Mutex::new(HashMap::new()),
            identify: Mutex::new(HashMap::new()),
            dirty: AtomicBool::new(false),
            device_generation: AtomicU64::new(0),
            fan: crate::fan::FanSubsystem::new(conflicts_cleared.clone(), gpu_telemetry.clone()),
            gpu_telemetry,
            conflicts_cleared,
            guard: Mutex::new(Default::default()),
            known_zone_counts: Mutex::new(HashMap::new()),
            ui_visible: AtomicBool::new(false),
        }
    }

    /// Re-assert every device's lighting: each backend re-arms / repaints on its
    /// next write, and the engine drops its "already sent" cache so static
    /// effects get re-staged too. Cheap; safe to call from any thread. Used on
    /// resume, display-on, and after the conflict guard shut something down.
    pub fn request_resync(&self) {
        self.manager.lock().resync_all();
        self.device_generation
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
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
