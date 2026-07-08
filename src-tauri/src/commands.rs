use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Instant;

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

/// Manager infos with user zone-name overrides applied. This is the only view
/// the frontend should ever see, so renamed headers survive a rescan/restart.
fn infos_with_names(state: &AppState) -> Vec<DeviceInfo> {
    let mut infos = state.manager.lock().infos();
    let names = state.zone_names.lock();
    for info in infos.iter_mut() {
        let Some(zmap) = names.get(&info.id) else {
            continue;
        };
        for zone in info.zones.iter_mut() {
            if let Some(name) = zmap.get(&zone.id) {
                zone.name = name.clone();
            }
        }
    }
    infos
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
    infos_with_names(&state)
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
    with_runtime(&state, &device_id, |rt| rt.brightness = brightness.min(100))
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
        let colors = rt.custom_colors.entry(zone_id).or_insert_with(Vec::new);
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
    Ok(infos_with_names(&state))
}

/// Rename a zone (e.g. label a JARGB header). Empty name clears the override,
/// reverting to the backend's default name. Returns the refreshed device list.
#[tauri::command]
pub fn rename_zone(
    state: State<'_, Arc<AppState>>,
    device_id: DeviceId,
    zone_id: String,
    name: String,
) -> CmdResult<Vec<DeviceInfo>> {
    let name = name.trim();
    {
        let mut names = state.zone_names.lock();
        let zmap = names.entry(device_id.clone()).or_default();
        if name.is_empty() {
            zmap.remove(&zone_id);
        } else {
            zmap.insert(zone_id, name.to_string());
        }
    }
    mark_dirty(&state);
    Ok(infos_with_names(&state))
}

/// Start an identify pulse on a zone so the user can spot which physical LED
/// strip is wired to which header. The effects engine animates a short white
/// pulse; it auto-expires (see engine `IDENTIFY_SECS`).
#[tauri::command]
pub fn identify_zone(state: State<'_, Arc<AppState>>, device_id: DeviceId, zone_id: String) {
    state
        .identify
        .lock()
        .insert(device_id, (zone_id, Instant::now()));
}

#[tauri::command]
pub fn rescan_devices(app: AppHandle, state: State<'_, Arc<AppState>>) -> Vec<DeviceInfo> {
    rescan_and_notify(&app, &state)
}

/// Re-detect hardware, bump the engine's device generation (so it drops cached
/// frames belonging to the old handles), re-seed runtime state, and push the
/// refreshed device list to the frontend. Shared by the rescan command and the
/// wake-from-sleep recovery path.
pub fn rescan_and_notify(app: &AppHandle, state: &AppState) -> Vec<DeviceInfo> {
    state.manager.lock().rescan();
    state.device_generation.fetch_add(1, Ordering::Relaxed);
    state.seed_runtime();
    // Restore saved config for any device that (re)appeared this scan, so a
    // hot-plug or wake-from-sleep re-enumeration doesn't leave it on defaults.
    crate::persistence::reapply_after_rescan(state);
    let infos = infos_with_names(state);
    let _ = app.emit("devices-changed", infos.clone());
    infos
}

// --- Fan control (case fans, NCT6687D-R) ---------------------------------
// Phase 2: actual speed control. `fan_confirm_channel` identifies a tach
// channel; `fan_map_header` discovers which PWM header drives it; `fan_set_speed`
// commands a clamped duty; `fan_sweep` measures the stall floor; `fan_heartbeat`
// keeps the watchdog from releasing control; `fan_stop` hands everything back to
// the BIOS. Every write is clamped through `safety::FanLimits`.
//
// IMPORTANT: every command that touches the chip is `async` and runs its
// blocking ring-0 work on `spawn_blocking`. Tauri executes *synchronous*
// command handlers on the main thread, so a long one (mapping/sweep do tens of
// seconds of `sleep`) would freeze the webview event loop and the OS would kill
// the app as unresponsive. Off-loading keeps the UI responsive while the
// hardware op runs (it still serializes on the subsystem's internal lock).

