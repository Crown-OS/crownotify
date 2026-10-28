//! The core thread: the store, the crownos-ipc service, expiry and the phone
//! link, all on one calloop loop.
//!
//! The windows run on crownui's event loop and the session bus on a thread of
//! its own; this loop is where everything they say meets the store, so a
//! request is applied without a lock. After every iteration that moved the
//! daemon's version, a fresh [`Snapshot`] goes out to the windows.

use std::{
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

use anyhow::Context;
use calloop::{
    EventLoop,
    channel::{Channel, Event as ChannelEvent},
    timer::{TimeoutAction, Timer},
};
use crownos_ipc::adapter::calloop::CalloopService;
use tokio::sync::watch;

use crate::{
    daemon::{Daemon, DaemonState, Request, RequestSender, Snapshot},
    ipc::{self, Incoming, Outgoing, OutgoingSender},
    model::CloseReason,
    proto,
    store::Store,
};

/// How long the expiry timer sleeps when nothing on screen expires by itself.
///
/// Re-arming to the exact next deadline is what makes expiry punctual; this is
/// only the interval at which the timer notices that a notification has been
/// posted since it last looked.
const IDLE_EXPIRY_POLL: Duration = Duration::from_secs(1);
/// How often the phone link retries a connection and flushes what is queued.
const PHONE_PUMP: Duration = Duration::from_millis(500);

/// The windows' handles to the running core.
pub struct Core {
    pub requests: RequestSender,
    pub snapshots: watch::Receiver<Arc<Snapshot>>,
}

/// Starts the core thread with `store`, fed by the session bus through
/// `incoming` and publishing to it through `outgoing`.
///
/// Returns once the crownos-ipc socket is bound, so a failure to serve it is
/// the caller's error rather than a thread dying quietly.
pub fn spawn(
    store: Store,
    incoming: Channel<Incoming>,
    outgoing: OutgoingSender,
) -> anyhow::Result<Core> {
    let (requests, request_channel) = calloop::channel::channel::<Request>();
    let (snapshots, snapshot_receiver) = watch::channel(Arc::new(Snapshot::default()));
    let (started, ready) = std::sync::mpsc::channel::<anyhow::Result<()>>();
    thread::Builder::new()
        .name("crownotify-core".to_string())
        .spawn(move || {
            let mut event_loop = match build(store, incoming, request_channel, outgoing) {
                Ok(event_loop) => event_loop,
                Err(error) => {
                    let _ = started.send(Err(error));
                    return;
                }
            };
            let _ = started.send(Ok(()));
            let mut published = None;
            let ran = event_loop.0.run(None, &mut event_loop.1, |state| {
                let version = state.version();
                if published != Some(version) {
                    published = Some(version);
                    snapshots.send_replace(Arc::new(state.borrow().snapshot()));
                }
            });
            if let Err(error) = ran {
                log::error!("the notification core stopped: {error}");
            }
        })
        .context("spawning the core thread")?;
    ready
        .recv()
        .context("the core thread exited before it started")??;
    Ok(Core {
        requests,
        snapshots: snapshot_receiver,
    })
}

type CoreLoop = (EventLoop<'static, DaemonState>, DaemonState);

fn build(
    store: Store,
    incoming: Channel<Incoming>,
    requests: Channel<Request>,
    outgoing: OutgoingSender,
) -> anyhow::Result<CoreLoop> {
    let event_loop: EventLoop<'static, DaemonState> =
        EventLoop::try_new().context("creating the core event loop")?;
    let handle = event_loop.handle();
    let (wake, wake_source) =
        calloop::ping::make_ping().context("creating the outgoing-event wake source")?;
    let mut daemon = Daemon::new(store, outgoing.clone());
    daemon.set_wake(wake);
    let state = DaemonState::new(daemon);

    handle
        .insert_source(incoming, |event, _, state| {
            if let ChannelEvent::Msg(incoming) = event {
                state.borrow_mut().apply(incoming);
            }
        })
        .map_err(|error| error.error)
        .context("registering the session-bus channel")?;

    handle
        .insert_source(requests, |event, _, state| {
            if let ChannelEvent::Msg(request) = event {
                state.borrow_mut().handle(request);
            }
        })
        .map_err(|error| error.error)
        .context("registering the window request channel")?;

    let service =
        ipc::serve(&handle, state.clone()).context("binding the crownotify crownos-ipc service")?;

    let mut buffer: Vec<Outgoing> = Vec::new();
    // `service` is parked in this callback on purpose. A `CalloopService`
    // deregisters every source it owns when dropped, so it has to outlive the
    // setup, and this callback is also the only place allowed to emit through
    // it — it never runs inside the service's own `dispatch`.
    handle
        .insert_source(wake_source, move |_, _, state| {
            state.borrow_mut().drain_outgoing(&mut buffer);
            for event in buffer.drain(..) {
                publish(&service, &outgoing, event);
            }
        })
        .map_err(|error| error.error)
        .context("registering the outgoing-event wake source")?;

    handle
        .insert_source(Timer::immediate(), |_deadline, _, state| {
            let mut daemon = state.borrow_mut();
            let now = Instant::now();
            for id in daemon.store.expire(now) {
                daemon.publish(Outgoing::Closed(id, CloseReason::Expired));
                daemon.bump();
            }
            TimeoutAction::ToDuration(daemon.store.next_expiry(now).unwrap_or(IDLE_EXPIRY_POLL))
        })
        .map_err(|error| error.error)
        .context("arming the notification expiry timer")?;

    handle
        .insert_source(Timer::from_duration(PHONE_PUMP), |_deadline, _, state| {
            state.borrow_mut().phone.pump();
            TimeoutAction::ToDuration(PHONE_PUMP)
        })
        .map_err(|error| error.error)
        .context("arming the phone link pump")?;

    Ok((event_loop, state))
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
