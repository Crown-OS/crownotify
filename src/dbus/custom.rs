/*
* Dbus interface to receive notifications from peer CrownOS Apps
*/

use iced::futures::channel::mpsc::UnboundedSender;
use zbus::{fdo::Result, interface};

use crate::models::{
    call::CallNotification, chat::ChatNotification, general::GeneralNotification,
    music::MusicNotification, Notification,
};
use crate::ui::components::icon::{Icon, LocalIcon};

pub struct CustomNotificationInterface {
    pub sender: UnboundedSender<Notification>,
}

impl CustomNotificationInterface {
    pub fn new(sender: UnboundedSender<Notification>) -> Self {
        Self { sender }
    }
}

#[interface(name = "io.crownos.crownotify")]
impl CustomNotificationInterface {
    fn open_notification_center(&self) -> Result<()> {
        println!("Notification center open");
        Ok(())
    }

    fn close_notification_center(&self) -> Result<()> {
        println!("Notification center closed");
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
        self.sender
            .unbounded_send(Notification::General(GeneralNotification {
                app_icon: Icon::Local(LocalIcon {}),
                app_name,
                summary,
                body,
                expire_timeout,
                action: actions,
            }))
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))
    }

    fn send_call_notification(&self, contact_name: String) -> Result<()> {
        self.sender
            .unbounded_send(Notification::Call(CallNotification {
                app_icon: Icon::Local(LocalIcon {}),
                contact_name,
            }))
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))
    }

    fn send_music_notification(&self, song_name: String, percentage: u8) -> Result<()> {
        self.sender
            .unbounded_send(Notification::Music(MusicNotification {
                app_icon: Icon::Local(LocalIcon {}),
                song_name,
                percentage,
            }))
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))
    }

    fn send_chat_notification(
        &self,
        app_name: String,
        title: String,
        body: String,
        timestamp: String,
    ) -> Result<()> {
        self.sender
            .unbounded_send(Notification::Chat(ChatNotification {
                icon: Icon::Local(LocalIcon {}),
                title,
                app_name,
                body,
                timestamp,
            }))
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))
    }
}
