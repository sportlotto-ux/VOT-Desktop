use crate::error::{AppError, AppResult};
use std::process::Command;

const FFMPEG: &str = "ffmpeg";

/// Check that ffmpeg is available in PATH.
/// Returns the version string on success.
pub fn check_ffmpeg() -> AppResult<String> {
    let output = Command::new(FFMPEG)
        .arg("-version")
        .output()
        .map_err(|_| {
            AppError::Subprocess(
                "ffmpeg not found. Install it:\n  Fedora: sudo dnf install ffmpeg\n  Ubuntu: sudo apt install ffmpeg\n  Arch:   sudo pacman -S ffmpeg".into(),
            )
        })?;

    if !output.status.success() {
        return Err(AppError::Subprocess(
            "ffmpeg is not working correctly".into(),
        ));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let first_line = stdout.lines().next().unwrap_or("");
    // "ffmpeg version 8.1.2 Copyright ..." -> "8.1.2"
    let version = first_line
        .split_whitespace()
        .nth(2)
        .unwrap_or("(unknown)")
        .to_string();
    Ok(version)
}
