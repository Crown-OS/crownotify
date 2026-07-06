use calloop::ping::Ping;
use zbus::{fdo::Result, interface};

use crate::{
    dbus::freedesktop::Inbox,
    models::{
        call::CallNotification, chat::ChatNotification, general::GeneralNotification,
        music::MusicNotification, Notification,
    },
};

pub struct CustomNotificationInterface {
    inbox: Inbox,
    waker: Ping,
}

impl CustomNotificationInterface {
    pub fn new(inbox: Inbox, waker: Ping) -> Self {
        Self { inbox, waker }
    }

    fn push(&self, notif: Notification) {
        if let Ok(mut inbox) = self.inbox.lock() {
            inbox.push_back(notif);
        }
        self.waker.ping();
    }
}

#[interface(name = "io.crownos.crownotify")]
impl CustomNotificationInterface {
    fn open_notification_center(&self) -> Result<()> {
        log::info!("notification center open");
        Ok(())
    }

    fn close_notification_center(&self) -> Result<()> {
        log::info!("notification center closed");
        Ok(())
    }

    fn send_general_notification(
        &self,
        app_name: String,
        summary: String,
        body: String,
        expire_timeout: u32,
        actions: Vec<String>,
    ) -> Result<()> {
        self.push(Notification::General(GeneralNotification {
            app_icon: None,
            app_name,
            summary,
            body,
            expire_timeout,
            action: actions,
        }));
        Ok(())
    }

    fn send_call_notification(
        &self,
        call_id: String,
        app_name: String,
        contact_avatar: String,
        contact_name: String,
        phone_number: String,
    ) -> Result<()> {
        self.push(Notification::Call(CallNotification {
            call_id,
            app_icon: None,
            app_name,
            contact_avatar,
            contact_name,
            phone_number,
        }));
        Ok(())
    }

    fn send_music_notification(&self, song_name: String, percentage: u8) -> Result<()> {
        self.push(Notification::Music(MusicNotification {
            app_icon: None,
            song_name,
            percentage,
        }));
        Ok(())
    }

    fn send_chat_notification(
        &self,
        app_name: String,
        title: String,
        body: String,
        timestamp: String,
    ) -> Result<()> {
        self.push(Notification::Chat(ChatNotification {
            icon: None,
            title,
            app_name,
            body,
            timestamp,
        }));
        Ok(())
    }
}
