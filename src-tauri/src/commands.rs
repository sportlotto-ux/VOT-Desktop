//! Tauri command handlers exposed to the frontend.

use crate::downloader;
use crate::error::AppResult;
use crate::pipeline::{self, SelectionKind};
use crate::types::{Format, ProgressEvent};
use serde::Deserialize;
use serde::Serialize;
use std::path::PathBuf;
use std::sync::Arc;
use tauri::Emitter;

// ---- Ping ----

#[derive(Serialize)]
pub struct PingResponse {
    pub message: String,
}

#[tauri::command]
pub fn ping() -> PingResponse {
    PingResponse {
        message: "pong from vot_desktop".to_string(),
    }
}

// ---- Format fetching ----

#[derive(Deserialize)]
pub struct FetchFormatsRequest {
    pub url: String,
    pub cookies_path: Option<String>,
}

#[tauri::command]
pub async fn fetch_formats(request: FetchFormatsRequest) -> AppResult<Vec<Format>> {
    let cookies = request.cookies_path.as_ref().map(PathBuf::from);
    downloader::fetch_formats(&request.url, cookies.as_deref()).await
}

#[derive(Deserialize)]
pub struct StartDownloadRequest {
    pub url: String,
    pub format_id: String,
    pub output_dir: String,
    pub cookies_path: Option<String>,
}

#[tauri::command]
pub async fn start_download(
    request: StartDownloadRequest,
    window: tauri::Window,
) -> AppResult<String> {
    let output_dir = expand_tilde(&request.output_dir);
    let cookies = request.cookies_path.as_ref().map(PathBuf::from);
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<ProgressEvent>();
    let w = window.clone();
    tokio::spawn(async move {
        while let Some(ev) = rx.recv().await {
            let _ = w.emit("process-progress", &ev);
        }
    });
    let result = downloader::download(
        &request.url,
        &request.format_id,
        &output_dir,
        cookies.as_deref(),
        Some(tx),
    )
    .await?;
    let path_str = result.to_string_lossy().to_string();
    let _ = window.emit("download-complete", &path_str);
    Ok(path_str)
}

#[derive(Deserialize)]
pub struct StartTranslateRequest {
    pub url: String,
    pub output_dir: String,
}

#[tauri::command]
pub async fn start_translate(
    request: StartTranslateRequest,
    window: tauri::Window,
) -> AppResult<Option<String>> {
    let output_dir = expand_tilde(&request.output_dir);
    let _ = window.emit("translation-progress", "Starting VOT translation...");
    match crate::translator::fetch_translation(&request.url, &output_dir).await {
        Ok(Some(path)) => {
            let path_str = path.to_string_lossy().to_string();
            let _ = window.emit("translation-complete", &path_str);
            Ok(Some(path_str))
        }
        Ok(None) => {
            let _ = window.emit("translation-not-found", ());
            Ok(None)
        }
        Err(e) => {
            let _ = window.emit("translation-error", &e.to_string());
            Err(e)
        }
    }
}

#[derive(Deserialize)]
pub struct ProcessRequest {
    pub url: String,
    pub format_id: String,
    pub kind: SelectionKind,
    pub output_dir: String,
    pub cookies_path: Option<String>,
    pub do_translate: bool,
}

#[derive(Serialize)]
pub struct CookieInfo {
    pub path: String,
    pub modified_secs: Option<i64>,
}

/// Return the last-modified time of a cookies file so the UI can warn
/// when the cookie file is stale (ADR-005).
#[tauri::command]
pub fn cookies_info(path: String) -> AppResult<CookieInfo> {
    let p = PathBuf::from(&path);
    downloader::validate_cookies_path(&p)?;
    let modified_secs = std::fs::symlink_metadata(&p)
        .ok()
        .and_then(|meta| meta.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64);
    Ok(CookieInfo {
        path,
        modified_secs,
    })
}

/// Check (once/day) whether yt-dlp/deno have newer versions available.
#[tauri::command]
pub async fn check_updates() -> AppResult<Vec<crate::updates::UpdateInfo>> {
    let cache_root = crate::binaries::cache_file_for("")
        .parent()
        .map(PathBuf::from);
    let root =
        cache_root.ok_or_else(|| crate::error::AppError::Subprocess("no cache dir".into()))?;
    Ok(crate::updates::check_updates(&root).await)
}

/// Download the latest release of `name` (yt-dlp|deno) into the cache.
#[tauri::command]
pub async fn update_binary(name: String) -> AppResult<String> {
    crate::updates::update_binary(&name).await
}

fn expand_tilde(path: &str) -> PathBuf {
    if path == "~" {
        return dirs::home_dir().unwrap_or_else(|| PathBuf::from(path));
    }
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(path)
}

#[tauri::command]
pub async fn start_process(
    request: ProcessRequest,
    window: tauri::Window,
) -> AppResult<pipeline::ProcessResponse> {
    let output_dir = expand_tilde(&request.output_dir);
    let cookies = request.cookies_path.as_ref().map(PathBuf::from);

    // Shared progress channel forwarded to the UI.
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<ProgressEvent>();
    let w = window.clone();
    tokio::spawn(async move {
        while let Some(ev) = rx.recv().await {
            let _ = w.emit("process-progress", &ev);
        }
    });

    let ctx = pipeline::ProcessContext {
        url: request.url,
        format_id: request.format_id,
        kind: request.kind,
        output_dir,
        cookies,
        do_translate: request.do_translate,
        progress: tx,
        events: Arc::new(window),
    };
    pipeline::run_process(ctx).await
}
