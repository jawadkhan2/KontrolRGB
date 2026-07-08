mod admin;
mod commands;
mod device;
mod effects;
mod fan;
mod gpu_telemetry;
mod persistence;
#[cfg(windows)]
mod power_watch;
mod process_guard;
mod state;

use std::sync::Arc;

use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Manager, WindowEvent,
};

use state::AppState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app_state = Arc::new(AppState::new());

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        // Launch-on-boot. When the OS starts us from the Run key we pass
        // `--minimized`; the window then stays hidden in the tray (see below).
        // A manual launch has no such flag, so the window shows normally.
        .plugin(
            tauri_plugin_autostart::Builder::new()
                .args(["--minimized"])
                .build(),
        )
        .manage(app_state.clone())
        .setup(move |app| {
            let handle = app.handle().clone();
            persistence::load_and_apply(&handle, &app_state);
            // Crash-recovery: if a prior unclean exit left the marker behind, the
            // in-process watchdog never got to run, so reclaim any stranded mapped
            // fan back to the BIOS now. Runs AFTER load_and_apply so the persisted
            // pwm_map is known.
            if let Ok(dir) = handle.path().app_data_dir() {
                app_state.fan.init_recovery(dir.join("fan_manual.lock"));
            }
            tauri::async_runtime::spawn(effects::engine::run(handle.clone(), app_state.clone()));
            tauri::async_runtime::spawn(persistence::run_saver(handle, app_state.clone()));

            // Monitor sleep / system suspend resets the USB RGB controllers
            // (fans revert to firmware rainbow, HID handles go dead). Watch for
            // wake events and re-probe the hardware automatically.
            #[cfg(windows)]
            {
                let wake_handle = app.handle().clone();
                let wake_state = app_state.clone();
                power_watch::spawn(move || wake_recovery(&wake_handle, &wake_state));
            }

            build_tray(app.handle(), app_state.clone())?;

            // The window is created hidden (visible: false in tauri.conf.json) so
            // a boot launch never flashes on screen. Show it now unless we were
            // started with --minimized (i.e. by the autostart Run entry), in
            // which case the app lives in the tray until the user clicks it.
            let launched_minimized = std::env::args().any(|a| a == "--minimized");
            if !launched_minimized {
                show_main_window(app.handle());
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            // Intercept the window's close button (X): hide to the system tray
            // instead of exiting. The app keeps running so effects/persistence
            // continue; the user quits explicitly from the tray menu.
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_devices,
            commands::get_device_state,
            commands::set_effect,
            commands::set_brightness,
            commands::set_zone_colors,
            commands::set_led_color,
            commands::resize_zone,
            commands::rename_zone,
            commands::identify_zone,
            commands::rescan_devices,
            commands::fan_status,
            commands::fan_snapshot,
            commands::fan_read,
            commands::fan_read_temps,
            commands::fan_stop,
            commands::fan_confirm_channel,
            commands::fan_map_header,
            commands::fan_burst_detect,
            commands::fan_set_speed,
            commands::fan_sweep,
            commands::fan_sweep_all,
            commands::fan_cancel_sweep,
            commands::fan_ec_capture,
            commands::fan_heartbeat,
            commands::fan_set_control_plan,
            commands::gpu_telemetry,
            commands::scan_rgb_conflicts,
            commands::kill_rgb_conflicts,
            commands::quit_app,
            commands::is_elevated,
            commands::relaunch_as_admin,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

/// Build the system tray icon with a context menu (Show / Quit) and wire up
/// click handling so a left click on the icon restores the main window.
fn build_tray(app: &tauri::AppHandle, state: Arc<AppState>) -> tauri::Result<()> {
    let show_item = MenuItem::with_id(app, "show", "Show KontrolRGB", true, None::<&str>)?;
    let quit_item = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show_item, &quit_item])?;

    TrayIconBuilder::with_id("main")
        .icon(app.default_window_icon().cloned().unwrap())
        .tooltip("KontrolRGB")
        .menu(&menu)
        // Don't pop the menu on left click; we use left click to show the window.
        .show_menu_on_left_click(false)
        .on_menu_event(move |app, event| match event.id.as_ref() {
            "show" => show_main_window(app),
            "quit" => {
                // Hand any held fans back to the BIOS before exiting so we never
                // leave a fan stranded at a fixed duty after the app is gone.
                let _ = state.fan.release_to_bios();
                persistence::save_now(app, &state);
                app.exit(0);
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main_window(tray.app_handle());
            }
        })
        .build(app)?;

    Ok(())
}

/// Wake-from-sleep recovery: re-probe all RGB hardware so dead HID handles are
/// replaced and direct mode is re-armed; the engine's generation bump then
/// re-pushes every device's configured effect. USB re-enumeration can lag the
/// wake event — if a device that was real before the rescan comes back as a
/// mock, wait and retry a few times before giving up.
#[cfg(windows)]
fn wake_recovery(app: &tauri::AppHandle, state: &Arc<AppState>) {
    use std::collections::HashSet;

    eprintln!("power-watch: wake detected, re-probing RGB hardware");
    let real_before: HashSet<String> = state
        .manager
        .lock()
        .infos()
        .into_iter()
        .map(|i| i.id)
        .filter(|id| !id.starts_with("mock-"))
        .collect();

    for attempt in 1..=5 {
        if attempt > 1 {
            std::thread::sleep(std::time::Duration::from_secs(3));
        }
        let after: HashSet<String> = commands::rescan_and_notify(app, state)
            .into_iter()
            .map(|i| i.id)
            .collect();
        if real_before.iter().all(|id| after.contains(id)) {
            return;
        }
        eprintln!("power-watch: not all devices re-enumerated yet (attempt {attempt}/5)");
    }
    eprintln!("power-watch: gave up waiting for missing devices; use Rescan to retry");
}

/// Show, unminimize, and focus the main window (used by the tray).
fn show_main_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}
