use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("WPE initialization failed")]
    InitFailed,

    #[error("Failed to create web view")]
    WebViewCreationFailed,

    #[error("Failed to create backend")]
    BackendCreationFailed,

    #[error("Failed to create renderer: {0}")]
    RendererCreationFailed(String),

    #[error("Render failed: {0}")]
    RenderFailed(String),

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

    #[error("X11 error: {0}")]
    X11Error(String),

    #[error("No display available (neither Wayland nor X11)")]
    NoDisplay,
}

pub type Result<T> = std::result::Result<T, Error>;
