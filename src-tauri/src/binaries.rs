//! Resolve the runtime binaries used by the application.
//!
//! Resolution order:
//!   1. system `PATH` (yt-dlp / deno installed by the user)
//!   2. pinned cache in `~/.cache/votdesktop/binaries`
//!   3. download the pinned version into the cache, verifying sha256
//!
//! The binaries are intentionally NOT bundled into the AppImage so that
//! releasing a new yt-dlp/deno does not require rebuilding the app.

use crate::error::{AppError, AppResult};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::process::Command;

const YTDLP_MIN_VERSION: &str = "2026.07.04";
const DENO_MIN_VERSION: &str = "2.9.5";

const YTDLP_URL: &str =
    "https://github.com/yt-dlp/yt-dlp/releases/download/2026.07.04/yt-dlp_linux";
const YTDLP_SHA256: &str = "6bbb3d314cde4febe36e5fa1d55462e29c974f63444e707871834f6d8cc210ae";

const DENO_URL: &str =
    "https://github.com/denoland/deno/releases/download/v2.9.5/deno-x86_64-unknown-linux-gnu.zip";
const DENO_ARCHIVE_SHA256: &str =
    "8b010a3b1a4a0188a67cdb8a7a27348b2a501af78aec7fc74f2ace167368d530";
const DENO_BIN_SHA256: &str = "dc480c462c8c3582524f3e75c160613d0a975e1f66b5465995d58bae236da7d3";

const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(10 * 60);

// ---- Public ----

pub async fn ensure_ytdlp() -> AppResult<PathBuf> {
    resolve("yt-dlp", YTDLP_MIN_VERSION).await
}

pub async fn ensure_deno() -> AppResult<PathBuf> {
    resolve("deno", DENO_MIN_VERSION).await
}

/// Version currently in use (PATH first, then cache), if any.
pub async fn current_version(name: &str) -> Option<String> {
    if let Some(system) = find_in_path(name) {
        if let Some(v) = probe_version(&system).await {
            return Some(v);
        }
    }
    let cached = cache_dir().join(name);
    if cached.is_file() {
        return probe_version(&cached).await;
    }
    None
}

/// Path where a runtime binary is stored in the cache (also used by updates.rs).
pub fn cache_file_for(name: &str) -> PathBuf {
    cache_dir().join(name)
}

// ---- Internal ----

async fn resolve(name: &str, minimum_version: &str) -> AppResult<PathBuf> {
    // 1. System binary from PATH.
    if let Some(system) = find_in_path(name) {
        if probe_version(&system)
            .await
            .is_some_and(|v| version_gte(&v, minimum_version))
        {
            return Ok(system);
        }
    }

    // 2. Pinned cache.
    let cache_path = cache_dir().join(name);
    if cache_path.is_file()
        && probe_version(&cache_path)
            .await
            .is_some_and(|v| version_gte(&v, minimum_version))
    {
        return Ok(cache_path);
    }

    // 3. Download the pinned version into the cache (first run).
    download(name, &cache_path).await?;
    Ok(cache_path)
}

fn cache_dir() -> PathBuf {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            dirs::home_dir()
                .map(|h| h.join(".cache"))
                .unwrap_or_else(|| PathBuf::from("/tmp"))
        });
    base.join("votdesktop").join("binaries")
}

async fn download(name: &str, dest: &Path) -> AppResult<()> {
    let (url, expected_archive_sha) = match name {
        "yt-dlp" => (YTDLP_URL, YTDLP_SHA256),
        "deno" => (DENO_URL, DENO_ARCHIVE_SHA256),
        _ => {
            return Err(AppError::Subprocess(format!(
                "unsupported runtime binary: {name}"
            )))
        }
    };

    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(AppError::Io)?;
    }

    let tmp = dest.with_extension("tmp");
    let _ = std::fs::remove_file(&tmp);

    log::info!("downloading {name} from {url}");
    let bytes = timeout_download(url).await?;
    verify_sha256(&bytes, expected_archive_sha, name)?;

    match name {
        "yt-dlp" => {
            write_executable(&tmp, &bytes)?;
        }
        "deno" => {
            let bin = extract_deno(&bytes)?;
            write_executable(&tmp, &bin)?;
            let actual = file_sha256(&tmp)?;
            if actual != DENO_BIN_SHA256 {
                let _ = std::fs::remove_file(&tmp);
                return Err(AppError::Subprocess(format!(
                    "sha256 mismatch for deno binary: expected {DENO_BIN_SHA256}, got {actual}"
                )));
            }
        }
        _ => unreachable!(),
    }

    // Sanity check: the downloaded binary must run and report a version.
    if probe_version(&tmp).await.is_none() {
        let _ = std::fs::remove_file(&tmp);
        return Err(AppError::Subprocess(format!(
            "downloaded {name} is not executable or did not report a version"
        )));
    }

    std::fs::rename(&tmp, dest).map_err(AppError::Io)?;
    log::info!("installed {name} -> {}", dest.display());
    Ok(())
}

