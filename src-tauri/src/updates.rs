//! Update checks for the runtime binaries (yt-dlp, deno).
//!
//! Like Parabolic: periodically query GitHub `releases/latest`, and if a newer
//! version exists than the one currently in use (PATH or cache), emit an
//! `update-available` event so the UI can offer a "Update" button. The user
//! triggers the actual download via the `update_binary` command; checksums are
//! taken from the release itself (not hardcoded).
//!
//! Checks are rate-limited to once per day via a marker file in the cache dir.

use crate::error::{AppError, AppResult};
use serde::Deserialize;
use std::path::Path;
use std::time::Duration;

const GITHUB_API: &str = "https://api.github.com/repos";
const CHECK_MARKER: &str = "last-update-check";
const CHECK_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
const HTTP_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, serde::Serialize)]
pub struct UpdateInfo {
    pub name: String,
    pub current: Option<String>,
    pub latest: String,
}

#[derive(Deserialize)]
struct Release {
    #[serde(rename = "tag_name")]
    tag_name: String,
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    #[serde(rename = "browser_download_url")]
    url: String,
}

/// Check whether yt-dlp/deno have newer versions available. Returns the list of
/// available updates (empty when everything is current). Rate-limited to once
/// per day; on network failure returns an empty list (silent degrade).
pub async fn check_updates(cache_root: &Path) -> Vec<UpdateInfo> {
    if touch_check_marker(cache_root).is_err() {
        return Vec::new();
    }

    let mut updates = Vec::new();
    for (name, repo, _) in [
        ("yt-dlp", "yt-dlp/yt-dlp", YTDLP_MIN_VERSION),
        ("deno", "denoland/deno", DENO_MIN_VERSION),
    ] {
        match check_one(name, repo).await {
            Ok(Some(info)) if info.current.as_deref() != Some(info.latest.as_str()) => {
                updates.push(info);
            }
            Ok(_) => {}
            Err(err) => {
                log::warn!("update check for {name} failed: {err}");
            }
        }
    }
    updates
}

/// Fetch the latest version of one binary and the version currently in use.
async fn check_one(name: &str, repo: &str) -> AppResult<Option<UpdateInfo>> {
    let release = fetch_release(repo).await?;
    let latest = release.tag_name.trim_start_matches('v').to_string();
    let current = crate::binaries::current_version(name).await;
    Ok(Some(UpdateInfo {
        name: name.to_string(),
        current,
        latest,
    }))
}

