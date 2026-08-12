use crate::error::{AppError, AppResult};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tokio::process::Command;
use tokio::time::timeout;

const VOT_TIMEOUT: Duration = Duration::from_secs(300);
const VOT_PACKAGE: &str = "npm:vot-cli-live@1.7.5";
const MAX_STDERR_BYTES: usize = 4 * 1024 * 1024;
static NEXT_WORK_ID: AtomicU64 = AtomicU64::new(1);

/// Run vot-cli-live via deno to get a Russian voice translation for the
/// given YouTube video.
///
/// Returns `Ok(Some(mp3_path))` on success, `Ok(None)` if vot-cli-live
/// exits cleanly but no translation was found (fallback — video not in
/// Yandex cache), or `Err(...)` on subprocess/timeout/io failure.
pub async fn fetch_translation(video_url: &str, output_dir: &Path) -> AppResult<Option<PathBuf>> {
    crate::downloader::validate_url(video_url)?;
    std::fs::create_dir_all(output_dir).map_err(AppError::Io)?;
    let work_dir = unique_work_dir(output_dir)?;

    let result = timeout(
        VOT_TIMEOUT,
        run_vot_command(video_url, output_dir, &work_dir),
    )
    .await;
    let _ = std::fs::remove_dir_all(&work_dir);

    match result {
        Ok(Ok(result)) => Ok(result),
        Ok(Err(AppError::Subprocess(msg))) if msg.contains("not found") => {
            // vot-cli-live exits with "not found" when Yandex has no
            // translation cached — this is a normal fallback.
            Ok(None)
        }
        Ok(Err(e)) => Err(e),
        Err(_) => {
            // Timeout occurred — handle gracefully.
            Err(AppError::Subprocess(
                "VOT translation timed out after 300 seconds".into(),
            ))
        }
    }
}

async fn run_vot_command(
    video_url: &str,
    output_dir: &Path,
    work_dir: &Path,
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
    let stderr_task =
        tokio::spawn(async move { crate::process::read_capped(stderr, MAX_STDERR_BYTES).await });
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