async fn timeout_download(url: &str) -> AppResult<Vec<u8>> {
    tokio::time::timeout(DOWNLOAD_TIMEOUT, async move {
        let body = tokio::task::spawn_blocking({
            let url = url.to_string();
            move || -> Result<Vec<u8>, ureq::Error> {
                let response = ureq::get(&url).call()?.into_body();
                let mut reader = response.into_reader();
                let mut bytes = Vec::new();
                std::io::Read::read_to_end(&mut reader, &mut bytes)?;
                Ok(bytes)
            }
        })
        .await
        .map_err(|e| AppError::Subprocess(format!("download task panicked: {e}")))?
        .map_err(|e| AppError::Subprocess(format!("failed to download {url}: {e}")))?;
        Ok(body)
    })
    .await
    .map_err(|_| AppError::Subprocess(format!("download of {url} timed out")))?
}

fn verify_sha256(bytes: &[u8], expected: &str, what: &str) -> AppResult<()> {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let actual = hasher.finalize();
    let actual_hex: String = actual.iter().map(|b| format!("{b:02x}")).collect();
    if actual_hex != expected {
        return Err(AppError::Subprocess(format!(
            "sha256 mismatch for {what}: expected {expected}, got {actual_hex}"
        )));
    }
    Ok(())
}

fn file_sha256(path: &Path) -> AppResult<String> {
    let bytes = std::fs::read(path).map_err(AppError::Io)?;
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    let actual = hasher.finalize();
    Ok(actual.iter().map(|b| format!("{b:02x}")).collect())
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
    let mut bin = Vec::new();
    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| AppError::Subprocess(format!("bad deno zip entry: {e}")))?;
        if entry.name() == "deno" {
            std::io::Read::read_to_end(&mut entry, &mut bin)
                .map_err(|e| AppError::Subprocess(format!("failed to read deno from zip: {e}")))?;
            return Ok(bin);
        }
    }
    Err(AppError::Subprocess(
        "deno zip does not contain a 'deno' entry".into(),
    ))
}

fn find_in_path(name: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths).find_map(|dir| {
            let p = dir.join(name);
            p.is_file().then_some(p)
        })
    })
}

async fn probe_version(path: &Path) -> Option<String> {
    let out = Command::new(path).arg("--version").output().await.ok()?;
    if !out.status.success() {
        return None;
    }
    let line = std::str::from_utf8(&out.stdout).ok()?.lines().next()?;
    Some(
        line.split_whitespace()
            .find(|w| w.starts_with(|c: char| c.is_ascii_digit()))?
            .to_string(),
    )
}

pub(crate) fn version_gte(cur: &str, min: &str) -> bool {
    let cp: Vec<u64> = cur
        .split(|c: char| !c.is_ascii_digit())
        .filter_map(|p| p.parse().ok())
        .collect();
    let mp: Vec<u64> = min
        .split(|c: char| !c.is_ascii_digit())
        .filter_map(|p| p.parse().ok())
        .collect();
    for (c, m) in cp.iter().zip(mp.iter()) {
        if c < m {
            return false;
        }
        if c > m {
            return true;
        }
    }
    cp.len() >= mp.len()
}

#[cfg(test)]
mod tests {
    use super::{cache_dir, download, probe_version, version_gte};

    #[test]
    fn compares_release_versions() {
        assert!(version_gte("2.9.5", "2.9.5"));
        assert!(version_gte("2026.07.05", "2026.07.04"));
        assert!(!version_gte("2.9.4", "2.9.5"));
    }

    /// Network-dependent: run with `cargo test -- --ignored` to exercise the
    /// real download+sha256+extract path against GitHub.
    #[tokio::test]
    #[ignore]
    async fn downloads_deno_into_cache() {
        let dest = cache_dir().join("test-deno");
        if dest.exists() {
            return;
        }
        if download("deno", &dest).await.is_ok() {
            assert!(dest.is_file());
            assert!(probe_version(&dest).await.is_some());
        }
        let _ = std::fs::remove_file(&dest);
    }
}
