mod commands;
mod device;
mod effects;
mod persistence;
mod state;

use std::sync::Arc;

use state::AppState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app_state = Arc::new(AppState::new());

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(app_state.clone())
        .setup(move |app| {
            let handle = app.handle().clone();
            persistence::load_and_apply(&handle, &app_state);
            tauri::async_runtime::spawn(effects::engine::run(
                handle.clone(),
                app_state.clone(),
            ));
            tauri::async_runtime::spawn(persistence::run_saver(handle, app_state.clone()));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_devices,
            commands::get_device_state,
            commands::set_effect,
            commands::set_brightness,
            commands::set_zone_colors,
            commands::set_led_color,
            commands::resize_zone,
            commands::apply_to_all,
            commands::rescan_devices,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
