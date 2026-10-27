//! Reaching the application a notification came from.
//!
//! Activating a card has always published `ActionInvoked`, which is what an app
//! that is listening acts on. An app that is *not* listening — most of them, and
//! every app that was not running when its notification arrived — got nothing at
//! all, so a click on the card did nothing visible.
//!
//! This is the fallback: when the notification names a desktop entry and its
//! sender claimed no default action of its own, the entry is launched. An app
//! that declared a default action wants to handle activation itself, and is left
//! to.

pub mod dbus;
pub mod desktop_entry;
pub mod exec;

use crate::error::Result;

pub use desktop_entry::DesktopEntry;
pub use exec::ExecLauncher;

/// Brings an application to the foreground, starting it if it is not running.
///
/// A trait so the daemon's activation path can be asserted without spawning a
/// process — the same reason [`NotificationSink`] is one.
///
/// [`NotificationSink`]: crate::ipc::NotificationSink
pub trait Launcher {
    /// `entry` is a desktop file id, with or without the `.desktop` suffix.
    fn activate(&self, entry: &str) -> Result<()>;
}

/// A launcher that does nothing, for a daemon built without one.
impl Launcher for () {
    fn activate(&self, _entry: &str) -> Result<()> {
        Ok(())
    }
}
