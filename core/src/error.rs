use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("destination unavailable or not writable: {0}")]
    DestUnavailable(PathBuf),

    /// A destination resolved to a path inside the card being imported — refused so
    /// cleanup can never delete the card's only copies. Pick a destination elsewhere.
    #[error("destination is inside the card being imported: {0}")]
    DestInsideCard(PathBuf),

    /// Cleanup refused: some files failed to copy, so the card is left untouched.
    #[error("cleanup blocked: {0} file(s) failed; card left untouched")]
    CleanupBlocked(usize),

    #[error("failed to delete source {path}: {source}")]
    Delete {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    /// Pre-flight: the new files don't fit at the destination, so nothing was copied.
    #[error("not enough space at {dest}: need {}, {} free", crate::util::fmt_bytes(*needed), crate::util::fmt_bytes(*available))]
    NoSpace { dest: PathBuf, needed: u64, available: u64 },

    #[error("eject failed for {0}")]
    EjectFailed(PathBuf),

    /// osascript returned a TCC / Automation authorization failure (-1743).
    #[error("Photos automation not authorized — grant access in System Settings ▸ Privacy & Security ▸ Automation")]
    PhotosNotAuthorized,

    /// A Photos rule ran on a platform without Apple Photos. Fails loudly so files
    /// are never silently left behind.
    #[error("Apple Photos import is only available on macOS")]
    PhotosUnsupported,

    /// Eject requested on a platform with no eject backend (e.g. Linux dev builds).
    #[error("ejecting drives is not supported on this platform")]
    EjectUnsupported,

    #[error("osascript error: {0}")]
    Osascript(String),

    #[error("config error: {0}")]
    Config(String),

    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, Error>;
