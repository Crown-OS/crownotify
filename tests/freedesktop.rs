//! Live session-bus cover for `org.freedesktop.Notifications`.
//!
//! These tests talk to the real interface over a real bus with a real
//! `#[proxy]` client, because the parts most likely to be wrong — the
//! signature of `Notify`, how a hint is spelled, whether a variant decodes —
//! are exactly the parts a unit test cannot see.
//!
//! Two things keep them honest on a developer's machine:
//!
//! * they **skip** when there is no session bus, so a container without one
//!   still passes rather than failing for an unrelated reason;
//! * they **serialise** behind a process-wide mutex, because a notification
//!   server is a well-known bus name and only one process may own it.
//!
//! The interface is served on an anonymous connection rather than under
//! [`BUS_NAME`], so the suite never fights the notification daemon the
//! developer is already running. One test does claim the name, and skips if it
//! is taken.

// `Notify` takes eight arguments; that is the specification's choice, not this
// file's.
#![allow(clippy::too_many_arguments)]

use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex, MutexGuard, OnceLock,
        atomic::{AtomicU32, Ordering},
    },
    time::Duration,
};

use crownotify::{
    dbus::freedesktop::{self, BUS_NAME, Notifications, OBJECT_PATH},
    ipc::{Incoming, Outgoing},
    model::{CloseReason, Content, DeviceKind, RelayDevice, Timeout, Urgency},
};
use smol::stream::StreamExt;
use zbus::{
    Connection, connection, proxy,
    zvariant::{Array, Value},
};

/// How long a signal or a reply may take before the test gives up.
const PATIENCE: Duration = Duration::from_secs(5);

/// Tests share well-known D-Bus names on the session bus, so they must not run
/// concurrently — otherwise the second registration silently queues and the
/// wrong server receives the call.
fn bus_test_lock() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    let lock = LOCK.get_or_init(|| Mutex::new(()));
    match lock.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

/// Whether there is a session bus to talk to at all.
fn session_bus_available() -> bool {
    if std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_some() {
        return true;
    }
    eprintln!("skipping: DBUS_SESSION_BUS_ADDRESS is unset, so there is no session bus");
    false
}

#[proxy(
    interface = "org.freedesktop.Notifications",
    default_service = "org.freedesktop.Notifications",
    default_path = "/org/freedesktop/Notifications"
)]
trait Notify {
    fn notify(
        &self,
        app_name: &str,
        replaces_id: u32,
        app_icon: &str,
        summary: &str,
        body: &str,
        actions: &[&str],
        hints: HashMap<&str, Value<'_>>,
        expire_timeout: i32,
    ) -> zbus::Result<u32>;

    fn close_notification(&self, id: u32) -> zbus::Result<()>;

    fn get_capabilities(&self) -> zbus::Result<Vec<String>>;

    fn get_server_information(&self) -> zbus::Result<(String, String, String, String)>;

    #[zbus(signal)]
    fn notification_closed(&self, id: u32, reason: u32) -> zbus::Result<()>;
}

/// A notification server on a private connection, plus the channel its
/// `Incoming` messages land in.
struct Server {
    connection: Connection,
    incoming: calloop::channel::Channel<Incoming>,
    ids: Arc<AtomicU32>,
}

impl Server {
    async fn start() -> Self {
        let (sender, incoming) = calloop::channel::channel();
        let ids = Arc::new(AtomicU32::new(0));
        let connection = connection::Builder::session()
            .expect("session bus connection failed")
            .serve_at(OBJECT_PATH, Notifications::new(sender, Arc::clone(&ids)))
            .expect("serve_at failed")
            .build()
            .await
            .expect("connection build failed");
        Self {
            connection,
            incoming,
            ids,
        }
    }

    /// A proxy aimed at this server's unique name rather than the well-known
    /// one, which is what lets the suite run beside a real notification daemon.
    async fn proxy(&self) -> NotifyProxy<'static> {
        let destination = self
            .connection
            .unique_name()
            .expect("a bus connection always has a unique name")
            .to_owned();
        NotifyProxy::builder(&self.connection)
            .destination(destination)
            .expect("destination")
            .path(OBJECT_PATH)
            .expect("path")
            .build()
            .await
            .expect("proxy build failed")
    }

    /// The next thing the interface asked the UI thread to do.
    fn next_incoming(&self) -> Incoming {
        self.incoming
            .try_recv()
            .expect("the interface should have delivered an Incoming")
    }
}

/// The `(iiibiiay)` image hint for a 1x1 opaque red pixel.
fn red_pixel() -> Value<'static> {
    Value::from((
        1i32,
        1i32,
        4i32,
        true,
        8i32,
        4i32,
        Array::from(vec![255u8, 0, 0, 255]),
    ))
}

