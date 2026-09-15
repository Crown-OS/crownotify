use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex, atomic::Ordering},
};

use calloop::ping::Ping;
use zbus::{fdo::Result, interface, object_server::SignalEmitter, zvariant::Value};

use crate::models::{Notification, general::GeneralNotification};

const NOTIFICATION_SPEC_VERSION: &str = "1.2";

pub type Inbox = Arc<Mutex<VecDeque<Notification>>>;

pub struct SystemNotificationInterface {
    inbox: Inbox,
    waker: Ping,
    current_id: std::sync::atomic::AtomicU32,
}

impl SystemNotificationInterface {
    pub fn new(inbox: Inbox, waker: Ping) -> Self {
        Self {
            inbox,
            waker,
            current_id: Default::default(),
        }
    }

    fn push(&self, notif: Notification) {
        if let Ok(mut inbox) = self.inbox.lock() {
            inbox.push_back(notif);
        }
        self.waker.ping();
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
        _app_icon: String,
        summary: String,
        body: String,
        actions: Vec<String>,
        _hints: HashMap<String, Value<'_>>,
        expire_timeout: i32,
    ) -> Result<u32> {
        let id = if replaces_id != 0 {
            replaces_id
        } else {
            self.current_id.fetch_add(1, Ordering::SeqCst) + 1
        };

        self.push(Notification::General(GeneralNotification {
            app_icon: None,
            app_name,
            summary,
            body,
            expire_timeout: expire_timeout.max(0) as u32,
            action: actions,
        }));

        Ok(id)
    }

    fn close_notification(&self, _id: u32) -> Result<()> {
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
