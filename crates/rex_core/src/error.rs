use thiserror::Error;

#[derive(Error, Debug)]
pub enum RexError {
    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Route error: {0}")]
    Route(String),

    #[error("Build error: {0}")]
    Build(String),

    #[error("Transform error: {0}")]
    Transform(String),

    #[error("Bundle error: {0}")]
    Bundle(String),

    #[error("V8 error: {0}")]
    V8(String),

    #[error("SSR error: {0}")]
    Ssr(String),

    #[error("Server error: {0}")]
    Server(String),

    #[error("File watcher error: {0}")]
    Watcher(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Redirect: {status} -> {destination}")]
    Redirect { status: u16, destination: String },
}
