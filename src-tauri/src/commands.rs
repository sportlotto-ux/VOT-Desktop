//! Tauri command handlers exposed to the frontend.

use crate::downloader;
use crate::error::{AppError, AppResult};
use crate::pipeline::{self, SelectionKind};
use crate::types::{Format, ProgressEvent};
use serde::Deserialize;
use serde::Serialize;
use std::path::PathBuf;
use std::sync::Arc;
use tauri::Emitter;

// ---- Format fetching ----

#[derive(Serialize)]
pub struct FetchedFormatsResponse {
    pub formats: Vec<Format>,
    pub description: Option<String>,
}

#[derive(Deserialize)]
pub struct FetchFormatsRequest {
    pub url: String,
    pub cookies_path: Option<String>,
}

#[tauri::command]
pub async fn fetch_formats(request: FetchFormatsRequest) -> AppResult<FetchedFormatsResponse> {
    let cookies = request.cookies_path.as_ref().map(PathBuf::from);
    let info = downloader::fetch_formats(&request.url, cookies.as_deref()).await?;
    Ok(FetchedFormatsResponse {
        formats: info.formats,
        description: info.description,
    })
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

/// Versions of the runtime components, shown in the UI. Queried actively by
/// the frontend on startup (a startup-time push event races webview load).
#[derive(Serialize)]
pub struct RuntimeVersions {
    pub ytdlp: Option<String>,
    pub deno: Option<String>,
    /// None when ffmpeg is missing or broken.
    pub ffmpeg: Option<String>,
}

#[tauri::command]
pub async fn runtime_versions() -> RuntimeVersions {
    // check_ffmpeg spawns a process; keep it off the async executor.
    let ffmpeg = tokio::task::spawn_blocking(crate::deps::check_ffmpeg)
        .await
        .ok()
        .and_then(|r| r.ok());
    RuntimeVersions {
        ytdlp: crate::binaries::current_version("yt-dlp").await,
        deno: crate::binaries::current_version("deno").await,
        ffmpeg,
    }
}

// ---- AI description translation (manual, user-triggered) ----

#[derive(Deserialize)]
pub struct TranslateDescriptionRequest {
    pub description: String,
    pub api_key: String,
    /// Model id from `gemini_models`; empty/unknown -> backend default.
    #[serde(default)]
    pub model: Option<String>,
    /// When set, the translation is written to `<dir>/description.ru.txt`.
    pub save_dir: Option<String>,
}

/// Translate a video description to Russian via Google AI Studio.
/// Returns the translated text.
#[tauri::command]
pub async fn translate_description(request: TranslateDescriptionRequest) -> AppResult<String> {
    let text = crate::gemini::translate_description(
        &request.description,
        &request.api_key,
        request.model.as_deref(),
    )
    .await?;

    if let Some(dir) = request.save_dir.as_deref().filter(|d| !d.trim().is_empty()) {
        let path = PathBuf::from(dir).join("description.ru.txt");
        std::fs::write(&path, &text).map_err(AppError::Io)?;
    }
    Ok(text)
}

/// Model ids available to this API key (for the UI dropdown).
#[tauri::command]
pub async fn gemini_models(api_key: String) -> AppResult<Vec<String>> {
    crate::gemini::list_models(&api_key).await
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
