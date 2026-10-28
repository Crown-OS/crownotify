//! The crownotify daemon.
//!
//! Everything here is wiring: the modules below own the behaviour, and this
//! file only decides what runs on which thread and in what order.
//!
//! There are three threads. The core thread's calloop loop owns the
//! [`Store`], the crownos-ipc service and the expiry timer, so a request
//! reaches the state without a lock. crownui's event loop runs the two windows
//! — the toast strip and the notification center — which follow the core's
//! snapshots and send it requests. A third thread runs the session bus,
//! because zbus wants an async executor and the notification interface has to
//! answer `Notify` synchronously; it reaches the core through a
//! `calloop::channel` and is fed replies through an `async_channel`.
//!
//! A failure on the D-Bus side is deliberately not fatal. The usual cause is
//! another notification server already owning
//! `org.freedesktop.Notifications`, and in that case everything CrownOS itself
//! sends — over crownos-ipc — still works.

use std::{
    sync::{Arc, atomic::AtomicU32},
    thread,
};

use anyhow::Context;
use crownotify::{
    config::Config,
    daemon, dbus,
    ipc::{Incoming, IncomingSender, Outgoing, OutgoingReceiver},
    profile::table::Profiles,
    store::Store,
    ui::{Appearance, Theme, Window, center, popup},
};
use crownui::prelude::View;

fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let config = Config::load();
    let theme = Theme::for_appearance(Appearance::from(config.appearance));

    let mut store = Store::new(config.store());
    if let Some(mode) = config.initial_focus_mode()
        && let Err(error) = store.set_focus_mode(Some(mode))
    {
        log::warn!("{error}; starting with no focus mode");
    }
    // Taken before the store moves to the core thread: the D-Bus thread has to
    // allocate ids from the same counter, because `Notify` must return an id
    // long before the core has seen the notification.
    let ids = store.id_counter();

    let (incoming_sender, incoming) = calloop::channel::channel::<Incoming>();
    let (outgoing, outgoing_receiver) = async_channel::unbounded::<Outgoing>();
    let core =
        daemon::spawn(store, incoming, outgoing).context("starting the notification core")?;

    thread::Builder::new()
        .name("crownotify-dbus".to_string())
        .spawn(move || smol::block_on(session_bus(incoming_sender, ids, outgoing_receiver)))
        .context("spawning the session-bus thread")?;

    let window = |config: &Config| Window {
        theme,
        profiles: Profiles::new(config),
        requests: core.requests.clone(),
        snapshots: core.snapshots.clone(),
    };
    crownui::run_windows(vec![
        (popup::popup(window(&config)).boxed(), popup::config()),
        (center::center(window(&config)).boxed(), center::config()),
    ])
    .context("running the crownotify windows")
}

/// Serves `org.freedesktop.Notifications` and turns the core's events into its
/// signals, until the process ends.
///
/// Neither failure ends the daemon: losing the freedesktop interface costs
/// third-party notifications, and taking the whole server down instead would
/// also cost every CrownOS one.
async fn session_bus(incoming: IncomingSender, ids: Arc<AtomicU32>, outgoing: OutgoingReceiver) {
    let connection = match dbus::serve(incoming, ids).await {
        Ok(connection) => connection,
        Err(error) => {
            log::error!(
                "{} is unavailable ({error:#}); crownos-ipc notifications still work",
                dbus::BUS_NAME
            );
            return;
        }
    };
    log::info!("serving {}", dbus::BUS_NAME);
    if let Err(error) = dbus::emit_signals(&connection, outgoing).await {
        log::error!("the notification signal driver stopped: {error:#}");
    }
}
