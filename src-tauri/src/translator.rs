use crate::error::{AppError, AppResult};
use crate::types::ProgressEvent;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::time::timeout;

/// Upper bound for the whole VOT stage: vot-cli-live must download the video's
/// audio itself, so long videos need considerably more than the old 300s.
const VOT_TIMEOUT: Duration = Duration::from_secs(600);
const VOT_PACKAGE: &str = "npm:vot-cli-live@1.7.5";
const MAX_STDERR_BYTES: usize = 4 * 1024 * 1024;
/// How often the heartbeat event is emitted while vot-cli-live is running.
const VOT_HEARTBEAT: Duration = Duration::from_secs(5);
static NEXT_WORK_ID: AtomicU64 = AtomicU64::new(1);

/// Run vot-cli-live via deno to get a Russian voice translation for the
/// given YouTube video.
///
/// `progress` receives heartbeat and stderr-line events so the UI log keeps
/// moving during this otherwise silent stage.
///
/// Returns `Ok(Some(mp3_path))` on success, `Ok(None)` if vot-cli-live
/// exits cleanly but no translation was found (fallback — video not in
/// Yandex cache), or `Err(...)` on subprocess/timeout/io failure.
pub async fn fetch_translation(
    video_url: &str,
    output_dir: &Path,
    progress: Option<tokio::sync::mpsc::UnboundedSender<ProgressEvent>>,
) -> AppResult<Option<PathBuf>> {
    crate::downloader::validate_url(video_url)?;
    std::fs::create_dir_all(output_dir).map_err(AppError::Io)?;
    let work_dir = unique_work_dir(output_dir)?;

    // Heartbeat keeps the UI log alive while vot-cli-live works silently.
    let heartbeat = progress
        .as_ref()
        .map(|tx| tokio::spawn(vot_heartbeat(tx.clone())));

    let result = timeout(
        VOT_TIMEOUT,
        run_vot_command(video_url, output_dir, &work_dir, progress),
    )
    .await;

    if let Some(task) = heartbeat {
        task.abort();
    }
    let _ = std::fs::remove_dir_all(&work_dir);

    match result {
        Ok(Ok(result)) => Ok(result),
        Ok(Err(AppError::Subprocess(msg))) if is_no_translation(&msg) => {
            // vot-cli-live reports "no cached translation on Yandex side" in
            // two shapes (see `is_no_translation`) — normal fallback.
            Ok(None)
        }
        Ok(Err(e)) => Err(e),
        Err(_) => {
            // Timeout occurred — handle gracefully.
            Err(AppError::Subprocess(format!(
                "VOT translation timed out after {} seconds",
                VOT_TIMEOUT.as_secs()
            )))
        }
    }
}

/// Periodic "still alive" event so the user sees the stage is running.
async fn vot_heartbeat(tx: tokio::sync::mpsc::UnboundedSender<ProgressEvent>) {
    let mut interval = tokio::time::interval(VOT_HEARTBEAT);
    interval.tick().await; // first tick fires immediately — skip it
    let mut elapsed: u64 = 0;
    loop {
        interval.tick().await;
        elapsed += VOT_HEARTBEAT.as_secs();
        let _ = tx.send(ProgressEvent {
            operation: "vot".into(),
            percent: 0.0,
            message: format!(
                "VOT: waiting for Yandex translation... {elapsed}s / {}s limit",
                VOT_TIMEOUT.as_secs()
            ),
        });
    }
}

/// Match the shapes of vot-cli-live's "no cached translation" failures:
///  - older versions: `Downloading failed! Link "..." not found`
///  - current (1.7.5): `Translation not available for this video`
///
/// A bare `contains("not found")` would also swallow unrelated infra errors
/// ("Module not found", etc.), silently skipping real failures.
fn is_no_translation(stderr_msg: &str) -> bool {
    let m = stderr_msg.to_lowercase();
    m.contains("translation not available")
        || (m.contains("downloading failed!") && m.contains("not found"))
}

