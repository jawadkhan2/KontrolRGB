//! Debounced config persistence: %APPDATA%\com.jawad.kontrolrgb\config.json

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::Manager;

use crate::device::types::DeviceId;
use crate::fan::FanConfig;
use crate::state::{AppState, DeviceRuntimeState};

#[derive(Serialize, Deserialize)]
struct ConfigFile {
    version: u32,
    #[serde(default)]
    fan: FanConfig,
    devices: HashMap<DeviceId, DeviceConfig>,
}

/// One device's saved config. `pub(crate)` (with private fields) because
/// `AppState` holds a map of these for devices that were absent at load — see
/// `stale_device_configs` — but all access stays in this module.
#[derive(Serialize, Deserialize, Clone)]
pub(crate) struct DeviceConfig {
    #[serde(flatten)]
    runtime: DeviceRuntimeState,
    #[serde(default)]
    zone_led_counts: HashMap<String, u32>,
    /// User-renamed zones: zone_id -> custom name.
    #[serde(default)]
    zone_names: HashMap<String, String>,
}

fn config_path(app: &tauri::AppHandle) -> Option<PathBuf> {
    app.path()
        .app_data_dir()
        .ok()
        .map(|d| d.join("config.json"))
}

/// Apply one saved device config to a live device: restore zone LED counts,
/// runtime state, and zone-name overrides. Shared by the initial load and the
/// post-rescan re-apply so a device that appears later gets its saved settings.
fn apply_device_config(
    device: &mut Box<dyn crate::device::RgbDevice>,
    runtime: &mut HashMap<DeviceId, DeviceRuntimeState>,
    zone_names: &mut HashMap<DeviceId, HashMap<String, String>>,
    id: &DeviceId,
    dev_config: DeviceConfig,
) {
    for (zone_id, count) in &dev_config.zone_led_counts {
        if let Err(e) = device.resize_zone(zone_id, *count) {
            eprintln!("restore resize {id}/{zone_id}: {e}");
        }
    }
    if !dev_config.zone_names.is_empty() {
        zone_names.insert(id.clone(), dev_config.zone_names);
    }
    runtime.insert(id.clone(), dev_config.runtime);
}

/// Load config and apply it: resize zones, seed runtime states. Call from
/// setup after the first device scan. Configs for devices that aren't present
/// are stashed (not dropped) so a later hot-plug/rescan restores them and a
/// save in the meantime doesn't erase them.
pub fn load_and_apply(app: &tauri::AppHandle, state: &AppState) {
    state.seed_runtime();
    let Some(path) = config_path(app) else { return };
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return;
    };
    let Ok(config) = serde_json::from_str::<ConfigFile>(&raw) else {
        eprintln!("config.json unreadable, using defaults");
        return;
    };

    state.fan.apply_config(config.fan);

    let mut manager = state.manager.lock();
    let mut runtime = state.runtime.lock();
    let mut zone_names = state.zone_names.lock();
    let mut stale = state.stale_device_configs.lock();
    for (id, dev_config) in config.devices {
        match manager.get_mut(&id) {
            Some(device) => {
                apply_device_config(device, &mut runtime, &mut zone_names, &id, dev_config);
            }
            // Device absent right now — keep its config so a rescan can restore
            // it and `snapshot` doesn't drop it on the next save.
            None => {
                stale.insert(id, dev_config);
            }
        }
    }
}

/// Re-apply any stashed config to devices that have since appeared (hot-plug,
/// wake-from-sleep re-enumeration, or a manual rescan). Removes each config from
/// the stale store once its device is present and restored. Call after a
/// `manager.rescan()` + `seed_runtime()`.
pub fn reapply_after_rescan(state: &AppState) {
    let mut manager = state.manager.lock();
    let mut runtime = state.runtime.lock();
    let mut zone_names = state.zone_names.lock();
    let mut stale = state.stale_device_configs.lock();
    if stale.is_empty() {
        return;
    }
    let present: Vec<DeviceId> = manager
        .infos()
        .into_iter()
        .map(|i| i.id)
        .filter(|id| stale.contains_key(id))
        .collect();
    for id in present {
        let Some(dev_config) = stale.remove(&id) else {
            continue;
        };
        if let Some(device) = manager.get_mut(&id) {
            apply_device_config(device, &mut runtime, &mut zone_names, &id, dev_config);
        }
    }
}

fn snapshot(state: &AppState) -> ConfigFile {
    let zone_counts: HashMap<DeviceId, HashMap<String, u32>> = state
        .manager
        .lock()
        .infos()
        .into_iter()
        .map(|info| {
            let counts = info
                .zones
                .iter()
                .filter(|z| z.resizable)
                .map(|z| (z.id.clone(), z.led_count))
                .collect();
            (info.id, counts)
        })
        .collect();

    let zone_names = state.zone_names.lock();
    let mut devices: HashMap<DeviceId, DeviceConfig> = state
        .runtime
        .lock()
        .iter()
        .map(|(id, rt)| {
            (
                id.clone(),
                DeviceConfig {
                    runtime: rt.clone(),
                    zone_led_counts: zone_counts.get(id).cloned().unwrap_or_default(),
                    zone_names: zone_names.get(id).cloned().unwrap_or_default(),
                },
            )
        })
        .collect();

    // Preserve configs for devices absent this session so unplugging a keyboard
    // (or a device missing at boot) never wipes its saved effect/zones. A live
    // device always wins over its stale copy, so `entry` here never clobbers.
    for (id, cfg) in state.stale_device_configs.lock().iter() {
        devices.entry(id.clone()).or_insert_with(|| cfg.clone());
    }

    ConfigFile {
        version: 1,
        fan: state.fan.config(),
        devices,
    }
}

fn replace_file(tmp: &std::path::Path, path: &std::path::Path) -> std::io::Result<()> {
    if !path.exists() {
        return std::fs::rename(tmp, path);
    }

    let backup = path.with_extension("json.bak");
    let _ = std::fs::remove_file(&backup);
    std::fs::rename(path, &backup)?;
    match std::fs::rename(tmp, path) {
        Ok(()) => {
            let _ = std::fs::remove_file(&backup);
            Ok(())
        }
        Err(err) => {
            let _ = std::fs::rename(&backup, path);
            Err(err)
        }
    }
}

fn save(app: &tauri::AppHandle, state: &AppState) -> Result<(), String> {
    let path = config_path(app).ok_or_else(|| "config path unavailable".to_string())?;
    let config = snapshot(state);
    let json = serde_json::to_string_pretty(&config).map_err(|e| e.to_string())?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json).map_err(|e| e.to_string())?;
    replace_file(&tmp, &path).map_err(|e| e.to_string())
}

pub fn save_now(app: &tauri::AppHandle, state: &AppState) {
    match save(app, state) {
        Ok(()) => state.dirty.store(false, Ordering::Relaxed),
        Err(e) => {
            eprintln!("config save failed: {e}");
            state.dirty.store(true, Ordering::Relaxed);
        }
    }
}

/// Background task: every 2 s, write the config if anything changed.
pub async fn run_saver(app: tauri::AppHandle, state: Arc<AppState>) {
    let mut interval = tokio::time::interval(Duration::from_secs(2));
    loop {
        interval.tick().await;
        if state.dirty.swap(false, Ordering::Relaxed) {
            if let Err(e) = save(&app, &state) {
                eprintln!("config save failed: {e}");
                state.dirty.store(true, Ordering::Relaxed);
            }
        }
    }
}
