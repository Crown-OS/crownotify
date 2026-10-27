//! Starting an application through `org.freedesktop.Application`.
//!
//! The path a `DBusActivatable=true` entry asks for, and the better one: the
//! bus starts the program if it is not running and routes the request to the
//! running instance if it is, so clicking a notification twice does not open
//! two windows.
//!
//! This runs on the compositor's thread, so the call is made on a short-lived
//! blocking connection with its own executor rather than borrowing the daemon's
//! session bus — which lives on the other thread and is busy serving `Notify`.

use zbus::blocking::Connection;

use crate::{error::Result, launch::DesktopEntry};

/// Calls `Activate` on the application's well-known name.
pub fn activate(entry: &str) -> Result<()> {
    let name = entry.trim_end_matches(".desktop").to_string();
    let path = DesktopEntry::object_path(&name);
    let connection = Connection::session()?;
    connection.call_method(
        Some(name.as_str()),
        path.as_str(),
        Some("org.freedesktop.Application"),
        "Activate",
        // The spec's `platform_data`: an activation token would go here, which
        // is what would stop the shell's focus-stealing prevention from
        // swallowing the window. crownshell does not bind `xdg_activation_v1`
        // yet, so the app still comes up — it may just not take focus, which is
        // the safer of the two failures.
        &std::collections::HashMap::<&str, zbus::zvariant::Value<'_>>::new(),
    )?;
    Ok(())
}
