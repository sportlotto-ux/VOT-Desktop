//! VotDesktop — Tauri backend entrypoint.

use std::path::PathBuf;
use tauri::Emitter;

mod binaries;
mod commands;
mod deps;
mod downloader;
mod error;
mod gemini;
mod mixer;
mod pipeline;
mod process;
mod translator;
mod types;
mod updates;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
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

            // Background update check (rate-limited to once/day).
            let app_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let cache_root = binaries::cache_file_for("").parent().map(PathBuf::from);
                if let Some(root) = cache_root {
                    let updates = crate::updates::check_updates(&root).await;
                    if !updates.is_empty() {
                        let _ = app_handle.emit("update-available", &updates);
                    }
                }
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::fetch_formats,
            commands::start_process,
            commands::cookies_info,
            commands::check_updates,
            commands::update_binary,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