async fn run_vot_command(
    video_url: &str,
    output_dir: &Path,
    work_dir: &Path,
    progress: Option<tokio::sync::mpsc::UnboundedSender<ProgressEvent>>,
) -> AppResult<Option<PathBuf>> {
    let home = std::env::var_os("HOME").ok_or_else(|| {
        AppError::Subprocess("HOME is not set, cannot resolve the deno cache".into())
    })?;
    let deno_cache = PathBuf::from(&home).join(".cache/deno");

    let mut cmd = Command::new(crate::binaries::ensure_deno().await?);
    cmd.arg("run")
        .arg("--allow-net")
        .arg("--allow-env")
        .arg(format!(
            "--allow-read={},{}",
            work_dir.display(),
            deno_cache.display()
        ))
        .arg(format!("--allow-write={}", work_dir.display()))
        .arg(VOT_PACKAGE)
        .arg("--quiet")
        .arg("--output")
        .arg(work_dir)
        .arg("--voice-style")
        .arg("live")
        .arg(video_url)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", &home)
        .env("TERM", "xterm")
        .env("CI", "1")
        .env("FORCE_COLOR", "1")
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    let mut child = cmd
        .spawn()
        .map_err(|e| AppError::Subprocess(format!("failed to run deno: {e}")))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| AppError::Subprocess("failed to capture deno stderr".into()))?;
    // Stream stderr lines to the UI log (vot-cli-live reports its progress
    // there) while retaining a capped buffer for error reporting.
    let stderr_task = tokio::spawn(async move {
        let mut reader = BufReader::new(stderr);
        let mut line = String::new();
        let mut retained: Vec<u8> = Vec::with_capacity(8192);
        loop {
            line.clear();
            match reader.read_line(&mut line).await {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    if retained.len() < MAX_STDERR_BYTES {
                        retained.extend_from_slice(line.as_bytes());
                    }
                    let trimmed = line.trim();
                    if !trimmed.is_empty() {
                        if let Some(ref tx) = progress {
                            let _ = tx.send(ProgressEvent {
                                operation: "vot".into(),
                                percent: 0.0,
                                message: format!("VOT: {trimmed}"),
                            });
                        }
                    }
                }
            }
        }
        retained
    });
    let status = child
        .wait()
        .await
        .map_err(|e| AppError::Subprocess(format!("failed to run deno: {e}")))?;
    let stderr = stderr_task.await.unwrap_or_default();

    if !status.success() {
        let stderr = String::from_utf8_lossy(&stderr);
        return Err(AppError::Subprocess(format!(
            "vot-cli-live failed: {stderr}"
        )));
    }

    let generated = find_mp3_output(work_dir)?;
    let extension = generated
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("mp3");
    let work_name = work_dir
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| AppError::Subprocess("invalid VOT work directory name".into()))?;
    let final_path = output_dir.join(format!(".{work_name}-translation.{extension}"));
    std::fs::rename(generated, &final_path).map_err(AppError::Io)?;
    Ok(Some(final_path))
}

/// Look for the most recently created regular audio file in an isolated work dir.
fn find_mp3_output(dir: &Path) -> AppResult<PathBuf> {
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .map_err(AppError::Io)?
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_type().ok().is_some_and(|kind| kind.is_file())
                && e.path()
                    .extension()
                    .is_some_and(|ext| ext == "mp3" || ext == "m4a")
        })
        .collect();

    if entries.is_empty() {
        return Err(AppError::Subprocess(
            "vot-cli-live completed but no output MP3 found".into(),
        ));
    }

    entries.sort_by_key(|e| e.path().metadata().ok().and_then(|m| m.modified().ok()));

    entries
        .last()
        .map(|entry| entry.path())
        .ok_or_else(|| AppError::Subprocess("translation output disappeared".into()))
}

fn unique_work_dir(output_dir: &Path) -> AppResult<PathBuf> {
    for attempt in 0..100 {
        let candidate = output_dir.join(format!(
            ".vot-work-{}-{}-{attempt}",
            std::process::id(),
            NEXT_WORK_ID.fetch_add(1, Ordering::Relaxed)
        ));
        match std::fs::create_dir(&candidate) {
            Ok(()) => return Ok(candidate),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(AppError::Io(error)),
        }
    }
    Err(AppError::Subprocess(
        "could not allocate a unique VOT work directory".into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::is_no_translation;

    #[test]
    fn matches_no_translation_shapes() {
        // vot-cli-live >= 1.7.x
        assert!(is_no_translation(
            "vot-cli-live failed: Translation not available for this video"
        ));
        // older vot-cli-live shape
        assert!(is_no_translation(
            "vot-cli-live failed: Downloading failed! Link \"https://...\" not found"
        ));
    }

    #[test]
    fn does_not_swallow_real_errors() {
        assert!(!is_no_translation(
            "vot-cli-live failed: error: Module not found"
        ));
        assert!(!is_no_translation(
            "vot-cli-live failed: network unreachable"
        ));
        assert!(!is_no_translation(""));
    }
}
