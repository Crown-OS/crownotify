use std::error::Error;
use std::future::pending;

use iced::futures::channel::mpsc;
use zbus::connection;

use crownotify::dbus::custom::CustomNotificationInterface;
use crownotify::dbus::freedesktop::SystemNotificationInterface;
use crownotify::models::Notification;
use crownotify::ui::app::create_window;

fn main() -> Result<(), Box<dyn Error>> {
    smol::block_on(async {
        let (tx, rx) = mpsc::unbounded::<Notification>();

        let system_notification_interface = SystemNotificationInterface::new(tx.clone());
        let custom_notification_interface = CustomNotificationInterface::new(tx.clone());

        let _system_conn = connection::Builder::session()?
            .name("org.freedesktop.Notifications")?
            .serve_at(
                "/org/freedesktop/Notifications",
                system_notification_interface,
            )?
            .build()
            .await?;

        let _custom_conn = connection::Builder::session()?
            .name("io.crownos.crownotify")?
            .serve_at("/io/crownos/crownotify", custom_notification_interface)?
            .build()
            .await?;

        create_window(rx)?;

        pending::<()>().await;
        Ok(())
    })
}
