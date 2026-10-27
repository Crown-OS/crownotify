//! The crate's typed error.
//!
//! Everything a caller might reasonably want to *react* to lives here, so the
//! D-Bus layer can map a missing notification onto the right reply instead of
//! stringly-matching an `anyhow` chain. Application boundaries — `main`,
//! service setup, io — keep using `anyhow::Result` with `.context(...)`.

use crate::model::NotificationId;

/// A failure the daemon's own code produced, as opposed to one that came out
/// of a library.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("notification {0} does not exist")]
    NoSuchNotification(NotificationId),
    #[error("notification {id} has no action {action:?}")]
    NoSuchAction { id: NotificationId, action: String },
    #[error("{0:?} is not an interruption level")]
    UnknownInterruptionLevel(String),
    #[error("no focus mode is called {0:?}")]
    UnknownFocusMode(String),
    #[error("no desktop entry named {0:?} could be launched")]
    NoSuchDesktopEntry(String),
    #[error("icon {0:?} could not be resolved")]
    IconNotFound(String),
    #[error("icon decode failed: {0}")]
    IconDecode(String),
    #[error(transparent)]
    Ipc(#[from] crownos_ipc::Error),
    #[error(transparent)]
    Dbus(#[from] zbus::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// `Result` defaulted to this crate's [`Error`].
pub type Result<T, E = Error> = std::result::Result<T, E>;