#[test]
fn notify_returns_a_live_id_and_delivers_every_hint_it_understood() {
    if !session_bus_available() {
        return;
    }
    let _guard = bus_test_lock();

    smol::block_on(async {
        let server = Server::start().await;
        let proxy = server.proxy().await;

        let hints = HashMap::from([
            ("urgency", Value::U8(2)),
            ("desktop-entry", Value::from("org.signal.Signal")),
            ("transient", Value::Bool(true)),
            ("resident", Value::Bool(true)),
            ("category", Value::from("x-crownos.chat")),
            ("x-crownos.conversation-id", Value::from("thread-3")),
            ("image-data", red_pixel()),
            ("suppress-sound", Value::Bool(true)),
            ("x-crownos.device-kind", Value::from("phone")),
            ("x-crownos.device-name", Value::from("Ada's phone")),
        ]);

        let id = proxy
            .notify(
                "Signal",
                0,
                "signal",
                "Ada",
                "On my way",
                &["default", "Open", "inline-reply", "Reply to Ada"],
                hints,
                7000,
            )
            .await
            .expect("Notify failed");
        assert_ne!(id, 0, "0 means 'new' in the spec and is never a live id");

        let Incoming::Post(notification) = server.next_incoming() else {
            panic!("Notify should deliver a Post");
        };
        assert_eq!(notification.id, id, "the reply must name the notification");
        assert_eq!(notification.app.name, "Signal");
        assert_eq!(notification.summary, "Ada");
        assert_eq!(notification.body, "On my way");
        assert_eq!(notification.urgency, Urgency::Critical);
        assert_eq!(notification.timeout, Timeout::Millis(7000));
        assert_eq!(
            notification.app.desktop_entry.as_deref(),
            Some("org.signal.Signal")
        );
        assert!(notification.transient, "the transient hint was set");
        assert!(notification.resident, "the resident hint was set");
        assert!(
            notification.image.is_some(),
            "the image-data hint should have decoded to pixels"
        );

        assert_eq!(
            notification.default_action().map(|a| a.label.as_str()),
            Some("Open")
        );
        assert!(
            notification.reply_action().is_some(),
            "the inline-reply action pair should have been recognised"
        );

        let Content::Chat(chat) = &notification.content else {
            panic!(
                "x-crownos.chat should select a chat card, got {:?}",
                notification.content
            );
        };
        assert_eq!(chat.conversation_id, "thread-3");
        assert_eq!(chat.sender, "Ada");

        assert_eq!(
            notification.relayed_from,
            Some(RelayDevice::new(DeviceKind::Phone, "Ada's phone")),
            "the device hints should have badged the card as relayed"
        );
    });
}

#[test]
fn a_replaces_id_is_preserved_so_a_notification_can_be_edited_in_place() {
    if !session_bus_available() {
        return;
    }
    let _guard = bus_test_lock();

    smol::block_on(async {
        let server = Server::start().await;
        let proxy = server.proxy().await;

        let first = proxy
            .notify("Mail", 0, "", "one", "", &[], HashMap::new(), -1)
            .await
            .expect("Notify failed");
        let Incoming::Post(_) = server.next_incoming() else {
            panic!("expected the first Post");
        };

        let second = proxy
            .notify(
                "Mail",
                first,
                "",
                "one, edited",
                "",
                &[],
                HashMap::new(),
                -1,
            )
            .await
            .expect("Notify failed");
        assert_eq!(second, first, "a replacement keeps the original id");

        let Incoming::Post(notification) = server.next_incoming() else {
            panic!("expected the replacement Post");
        };
        assert_eq!(notification.id, first);
        assert_eq!(notification.summary, "one, edited");
        assert_eq!(
            server.ids.load(Ordering::Relaxed),
            1,
            "a replacement must not burn a fresh id"
        );
    });
}

#[test]
fn a_malformed_image_hint_costs_the_icon_and_nothing_else() {
    if !session_bus_available() {
        return;
    }
    let _guard = bus_test_lock();

    smol::block_on(async {
        let server = Server::start().await;
        let proxy = server.proxy().await;

        // 64x64 RGBA claimed, two bytes supplied: geometry that does not
        // describe the buffer it arrived with.
        let lie = Value::from((
            64i32,
            64i32,
            256i32,
            true,
            8i32,
            4i32,
            Array::from(vec![0u8, 0]),
        ));
        let id = proxy
            .notify(
                "Hostile",
                0,
                "",
                "still here",
                "",
                &[],
                HashMap::from([("image-data", lie)]),
                -1,
            )
            .await
            .expect("a broken hint must not fail the call");
        assert_ne!(id, 0);

        let Incoming::Post(notification) = server.next_incoming() else {
            panic!("the notification should still have been delivered");
        };
        assert_eq!(notification.summary, "still here");
        assert!(
            notification.image.is_none(),
            "the undecodable image should have been dropped, not guessed at"
        );

        let next = proxy
            .notify(
                "Mail",
                0,
                "",
                "and the service is alive",
                "",
                &[],
                HashMap::new(),
                -1,
            )
            .await
            .expect("the service should still be serving");
        assert_ne!(next, id, "the server kept allocating ids");
    });
}

