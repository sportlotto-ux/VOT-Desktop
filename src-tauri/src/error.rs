//! Typed errors returned to the frontend via Tauri IPC.
//!
//! All variants serialize to a JSON object `{ "kind": "...", "message": "..." }`
//! so the TS layer can switch on `kind` and show a human-readable message.

use serde::Serialize;

#[derive(Debug, thiserror::Error)]
#[allow(dead_code)]
pub enum AppError {
    #[error("tauri error: {0}")]
    Tauri(String),

    #[error("invalid input: {0}")]
    InvalidInput(String),

    #[error("subprocess failed: {0}")]
    Subprocess(String),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

impl Serialize for AppError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let kind = match self {
            AppError::Tauri(_) => "tauri",
            AppError::InvalidInput(_) => "invalid_input",
            AppError::Subprocess(_) => "subprocess",
            AppError::Io(_) => "io",
        };
        let mut s = serializer.serialize_struct("AppError", 2)?;
        s.serialize_field("kind", kind)?;
        s.serialize_field("message", &self.to_string())?;
        s.end()
    }
}

pub type AppResult<T> = Result<T, AppError>;
