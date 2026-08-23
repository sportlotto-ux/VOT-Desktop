//! Description translation via Google AI Studio (Gemini API).
//!
//! The user provides their own API key (aistudio.google.com); requests go
//! directly from the Rust backend so the key never touches the webview CSP
//! surface.

use crate::error::{AppError, AppResult};
use std::time::Duration;

const GEMINI_API: &str = "https://generativelanguage.googleapis.com/v1beta/models";
/// Fallback when the frontend sends no/unknown model (e.g. ListModels
/// unavailable). `gemini-2.0-flash` was shut down by Google on 2026-06-01 —
/// keep this pinned to a live model.
pub const DEFAULT_MODEL: &str = "gemini-3.5-flash";
const HTTP_TIMEOUT: Duration = Duration::from_secs(90);
const MAX_DESCRIPTION_BYTES: usize = 64 * 1024;
const MAX_OUTPUT_TOKENS: u32 = 8192;

const PROMPT: &str = "\
Translate the following YouTube video description into Russian. \
Keep the original formatting: preserve line breaks, links, hashtags and @mentions unchanged. \
Do not add any commentary — return only the translation.\n\n---\n";

/// Translate a video description to Russian. Returns the translated text.
/// `model` — id from ListModels; unknown values fall back to `DEFAULT_MODEL`.
pub async fn translate_description(
    text: &str,
    api_key: &str,
    model: Option<&str>,
) -> AppResult<String> {
    if text.trim().is_empty() {
        return Err(AppError::InvalidInput("description is empty".into()));
    }
    if text.len() > MAX_DESCRIPTION_BYTES {
        return Err(AppError::InvalidInput(
            "description is too large (over 64 KiB)".into(),
        ));
    }
    let key = api_key.trim();
    if key.is_empty()
        || key.len() > 256
        || !key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(AppError::InvalidInput(
            "AI Studio API key has invalid format".into(),
        ));
    }
    // The value goes into the URL path — keep it strict even though it
    // normally comes from our own ListModels response.
    let model = match model {
        Some(m)
            if !m.is_empty()
                && m.len() <= 64
                && m.chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_')) =>
        {
            m
        }
        _ => DEFAULT_MODEL,
    };

    let body = serde_json::json!({
        "contents": [{"parts": [{"text": format!("{PROMPT}{text}")}]}],
        "generationConfig": {
            "temperature": 0.2,
            "maxOutputTokens": MAX_OUTPUT_TOKENS,
        },
        "safetySettings": [],
    });

    let url = format!("{GEMINI_API}/{model}:generateContent?key={key}");
    let response_text = tokio::time::timeout(HTTP_TIMEOUT, async {
        tokio::task::spawn_blocking(move || -> Result<String, ureq::Error> {
            let mut reader = ureq::post(&url)
                .header("Content-Type", "application/json")
                .send(body.to_string())?
                .into_body()
                .into_reader();
            let mut text = String::new();
            std::io::Read::read_to_string(&mut reader, &mut text)?;
            Ok(text)
        })
        .await
        .map_err(|e| AppError::Subprocess(format!("gemini task panicked: {e}")))?
        .map_err(|e| AppError::Subprocess(format!("Gemini API request failed: {e}")))
    })
    .await
    .map_err(|_| AppError::Subprocess("Gemini API request timed out".into()))??;

    parse_gemini_response(&response_text)
}

/// Fetch model ids available to this API key (only those supporting
/// `generateContent`), e.g. ["gemini-3.5-flash", ...]. Used to populate the
/// UI dropdown so we never hardcode model names.
pub async fn list_models(api_key: &str) -> AppResult<Vec<String>> {
    let key = api_key.trim();
    if key.is_empty() || key.len() > 256 {
        return Err(AppError::InvalidInput("API key is empty".into()));
    }
    let url = format!("{GEMINI_API}?key={key}&pageSize=1000");
    let body = tokio::time::timeout(HTTP_TIMEOUT, async {
        tokio::task::spawn_blocking(move || -> Result<String, ureq::Error> {
            let mut reader = ureq::get(&url)
                .header("Accept", "application/json")
                .call()?
                .into_body()
                .into_reader();
            let mut text = String::new();
            std::io::Read::read_to_string(&mut reader, &mut text)?;
            Ok(text)
        })
        .await
        .map_err(|e| AppError::Subprocess(format!("gemini task panicked: {e}")))?
        .map_err(|e| AppError::Subprocess(format!("Gemini API request failed: {e}")))
    })
    .await
    .map_err(|_| AppError::Subprocess("Gemini API request timed out".into()))??;

    #[derive(serde::Deserialize)]
    struct ModelsResponse {
        models: Option<Vec<Entry>>,
    }
    #[derive(serde::Deserialize)]
    struct Entry {
        name: String,
        #[serde(rename = "supportedGenerationMethods", default)]
        methods: Vec<String>,
    }

    let parsed: ModelsResponse = serde_json::from_str(&body)
        .map_err(|e| AppError::Subprocess(format!("failed to parse Gemini models: {e}")))?;
    let mut ids: Vec<String> = parsed
        .models
        .unwrap_or_default()
        .into_iter()
        .filter(|m| m.methods.iter().any(|x| x == "generateContent"))
        .filter_map(|m| m.name.strip_prefix("models/").map(str::to_string))
        .collect();
    ids.sort();
    Ok(ids)
}

fn parse_gemini_response(raw: &str) -> AppResult<String> {
    #[derive(serde::Deserialize)]
    struct Response {
        candidates: Option<Vec<Candidate>>,
        #[serde(rename = "error")]
        api_error: Option<ApiError>,
    }
    #[derive(serde::Deserialize)]
    struct Candidate {
        content: Content,
    }
    #[derive(serde::Deserialize)]
    struct Content {
        parts: Vec<Part>,
    }
    #[derive(serde::Deserialize)]
    struct Part {
        text: Option<String>,
    }
    #[derive(serde::Deserialize)]
    struct ApiError {
        message: String,
    }

    let parsed: Response = serde_json::from_str(raw)
        .map_err(|e| AppError::Subprocess(format!("failed to parse Gemini response: {e}")))?;

    if let Some(err) = parsed.api_error {
        return Err(AppError::Subprocess(format!(
            "Gemini API error: {}",
            err.message
        )));
    }

    let text = parsed
        .candidates
        .and_then(|c| c.into_iter().next())
        .and_then(|c| c.content.parts.into_iter().find_map(|p| p.text))
        .filter(|t| !t.trim().is_empty())
        .ok_or_else(|| AppError::Subprocess("Gemini returned an empty response".into()))?;

    // Trim prompt guard markers if the model echoes them.
    Ok(text.trim().trim_start_matches("---").trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::parse_gemini_response;

    #[test]
    fn parses_candidate_text() {
        let raw = r#"{"candidates":[{"content":{"parts":[{"text":"Привет мир"}]}}]}"#;
        assert_eq!(parse_gemini_response(raw).unwrap(), "Привет мир");
    }

    #[test]
    fn surfaces_api_errors() {
        let raw = r#"{"error":{"code":400,"message":"API key not valid"}}"#;
        let err = parse_gemini_response(raw).unwrap_err().to_string();
        assert!(err.contains("API key not valid"));
    }

    #[test]
    fn rejects_empty_candidates() {
        let raw = r#"{"candidates":[]}"#;
        assert!(parse_gemini_response(raw).is_err());
    }
}
