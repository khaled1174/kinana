use serde::Serialize;
use std::fmt;

#[derive(Debug, Clone, Serialize)]
pub struct Error {
    pub code: String,
    pub reason_code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub activation_preview: Option<Box<crate::activation_risk::Decision>>,
    #[serde(skip)]
    pub needs_decision: bool,
}
impl Error {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            reason_code: code.into(),
            message: message.into(),
            activation_preview: None,
            needs_decision: false,
        }
    }
    pub fn decision(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            reason_code: code.into(),
            message: message.into(),
            activation_preview: None,
            needs_decision: true,
        }
    }
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}
impl std::error::Error for Error {}
impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        let reason = match e.kind() {
            std::io::ErrorKind::PermissionDenied => "IO_PERMISSION_DENIED",
            std::io::ErrorKind::NotFound => "IO_NOT_FOUND",
            std::io::ErrorKind::AlreadyExists => "IO_ALREADY_EXISTS",
            _ => "IO_ERROR",
        };
        let mut error = Self::new("IO_ERROR", e.to_string());
        error.reason_code = reason.into();
        error
    }
}
impl From<rusqlite::Error> for Error {
    fn from(e: rusqlite::Error) -> Self {
        Self::new("INDEX_ERROR", e.to_string())
    }
}
impl From<toml::de::Error> for Error {
    fn from(e: toml::de::Error) -> Self {
        Self::new("INVALID_TOML", e.to_string())
    }
}
impl From<toml::ser::Error> for Error {
    fn from(e: toml::ser::Error) -> Self {
        Self::new("INVALID_TOML", e.to_string())
    }
}
pub type Result<T> = std::result::Result<T, Error>;
