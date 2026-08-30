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

    // Mixing runs (checkbox "mix with Russian ozvuchka") need an isolated work
    // dir: intermediate streams land there and only the finished mixed file is
    // published into the user's output dir. Works the same for video and audio
    // downloads — the mix step is type-agnostic.
    let needs_work_dir = ctx.do_translate;
    let (work_dir, download_dir) = if needs_work_dir {
        let work = create_work_dir(&ctx.output_dir)?;
        (Some(work.clone()), work)
    } else {
        (None, ctx.output_dir.clone())
    };

    let result = run_pipeline(&ctx, &download_dir).await;

    if let Some(work) = work_dir {
        // Never destroy a published artifact: if publishing failed, the mixed
        // file still lives in the work dir — deleting it would silently lose
        // the user's download while the UI reported success. On failure the
        // dir (with the mixed file) is kept and the error surfaces to the UI.
        let publish_failed = matches!(&result, Err(AppError::Subprocess(msg))
            if msg.starts_with(PUBLISH_FAILED_PREFIX));
        if !publish_failed {
            let _ = std::fs::remove_dir_all(work);
        }
    }
    result.map(Artifact::into_response)
}

/// Unified pipeline: download what the user picked (video or audio), then —
/// when the mix checkbox is on — fetch the Yandex translation and mix it with
/// the original audio into the matching container. The mix step is identical
/// for video and audio; only the downloaded inputs differ slightly.
async fn run_pipeline(ctx: &ProcessContext, download_dir: &Path) -> AppResult<Artifact> {
    match ctx.kind {
        SelectionKind::Video => {
            if !ctx.do_translate {
                ctx.events.step("Downloading video...");
                let path = download_stream(ctx, &ctx.format_id, download_dir).await?;
                return Ok(Artifact::new(path, ArtifactKind::Video));
            }

            // Mix path: video stream and audio stream are fetched separately
            // so ffmpeg can layer original + translation over the video.
            ctx.events.step("Downloading video...");
            let video = Artifact::new(
                download_stream(ctx, &video_only_format(&ctx.format_id), download_dir).await?,
                ArtifactKind::Video,
            );
            ctx.events.step("Downloading audio...");
            let audio = Artifact::new(
                download_stream(ctx, "bestaudio[ext=m4a]/bestaudio", download_dir).await?,
                ArtifactKind::Audio,
            );

            let translation = fetch_translation(ctx, download_dir).await?;
            let Some(translation) = translation else {
                ctx.events.translation_not_found();
                // No cached translation on Yandex side — fall back to the
                // combined format so the user still gets video with audio.
                let fallback = download_stream(
                    ctx,
                    &format_with_best_audio(&ctx.format_id),
                    &ctx.output_dir,
                )
                .await?;
                return Ok(Artifact::new(fallback, ArtifactKind::Video));
            };

            ctx.events.step("Mixing audio tracks...");
            let mix_dir = video.path.parent().unwrap_or(&ctx.output_dir).to_path_buf();
            let mixed = crate::mixer::mix(
                &video.path,
                &audio.path,
                &translation.path,
                &mix_dir,
                Some(ctx.progress.clone()),
            )
            .await?;
            ctx.events.mix_complete(&mixed.to_string_lossy());

            let published = publish_work_artifact(&mixed, &ctx.output_dir)?;
            Ok(Artifact::new(published, ArtifactKind::Mixed))
        }
        SelectionKind::Audio => {
            ctx.events.step("Downloading audio...");
            let audio = Artifact::new(
                download_stream(ctx, &ctx.format_id, download_dir).await?,
                ArtifactKind::Audio,
            );

            if !ctx.do_translate {
                return Ok(audio);
            }

            let translation = fetch_translation(ctx, download_dir).await?;
            let Some(translation) = translation else {
                ctx.events.translation_not_found();
                // No cached translation — keep the plain original audio file.
                let published = publish_work_artifact(&audio.path, &ctx.output_dir)?;
                return Ok(Artifact::new(published, ArtifactKind::Audio));
            };

            ctx.events.step("Mixing audio tracks...");
            let mix_dir = audio.path.parent().unwrap_or(&ctx.output_dir).to_path_buf();
            let mixed = crate::mixer::mix_audio(
                &audio.path,
                &translation.path,
                &mix_dir,
                Some(ctx.progress.clone()),
            )
            .await?;
            ctx.events.mix_complete(&mixed.to_string_lossy());

            let published = publish_work_artifact(&mixed, &ctx.output_dir)?;
            Ok(Artifact::new(published, ArtifactKind::Mixed))
        }
    }
}

