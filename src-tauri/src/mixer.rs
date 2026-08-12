//! MIXER_PRESET_VERSION = 2
//!
//! Adapted from:
//!   Source: /home/user/podman/triada/workspace/mediabot2.0/src/handlers/media_utils.py
//!   Source lines: 369-371
//!   Source sha256: cec6bd93eb6882941af42e9f02d6c837a613d3d0a13da3b125e7b46faa7a1cfa
//!
//! Changes from source: input indices renumbered for 3-input layout
//! (video:0, original_audio:1, translation:2) instead of 2-input
//! (video_with_audio:0, translation:1).

use crate::error::{AppError, AppResult};
use crate::types::ProgressEvent;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::time::timeout;

#[allow(dead_code)]
pub const MIXER_PRESET_VERSION: u32 = 2;
const FFMPEG: &str = "ffmpeg";
const FFPROBE: &str = "ffprobe";
const MIX_TIMEOUT: Duration = Duration::from_secs(2 * 60 * 60);
const PROBE_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_STDERR_BYTES: usize = 4 * 1024 * 1024;
const FILTER_COMPLEX: &str = include_str!("filter_complex.txt");

/// Output container/codec pair chosen from the video stream's codec.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutputProfile {
    pub container: &'static str,
    pub audio_codec: &'static str,
}

impl OutputProfile {
    fn mp4() -> Self {
        Self {
            container: "mp4",
            audio_codec: "aac",
        }
    }
    fn webm() -> Self {
        Self {
            container: "webm",
            audio_codec: "libopus",
        }
    }
}

/// Pick a mux-compatible container based on the video stream codec.
/// VP8/VP9 live in Matroska/WebM; everything else (h264, av1, ...) goes to MP4.
fn output_profile(video_codec: Option<&str>) -> OutputProfile {
    match video_codec {
        Some("vp8" | "vp9") => OutputProfile::webm(),
        _ => OutputProfile::mp4(),
    }
}

/// Mix video + original audio + Russian voice translation into one output file.
///
/// - `original_audio`: bestaudio from yt-dlp (e.g., .m4a)
/// - `translation_mp3`: VOT output (.mp3)
/// - `video`: video-only stream from yt-dlp
/// - `output_dir`: directory receiving the resulting `.mixed.<ext>` file
/// - `progress`: channel to send progress updates
///
/// Returns the path to the mixed output file.
pub async fn mix(
    original_audio: &Path,
    translation_mp3: &Path,
    video: &Path,
    output_dir: &Path,
    progress: Option<tokio::sync::mpsc::UnboundedSender<ProgressEvent>>,
) -> AppResult<PathBuf> {
    std::fs::create_dir_all(output_dir).map_err(AppError::Io)?;

    let video_codec = timeout(PROBE_TIMEOUT, probe_video_codec(video))
        .await
        .ok()
        .flatten();
    let profile = output_profile(video_codec.as_deref());

    let stem = video
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    let output = output_dir.join(format!("{stem}.mixed.{}", profile.container));

    timeout(
        MIX_TIMEOUT,
        mix_inner(
            original_audio,
            translation_mp3,
            video,
            &output,
            profile,
            progress,
        ),
    )
    .await
    .map_err(|_| AppError::Subprocess("ffmpeg mix timed out".into()))??;

    Ok(output)
}

