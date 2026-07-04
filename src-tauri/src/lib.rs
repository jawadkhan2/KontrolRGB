mod admin;
mod commands;
mod device;
mod effects;
mod fan;
mod gpu_telemetry;
mod persistence;
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
        .manage(app_state.clone())
        .setup(move |app| {
            let handle = app.handle().clone();
            persistence::load_and_apply(&handle, &app_state);
            tauri::async_runtime::spawn(effects::engine::run(handle.clone(), app_state.clone()));
            tauri::async_runtime::spawn(persistence::run_saver(handle, app_state.clone()));

            build_tray(app.handle(), app_state.clone())?;
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
            commands::apply_to_all,
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

/// Show, unminimize, and focus the main window (used by the tray).
fn show_main_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}
