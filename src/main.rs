//! The crownotify daemon.
//!
//! Everything here is wiring: the modules below own the behaviour, and this
//! file only decides what runs on which thread and in what order.
//!
//! There are exactly two threads. The compositor's calloop event loop owns the
//! [`Store`], both layer surfaces and the crownos-ipc service, so a request
//! reaches the state without a lock. A second thread runs the session bus,
//! because zbus wants an async executor and the notification interface has to
//! answer `Notify` synchronously; it reaches the loop through a
//! `calloop::channel` and is fed replies through an `async_channel`.
//!
//! A failure on the D-Bus side is deliberately not fatal. The usual cause is
//! another notification server already owning
//! `org.freedesktop.Notifications`, and in that case everything CrownOS itself
//! sends — over crownos-ipc — still works.

use std::{
    sync::{Arc, atomic::AtomicU32},
    thread,
    time::{Duration, Instant},
};

use anyhow::Context;
use calloop::{
    channel::Event as ChannelEvent,
    timer::{TimeoutAction, Timer},
};
use crownos_ipc::adapter::calloop::CalloopService;
use crownotify::proto;
use crownotify::{
    config::Config,
    dbus,
    ipc::{self, Incoming, IncomingSender, Outgoing, OutgoingReceiver, OutgoingSender},
    model::CloseReason,
    store::Store,
    surface::{Center, Popup, Shared, SharedState, center, popup},
    ui::{Appearance, Theme},
};

/// How long the expiry timer sleeps when nothing on screen expires by itself.
///
/// Re-arming to the exact next deadline is what makes expiry punctual; this is
/// only the interval at which the timer notices that a notification has been
/// posted since it last looked. It costs nothing measurable — crownshell
/// already wakes the process twice a second for the surface tick.
const IDLE_EXPIRY_POLL: Duration = Duration::from_secs(1);

fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let config = Config::load();
    let theme = Theme::for_appearance(Appearance::from(config.appearance));

    let profiles = crownotify::profile::table::Profiles::new(&config);
    let mut store = Store::new(config.store());
    if let Some(mode) = config.initial_focus_mode()
        && let Err(error) = store.set_focus_mode(Some(mode))
    {
        log::warn!("{error}; starting with no focus mode");
    }
    // Taken before the store moves into the event loop: the D-Bus thread has to
    // allocate ids from the same counter, because `Notify` must return an id
    // long before the UI thread has seen the notification.
    let ids = store.id_counter();

    let (incoming_sender, incoming) = calloop::channel::channel::<Incoming>();
    let (outgoing, outgoing_receiver) = async_channel::unbounded::<Outgoing>();

    thread::Builder::new()
        .name("crownotify-dbus".to_string())
        .spawn(move || smol::block_on(session_bus(incoming_sender, ids, outgoing_receiver)))
        .context("spawning the session-bus thread")?;

    crownshell::run(move |app| {
        let (wake, wake_source) =
            calloop::ping::make_ping().context("creating the outgoing-event wake source")?;

        let mut shared = Shared::new(store, theme, outgoing.clone());
        shared.set_profiles(profiles);
        shared.set_wake(wake);
        let state = SharedState::new(shared);

        app.create_window(popup::config(), Popup::new(state.clone()));
        app.create_window(center::config(), Center::new(state.clone()));

        let handle = app.loop_handle();

        handle
            .insert_source(incoming, {
                let state = state.clone();
                move |event, _, _app| {
                    if let ChannelEvent::Msg(incoming) = event {
                        state.borrow_mut().apply(incoming);
                    }
                }
            })
            .map_err(|error| error.error)
            .context("registering the session-bus channel with the event loop")?;

        let service = ipc::serve(&handle, state.clone())
            .context("binding the crownotify crownos-ipc service")?;

        handle
            .insert_source(wake_source, {
                let state = state.clone();
                let mut buffer: Vec<Outgoing> = Vec::new();
                // `service` is parked in this callback on purpose. A
                // `CalloopService` deregisters every source it owns when
                // dropped, so it has to outlive the setup closure, and this
                // callback is also the only place allowed to emit through it —
                // it never runs inside the service's own `dispatch`.
                move |_, _, _app| {
                    state.borrow_mut().drain_outgoing(&mut buffer);
                    for event in buffer.drain(..) {
                        publish(&service, &outgoing, event);
                    }
                }
            })
            .map_err(|error| error.error)
            .context("registering the outgoing-event wake source")?;

        handle
            .insert_source(Timer::immediate(), {
                let state = state.clone();
                move |_deadline, _, _app| {
                    let mut shared = state.borrow_mut();
                    let now = Instant::now();
                    let expired = shared.store.expire(now);
                    for id in expired {
                        shared.publish(Outgoing::Closed(id, CloseReason::Expired));
                        shared.bump();
                    }
                    TimeoutAction::ToDuration(
                        shared.store.next_expiry(now).unwrap_or(IDLE_EXPIRY_POLL),
                    )
                }
            })
            .map_err(|error| error.error)
            .context("arming the notification expiry timer")?;

        Ok(())
    })
    .context("running the crownotify surfaces")
}

/// Serves `org.freedesktop.Notifications` and turns the UI thread's events into
/// its signals, until the process ends.
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

/// Publishes one event to both transports.
///
/// Every subscriber of either transport hears about everything it has a name
/// for; the D-Bus side quietly drops the two events the freedesktop interface
/// has no signal for.
fn publish(service: &CalloopService, outgoing: &OutgoingSender, event: Outgoing) {
    if let Err(error) = emit(service, &event) {
        log::warn!("broadcasting {event:?} over crownos-ipc failed: {error}");
    }
    if let Err(error) = outgoing.try_send(event) {
        log::debug!("the session-bus thread is not listening: {error}");
    }
}

/// Broadcasts `event` to the crownos-ipc subscribers.
fn emit(service: &CalloopService, event: &Outgoing) -> Result<(), crownos_ipc::Error> {
    match event {
        Outgoing::Closed(id, reason) => service.emit(&proto::NotificationClosed {
            id: *id,
            reason: reason.as_u32() as u8,
        }),
        Outgoing::ActionInvoked(id, action) => service.emit(&proto::ActionInvoked {
            id: *id,
            action: action.clone(),
        }),
        Outgoing::Replied(id, text) => service.emit(&proto::ReplySubmitted {
            id: *id,
            text: text.clone(),
        }),
        Outgoing::CenterVisibility(open) => service.emit(&proto::CenterVisibility { open: *open }),
        Outgoing::DoNotDisturbChanged(enabled) => {
            service.emit(&proto::DoNotDisturbChanged { enabled: *enabled })
        }
        Outgoing::FocusModeChanged(name) => service.emit(&proto::FocusModeChanged {
            name: name.clone().unwrap_or_default(),
        }),
    }
}