async fn mix_inner(
    original_audio: &Path,
    translation_mp3: &Path,
    video: &Path,
    output: &Path,
    profile: OutputProfile,
    progress: Option<tokio::sync::mpsc::UnboundedSender<ProgressEvent>>,
) -> AppResult<()> {
    // Get input video duration for progress estimation.
    let total_secs = timeout(PROBE_TIMEOUT, get_duration_secs(video))
        .await
        .ok()
        .flatten()
        .unwrap_or(300.0);

    let mut cmd = Command::new(FFMPEG);
    cmd.arg("-i")
        .arg(video) // 0: video (no audio)
        .arg("-i")
        .arg(original_audio) // 1: original audio
        .arg("-i")
        .arg(translation_mp3) // 2: translation
        .arg("-filter_complex")
        .arg(FILTER_COMPLEX.trim_end())
        .arg("-map")
        .arg("0:v")
        .arg("-map")
        .arg("[mix]")
        .arg("-c:v")
        .arg("copy")
        .arg("-c:a")
        .arg(profile.audio_codec)
        .arg("-b:a")
        .arg("192k")
        .arg("-progress")
        .arg("pipe:1") // progress to stdout
        .arg("-y")
        .arg(output)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    let mut child = cmd
        .spawn()
        .map_err(|e| AppError::Subprocess(format!("failed to spawn ffmpeg: {e}")))?;

    // Read stderr concurrently (errors and log messages).
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| AppError::Subprocess("failed to capture ffmpeg stderr".into()))?;
    let stderr_buf = std::sync::Arc::new(tokio::sync::Mutex::new(Vec::new()));
    let stderr_clone = stderr_buf.clone();
    let stderr_task = tokio::spawn(async move {
        let buf = crate::process::read_capped(stderr, MAX_STDERR_BYTES).await;
        *stderr_clone.lock().await = buf;
    });

    // Parse ffmpeg progress from stdout (key=value lines).
    if let Some(stdout) = child.stdout.take() {
        let mut reader = BufReader::new(stdout);
        let mut line = String::new();
        loop {
            line.clear();
            let n = reader
                .read_line(&mut line)
                .await
                .map_err(|e| AppError::Subprocess(format!("read ffmpeg stdout: {e}")))?;
            if n == 0 {
                break;
            }
            let trimmed = line.trim();
            if let Some(time_str) = trimmed.strip_prefix("out_time=") {
                if let Some(secs) = parse_ffmpeg_time(time_str) {
                    let pct = ((secs / total_secs) * 100.0).min(99.0);
                    if let Some(ref tx) = progress {
                        let _ = tx.send(ProgressEvent {
                            operation: "mix".into(),
                            percent: pct,
                            message: format!("Mixing... {:.0}%", pct),
                        });
                    }
                }
            }
        }
    }

    let status = child
        .wait()
        .await
        .map_err(|e| AppError::Subprocess(format!("wait ffmpeg: {e}")))?;

    stderr_task.await.ok();
    let stderr_bytes = stderr_buf.lock().await;
    let stderr = String::from_utf8_lossy(&stderr_bytes).to_string();

    if !status.success() {
        return Err(AppError::Subprocess(format!("ffmpeg mix failed: {stderr}")));
    }

    if !output.exists() {
        return Err(AppError::Subprocess(
            "ffmpeg completed but output file not found".into(),
        ));
    }

    if let Some(ref tx) = progress {
        let _ = tx.send(ProgressEvent {
            operation: "mix".into(),
            percent: 100.0,
            message: "Mix complete".into(),
        });
    }

    Ok(())
}

/// Get the codec name of the first video stream using ffprobe.
async fn probe_video_codec(path: &Path) -> Option<String> {
    let output = Command::new(FFPROBE)
        .arg("-v")
        .arg("error")
        .arg("-select_streams")
        .arg("v:0")
        .arg("-show_entries")
        .arg("stream=codec_name")
        .arg("-of")
        .arg("csv=p=0")
        .arg(path)
        .kill_on_drop(true)
        .output()
        .await
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&output.stdout);
    let codec = s.trim();
    if codec.is_empty() {
        None
    } else {
        Some(codec.to_string())
    }
}

/// Get media duration in seconds using ffprobe.
async fn get_duration_secs(path: &Path) -> Option<f64> {
    let output = Command::new("ffprobe")
        .arg("-v")
        .arg("error")
        .arg("-show_entries")
        .arg("format=duration")
        .arg("-of")
        .arg("csv=p=0")
        .arg(path)
        .kill_on_drop(true)
        .output()
        .await
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&output.stdout);
    s.trim().parse::<f64>().ok()
}

/// Parse ffmpeg time format (HH:MM:SS.mmm or MM:SS.mmm) to seconds.
fn parse_ffmpeg_time(s: &str) -> Option<f64> {
    let parts: Vec<&str> = s.trim().split(':').collect();
    match parts.len() {
        3 => {
            let h: f64 = parts[0].parse().ok()?;
            let m: f64 = parts[1].parse().ok()?;
            let sec: f64 = parts[2].parse().ok()?;
            Some(h * 3600.0 + m * 60.0 + sec)
        }
        2 => {
            let m: f64 = parts[0].parse().ok()?;
            let sec: f64 = parts[1].parse().ok()?;
            Some(m * 60.0 + sec)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{output_profile, parse_ffmpeg_time};

    #[test]
    fn parses_ffmpeg_timestamps() {
        assert_eq!(parse_ffmpeg_time("01:02:03.500"), Some(3723.5));
        assert_eq!(parse_ffmpeg_time("02:03.500"), Some(123.5));
        assert_eq!(parse_ffmpeg_time("invalid"), None);
    }

    #[test]
    fn vp9_maps_to_webm() {
        assert_eq!(output_profile(Some("vp9")).container, "webm");
        assert_eq!(output_profile(Some("vp8")).container, "webm");
        assert_eq!(output_profile(Some("vp9")).audio_codec, "libopus");
    }

    #[test]
    fn h264_and_unknown_map_to_mp4() {
        assert_eq!(output_profile(Some("h264")).container, "mp4");
        assert_eq!(output_profile(Some("av1")).container, "mp4");
        assert_eq!(output_profile(None).container, "mp4");
        assert_eq!(output_profile(Some("h264")).audio_codec, "aac");
    }
}
