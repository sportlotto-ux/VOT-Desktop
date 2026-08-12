use crate::error::{AppError, AppResult};
use crate::types::{Format, ProgressEvent, YtDlpVideoInfo};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, SystemTime};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::time::timeout;
use url::Url;

const FETCH_FORMATS_TIMEOUT: Duration = Duration::from_secs(120);
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(2 * 60 * 60);
const MAX_STDERR_BYTES: usize = 4 * 1024 * 1024;

async fn js_runtime_arg() -> AppResult<(String, String)> {
    let deno = crate::binaries::ensure_deno().await?;
    Ok(("--js-runtimes".into(), format!("deno:{}", deno.display())))
}

pub async fn fetch_formats(url: &str, cookies: Option<&Path>) -> AppResult<Vec<Format>> {
    validate_url(url)?;

    let mut cmd = Command::new(crate::binaries::ensure_ytdlp().await?);

    let (js_flag, js_val) = js_runtime_arg().await?;
    cmd.arg("--dump-json")
        .arg("--no-warnings")
        .arg("--no-playlist")
        .arg(&js_flag)
        .arg(&js_val)
        .arg(url)
        .kill_on_drop(true);

    if let Some(cookies_path) = cookies {
        validate_cookies_path(cookies_path)?;
        cmd.arg("--cookies").arg(cookies_path);
    }

    let output = timeout(FETCH_FORMATS_TIMEOUT, cmd.output())
        .await
        .map_err(|_| AppError::Subprocess("yt-dlp format fetch timed out".into()))?
        .map_err(|e| AppError::Subprocess(format!("failed to run yt-dlp: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(AppError::Subprocess(format!(
            "yt-dlp dump-json failed: {stderr}"
        )));
    }

    // yt-dlp --dump-json outputs a single JSON object with a `formats` array.
    let stdout = String::from_utf8_lossy(&output.stdout);
    let info: YtDlpVideoInfo = serde_json::from_str(&stdout)
        .map_err(|e| AppError::Subprocess(format!("failed to parse yt-dlp JSON: {e}")))?;

    let formats: Vec<Format> = info.formats.into_iter().map(Format::from).collect();

    if formats.is_empty() {
        return Err(AppError::Subprocess(
            "yt-dlp returned an empty format list".into(),
        ));
    }
    Ok(formats)
}

pub async fn download(
    url: &str,
    format_id: &str,
    output_dir: &Path,
    cookies: Option<&Path>,
    progress: Option<tokio::sync::mpsc::UnboundedSender<ProgressEvent>>,
) -> AppResult<PathBuf> {
    timeout(
        DOWNLOAD_TIMEOUT,
        download_inner(url, format_id, output_dir, cookies, progress),
    )
    .await
    .map_err(|_| AppError::Subprocess("yt-dlp download timed out".into()))?
}

