mod admin;
mod commands;
mod device;
mod effects;
mod fan;
mod gpu_telemetry;
mod logging;
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
    logging::init();
    // If we were spawned by our own "relaunch as admin", the old unelevated
    // process still owns the HID handles, the fan driver and the single-instance
    // mutex. Block until it's gone before we touch any of it.
    admin::wait_for_parent_exit();

    tauri::Builder::default()
        // Registered first, deliberately: plugin setups run in registration
        // order, so this one claims the mutex — and bounces a second launch back
        // into the running instance — before anything else initialises. Two
        // processes driving the EC and the RGB controllers at once corrupts both,
        // so nothing that touches hardware may run ahead of this. That includes
        // `AppState::new()`, which probes the devices; it is built inside `setup`
        // (below) rather than here, because a bounced instance exits during
        // plugin setup and must never have opened a device handle.
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            show_main_window(app);
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        // Launch-on-boot. When the OS starts us from the Run key we pass
        // `--minimized`; the window then stays hidden and the app lives only in
        // the system tray (see below). A manual launch has no such flag, so the
        // window shows normally.
        .plugin(
            tauri_plugin_autostart::Builder::new()
                .args(["--minimized"])
                .build(),
        )
        .setup(|app| {
            // First hardware contact: DeviceManager::new() probes the USB/HID
            // controllers. Safe here — the single-instance plugin has already
            // run, so we know we're the only process.
            let app_state = Arc::new(AppState::new());
            app.manage(app_state.clone());

            let handle = app.handle().clone();
            persistence::load_and_apply(&handle, &app_state);
            // Crash-recovery: if a prior unclean exit left the marker behind, the
            // in-process watchdog never got to run, so reclaim any stranded mapped
            // fan back to the BIOS now. Runs AFTER load_and_apply so the persisted
            // pwm_map is known.
            if let Ok(dir) = handle.path().app_data_dir() {
                app_state.fan.init_recovery(dir.join("fan_manual.lock"));
            }
            // Conflict guard runs its first pass right away (before the engine
            // has anything to write), so a boot launch with no visible window
            // still clears competing RGB/fan software and enables hardware.
            process_guard::spawn_enforcer(handle.clone(), app_state.clone());
            tauri::async_runtime::spawn(effects::engine::run(handle.clone(), app_state.clone()));
            spawn_device_watch(handle.clone(), app_state.clone());
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
            // a boot launch never flashes on screen. A manual launch shows it
            // normally. A boot launch (--minimized, from the autostart Run entry
            // or carried through the admin relaunch) never shows it at all: no
            // window, no taskbar button, just the tray icon. Clicking the tray
            // icon opens it.
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
                if let Some(state) = window.try_state::<Arc<AppState>>() {
                    state
                        .ui_visible
                        .store(false, std::sync::atomic::Ordering::Relaxed);
                }
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
            commands::ack_rgb_conflicts,
            commands::get_guard_config,
            commands::set_guard_config,
            commands::open_log_folder,
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

/// Wake / display-on recovery. Deliberately *non-destructive*: working devices
/// are never torn down (a transient probe failure used to swap the board for a
/// mock permanently). Instead every device is told to re-assert its state — the
/// MSI writer re-arms direct mode, the keyboard repaints, the GPU re-sends its
/// colour — and the fan loop re-applies its duties (the Super-I/O drops manual
/// mode across S3). Backends whose USB handle died reconnect on their own; any
/// slot that is still a mock gets a few re-probe attempts while USB settles.
#[cfg(windows)]
fn wake_recovery(app: &tauri::AppHandle, state: &Arc<AppState>) {
    crate::klog!("power-watch: wake/display-on — resyncing RGB devices and fan control");
    state.request_resync();
    state.fan.on_resume();

    for attempt in 0..5 {
        if attempt > 0 {
            std::thread::sleep(std::time::Duration::from_secs(3));
        }
        if state.manager.lock().missing_slots().is_empty() {
            return;
        }
        commands::retry_missing_devices(app, state, true);
    }
}

/// Background hot-plug watcher: every `DEVICE_WATCH_EVERY`, re-probe slots that
/// are on a mock but whose real hardware this user has saved config for. This
/// is what brings a board/keyboard back if it was missing at boot or after a
/// manual rescan, with no user action and no wake event required.
const DEVICE_WATCH_EVERY: std::time::Duration = std::time::Duration::from_secs(20);

fn spawn_device_watch(app: tauri::AppHandle, state: Arc<AppState>) {
    let _ = std::thread::Builder::new()
        .name("device-watch".into())
        .spawn(move || loop {
            std::thread::sleep(DEVICE_WATCH_EVERY);
            commands::retry_missing_devices(&app, &state, true);
        });
}

/// Show, unminimize, and focus the main window (used by the tray).
fn show_main_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
        if let Some(state) = app.try_state::<Arc<AppState>>() {
            state
                .ui_visible
                .store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }
}
