//! VotDesktop — Tauri backend entrypoint.

use tauri::{Emitter, Manager};

mod binaries;
mod commands;
mod deps;
mod downloader;
mod error;
mod mixer;
mod process;
mod translator;
mod types;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            if let Ok(resource_dir) = app.path().resource_dir() {
                crate::binaries::init_resource_dir(resource_dir);
            }

            if cfg!(debug_assertions) {
                app.handle().plugin(
                    tauri_plugin_log::Builder::default()
                        .level(log::LevelFilter::Info)
                        .build(),
                )?;
            }

            let app_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                match crate::deps::check_ffmpeg() {
                    Ok(version) => {
                        log::info!("{version}");
                        let _ = app_handle.emit("ffmpeg-status", &version);
                    }
                    Err(e) => {
                        log::error!("{}", e);
                        let _ = app_handle.emit("ffmpeg-missing", &e.to_string());
                    }
                }
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::ping,
            commands::fetch_formats,
            commands::start_download,
            commands::start_translate,
            commands::start_process,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
