mod dbus;
use std::error::Error;
use std::future::pending;
use zbus::connection;

use crate::dbus::freedesktop::NotificationDaemon;

fn main() -> Result<(), Box<dyn Error>> {
    smol::block_on(async {
        let daemon = NotificationDaemon::default();
        let _conn = connection::Builder::session()?
            .name("org.freedesktop.Notifications")?
            .serve_at("/org/freedesktop/Notifications", daemon)?
            .build()
            .await?;

        pending::<()>().await;
        Ok(())
    })
}