/// Run a blocking fan-subsystem call off the main thread.
async fn fan_blocking<T, F>(state: &State<'_, Arc<AppState>>, f: F) -> CmdResult<T>
where
    T: Send + 'static,
    F: FnOnce(&AppState) -> Result<T, crate::fan::FanError> + Send + 'static,
{
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || f(&state))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}

/// Driver/chip availability + detection detail. Drives the Fan page's banner.
#[tauri::command]
pub async fn fan_status(state: State<'_, Arc<AppState>>) -> CmdResult<crate::fan::FanStatus> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || state.fan.status())
        .await
        .map_err(|e| e.to_string())
}

/// Status + RPM/PWM + temperature readings in one chip-lock pass.
#[tauri::command]
pub async fn fan_snapshot(state: State<'_, Arc<AppState>>) -> CmdResult<crate::fan::FanSnapshot> {
    fan_blocking(&state, |s| s.fan.snapshot()).await
}

/// Live per-channel RPM / PWM readings. Polled by the Fan page.
#[tauri::command]
pub async fn fan_read(
    state: State<'_, Arc<AppState>>,
) -> CmdResult<Vec<crate::fan::ChannelReading>> {
    fan_blocking(&state, |s| s.fan.read()).await
}

/// Temperature sensor readings from the NCT6687D. Returns only sensors with
/// plausible values; unconnected sensors are omitted.
#[tauri::command]
pub async fn fan_read_temps(
    state: State<'_, Arc<AppState>>,
) -> CmdResult<Vec<crate::fan::TempReading>> {
    fan_blocking(&state, |s| s.fan.read_temps()).await
}

/// STOP / panic button: clear all manual-mode bits so every header returns to
/// the BIOS fan curve. Always safe to call.
#[tauri::command]
pub async fn fan_stop(state: State<'_, Arc<AppState>>) -> CmdResult<()> {
    fan_blocking(&state, |s| s.fan.release_to_bios()).await
}

/// Record which RPM/tach channel the mapping wizard confirmed is a case fan.
/// Validates the live RPM signal, but does not mark the channel as PWM-writable.
#[tauri::command]
pub async fn fan_confirm_channel(state: State<'_, Arc<AppState>>, index: u8) -> CmdResult<()> {
    fan_blocking(&state, move |s| {
        s.fan.confirm_channel(index)?;
        mark_dirty(s);
        Ok(())
    })
    .await
}

/// Discover which PWM header drives a confirmed tach channel (nudges each header
/// and watches the tach respond). Spins fans briefly; self-restores. Returns the
/// header index, and persists the mapping. Long-running (tens of seconds).
#[tauri::command]
pub async fn fan_map_header(state: State<'_, Arc<AppState>>, rpm_channel: u8) -> CmdResult<u8> {
    fan_blocking(&state, move |s| {
        if !s.conflicts_cleared.load(Ordering::Relaxed) {
            return Err(crate::fan::FanError::Refused(
                "kill conflicting processes first".into(),
            ));
        }
        let header = s.fan.map_pwm_header(rpm_channel)?;
        mark_dirty(s);
        Ok(header)
    })
    .await
}

/// Burst auto-detect: drive every controllable header (except the pump) to 100%,
/// hold until each spun-up fan stops accelerating (reaches its plateau), then
/// auto-map every header that reported a live RPM. The pump is always left under
/// BIOS control. Persists the discovered mappings. Holds the burst for
/// `duration_secs` (frontend-configured); off-loaded to a blocking thread. Emits
/// `fan-burst-progress` per sample so the debug modal can show live per-fan RPM.
#[tauri::command]
pub async fn fan_burst_detect(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    duration_secs: u64,
) -> CmdResult<crate::fan::BurstResult> {
    fan_blocking(&state, move |s| {
        if !s.conflicts_cleared.load(Ordering::Relaxed) {
            return Err(crate::fan::FanError::Refused(
                "kill conflicting processes first".into(),
            ));
        }
        let result = s.fan.burst_detect(duration_secs, |p| {
            let _ = app.emit("fan-burst-progress", p);
        })?;
        mark_dirty(s);
        Ok(result)
    })
    .await
}