/// Download the latest release of `name` into the cache, verifying sha256
/// checksums published with the release.
pub async fn update_binary(name: &str) -> AppResult<String> {
    let (repo, min_version) = match name {
        "yt-dlp" => ("yt-dlp/yt-dlp", YTDLP_MIN_VERSION),
        "deno" => ("denoland/deno", DENO_MIN_VERSION),
        _ => {
            return Err(AppError::Subprocess(format!(
                "unsupported runtime binary: {name}"
            )))
        }
    };
    let release = fetch_release(repo).await?;
    let latest = release.tag_name.trim_start_matches('v').to_string();
    if !crate::binaries::version_gte(&latest, min_version) {
        return Err(AppError::Subprocess(format!(
            "release {latest} is older than the supported minimum {min_version}"
        )));
    }

    let dest = crate::binaries::cache_file_for(name);
    let parent = dest.parent().map(Path::to_path_buf).unwrap_or_default();
    std::fs::create_dir_all(&parent).map_err(AppError::Io)?;

    match name {
        "yt-dlp" => {
            let bin = release
                .assets
                .iter()
                .find(|a| a.name == "yt-dlp_linux")
                .ok_or_else(|| AppError::Subprocess("yt-dlp_linux asset not found".into()))?;
            let sums = release
                .assets
                .iter()
                .find(|a| a.name == "SHA2-256SUMS")
                .ok_or_else(|| AppError::Subprocess("SHA2-256SUMS asset not found".into()))?;
            let bytes = http_get_bytes(&bin.url).await?;
            let sums_text = http_get_string(&sums.url).await?;
            let expected = parse_sha256_sums(&sums_text, "yt-dlp_linux").ok_or_else(|| {
                AppError::Subprocess("yt-dlp_linux checksum missing from release".into())
            })?;
            verify_sha256(&bytes, &expected, "yt-dlp")?;
            write_executable(&dest, &bytes)?;
        }
        "deno" => {
            let zip = release
                .assets
                .iter()
                .find(|a| a.name == "deno-x86_64-unknown-linux-gnu.zip")
                .ok_or_else(|| {
                    AppError::Subprocess("deno-x86_64-unknown-linux-gnu.zip not found".into())
                })?;
            let zip_sha = release
                .assets
                .iter()
                .find(|a| a.name == "deno-x86_64-unknown-linux-gnu.zip.sha256sum")
                .ok_or_else(|| AppError::Subprocess("deno zip sha256sum not found".into()))?;
            let bin_sha = release
                .assets
                .iter()
                .find(|a| a.name == "deno-x86_64-unknown-linux-gnu.sha256sum")
                .ok_or_else(|| AppError::Subprocess("deno bin sha256sum not found".into()))?;

            let zip_bytes = http_get_bytes(&zip.url).await?;
            let expected_zip = parse_single_sha256sum(
                &http_get_string(&zip_sha.url).await?,
                "deno-x86_64-unknown-linux-gnu.zip",
            )?;
            verify_sha256(&zip_bytes, &expected_zip, "deno zip")?;

            let bin = extract_deno(&zip_bytes)?;
            let expected_bin = parse_single_sha256sum(
                &http_get_string(&bin_sha.url).await?,
                "deno-x86_64-unknown-linux-gnu",
            )?;
            verify_sha256(&bin, &expected_bin, "deno binary")?;
            write_executable(&dest, &bin)?;
        }
        _ => unreachable!(),
    }

    Ok(latest)
}

fn parse_sha256_sums(text: &str, filename: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let mut parts = line.split_whitespace();
        let hash = parts.next()?;
        let name = parts.next()?;
        if name.trim_start_matches("*") == filename {
            Some(hash.to_string())
        } else {
            None
        }
    })
}

fn parse_single_sha256sum(text: &str, _filename: &str) -> AppResult<String> {
    text.lines()
        .next()
        .map(|l| l.split_whitespace().next().unwrap_or("").trim().to_string())
        .filter(|h| !h.is_empty() && h.len() == 64)
        .ok_or_else(|| AppError::Subprocess("invalid sha256sum file".into()))
}

fn verify_sha256(bytes: &[u8], expected: &str, what: &str) -> AppResult<()> {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let actual: String = hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    if actual != expected {
        return Err(AppError::Subprocess(format!(
            "sha256 mismatch for {what}: expected {expected}, got {actual}"
        )));
    }
    Ok(())
}

fn write_executable(path: &Path, bytes: &[u8]) -> AppResult<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::write(path, bytes).map_err(AppError::Io)?;
    let mut perms = std::fs::metadata(path).map_err(AppError::Io)?.permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(path, perms).map_err(AppError::Io)?;
    Ok(())
}

fn extract_deno(zip_bytes: &[u8]) -> AppResult<Vec<u8>> {
    let reader = std::io::Cursor::new(zip_bytes);
    let mut archive = zip::ZipArchive::new(reader)
        .map_err(|e| AppError::Subprocess(format!("bad deno zip: {e}")))?;
    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| AppError::Subprocess(format!("bad deno zip entry: {e}")))?;
        if entry.name() == "deno" {
            let mut bin = Vec::new();
            std::io::Read::read_to_end(&mut entry, &mut bin)
                .map_err(|e| AppError::Subprocess(format!("failed to read deno from zip: {e}")))?;
            return Ok(bin);
        }
    }
    Err(AppError::Subprocess(
        "deno zip does not contain a 'deno' entry".into(),
    ))
}

async fn fetch_release(repo: &str) -> AppResult<Release> {
    let url = format!("{GITHUB_API}/{repo}/releases/latest");
    let text = http_get_string(&url).await?;
    serde_json::from_str(&text)
        .map_err(|e| AppError::Subprocess(format!("failed to parse release JSON: {e}")))
}

