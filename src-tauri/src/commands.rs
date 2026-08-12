//! Tauri command handlers exposed to the frontend.

use crate::downloader;
use crate::error::AppResult;
use crate::types::{Format, ProgressEvent};
use serde::Deserialize;
use serde::Serialize;
use std::path::PathBuf;
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
    pub output_dir: String,
    pub cookies_path: Option<String>,
    pub do_translate: bool,
}

#[derive(Serialize)]
pub struct ProcessResponse {
    pub video_path: String,
    pub translation_path: Option<String>,
    pub mixed_path: Option<String>,
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
) -> AppResult<ProcessResponse> {
    let output_dir = expand_tilde(&request.output_dir);
    std::fs::create_dir_all(&output_dir).map_err(crate::error::AppError::Io)?;
    let cookies = request.cookies_path.as_ref().map(PathBuf::from);

    // Create a shared progress channel.
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<ProgressEvent>();
    let w = window.clone();
    tokio::spawn(async move {
        while let Some(ev) = rx.recv().await {
            let _ = w.emit("process-progress", &ev);
        }
    });

    // Audio-only if format starts with bestaudio/ba or is a pure audio ID from yt-dlp
    let fid = &request.format_id;
    let is_audio_only = fid.starts_with("bestaudio")
        || fid.starts_with("ba")
        || fid == "A"
        || (!fid.contains("+") && !fid.contains("bestvideo") && !fid.contains("bv"));
    let is_video = !is_audio_only;

    let work_dir = if request.do_translate && is_video {
        Some(create_work_dir(&output_dir)?)
    } else {
        None
    };
    let download_dir = work_dir.as_deref().unwrap_or(&output_dir);

    let video_path: PathBuf;
    let audio_path: Option<PathBuf>;

    if is_video {
        // Step 1a: download video stream.
        // When translation is requested, strip the audio portion from the
        // format spec — we'll download it separately in step 1b.
        let _ = window.emit("process-step", "Downloading video...");
        let video_fid = if request.do_translate {
            video_only_format(&request.format_id)
        } else {
            request.format_id.clone()
        };
        let video_result = downloader::download(
            &request.url,
            &video_fid,
            download_dir,
            cookies.as_deref(),
            Some(tx.clone()),
        )
        .await;
        video_path = match video_result {
            Ok(path) => path,
            Err(error) => {
                cleanup_work_dir(work_dir.as_deref());
                return Err(error);
            }
        };

        if request.do_translate {
            // Step 1b: download best audio separately for the mixer.
            let _ = window.emit("process-step", "Downloading audio...");
            let audio_result = downloader::download(
                &request.url,
                "bestaudio[ext=m4a]/bestaudio",
                download_dir,
                cookies.as_deref(),
                Some(tx.clone()),
            )
            .await;
            audio_path = match audio_result {
                Ok(path) => Some(path),
                Err(error) => {
                    cleanup_work_dir(work_dir.as_deref());
                    return Err(error);
                }
            };
        } else {
            audio_path = None;
        }
    } else {
        // Audio only: just download
        let _ = window.emit("process-step", "Downloading audio...");
        video_path = downloader::download(
            &request.url,
            &request.format_id,
            &output_dir,
            cookies.as_deref(),
            Some(tx.clone()),
        )
        .await?;
        audio_path = None;
    }

    if request.do_translate && is_video {
        // Step 2: translate
        let _ = window.emit("process-step", "Getting Yandex voice translation (VOT)...");
        let translation =
            match crate::translator::fetch_translation(&request.url, download_dir).await {
                Ok(Some(path)) => {
                    let s = path.to_string_lossy().to_string();
                    let _ = window.emit("translation-complete", &s);
                    path
                }
                Ok(None) => {
                    let _ = window.emit("translation-not-found", "VOT translation not available");
                    let fallback = downloader::download(
                        &request.url,
                        &format_with_best_audio(&request.format_id),
                        &output_dir,
                        cookies.as_deref(),
                        Some(tx.clone()),
                    )
                    .await;
                    cleanup_work_dir(work_dir.as_deref());
                    let fallback = fallback?;
                    let fallback_str = fallback.to_string_lossy().to_string();
                    return Ok(ProcessResponse {
                        video_path: fallback_str,
                        translation_path: None,
                        mixed_path: None,
                    });
                }
                Err(e) => {
                    let _ = window.emit("translation-error", &e.to_string());
                    cleanup_work_dir(work_dir.as_deref());
                    return Err(e);
                }
            };

        // Step 3: mix video + original audio + translation → mixed.mp4
        let _ = window.emit("process-step", "Mixing audio tracks...");
        let mixed = output_dir.join(format!(
            "{}.mixed.mp4",
            video_path.file_stem().unwrap_or_default().to_string_lossy(),
        ));
        let orig = audio_path.as_deref().unwrap_or(&video_path);
        if let Err(error) =
            crate::mixer::mix(orig, &translation, &video_path, &mixed, Some(tx.clone())).await
        {
            cleanup_work_dir(work_dir.as_deref());
            return Err(error);
        }
        let _ = window.emit("mix-complete", &mixed.to_string_lossy().to_string());
        cleanup_work_dir(work_dir.as_deref());

        let mixed_str = mixed.to_string_lossy().to_string();
        return Ok(ProcessResponse {
            video_path: mixed_str.clone(),
            translation_path: None,
            mixed_path: Some(mixed_str),
        });
    }

    let video_str = video_path.to_string_lossy().to_string();
    Ok(ProcessResponse {
        video_path: video_str,
        translation_path: None,
        mixed_path: None,
    })
}

/// Strip audio portion from combined format specs.
/// `bestvideo[height<=480]+bestaudio/best` → `bestvideo[height<=480]`
fn video_only_format(fid: &str) -> String {
    fid.split('+').next().unwrap_or(fid).to_string()
}

fn format_with_best_audio(fid: &str) -> String {
    if fid.contains('+') {
        fid.to_string()
    } else {
        format!("{fid}+bestaudio/best")
    }
}

fn create_work_dir(output_dir: &std::path::Path) -> AppResult<PathBuf> {
    for attempt in 0..100 {
        let candidate = output_dir.join(format!(".vot-process-{}-{attempt}", std::process::id()));
        match std::fs::create_dir(&candidate) {
            Ok(()) => return Ok(candidate),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(crate::error::AppError::Io(error)),
        }
    }
    Err(crate::error::AppError::Subprocess(
        "could not allocate a unique process work directory".into(),
    ))
}

fn cleanup_work_dir(work_dir: Option<&std::path::Path>) {
    if let Some(path) = work_dir {
        let _ = std::fs::remove_dir_all(path);
    }
}

#[cfg(test)]
mod tests {
    use super::{format_with_best_audio, video_only_format};

    #[test]
    fn preserves_or_adds_audio_selector() {
        assert_eq!(
            format_with_best_audio("bestvideo+bestaudio"),
            "bestvideo+bestaudio"
        );
        assert_eq!(
            format_with_best_audio("bestvideo"),
            "bestvideo+bestaudio/best"
        );
    }

    #[test]
    fn strips_combined_selector() {
        assert_eq!(video_only_format("bestvideo+bestaudio"), "bestvideo");
    }
}