/// Command a mapped fan to a duty (%). Clamped to its safe window; returns the
/// percentage actually applied. Takes manual control and arms the watchdog.
#[tauri::command]
pub async fn fan_set_speed(
    state: State<'_, Arc<AppState>>,
    rpm_channel: u8,
    pct: u8,
) -> CmdResult<u8> {
    fan_blocking(&state, move |s| {
        if !s.conflicts_cleared.load(Ordering::Relaxed) {
            return Err(crate::fan::FanError::Refused(
                "kill conflicting processes first".into(),
            ));
        }
        s.fan.set_speed(rpm_channel, pct)
    })
    .await
}

/// Live per-sample progress emitted during a sweep (event `fan-sweep-progress`),
/// so the calibration UI can show duty/RPM while the chip lock is held.
#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct SweepProgress {
    rpm_channel: u8,
    pct: u8,
    rpm: u16,
    phase: &'static str,
}

/// Sweep a mapped fan's duty to measure its stall floor and top RPM, tightening
/// the safety limits. Long-running (~30s); restores the fan to the BIOS after.
/// Emits `fan-sweep-progress` events per sample.
#[tauri::command]
pub async fn fan_sweep(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    rpm_channel: u8,
) -> CmdResult<crate::fan::SweepResult> {
    fan_blocking(&state, move |s| {
        if !s.conflicts_cleared.load(Ordering::Relaxed) {
            return Err(crate::fan::FanError::Refused(
                "kill conflicting processes first".into(),
            ));
        }
        let result = s.fan.sweep(rpm_channel, |pct, rpm, phase| {
            let _ = app.emit(
                "fan-sweep-progress",
                SweepProgress {
                    rpm_channel,
                    pct,
                    rpm,
                    phase,
                },
            );
        })?;
        // Persist the freshly measured limits to disk immediately, not via the 2s
        // debounce — a sweep takes minutes, and the user often closes the app right
        // after, which could otherwise drop the calibration before the saver ran.
        crate::persistence::save_now(&app, s);
        Ok(result)
    })
    .await
}

/// Sweep every mapped, non-pump fan simultaneously ("Calibrate all"): all fans
/// walk the same duty ladder together, so the whole run costs about one sweep's
/// wall time instead of one per fan. Emits `fan-sweep-progress` per fan per
/// sample (same event as a single sweep, keyed by `rpmChannel`). Returns
/// `(rpmChannel, result)` pairs.
#[tauri::command]
pub async fn fan_sweep_all(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
) -> CmdResult<Vec<(u8, crate::fan::SweepResult)>> {
    fan_blocking(&state, move |s| {
        if !s.conflicts_cleared.load(Ordering::Relaxed) {
            return Err(crate::fan::FanError::Refused(
                "kill conflicting processes first".into(),
            ));
        }
        let result = s.fan.sweep_all(|rpm_channel, pct, rpm, phase| {
            let _ = app.emit(
                "fan-sweep-progress",
                SweepProgress {
                    rpm_channel,
                    pct,
                    rpm,
                    phase,
                },
            );
        })?;
        // Same immediate persist as a single sweep — don't lose minutes of
        // calibration to the debounce if the user closes the app right after.
        crate::persistence::save_now(&app, s);
        Ok(result)
    })
    .await
}

/// Cancel an in-flight sweep (the modal's Stop button). Lock-free, so it lands
/// even though the sweep thread holds the subsystem lock the whole time; the
/// sweep releases the fan back to the BIOS and returns a cancelled error.
#[tauri::command]
pub fn fan_cancel_sweep(state: State<'_, Arc<AppState>>) {
    state.fan.cancel_sweep();
}

