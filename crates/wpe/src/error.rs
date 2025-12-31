use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("WPE initialization failed")]
    InitFailed,

    #[error("Failed to create web view")]
    WebViewCreationFailed,

    #[error("Failed to create backend")]
    BackendCreationFailed,

    #[error("Invalid URL: {0}")]
    InvalidUrl(String),

    #[error("JavaScript evaluation failed: {0}")]
    JavaScriptError(String),

    #[error("IPC error: {0}")]
    IpcError(String),

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("Window handle error")]
    WindowHandle,
}

pub type Result<T> = std::result::Result<T, Error>;
