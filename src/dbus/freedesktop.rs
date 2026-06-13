/*
* Dbus interface to receive notifications though dbus based on freedesktop.org specifications
* (https://specifications.freedesktop.org/notification/latest/protocol.html)
*/

use std::{collections::HashMap, sync::atomic::Ordering};

use iced::futures::channel::mpsc::UnboundedSender;
use zbus::{fdo::Result, interface, object_server::SignalEmitter, zvariant::Value};

use crate::models::{general::GeneralNotification, Notification};
use crate::ui::components::icon::{Icon, LocalIcon};

const NOTIFICATION_SPEC_VERSION: &str = "1.2";
pub struct SystemNotificationInterface {
    sender: UnboundedSender<Notification>,
    current_id: std::sync::atomic::AtomicU32,
}

impl SystemNotificationInterface {
    pub fn new(sender: UnboundedSender<Notification>) -> Self {
        Self {
            sender,
            current_id: Default::default(),
        }
    }
}

#[interface(name = "org.freedesktop.Notifications")]
impl SystemNotificationInterface {
    fn get_server_information(&self) -> (String, String, String, String) {
        (
            env!("CARGO_PKG_NAME").to_string(),
            env!("CARGO_PKG_AUTHORS").to_string(),
            env!("CARGO_PKG_VERSION").to_string(),
            NOTIFICATION_SPEC_VERSION.to_string(),
        )
    }

    fn get_capabilities(&self) -> Vec<String> {
        vec![
            "action-icons".to_string(),
            "actions".to_string(),
            "body".to_string(),
            "body-images".to_string(),
            "icon-multi".to_string(),
            "icon-static".to_string(),
            "persistence".to_string(),
            "sound".to_string(),
        ]
    }

    fn notify(
        &self,
        app_name: String,
        replaces_id: u32,
        app_icon: String,
        summary: String,
        body: String,
        actions: Vec<String>,
        hints: HashMap<String, Value<'_>>,
        expire_timeout: i32,
    ) -> Result<u32> {
        let id = if replaces_id != 0 {
            replaces_id
        } else {
            self.current_id.fetch_add(1, Ordering::SeqCst) + 1
        };

        self.sender
            .unbounded_send(Notification::General(GeneralNotification {
                app_icon: Icon::Local(LocalIcon {}),
                app_name,
                summary,
                body,
                expire_timeout: expire_timeout.max(0) as u32,
                action: actions,
            }))
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;

        Ok(id)
    }

    fn close_notification(&self, id: u32) -> Result<()> {
        println!("close");
        Ok(())
    }

    #[zbus(signal)]
    async fn notitication_closed(
        signal: &SignalEmitter<'_>,
        id: u32,
        reason: u32,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn action_invoked(
        signal: &SignalEmitter<'_>,
        id: u32,
        action_key: String,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn activation_token(
        signal: &SignalEmitter<'_>,
        id: u32,
        activation_token: String,
    ) -> zbus::Result<()>;
}
