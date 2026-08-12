//! Build script for VotDesktop.
//!
//! Responsibilities:
//! 1. Run `tauri_build::build()` to embed Tauri context.
//! 2. Validate that the Rust `FILTER_COMPLEX` constant in `src/mixer.rs`
//!    stays in sync with the source of truth in
//!    `mediabot2.0/src/handlers/media_utils.py` (per ADR-003 in
//!    `docs/REGULATION.md`).
//!
//! To enable sync validation, set the `VOT_MEDIABOT_SRC` env var to the
//! absolute path of `media_utils.py`. If unset, a warning is printed and
//! validation is skipped (soft-fail), so devs without a mediabot2.0
//! checkout can still build.

use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

const FILTER_COMPLEX_LINES: (u32, u32) = (369, 371);
const FILTER_COMPLEX_FILE: &str = "src/filter_complex.txt";

/// SHA-256 of the joined filter_complex content from
/// `mediabot2.0/src/handlers/media_utils.py` lines 369-371.
/// Bump this (and MIXER_PRESET_VERSION in src/mixer.rs) when the source
/// is updated. See docs/REGULATION.md ADR-003.
const MIXER_PRESET_SOURCE_SHA256: &str =
    "cec6bd93eb6882941af42e9f02d6c837a613d3d0a13da3b125e7b46faa7a1cfa";
const MIXER_PRESET_RUST_SHA256: &str =
    "0f05793f38fd3f7d540fdfd0ba60306063f6c185a9153887380cadd35a591977";

fn main() {
    println!("cargo:rerun-if-env-changed=VOT_MEDIABOT_SRC");
    tauri_build::build();
    validate_mixer_preset();
}

fn validate_mixer_preset() {
    let rust_filter_path = PathBuf::from(FILTER_COMPLEX_FILE);
    let rust_sha = match compute_file_sha256(&rust_filter_path) {
        Ok(s) => s,
        Err(e) => panic!("failed to hash {}: {e}", rust_filter_path.display()),
    };
    if rust_sha != MIXER_PRESET_RUST_SHA256 {
        panic!(
            "{} is out of sync: expected sha256 {}, got {}",
            rust_filter_path.display(),
            MIXER_PRESET_RUST_SHA256,
            rust_sha
        );
    }
    println!("cargo:rerun-if-changed={FILTER_COMPLEX_FILE}");

    let Some(src_path) = env::var_os("VOT_MEDIABOT_SRC") else {
        println!(
            "cargo:warning=VOT_MEDIABOT_SRC not set, skipping filter_complex sync check \
             (ADR-003 soft-fail)"
        );
        return;
    };

    let src_path = PathBuf::from(src_path);
    if !src_path.exists() {
        println!(
            "cargo:warning=VOT_MEDIABOT_SRC points to '{}' which does not exist; skipping sync check",
            src_path.display()
        );
        return;
    }

    let expected_sha = MIXER_PRESET_SOURCE_SHA256;
    let actual_sha = match compute_filter_complex_sha256(&src_path) {
        Ok(s) => s,
        Err(e) => {
            println!(
                "cargo:warning=failed to compute filter_complex sha256: {e}; skipping sync check"
            );
            return;
        }
    };

    println!("cargo:rerun-if-changed={}", src_path.display());

    if actual_sha != expected_sha {
        panic!(
            "mixer.rs out of sync with {}: expected sha256 {expected_sha}, got {actual_sha}.\n\
             Update MIXER_PRESET_VERSION in src-tauri/src/mixer.rs and the\n\
             MIXER_PRESET_SOURCE_SHA256 constant in src-tauri/build.rs.\n\
             See docs/REGULATION.md ADR-003.",
            src_path.display()
        );
    }
}

fn compute_file_sha256(path: &std::path::Path) -> Result<String, String> {
    let output = Command::new("sha256sum")
        .arg(path)
        .output()
        .map_err(|e| format!("sha256sum failed: {e}"))?;
    if !output.status.success() {
        return Err(format!("sha256sum exited with status {}", output.status));
    }
    let stdout = String::from_utf8(output.stdout).map_err(|e| e.to_string())?;
    stdout
        .split_whitespace()
        .next()
        .map(str::to_string)
        .ok_or_else(|| "no sha256 in output".to_string())
}

fn compute_filter_complex_sha256(path: &std::path::Path) -> Result<String, String> {
    let content = fs::read_to_string(path).map_err(|e| e.to_string())?;
    let (start, end) = FILTER_COMPLEX_LINES;
    let mut joined = String::new();
    for (idx, line) in content.lines().enumerate() {
        let n = u32::try_from(idx + 1).map_err(|e| e.to_string())?;
        if n < start || n > end {
            continue;
        }
        // Strip whitespace, then Python string-literal quotes and any trailing
        // list punctuation (`"` and `,`). trim_end_matches trims ALL of them
        // until a non-matching char is hit, which handles `",` correctly.
        let trimmed = line
            .trim()
            .trim_start_matches('"')
            .trim_end_matches(['"', ',']);
        joined.push_str(trimmed);
    }
    if joined.is_empty() {
        return Err(format!(
            "no content extracted from lines {start}-{end} of {}",
            path.display()
        ));
    }

    let tmp = std::env::temp_dir().join(format!(
        "vot-desktop-filter-complex-{}.txt",
        std::process::id()
    ));
    fs::write(&tmp, &joined).map_err(|e| e.to_string())?;

    let output = Command::new("sha256sum")
        .arg(&tmp)
        .output()
        .map_err(|e| format!("sha256sum failed: {e}"))?;

    let _ = fs::remove_file(&tmp);

    if !output.status.success() {
        return Err(format!("sha256sum exited with status {}", output.status));
    }
    let stdout = String::from_utf8(output.stdout).map_err(|e| e.to_string())?;
    let sha = stdout
        .split_whitespace()
        .next()
        .ok_or_else(|| "no sha256 in output".to_string())?;
    Ok(sha.to_string())
}