/// Shared step: fetch the Yandex VOT translation into `download_dir`.
/// Returns `Ok(None)` when Yandex has no cached translation for the video.
async fn fetch_translation(
    ctx: &ProcessContext,
    download_dir: &Path,
) -> AppResult<Option<Artifact>> {
    ctx.events.step("Getting Yandex voice translation (VOT)...");
    match crate::translator::fetch_translation(&ctx.url, download_dir, Some(ctx.progress.clone()))
        .await
    {
        Ok(Some(path)) => {
            let translation = Artifact::new(path, ArtifactKind::Translation);
            ctx.events
                .translation_complete(&translation.path.to_string_lossy());
            Ok(Some(translation))
        }
        Ok(None) => Ok(None),
        Err(error) => {
            ctx.events.translation_error(&error.to_string());
            Err(error)
        }
    }
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

/// Error prefix for publish failures. `run_process` checks for it to decide
/// whether the work dir may be deleted (a failed publish must NOT trigger
/// cleanup, or the mixed file would be silently destroyed).
const PUBLISH_FAILED_PREFIX: &str = "publish failed: ";

/// Move a finished artifact (inside the per-video work folder) into the
/// user's output dir, relocating the whole folder. Returns the new path.
///
/// Falls back to a recursive copy when rename fails (e.g. cross-device), and
/// verifies the file actually arrived at the destination — a silent loss here
/// used to delete the only copy during work-dir cleanup.
fn publish_work_artifact(file: &Path, output_root: &Path) -> AppResult<PathBuf> {
    let folder = file
        .parent()
        .ok_or_else(|| AppError::Subprocess("artifact has no parent folder".into()))?;
    let name = folder
        .file_name()
        .ok_or_else(|| AppError::Subprocess("work folder has invalid name".into()))?;
    let dest = output_root.join(name);

    if folder != dest {
        // Canonical compare guards against same-folder false positives
        // (e.g. differing path casing/symlinks).
        let same = match (folder.canonicalize(), dest.canonicalize()) {
            (Ok(a), Ok(b)) => a == b,
            _ => false,
        };
        if !same {
            // Replace any stale folder left by a previous run of the same video.
            let _ = std::fs::remove_dir_all(&dest);
            if std::fs::rename(folder, &dest).is_err() {
                // Cross-device or locked: fall back to copying the folder.
                copy_dir_recursive(folder, &dest)?;
            }
        }
    }

    let final_file = dest.join(
        file.file_name()
            .ok_or_else(|| AppError::Subprocess("artifact has invalid name".into()))?,
    );
    if !final_file.is_file() {
        return Err(AppError::Subprocess(format!(
            "{PUBLISH_FAILED_PREFIX}mixed file missing after publish: {}",
            final_file.display()
        )));
    }
    Ok(final_file)
}

/// Recursive directory copy used as fallback when rename fails
/// (e.g. source and destination on different filesystems).
fn copy_dir_recursive(src: &Path, dst: &Path) -> AppResult<()> {
    std::fs::create_dir_all(dst).map_err(AppError::Io)?;
    for entry in std::fs::read_dir(src).map_err(AppError::Io)? {
        let entry = entry.map_err(AppError::Io)?;
        let entry_ty = entry.file_type().map_err(AppError::Io)?;
        let target = dst.join(entry.file_name());
        if entry_ty.is_dir() {
            copy_dir_recursive(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), &target).map_err(AppError::Io)?;
        }
    }
    Ok(())
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
    use super::{
        format_with_best_audio, video_only_format, PipelineEvents, ProcessContext, SelectionKind,
    };
    use crate::types::ProgressEvent;
    use std::path::Path;
    use std::sync::Arc;
    use tokio::sync::mpsc::unbounded_channel;

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

    // ---- Live regression: audio + do_translate must survive work-dir cleanup ----

    struct NoopEvents;

    impl PipelineEvents for NoopEvents {
        fn step(&self, _: &str) {}
        fn translation_complete(&self, _: &str) {}
        fn translation_not_found(&self) {}
        fn translation_error(&self, _: &str) {}
        fn mix_complete(&self, _: &str) {}
    }

    /// Network-dependent (downloads real audio): run with `cargo test -- --ignored`.
    /// Regression for the deleted-artifact bug: with `do_translate: true` the
    /// audio artifact used to be written into the temporary work dir and then
    /// removed by the cleanup, while the UI reported success.
    #[tokio::test]
    #[ignore]
    async fn audio_with_translate_flag_survives_cleanup() {
        let out = std::env::temp_dir().join(format!("vot-audio-reg-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&out);
        std::fs::create_dir_all(&out).unwrap();

        let (tx, mut rx) = unbounded_channel::<ProgressEvent>();
        tokio::spawn(async move { while rx.recv().await.is_some() {} });

        let ctx = ProcessContext {
            url: "https://youtube.com/watch?v=jNQXAC9IVRw".into(),
            format_id: "140".into(),
            kind: SelectionKind::Audio,
            output_dir: out.clone(),
            cookies: None,
            do_translate: true,
            progress: tx,
            events: Arc::new(NoopEvents),
        };

        let response = super::run_process(ctx)
            .await
            .expect("run_process must succeed");
        let saved = Path::new(&response.video_path).to_path_buf();
        assert!(
            saved.is_file(),
            "artifact must exist on disk after run_process, got: {}",
            response.video_path
        );
        assert!(
            !saved.to_string_lossy().contains(".vot-process-"),
            "artifact must not live in the (deleted) work dir: {}",
            response.video_path
        );

        let _ = std::fs::remove_dir_all(&out);
    }
}