async fn download_inner(
    url: &str,
    format_id: &str,
    output_dir: &Path,
    cookies: Option<&Path>,
    progress: Option<tokio::sync::mpsc::UnboundedSender<ProgressEvent>>,
) -> AppResult<PathBuf> {
    validate_url(url)?;
    validate_format_id(format_id)?;
    std::fs::create_dir_all(output_dir).map_err(AppError::Io)?;

    let output_path = output_dir.join("%(title).100s_%(id)s.%(ext)s");
    let started_at = SystemTime::now();

    let mut cmd = Command::new(crate::binaries::ensure_ytdlp().await?);
    let (js_flag, js_val) = js_runtime_arg().await?;
    cmd.arg("-f")
        .arg(format_id)
        .arg("-o")
        .arg(&output_path)
        .arg("--no-playlist")
        .arg("--no-part")
        .arg("--newline")
        .arg("--progress")
        .arg("--print")
        .arg("after_move:%(filepath)s")
        .arg(&js_flag)
        .arg(&js_val)
        .arg(url)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    if let Some(cookies_path) = cookies {
        validate_cookies_path(cookies_path)?;
        cmd.arg("--cookies").arg(cookies_path);
    }

    let mut child = cmd
        .spawn()
        .map_err(|e| AppError::Subprocess(format!("failed to spawn yt-dlp: {e}")))?;

    // Read stderr concurrently so the pipe doesn't fill up.
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| AppError::Subprocess("failed to capture yt-dlp stderr".into()))?;
    let stderr_buf = std::sync::Arc::new(tokio::sync::Mutex::new(Vec::new()));
    let stderr_buf_clone = stderr_buf.clone();
    let stderr_task = tokio::spawn(async move {
        let buf = crate::process::read_capped(stderr, MAX_STDERR_BYTES).await;
        *stderr_buf_clone.lock().await = buf;
    });

    // Stream stdout for progress and destination detection.
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| AppError::Subprocess("failed to capture yt-dlp stdout".into()))?;
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    let mut final_path: Option<PathBuf> = None;

    loop {
        line.clear();
        let n = reader
            .read_line(&mut line)
            .await
            .map_err(|e| AppError::Subprocess(format!("read yt-dlp stdout: {e}")))?;
        if n == 0 {
            break;
        }
        let trimmed = line.trim();

        // Parse download progress: [download]  45.2% of ~1.23GiB at ...
        if let Some(pct) = parse_ytdlp_percent(&line) {
            if let Some(ref tx) = progress {
                let _ = tx.send(ProgressEvent {
                    operation: "download".into(),
                    percent: pct,
                    message: trimmed.into(),
                });
            }
        }

        // --print after_move:filepath emits the final absolute path as
        // the very last line — catch it here.
        if !trimmed.starts_with('[')
            && !trimmed.starts_with(|c: char| c.is_ascii_digit() || c == '.')
        {
            let p = PathBuf::from(trimmed);
            if p.is_absolute() {
                final_path = Some(p);
            }
        }
    }

    let status = child
        .wait()
        .await
        .map_err(|e| AppError::Subprocess(format!("wait yt-dlp: {e}")))?;

    stderr_task.await.ok();
    let stderr_bytes = stderr_buf.lock().await;
    let stderr_text = String::from_utf8_lossy(&stderr_bytes);

    if !status.success() {
        return Err(AppError::Subprocess(format!(
            "yt-dlp download failed: {stderr_text}"
        )));
    }

    // --print after_move:filepath gives the definitive final path.
    // Fallback: scan output dir for files containing the video ID,
    // sorted by modification time (newest).
    let video_id = extract_video_id(url).unwrap_or("video");
    let path = final_path
        .filter(|p| p.exists())
        .or_else(|| {
            let mut entries: Vec<_> = std::fs::read_dir(output_dir)
                .ok()?
                .filter_map(|e| e.ok())
                .filter(|e| {
                    let p = e.path();
                    let name = p.to_string_lossy();
                    name.contains(video_id)
                        && !name.ends_with(".part")
                        && !name.contains(".mixed.")
                        && e.file_type().ok().is_some_and(|t| t.is_file())
                        && p.metadata()
                            .ok()
                            .and_then(|m| m.modified().ok())
                            .is_some_and(|modified| modified >= started_at)
                })
                .collect();
            entries.sort_by_key(|e| e.path().metadata().ok().and_then(|m| m.modified().ok()));
            entries.into_iter().last().map(|e| e.path())
        })
        .ok_or_else(|| AppError::Subprocess("could not determine downloaded file path".into()))?;

    Ok(path)
}

/// Parse yt-dlp percentage from a progress line.
/// e.g. "[download]  45.2% of ~1.23GiB at 5.3MiB/s ETA 00:45" → Some(45.2)
fn parse_ytdlp_percent(line: &str) -> Option<f64> {
    let line = line.trim();
    let rest = line.strip_prefix("[download]")?;
    let rest = rest.trim();
    let pct_str = rest.split('%').next()?;
    let pct_str = pct_str.trim();
    pct_str
        .parse::<f64>()
        .ok()
        .filter(|&n| (0.0..=100.0).contains(&n))
}

/// Extract video ID from a YouTube URL.
/// e.g. "https://youtube.com/watch?v=ABC123" -> "ABC123"
fn extract_video_id(url: &str) -> Option<&str> {
    let url = url.trim();
    if let Some(pos) = url.find("v=") {
        let after = &url[pos + 2..];
        let end = after.find(['&', '#', '?']).unwrap_or(after.len());
        Some(&after[..end])
    } else if let Some(pos) = url.find("youtu.be/") {
        let after = &url[pos + 9..];
        let end = after.find(['/', '?', '#']).unwrap_or(after.len());
        Some(&after[..end])
    } else {
        None
    }
}