#[test]
fn close_notification_asks_the_ui_to_close_with_the_specs_reason() {
    if !session_bus_available() {
        return;
    }
    let _guard = bus_test_lock();

    smol::block_on(async {
        let server = Server::start().await;
        let proxy = server.proxy().await;

        proxy
            .close_notification(42)
            .await
            .expect("CloseNotification failed");

        match server.next_incoming() {
            Incoming::Close(id, reason) => {
                assert_eq!(id, 42);
                assert_eq!(
                    reason,
                    CloseReason::Closed,
                    "reason 3 is 'closed by a call to CloseNotification'"
                );
            }
            other => panic!("expected a Close, got {other:?}"),
        }
    });
}

#[test]
fn get_capabilities_advertises_inline_reply_alongside_the_spec_set() {
    if !session_bus_available() {
        return;
    }
    let _guard = bus_test_lock();

    smol::block_on(async {
        let server = Server::start().await;
        let capabilities = server
            .proxy()
            .await
            .get_capabilities()
            .await
            .expect("GetCapabilities failed");

        for expected in [
            "actions",
            "action-icons",
            "body",
            "body-markup",
            "body-hyperlinks",
            "body-images",
            "icon-static",
            "persistence",
            "sound",
            "inline-reply",
        ] {
            assert!(
                capabilities.iter().any(|capability| capability == expected),
                "{expected} is missing from {capabilities:?}"
            );
        }
    });
}

#[test]
fn get_server_information_identifies_this_daemon_and_the_spec_it_implements() {
    if !session_bus_available() {
        return;
    }
    let _guard = bus_test_lock();

    smol::block_on(async {
        let server = Server::start().await;
        let (name, vendor, version, spec) = server
            .proxy()
            .await
            .get_server_information()
            .await
            .expect("GetServerInformation failed");

        assert_eq!(name, "crownotify");
        assert!(!vendor.is_empty(), "the vendor should name the author");
        assert!(
            version.split('.').count() >= 2,
            "the version should look like a version, got {version:?}"
        );
        assert_eq!(spec, "1.2");
    });
}

#[test]
fn the_signal_driver_turns_an_outgoing_closure_into_a_notification_closed() {
    if !session_bus_available() {
        return;
    }
    let _guard = bus_test_lock();

    smol::block_on(async {
        let server = Server::start().await;
        let proxy = server.proxy().await;
        let mut closures = proxy
            .receive_notification_closed()
            .await
            .expect("signal subscription failed");

        let (outgoing, receiver) = async_channel::unbounded();
        outgoing
            .send(Outgoing::Closed(9, CloseReason::Expired))
            .await
            .expect("queueing an outgoing event");
        // The driver runs until the channel closes, which is what makes it
        // finite here; in the daemon the sender lives as long as the UI does.
        outgoing.close();
        freedesktop::emit_signals(&server.connection, receiver)
            .await
            .expect("the signal driver should not fail");

        let signal = smol::future::or(closures.next(), async {
            smol::Timer::after(PATIENCE).await;
            None
        })
        .await
        .expect("no NotificationClosed arrived");

        let args = signal.args().expect("signal arguments");
        assert_eq!(*args.id(), 9);
        assert_eq!(
            *args.reason(),
            CloseReason::Expired.as_u32(),
            "the reason travels as the spec's wire value"
        );
    });
}

#[test]
fn serve_claims_the_well_known_notification_name() {
    if !session_bus_available() {
        return;
    }
    let _guard = bus_test_lock();

    smol::block_on(async {
        let connection = Connection::session()
            .await
            .expect("session bus connection failed");
        let dbus = zbus::fdo::DBusProxy::new(&connection)
            .await
            .expect("bus proxy failed");
        let taken = dbus
            .name_has_owner(BUS_NAME.try_into().expect("a valid bus name"))
            .await
            .expect("NameHasOwner failed");
        if taken {
            eprintln!("skipping: {BUS_NAME} is already owned by another notification server");
            return;
        }

        let (sender, _incoming) = calloop::channel::channel();
        let served = freedesktop::serve(sender, Arc::new(AtomicU32::new(0)))
            .await
            .expect("serve should claim the notification name");

        assert!(
            dbus.name_has_owner(BUS_NAME.try_into().expect("a valid bus name"))
                .await
                .expect("NameHasOwner failed"),
            "{BUS_NAME} should be owned once serve returns"
        );
        drop(served);
    });
}
