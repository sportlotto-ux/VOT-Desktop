//! Resolve the trusted runtime binaries used by the application.

use crate::error::{AppError, AppResult};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use tokio::process::Command;

const DENO_BUNDLED_NAME: &str = "deno-x86_64-x86_64-unknown-linux-gnu";
const YTDLP_BUNDLED_NAME: &str = "yt-dlp-x86_64-x86_64-unknown-linux-gnu";
const DENO_MIN_VERSION: &str = "2.9.5";
const YTDLP_MIN_VERSION: &str = "2026.07.04";

static RESOURCE_DIR: OnceLock<PathBuf> = OnceLock::new();

pub fn init_resource_dir(path: PathBuf) {
    let _ = RESOURCE_DIR.set(path);
}

// ---- Public ----

pub async fn ensure_ytdlp() -> AppResult<PathBuf> {
    resolve_binary("yt-dlp", YTDLP_BUNDLED_NAME, YTDLP_MIN_VERSION).await
}

pub async fn ensure_deno() -> AppResult<PathBuf> {
    resolve_binary("deno", DENO_BUNDLED_NAME, DENO_MIN_VERSION).await
}

// ---- Internal ----

async fn resolve_binary(
    name: &str,
    bundled_name: &str,
    minimum_version: &str,
) -> AppResult<PathBuf> {
    if let Some(resource_dir) = RESOURCE_DIR.get() {
        let bundled = resource_dir.join("binaries").join(bundled_name);
        if bundled.is_file()
            && probe_version(&bundled)
                .await
                .is_some_and(|v| version_gte(&v, minimum_version))
        {
            return Ok(bundled);
        }
    }

    if let Some(system) = find_in_path(name) {
        if probe_version(&system)
            .await
            .is_some_and(|v| version_gte(&v, minimum_version))
        {
            return Ok(system);
        }
    }

    Err(AppError::Subprocess(format!(
        "{name} >= {minimum_version} was not found in the application bundle or PATH"
    )))
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

fn version_gte(cur: &str, min: &str) -> bool {
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
    use super::version_gte;

    #[test]
    fn compares_release_versions() {
        assert!(version_gte("2.9.5", "2.9.5"));
        assert!(version_gte("2026.07.05", "2026.07.04"));
        assert!(!version_gte("2.9.4", "2.9.5"));
    }
}
