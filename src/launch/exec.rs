//! Starting an application by spawning its `Exec` line.
//!
//! The fallback for the entries that are not D-Bus activatable, which is still
//! most of them. Nothing here waits for the child or reads its output: the
//! daemon's job is to start the program, and blocking the compositor's event
//! loop on somebody else's start-up would freeze every notification on screen.

use std::process::{Command, Stdio};

use crate::{
    error::{Error, Result},
    launch::{DesktopEntry, Launcher, dbus},
};

/// Launches an application by running the `Exec` from its desktop entry.
#[derive(Debug, Default, Clone, Copy)]
pub struct ExecLauncher;

impl Launcher for ExecLauncher {
    fn activate(&self, entry: &str) -> Result<()> {
        let Some(desktop) = DesktopEntry::find(entry) else {
            return Err(Error::NoSuchDesktopEntry(entry.to_string()));
        };
        if desktop.dbus_activatable {
            // The standard `org.freedesktop.Application` path: the bus starts
            // the program if it is not running and hands the request to the
            // running instance if it is, which is what stops a second window
            // opening every time a notification is clicked.
            match dbus::activate(entry) {
                Ok(()) => return Ok(()),
                Err(error) => log::debug!("D-Bus activation of {entry:?} failed: {error}"),
            }
        }
        let Some(exec) = desktop.exec.as_deref() else {
            return Err(Error::NoSuchDesktopEntry(entry.to_string()));
        };
        let mut parts = exec.split_whitespace();
        let Some(program) = parts.next() else {
            return Err(Error::NoSuchDesktopEntry(entry.to_string()));
        };
        Command::new(program)
            .args(parts)
            // Detached from the daemon's own streams: an app that writes to
            // stdout must not end up interleaved with the daemon's log.
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        Ok(())
    }
}