pub(crate) fn validate_url(url: &str) -> AppResult<()> {
    forbid_chars(url)?;
    let parsed = Url::parse(url.trim())
        .map_err(|_| AppError::InvalidInput("URL must be a valid HTTPS YouTube URL".into()))?;
    let valid_host = matches!(
        parsed.host_str(),
        Some("youtube.com" | "www.youtube.com" | "m.youtube.com" | "youtu.be")
    );
    if parsed.scheme() != "https"
        || !valid_host
        || parsed.port().is_some()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.path() == "/"
    {
        return Err(AppError::InvalidInput(
            "URL must be a valid HTTPS YouTube URL".into(),
        ));
    }
    Ok(())
}

fn validate_format_id(id: &str) -> AppResult<()> {
    forbid_chars(id)?;
    if id.is_empty() || id.len() > 128 {
        return Err(AppError::InvalidInput(
            "format ID must be 1-128 characters".into(),
        ));
    }
    let allowed = |c: char| {
        c.is_ascii_alphanumeric()
            || matches!(
                c,
                '-' | '_'
                    | '.'
                    | '+'
                    | '/'
                    | '['
                    | ']'
                    | '<'
                    | '>'
                    | '='
                    | '('
                    | ')'
                    | '?'
                    | '*'
                    | ':'
            )
    };
    if !id.chars().all(allowed) {
        return Err(AppError::InvalidInput(
            "format ID contains invalid characters".into(),
        ));
    }
    Ok(())
}

fn validate_cookies_path(path: &Path) -> AppResult<()> {
    const MAX_COOKIES_BYTES: u64 = 10 * 1024 * 1024;
    let meta = std::fs::symlink_metadata(path).map_err(|_| {
        AppError::InvalidInput(format!("cookies file not found: {}", path.display()))
    })?;
    if !meta.file_type().is_file() {
        return Err(AppError::InvalidInput(format!(
            "cookies path must be a regular non-symlink file: {}",
            path.display()
        )));
    }
    #[cfg(unix)]
    if meta.uid() != unsafe { libc::geteuid() } {
        return Err(AppError::InvalidInput(format!(
            "cookies file must be owned by the current user: {}",
            path.display()
        )));
    }
    if meta.len() > MAX_COOKIES_BYTES {
        return Err(AppError::InvalidInput(
            "cookies file is larger than 10 MiB".into(),
        ));
    }
    forbid_chars(&path.to_string_lossy())?;
    Ok(())
}

fn forbid_chars(s: &str) -> AppResult<()> {
    if s.contains([';', '|', '`', '$', '\n', '\0']) {
        return Err(AppError::InvalidInput(
            "input contains forbidden characters".into(),
        ));
    }
    Ok(())
}

#[cfg(unix)]
use std::os::unix::fs::MetadataExt;

#[cfg(test)]
mod tests {
    use super::{extract_video_id, validate_cookies_path, validate_url};

    #[test]
    fn accepts_only_https_youtube_urls() {
        assert!(validate_url("https://youtube.com/watch?v=abc").is_ok());
        assert!(validate_url("https://youtu.be/abc").is_ok());
        assert!(validate_url("http://youtube.com/watch?v=abc").is_err());
        assert!(validate_url("https://youtube.com.evil.test/watch?v=abc").is_err());
    }

    #[test]
    fn extracts_video_ids() {
        assert_eq!(
            extract_video_id("https://youtube.com/watch?v=abc&x=1"),
            Some("abc")
        );
        assert_eq!(extract_video_id("https://youtu.be/abc?t=1"), Some("abc"));
    }

    #[cfg(unix)]
    #[test]
    fn rejects_cookie_symlinks() {
        use std::os::unix::fs::symlink;

        let root = std::env::temp_dir().join(format!("vot-desktop-test-{}", std::process::id()));
        let target = root.join("cookies.txt");
        let link = root.join("cookies-link.txt");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(&target, "# Netscape HTTP Cookie File\n").unwrap();
        symlink(&target, &link).unwrap();

        assert!(validate_cookies_path(&link).is_err());

        let _ = std::fs::remove_dir_all(root);
    }
}
