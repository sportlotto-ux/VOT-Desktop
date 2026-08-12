use serde::Deserialize;

/// Full video info from yt-dlp --dump-json (top-level object).
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct YtDlpVideoInfo {
    pub title: Option<String>,
    pub formats: Vec<YtDlpFormat>,
}

/// Raw format entry from yt-dlp --dump-json (inside `formats` array).
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct YtDlpFormat {
    pub format_id: String,
    pub ext: String,
    #[serde(rename = "format_note")]
    pub quality: Option<String>,
    pub filesize: Option<i64>,
    pub filesize_approx: Option<i64>,
    pub vcodec: Option<String>,
    pub acodec: Option<String>,
    pub tbr: Option<f64>,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub fps: Option<f64>,
}

/// Clean format representation exposed to the frontend.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Format {
    pub id: String,
    pub ext: String,
    pub quality: String,
    pub filesize: String,
    pub has_video: bool,
    pub has_audio: bool,
}

impl From<YtDlpFormat> for Format {
    fn from(f: YtDlpFormat) -> Self {
        let filesize = f
            .filesize
            .or(f.filesize_approx)
            .map(human_size)
            .unwrap_or_else(|| "?".into());
        let has_video = f.vcodec.as_deref().is_some_and(|c| c != "none");
        let has_audio = f.acodec.as_deref().is_some_and(|c| c != "none");
        let quality = f.quality.unwrap_or_else(|| {
            f.tbr
                .map(|b| format!("{:.0}k", b))
                .unwrap_or_else(|| "?".into())
        });
        Format {
            id: f.format_id,
            ext: f.ext,
            quality,
            filesize,
            has_video,
            has_audio,
        }
    }
}

/// Event payload for progress updates emitted to the frontend.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ProgressEvent {
    pub operation: String,
    pub percent: f64,
    pub message: String,
}

fn human_size(bytes: i64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    let b = bytes as f64;
    if b >= GB {
        format!("{:.1} GB", b / GB)
    } else if b >= MB {
        format!("{:.1} MB", b / MB)
    } else if b >= KB {
        format!("{:.0} KB", b / KB)
    } else {
        format!("{bytes} B")
    }
}
