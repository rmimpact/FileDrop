mod backend;

use std::{path::PathBuf, sync::Arc};

use backend::{AppInfo, AppSettings, Backend, BackendEvent, NearbyDevice, SelectedFileInfo};
use tauri::{Emitter, Manager};

struct ManagedBackend(Arc<Backend>);

#[tauri::command]
fn get_app_info(state: tauri::State<'_, ManagedBackend>) -> AppInfo {
    state.0.app_info()
}

#[tauri::command]
fn get_nearby_devices(state: tauri::State<'_, ManagedBackend>) -> Vec<NearbyDevice> {
    state.0.nearby_devices()
}

#[tauri::command]
fn inspect_files(
    state: tauri::State<'_, ManagedBackend>,
    paths: Vec<String>,
) -> Result<Vec<SelectedFileInfo>, String> {
    state
        .0
        .inspect_files(paths.into_iter().map(PathBuf::from).collect())
}

#[tauri::command]
fn set_discoverable(
    state: tauri::State<'_, ManagedBackend>,
    enabled: bool,
) -> Result<AppSettings, String> {
    state.0.set_discoverable(enabled)
}

#[tauri::command]
fn set_auto_open_received(
    state: tauri::State<'_, ManagedBackend>,
    enabled: bool,
) -> Result<AppSettings, String> {
    state.0.set_auto_open_received(enabled)
}

#[tauri::command]
fn send_files(
    state: tauri::State<'_, ManagedBackend>,
    device_id: String,
    paths: Vec<String>,
) -> Result<String, String> {
    state
        .0
        .clone()
        .send_files(&device_id, paths.into_iter().map(PathBuf::from).collect())
}

#[tauri::command]
fn respond_to_offer(
    state: tauri::State<'_, ManagedBackend>,
    transfer_id: String,
    accepted: bool,
) -> Result<(), String> {
    state.0.respond_to_offer(&transfer_id, accepted)
}

#[tauri::command]
fn cancel_transfer(
    state: tauri::State<'_, ManagedBackend>,
    transfer_id: String,
) -> Result<(), String> {
    state.0.cancel_transfer(&transfer_id)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let app_data_directory = app.path().app_data_dir()?;
            let download_directory =
                dirs::download_dir().unwrap_or_else(|| app_data_directory.join("Downloads"));
            let app_handle = app.handle().clone();
            let emit = Arc::new(move |event: BackendEvent| match event {
                BackendEvent::DevicesChanged(devices) => {
                    let _ = app_handle.emit("devices-changed", devices);
                }
                BackendEvent::SettingsChanged(settings) => {
                    let _ = app_handle.emit("settings-changed", settings);
                }
                BackendEvent::NetworkStatusChanged(online) => {
                    let _ = app_handle.emit("network-status-changed", online);
                }
                BackendEvent::IncomingOffer(offer) => {
                    let _ = app_handle.emit("incoming-offer", offer);
                    if let Some(window) = app_handle.get_webview_window("main") {
                        let _ = window.unminimize();
                        let _ = window.show();
                        let _ = window.set_focus();
                    }
                }
                BackendEvent::TransferProgress(progress) => {
                    let _ = app_handle.emit("transfer-progress", progress);
                }
                BackendEvent::TransferFinished(finished) => {
                    let _ = app_handle.emit("transfer-finished", finished);
                }
                BackendEvent::TransferFailed(failed) => {
                    let _ = app_handle.emit("transfer-failed", failed);
                }
            });
            let backend = Backend::start(&app_data_directory, download_directory, emit)
                .map_err(std::io::Error::other)?;
            app.manage(ManagedBackend(backend));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_app_info,
            get_nearby_devices,
            inspect_files,
            set_discoverable,
            set_auto_open_received,
            send_files,
            respond_to_offer,
            cancel_transfer,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
