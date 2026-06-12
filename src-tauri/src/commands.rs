use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use crate::device::types::{Color, DeviceId, DeviceInfo, EffectConfig};
use crate::state::{AppState, DeviceRuntimeState};

type CmdResult<T> = Result<T, String>;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceStateDto {
    pub effect: EffectConfig,
    pub brightness: u8,
    pub custom_colors: HashMap<String, Vec<Color>>,
}

fn mark_dirty(state: &AppState) {
    state.dirty.store(true, Ordering::Relaxed);
}

fn with_runtime<T>(
    state: &AppState,
    device_id: &DeviceId,
    f: impl FnOnce(&mut DeviceRuntimeState) -> T,
) -> CmdResult<T> {
    let mut runtime = state.runtime.lock();
    let rt = runtime
        .get_mut(device_id)
        .ok_or_else(|| format!("unknown device: {device_id}"))?;
    let out = f(rt);
    mark_dirty(state);
    Ok(out)
}

#[tauri::command]
pub fn list_devices(state: State<'_, Arc<AppState>>) -> Vec<DeviceInfo> {
    state.manager.lock().infos()
}

#[tauri::command]
pub fn get_device_state(
    state: State<'_, Arc<AppState>>,
    device_id: DeviceId,
) -> CmdResult<DeviceStateDto> {
    let runtime = state.runtime.lock();
    let rt = runtime
        .get(&device_id)
        .ok_or_else(|| format!("unknown device: {device_id}"))?;
    Ok(DeviceStateDto {
        effect: rt.effect.clone(),
        brightness: rt.brightness,
        custom_colors: rt.custom_colors.clone(),
    })
}

#[tauri::command]
pub fn set_effect(
    state: State<'_, Arc<AppState>>,
    device_id: DeviceId,
    effect: EffectConfig,
) -> CmdResult<()> {
    with_runtime(&state, &device_id, |rt| rt.effect = effect)
}

#[tauri::command]
pub fn set_brightness(
    state: State<'_, Arc<AppState>>,
    device_id: DeviceId,
    brightness: u8,
) -> CmdResult<()> {
    with_runtime(&state, &device_id, |rt| {
        rt.brightness = brightness.min(100)
    })
}

#[tauri::command]
pub fn set_zone_colors(
    state: State<'_, Arc<AppState>>,
    device_id: DeviceId,
    zone_id: String,
    colors: Vec<Color>,
) -> CmdResult<()> {
    with_runtime(&state, &device_id, |rt| {
        rt.custom_colors.insert(zone_id, colors);
        rt.effect = EffectConfig::Custom;
    })
}

#[tauri::command]
pub fn set_led_color(
    state: State<'_, Arc<AppState>>,
    device_id: DeviceId,
    zone_id: String,
    led_index: u32,
    color: Color,
) -> CmdResult<()> {
    let led_count = zone_led_count(&state, &device_id, &zone_id)?;
    with_runtime(&state, &device_id, |rt| {
        let colors = rt
            .custom_colors
            .entry(zone_id)
            .or_insert_with(Vec::new);
        colors.resize(led_count, Color::BLACK);
        if let Some(slot) = colors.get_mut(led_index as usize) {
            *slot = color;
        }
        rt.effect = EffectConfig::Custom;
    })
}

fn zone_led_count(state: &AppState, device_id: &DeviceId, zone_id: &str) -> CmdResult<usize> {
    let mut manager = state.manager.lock();
    let device = manager
        .get_mut(device_id)
        .ok_or_else(|| format!("unknown device: {device_id}"))?;
    device
        .info()
        .zones
        .iter()
        .find(|z| z.id == zone_id)
        .map(|z| z.led_count as usize)
        .ok_or_else(|| format!("unknown zone: {zone_id}"))
}

#[tauri::command]
pub fn resize_zone(
    state: State<'_, Arc<AppState>>,
    device_id: DeviceId,
    zone_id: String,
    led_count: u32,
) -> CmdResult<Vec<DeviceInfo>> {
    {
        let mut manager = state.manager.lock();
        let device = manager
            .get_mut(&device_id)
            .ok_or_else(|| format!("unknown device: {device_id}"))?;
        device
            .resize_zone(&zone_id, led_count)
            .map_err(|e| e.to_string())?;
    }
    // Trim stale custom colors so Custom mode doesn't paint beyond the zone.
    {
        let mut runtime = state.runtime.lock();
        if let Some(rt) = runtime.get_mut(&device_id) {
            if let Some(colors) = rt.custom_colors.get_mut(&zone_id) {
                colors.resize(led_count as usize, Color::BLACK);
            }
        }
    }
    mark_dirty(&state);
    Ok(state.manager.lock().infos())
}

#[tauri::command]
pub fn apply_to_all(
    state: State<'_, Arc<AppState>>,
    effect: EffectConfig,
    brightness: Option<u8>,
) -> CmdResult<()> {
    let mut runtime = state.runtime.lock();
    for rt in runtime.values_mut() {
        rt.effect = effect.clone();
        if let Some(b) = brightness {
            rt.brightness = b.min(100);
        }
    }
    drop(runtime);
    mark_dirty(&state);
    Ok(())
}

#[tauri::command]
pub fn rescan_devices(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
) -> Vec<DeviceInfo> {
    let infos = {
        let mut manager = state.manager.lock();
        manager.rescan();
        manager.infos()
    };
    state.seed_runtime();
    let _ = app.emit("devices-changed", infos.clone());
    infos
}