async fn http_get_bytes(url: &str) -> AppResult<Vec<u8>> {
    let url = url.to_string();
    let url_for_blocking = url.clone();
    let url_for_blocking_err = url_for_blocking.clone();
    tokio::time::timeout(HTTP_TIMEOUT, async move {
        tokio::task::spawn_blocking(move || -> Result<Vec<u8>, ureq::Error> {
            let response = ureq::get(&url_for_blocking).call()?.into_body();
            let mut reader = response.into_reader();
            let mut bytes = Vec::new();
            std::io::Read::read_to_end(&mut reader, &mut bytes)?;
            Ok(bytes)
        })
        .await
        .map_err(|e| AppError::Subprocess(format!("update task panicked: {e}")))?
        .map_err(|e| {
            AppError::Subprocess(format!("failed to download {url_for_blocking_err}: {e}"))
        })
    })
    .await
    .map_err(|_| AppError::Subprocess(format!("download of {url} timed out")))?
}

async fn http_get_string(url: &str) -> AppResult<String> {
    let bytes = http_get_bytes(url).await?;
    String::from_utf8(bytes)
        .map_err(|e| AppError::Subprocess(format!("non-UTF8 response from {url}: {e}")))
}

/// Update the marker file. Returns Ok only when the check should run now
/// (i.e. the last check was more than 24h ago or the marker does not exist).
fn touch_check_marker(cache_root: &Path) -> AppResult<()> {
    let marker = cache_root.join(CHECK_MARKER);
    if let Ok(meta) = std::fs::metadata(&marker) {
        if let Ok(modified) = meta.modified() {
            if modified.elapsed().ok().is_some_and(|e| e < CHECK_INTERVAL) {
                return Err(AppError::Subprocess(
                    "update check already ran today".into(),
                ));
            }
        }
    }
    std::fs::create_dir_all(cache_root).map_err(AppError::Io)?;
    std::fs::write(&marker, now_seconds().to_string()).map_err(AppError::Io)?;
    Ok(())
}

fn now_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// Keep min-version pins in one place so `check_updates`/`update_binary` agree
// with `binaries::ensure_*`.
const YTDLP_MIN_VERSION: &str = "2026.07.04";
const DENO_MIN_VERSION: &str = "2.9.5";

#[cfg(test)]
mod tests {
    use super::{parse_sha256_sums, parse_single_sha256sum};

    #[test]
    fn parses_ytdlp_sums() {
        let text = concat!(
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855  yt-dlp\n",
            "6bbb3d314cde4febe36e5fa1d55462e29c974f63444e707871834f6d8cc210ae *yt-dlp_linux\n",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa  yt-dlp.exe\n",
        );
        assert_eq!(
            parse_sha256_sums(text, "yt-dlp_linux").as_deref(),
            Some("6bbb3d314cde4febe36e5fa1d55462e29c974f63444e707871834f6d8cc210ae")
        );
    }

    #[test]
    fn parses_deno_sha256sum() {
        let line = "dc480c462c8c3582524f3e75c160613d0a975e1f66b5465995d58bae236da7d3  deno\n";
        assert!(parse_single_sha256sum(line, "deno").is_ok());
        assert!(parse_single_sha256sum("short\n", "deno").is_err());
    }

    #[tokio::test]
    #[ignore]
    async fn fetches_latest_from_github() {
        let release = super::fetch_release("yt-dlp/yt-dlp").await.unwrap();
        assert!(!release.tag_name.is_empty());
        assert!(!release.assets.is_empty());
    }

    #[tokio::test]
    #[ignore]
    async fn updates_ytdlp_and_deno_into_cache() {
        let root = std::env::temp_dir().join(format!("votdesktop-upd-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        std::env::set_var("XDG_CACHE_HOME", &root);

        let ver = super::update_binary("yt-dlp").await.unwrap();
        assert!(!ver.is_empty());
        let ver = super::update_binary("deno").await.unwrap();
        assert!(!ver.is_empty());

        let _ = std::fs::remove_dir_all(&root);
    }
}