/// DIAGNOSTIC: passive EC register capture for reverse-engineering how MSI
/// Center drives the SYS fans. `fan_ec_capture("baseline")` stores a snapshot;
/// after moving a SYS fan in MSI Center, `fan_ec_capture("high")` returns the
/// `[addr, old, new]` bytes that changed. Pure reads — never writes the chip.
/// Console: `await window.__TAURI__.core.invoke('fan_ec_capture', {label:'baseline'})`.
#[tauri::command]
pub async fn fan_ec_capture(
    state: State<'_, Arc<AppState>>,
    label: String,
) -> CmdResult<Vec<(u16, u8, u8)>> {
    fan_blocking(&state, move |s| s.fan.ec_capture(label)).await
}

/// UI heartbeat: keep the watchdog from releasing control while a fan is held.
/// Non-blocking (`try_lock`), so it's safe to leave synchronous. Mostly vestigial
/// now that the backend control loop self-beats; kept for the manual-write path.
#[tauri::command]
pub fn fan_heartbeat(state: State<'_, Arc<AppState>>) {
    state.fan.heartbeat();
}

/// Install the background fan-control plan (per-fan curve/manual modes). The
/// backend's control loop owns it from here, driving each mapped fan on its own
/// thread — so fans keep tracking temperature even when the window is hidden and
/// its JS timers are throttled. Pushed by the UI on any mode/curve/STOP change.
#[tauri::command]
pub async fn fan_set_control_plan(
    state: State<'_, Arc<AppState>>,
    plan: crate::fan::FanControlPlan,
) -> CmdResult<()> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || state.fan.set_control_plan(plan))
        .await
        .map_err(|e| e.to_string())
}

// --- GPU telemetry (NVML) -------------------------------------------------

/// Live GPU temp / core clock / fan % / power for the GPU page. Polled ~1/s by
/// the frontend. Off-loaded to a blocking thread: the first call lazily loads
/// `nvml.dll`, and NVML's getters, while quick, are still FFI we keep off the
/// webview's main thread. Reports `available: false` with no driver/GPU.
#[tauri::command]
pub async fn gpu_telemetry(
    state: State<'_, Arc<AppState>>,
) -> CmdResult<crate::gpu_telemetry::GpuTelemetry> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || state.gpu_telemetry.read())
        .await
        .map_err(|e| e.to_string())
}

// --- Startup conflict guard -----------------------------------------------

#[tauri::command]
pub fn scan_rgb_conflicts(
    state: State<'_, Arc<AppState>>,
) -> Vec<crate::process_guard::ConflictProcess> {
    let found = crate::process_guard::scan();
    if found.is_empty() {
        state.conflicts_cleared.store(true, Ordering::Relaxed);
    }
    found
}

#[tauri::command]
pub fn kill_rgb_conflicts(state: State<'_, Arc<AppState>>, pids: Vec<u32>) -> CmdResult<()> {
    crate::process_guard::kill(&pids)?;
    state.conflicts_cleared.store(true, Ordering::Relaxed);
    Ok(())
}

/// Clean shutdown: hand any held fans back to the BIOS, flush config to disk,
/// then exit. Called from the conflict modal's "Quit App" button so the user
/// can close rather than proceed with competing software still running.
#[tauri::command]
pub fn quit_app(app: AppHandle, state: State<'_, Arc<AppState>>) {
    let _ = state.fan.release_to_bios();
    crate::persistence::save_now(&app, &state);
    app.exit(0);
}

/// Whether the current process is running with administrator rights.
#[tauri::command]
pub fn is_elevated() -> bool {
    crate::admin::is_elevated()
}

/// Relaunch the app elevated (pops UAC). On success the elevated instance is
/// starting, so we cleanly shut this one down — release fans and flush config
/// first, exactly like a normal quit, to avoid leaving a fan stranded or losing
/// unsaved settings. If the user declines UAC we return the error and stay put.
#[tauri::command]
pub fn relaunch_as_admin(app: AppHandle, state: State<'_, Arc<AppState>>) -> CmdResult<()> {
    crate::admin::relaunch_as_admin()?;
    let _ = state.fan.release_to_bios();
    crate::persistence::save_now(&app, &state);
    app.exit(0);
    Ok(())
}
