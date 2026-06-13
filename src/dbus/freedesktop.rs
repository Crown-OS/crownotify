use std::{collections::HashMap, sync::atomic::Ordering};

use zbus::{fdo::Result, interface, zvariant::Value};

const NOTIFICATION_SPEC_VERSION: &str = "1.2";

#[derive(Default)]
pub struct NotificationDaemon {
    current_id: std::sync::atomic::AtomicU32,
}

#[interface(name = "org.freedesktop.Notifications")]
impl NotificationDaemon {
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
        let id = self.current_id.fetch_add(1, Ordering::SeqCst) + 1;
        println!(
            "App Name: {}; body: {}; actions: {:?}",
            app_name, body, actions
        );
        Ok(id)
    }

    fn close_notification(&self, id: u32) -> Result<()> {
        println!("close");
        Ok(())
    }
}
