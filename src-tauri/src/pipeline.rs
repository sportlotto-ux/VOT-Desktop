//! Orchestrates the end-to-end processing pipeline:
//! download → (translate) → mix, emitting typed events to the UI.

use crate::downloader;
use crate::error::{AppError, AppResult};
use crate::types::ProgressEvent;
use serde::Deserialize;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tauri::Emitter;
use tokio::sync::mpsc::UnboundedSender;

/// The kind of stream the user selected. Sent explicitly by the frontend,
/// replacing string-based heuristics on the format ID.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SelectionKind {
    Video,
    Audio,
}

/// A concrete file produced by a pipeline step.
#[derive(Debug)]
pub struct Artifact {
    pub path: PathBuf,
    pub kind: ArtifactKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtifactKind {
    Video,
    Audio,
    Translation,
    Mixed,
}

impl Artifact {
    fn new(path: PathBuf, kind: ArtifactKind) -> Self {
        Self { path, kind }
    }

    /// Convert this terminal artifact into the IPC response. A `Mixed`
    /// artifact is reported as the mixed path; anything else is a plain
    /// video/audio download.
    fn into_response(self) -> ProcessResponse {
        match self.kind {
            ArtifactKind::Mixed => {
                let s = self.path.to_string_lossy().to_string();
                ProcessResponse {
                    video_path: s.clone(),
                    translation_path: None,
                    mixed_path: Some(s),
                }
            }
            _ => ProcessResponse::plain(self.path.to_string_lossy()),
        }
    }
}

/// UI-facing events emitted along the way. Implemented for `tauri::Window`
/// and replaced by a mock in tests.
pub trait PipelineEvents: Send + Sync {
    fn step(&self, msg: &str);
    fn translation_complete(&self, path: &str);
    fn translation_not_found(&self);
    fn translation_error(&self, err: &str);
    fn mix_complete(&self, path: &str);
}

impl PipelineEvents for tauri::Window {
    fn step(&self, msg: &str) {
        let _ = self.emit("process-step", msg);
    }
    fn translation_complete(&self, path: &str) {
        let _ = self.emit("translation-complete", path);
    }
    fn translation_not_found(&self) {
        let _ = self.emit("translation-not-found", ());
    }
    fn translation_error(&self, err: &str) {
        let _ = self.emit("translation-error", err);
    }
    fn mix_complete(&self, path: &str) {
        let _ = self.emit("mix-complete", path);
    }
}

/// Everything a pipeline run needs: validated inputs, progress channel, UI events.
pub struct ProcessContext {
    pub url: String,
    pub format_id: String,
    pub kind: SelectionKind,
    pub output_dir: PathBuf,
    pub cookies: Option<PathBuf>,
    pub do_translate: bool,
    pub progress: UnboundedSender<ProgressEvent>,
    pub events: Arc<dyn PipelineEvents>,
}

#[derive(Serialize)]
pub struct ProcessResponse {
    pub video_path: String,
    pub translation_path: Option<String>,
    pub mixed_path: Option<String>,
}

impl ProcessResponse {
    fn plain(path: impl Into<String>) -> Self {
        Self {
            video_path: path.into(),
            translation_path: None,
            mixed_path: None,
        }
    }
}

pub async fn run_process(ctx: ProcessContext) -> AppResult<ProcessResponse> {
    std::fs::create_dir_all(&ctx.output_dir).map_err(AppError::Io)?;

    // Translation runs use an isolated work dir so partial streams never
    // leak into the user's output folder; everything else writes in place.
    let (work_dir, download_dir) = if ctx.do_translate {
        let work = create_work_dir(&ctx.output_dir)?;
        (Some(work.clone()), work)
    } else {
        (None, ctx.output_dir.clone())
    };

    let result = match ctx.kind {
        SelectionKind::Video => run_video(&ctx, &download_dir).await,
        SelectionKind::Audio => run_audio(&ctx, &download_dir).await,
    };

    if let Some(work) = work_dir {
        let _ = std::fs::remove_dir_all(work);
    }
    result.map(Artifact::into_response)
}

async fn run_video(ctx: &ProcessContext, download_dir: &Path) -> AppResult<Artifact> {
    let events = &ctx.events;

    if !ctx.do_translate {
        // Single combined download — the format ID already carries audio.
        events.step("Downloading video...");
        let path = download_stream(ctx, &ctx.format_id, download_dir).await?;
        return Ok(Artifact::new(path, ArtifactKind::Video));
    }

    // Translation path: video stream and audio stream are fetched separately
    // so the mixer can layer original + translation.
    events.step("Downloading video...");
    let video = Artifact::new(
        download_stream(ctx, &video_only_format(&ctx.format_id), download_dir).await?,
        ArtifactKind::Video,
    );

    events.step("Downloading audio...");
    let audio = Artifact::new(
        download_stream(ctx, "bestaudio[ext=m4a]/bestaudio", download_dir).await?,
        ArtifactKind::Audio,
    );

    events.step("Getting Yandex voice translation (VOT)...");
    match crate::translator::fetch_translation(&ctx.url, download_dir, Some(ctx.progress.clone()))
        .await
    {
        Ok(Some(path)) => {
            let translation = Artifact::new(path, ArtifactKind::Translation);
            events.translation_complete(&translation.path.to_string_lossy());

            events.step("Mixing audio tracks...");
            // Write next to the downloaded video (its per-video folder),
            // not into the root output dir.
            let mix_dir = video.path.parent().unwrap_or(&ctx.output_dir).to_path_buf();
            let mixed = crate::mixer::mix(
                &video.path,
                &audio.path,
                &translation.path,
                &mix_dir,
                Some(ctx.progress.clone()),
            )
            .await?;
            events.mix_complete(&mixed.to_string_lossy());

            // The per-video folder lives inside the temp work dir during
            // translation runs; publish it into the user's output dir.
            let published = publish_work_artifact(&mixed, &ctx.output_dir)?;
            Ok(Artifact::new(published, ArtifactKind::Mixed))
        }
        Ok(None) => {
            events.translation_not_found();
            // No cached translation on Yandex side — fall back to the combined
            // format so the user still gets video with its original audio.
            let fallback = download_stream(
                ctx,
                &format_with_best_audio(&ctx.format_id),
                &ctx.output_dir,
            )
            .await?;
            Ok(Artifact::new(fallback, ArtifactKind::Video))
        }
        Err(error) => {
            events.translation_error(&error.to_string());
            Err(error)
        }
    }
}

async fn run_audio(ctx: &ProcessContext, download_dir: &Path) -> AppResult<Artifact> {
    ctx.events.step("Downloading audio...");
    let path = download_stream(ctx, &ctx.format_id, download_dir).await?;
    Ok(Artifact::new(path, ArtifactKind::Audio))
}

async fn download_stream(ctx: &ProcessContext, format_id: &str, dir: &Path) -> AppResult<PathBuf> {
    downloader::download(
        &ctx.url,
        format_id,
        dir,
        ctx.cookies.as_deref(),
        Some(ctx.progress.clone()),
    )
    .await
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

/// Move a finished artifact (inside the per-video work folder) into the
/// user's output dir, relocating the whole folder. Returns the new path.
fn publish_work_artifact(file: &Path, output_root: &Path) -> AppResult<PathBuf> {
    let folder = file
        .parent()
        .ok_or_else(|| AppError::Subprocess("artifact has no parent folder".into()))?;
    let name = folder
        .file_name()
        .ok_or_else(|| AppError::Subprocess("work folder has invalid name".into()))?;
    let dest = output_root.join(name);
    if folder != dest {
        // Replace any stale folder left by a previous run of the same video.
        let _ = std::fs::remove_dir_all(&dest);
        std::fs::rename(folder, &dest).map_err(AppError::Io)?;
    }
    Ok(dest.join(
        file.file_name()
            .ok_or_else(|| AppError::Subprocess("artifact has invalid name".into()))?,
    ))
}

fn create_work_dir(output_dir: &Path) -> AppResult<PathBuf> {
    for attempt in 0..100 {
        let candidate = output_dir.join(format!(".vot-process-{}-{attempt}", std::process::id()));
        match std::fs::create_dir(&candidate) {
            Ok(()) => return Ok(candidate),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(AppError::Io(error)),
        }
    }
    Err(AppError::Subprocess(
        "could not allocate a unique process work directory".into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::{format_with_best_audio, video_only_format, SelectionKind};

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

    #[test]
    fn deserializes_selection_kind() {
        let video: SelectionKind = serde_json::from_str("\"video\"").unwrap();
        let audio: SelectionKind = serde_json::from_str("\"audio\"").unwrap();
        assert_eq!(video, SelectionKind::Video);
        assert_eq!(audio, SelectionKind::Audio);
    }
}
