use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("destination unavailable or not writable: {0}")]
    DestUnavailable(PathBuf),

    /// Cleanup refused: some files failed to copy, so the card is left untouched.
    #[error("cleanup blocked: {0} file(s) failed; card left untouched")]
    CleanupBlocked(usize),

    #[error("failed to delete source {path}: {source}")]
    Delete {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("eject failed for {0}")]
    EjectFailed(PathBuf),

    /// osascript returned a TCC / Automation authorization failure (-1743).
    #[error("Photos automation not authorized — grant access in System Settings ▸ Privacy & Security ▸ Automation")]
    PhotosNotAuthorized,

    #[error("osascript error: {0}")]
    Osascript(String),

    #[error("config error: {0}")]
    Config(String),

    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, Error>;
