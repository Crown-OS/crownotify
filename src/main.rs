use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    thread,
};

use anyhow::{anyhow, Result};
use calloop::ping::{make_ping, Ping};
use crownshell::{Anchor, App, KeyboardInteractivity, Layer, WindowConfig};

use crownotify::{
    dbus::{custom::CustomNotificationInterface, freedesktop::SystemNotificationInterface},
    models::Notification,
    notify_handler::NotifyHandler,
};

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let inbox: Arc<Mutex<VecDeque<Notification>>> = Arc::new(Mutex::new(VecDeque::new()));
    let (waker, ping_source) = make_ping().map_err(|e| anyhow!("make_ping: {e}"))?;

    let dbus_inbox = inbox.clone();
    let dbus_waker = waker.clone();
    thread::spawn(move || {
        smol::block_on(async move {
            if let Err(e) = start_dbus(dbus_inbox, dbus_waker).await {
                log::error!("dbus setup failed: {e:#}");
            }
            std::future::pending::<()>().await;
        });
    });

    crownshell::run(move |app| {
        let handler = NotifyHandler::new(inbox.clone());
        let config = WindowConfig {
            namespace: "crownotify".to_string(),
            layer: Layer::Top,
            anchor: Anchor::TOP | Anchor::RIGHT | Anchor::BOTTOM,
            size: (400, 0),
            exclusive_zone: 0,
            keyboard_interactivity: KeyboardInteractivity::None,
            blur: false,
            ..Default::default()
        };
        app.create_window(config, handler);
        app.loop_handle
            .insert_source(ping_source, |_, _, app: &mut App| {
                let App {
                    compositor_state,
                    qh,
                    windows,
                    ..
                } = app;
                for window in windows.iter_mut() {
                    window.request_frame(compositor_state, qh);
                }
            })
            .map_err(|e| anyhow!("insert ping source: {}", e.error))?;
        Ok(())
    })
}

async fn start_dbus(inbox: Arc<Mutex<VecDeque<Notification>>>, waker: Ping) -> Result<()> {
    let system = SystemNotificationInterface::new(inbox.clone(), waker.clone());
    let custom = CustomNotificationInterface::new(inbox, waker);

    let system_conn = zbus::connection::Builder::session()?
        .name("org.freedesktop.Notifications")?
        .serve_at("/org/freedesktop/Notifications", system)?
        .build()
        .await?;

    let custom_conn = zbus::connection::Builder::session()?
        .name("io.crownos.crownotify")?
        .serve_at("/io/crownos/crownotify", custom)?
        .build()
        .await?;

    // Keep connections alive for the process lifetime.
    std::mem::forget(system_conn);
    std::mem::forget(custom_conn);
    Ok(())
}
