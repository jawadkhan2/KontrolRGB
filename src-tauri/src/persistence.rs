//! Debounced config persistence: %APPDATA%\com.jawad.kontrolrgb\config.json

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::Manager;

use crate::device::types::DeviceId;
use crate::state::{AppState, DeviceRuntimeState};

#[derive(Serialize, Deserialize)]
struct ConfigFile {
    version: u32,
    devices: HashMap<DeviceId, DeviceConfig>,
}

#[derive(Serialize, Deserialize)]
struct DeviceConfig {
    #[serde(flatten)]
    runtime: DeviceRuntimeState,
    #[serde(default)]
    zone_led_counts: HashMap<String, u32>,
}

fn config_path(app: &tauri::AppHandle) -> Option<PathBuf> {
    app.path().app_data_dir().ok().map(|d| d.join("config.json"))
}

/// Load config and apply it: resize zones, seed runtime states. Call from
/// setup after the first device scan. Unknown device ids are ignored.
pub fn load_and_apply(app: &tauri::AppHandle, state: &AppState) {
    state.seed_runtime();
    let Some(path) = config_path(app) else { return };
    let Ok(raw) = std::fs::read_to_string(&path) else { return };
    let Ok(config) = serde_json::from_str::<ConfigFile>(&raw) else {
        eprintln!("config.json unreadable, using defaults");
        return;
    };

    let mut manager = state.manager.lock();
    let mut runtime = state.runtime.lock();
    for (id, dev_config) in config.devices {
        let Some(device) = manager.get_mut(&id) else { continue };
        for (zone_id, count) in &dev_config.zone_led_counts {
            if let Err(e) = device.resize_zone(zone_id, *count) {
                eprintln!("restore resize {id}/{zone_id}: {e}");
            }
        }
        runtime.insert(id, dev_config.runtime);
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

    let devices = state
        .runtime
        .lock()
        .iter()
        .map(|(id, rt)| {
            (
                id.clone(),
                DeviceConfig {
                    runtime: rt.clone(),
                    zone_led_counts: zone_counts.get(id).cloned().unwrap_or_default(),
                },
            )
        })
        .collect();

    ConfigFile { version: 1, devices }
}

fn save(app: &tauri::AppHandle, state: &AppState) {
    let Some(path) = config_path(app) else { return };
    let config = snapshot(state);
    let Ok(json) = serde_json::to_string_pretty(&config) else { return };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let tmp = path.with_extension("json.tmp");
    if std::fs::write(&tmp, json).is_ok() {
        if let Err(e) = std::fs::rename(&tmp, &path) {
            eprintln!("config save failed: {e}");
        }
    }
}

/// Background task: every 2 s, write the config if anything changed.
pub async fn run_saver(app: tauri::AppHandle, state: Arc<AppState>) {
    let mut interval = tokio::time::interval(Duration::from_secs(2));
    loop {
        interval.tick().await;
        if state.dirty.swap(false, Ordering::Relaxed) {
            save(&app, &state);
        }
    }
}
