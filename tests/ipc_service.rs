//! End-to-end cover for the crownos-ipc `crownotify` service.
//!
//! The service under test is the real one: a real Unix socket, the real
//! generated dispatcher and the real generated client on the other end of it.
//! Only the sink is a stand-in, which is the whole point of
//! [`NotificationSink`] — the transport can be proven correct without a
//! compositor, a GPU or a store.
//!
//! The service needs a calloop `LoopHandle`, so each test runs one on a worker
//! thread with a short dispatch timeout and a stop flag. Every poll loop here
//! has a deadline: a test that hangs forever tells you nothing.

use std::{
    sync::{
        Arc, Barrier, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use crownotify::proto;
use crownotify::{
    ipc::{NotificationSink, serve_in},
    model::{
        CloseReason, Content, DeviceKind, Notification, NotificationId, RelayDevice, Timeout,
        Urgency,
    },
};

/// Long enough to absorb a loaded CI machine, short enough that a genuine
/// hang fails the run rather than stalling it.
const TIMEOUT: Option<Duration> = Some(Duration::from_secs(5));
/// The id the mock sink files the first notification under.
const FIRST_ID: NotificationId = 501;

/// Everything the sink was asked to do, in order.
#[derive(Default)]
struct Recorded {
    posted: Vec<Notification>,
    closed: Vec<(NotificationId, CloseReason)>,
    center: Vec<Option<bool>>,
    do_not_disturb: Vec<bool>,
    focus: Vec<Option<String>>,
    dismissed_all: usize,
    invoked: Vec<(NotificationId, String)>,
}

/// A sink whose record the test thread can read while the service thread
/// writes it.
#[derive(Clone, Default)]
struct SharedSink(Arc<Mutex<Recorded>>);

impl SharedSink {
    fn read<R>(&self, f: impl FnOnce(&Recorded) -> R) -> R {
        f(&self.0.lock().expect("the sink lock is never poisoned"))
    }

    fn with<R>(&self, f: impl FnOnce(&mut Recorded) -> R) -> R {
        f(&mut self.0.lock().expect("the sink lock is never poisoned"))
    }
}

impl NotificationSink for SharedSink {
    fn post(&mut self, notification: Notification) -> NotificationId {
        self.with(|recorded| {
            recorded.posted.push(notification);
            FIRST_ID + recorded.posted.len() as NotificationId - 1
        })
    }

    fn close(&mut self, id: NotificationId, reason: CloseReason) {
        self.with(|recorded| recorded.closed.push((id, reason)));
    }

    fn set_center_open(&mut self, open: Option<bool>) {
        self.with(|recorded| recorded.center.push(open));
    }

    fn set_do_not_disturb(&mut self, enabled: bool) {
        self.with(|recorded| recorded.do_not_disturb.push(enabled));
    }

    fn set_focus_mode(&mut self, name: Option<&str>) {
        self.with(|recorded| recorded.focus.push(name.map(str::to_string)));
    }

    fn dismiss_all(&mut self) {
        self.with(|recorded| recorded.dismissed_all += 1);
    }

    fn invoke_action(&mut self, id: NotificationId, action: &str) {
        self.with(|recorded| recorded.invoked.push((id, action.to_string())));
    }
}

/// An event the test wants the service to broadcast.
///
/// It has to be emitted from the loop thread — `CalloopService` is not `Send`,
/// and emitting from inside `dispatch` is exactly what its re-entrancy rule
/// forbids — so the test queues one here and the worker emits it between
/// dispatches.
enum Emit {
    Closed(NotificationId, u8),
    Replied(NotificationId, String),
    DoNotDisturbChanged(bool),
}

/// A live service on a worker thread, torn down when it goes out of scope.
struct Service {
    directory: tempfile::TempDir,
    sink: SharedSink,
    queue: Arc<Mutex<Vec<Emit>>>,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}

impl Service {
    fn start() -> Self {
        let directory = tempfile::tempdir().expect("temp dir");
        let sink = SharedSink::default();
        let queue: Arc<Mutex<Vec<Emit>>> = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let bound = Arc::new(Barrier::new(2));

        let path = directory.path().to_path_buf();
        let worker = thread::spawn({
            let sink = sink.clone();
            let queue = Arc::clone(&queue);
            let stop = Arc::clone(&stop);
            let bound = Arc::clone(&bound);
            move || {
                let mut event_loop: calloop::EventLoop<'static, ()> =
                    calloop::EventLoop::try_new().expect("event loop");
                let handle = event_loop.handle();
                let service = serve_in(&handle, sink, &path).expect("the service binds");
                bound.wait();

                while !stop.load(Ordering::Relaxed) {
                    event_loop
                        .dispatch(Duration::from_millis(5), &mut ())
                        .expect("dispatch");
                    for event in queue.lock().expect("emit queue").drain(..) {
                        let emitted = match event {
                            Emit::Closed(id, reason) => {
                                service.emit(&proto::NotificationClosed { id, reason })
                            }
                            Emit::Replied(id, text) => {
                                service.emit(&proto::ReplySubmitted { id, text })
                            }
                            Emit::DoNotDisturbChanged(enabled) => {
                                service.emit(&proto::DoNotDisturbChanged { enabled })
                            }
                        };
                        emitted.expect("the service can always broadcast");
                    }
                }
            }
        });
        bound.wait();

        Self {
            directory,
            sink,
            queue,
            stop,
            worker: Some(worker),
        }
    }

    fn client(&self) -> proto::Client {
        proto::Client::connect_in(self.directory.path()).expect("connect")
    }

    fn emit(&self, event: Emit) {
        self.queue.lock().expect("emit queue").push(event);
    }
}

impl Drop for Service {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            worker.join().expect("the service thread exits cleanly");
        }
    }
}

/// Waits until `ready` holds, pumping the client so replies and events arrive.
///
/// Bounded on purpose: an assertion that never fires should fail the test, not
/// wedge the suite.
fn poll_until(
    client: &mut proto::Client,
    what: &str,
    mut ready: impl FnMut(&mut proto::Client) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if ready(client) {
            return;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        let _ = client.handle_readable();
        thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn every_notification_method_reaches_the_sink_and_returns_the_id_it_was_filed_under() {
    let service = Service::start();
    let mut client = service.client();

    let general = client
        .send_general_notification(
            "Mail".into(),
            "Ada".into(),
            "See you at 5".into(),
            4000,
            vec!["default".into()],
        )
        .expect("request");
    assert_eq!(
        client.wait(general, TIMEOUT).expect("reply"),
        FIRST_ID,
        "the reply must be the id the sink chose, not one the transport invented"
    );

    let call = client
        .send_call_notification(
            "call-42".into(),
            "Phone".into(),
            "/tmp/ada.png".into(),
            "Ada".into(),
            "+44 7700 900000".into(),
        )
        .expect("request");
    assert_eq!(client.wait(call, TIMEOUT).expect("reply"), FIRST_ID + 1);

    let music = client
        .send_music_notification("Blue Monday".into(), 33)
        .expect("request");
    assert_eq!(client.wait(music, TIMEOUT).expect("reply"), FIRST_ID + 2);

    let chat = client
        .send_chat_notification(
            "Signal".into(),
            "Ada".into(),
            "On my way".into(),
            "2026-08-09T10:30:00+00:00".into(),
        )
        .expect("request");
    assert_eq!(client.wait(chat, TIMEOUT).expect("reply"), FIRST_ID + 3);

    let app = client
        .send_app_notification("com.spotify.Client".into(), "music".into(), vec![1, 2, 3])
        .expect("request");
    assert_eq!(client.wait(app, TIMEOUT).expect("reply"), FIRST_ID + 4);

    let relayed = client
        .send_relayed_notification(
            "phone".into(),
            "Ada's phone".into(),
            "Signal".into(),
            "Ada".into(),
            "On my way".into(),
            0,
            vec!["inline-reply".into()],
        )
        .expect("request");
    assert_eq!(client.wait(relayed, TIMEOUT).expect("reply"), FIRST_ID + 5);

    service.sink.read(|recorded| {
        assert_eq!(recorded.posted.len(), 6, "six notifications were posted");

        let general = &recorded.posted[0];
        assert_eq!(general.app.name, "Mail");
        assert_eq!(general.summary, "Ada");
        assert_eq!(general.body, "See you at 5");
        assert_eq!(general.timeout, Timeout::Millis(4000));
        assert_eq!(
            general.default_action().map(|a| a.key.as_str()),
            Some("default")
        );

        let call = &recorded.posted[1];
        assert_eq!(
            call.urgency,
            Urgency::Critical,
            "a ringing phone is critical"
        );
        assert_eq!(call.timeout, Timeout::Never);
        let Content::Call(payload) = &call.content else {
            panic!("expected a call card, got {:?}", call.content);
        };
        assert_eq!(payload.call_id, "call-42");
        assert_eq!(payload.contact, "Ada");

        let Content::Music(payload) = &recorded.posted[2].content else {
            panic!("expected a music card");
        };
        assert_eq!(payload.title, "Blue Monday");
        assert_eq!(payload.percent, 33);

        let chat = &recorded.posted[3];
        assert!(
            chat.reply_action().is_some(),
            "a chat card can be replied to"
        );
        let Content::Chat(payload) = &chat.content else {
            panic!("expected a chat card");
        };
        assert_eq!(payload.sender, "Ada");

        let app = &recorded.posted[4];
        assert_eq!(app.app.desktop_entry.as_deref(), Some("com.spotify.Client"));
        assert!(
            matches!(app.content, Content::Music(_)),
            "the kind selects the card, got {:?}",
            app.content
        );

        let relayed = &recorded.posted[5];
        assert_eq!(
            relayed.relayed_from,
            Some(RelayDevice::new(DeviceKind::Phone, "Ada's phone")),
            "the relaying device is what badges the card"
        );
        assert!(
            recorded.posted[0].relayed_from.is_none(),
            "an ordinary notification must not be badged as relayed"
        );
    });
}

#[test]
fn a_refused_notification_reports_the_handler_error_to_the_caller() {
    let service = Service::start();
    let mut client = service.client();

    let pending = client
        .send_music_notification("Blue Monday".into(), 250)
        .expect("request");
    let error = client
        .wait(pending, TIMEOUT)
        .expect_err("250% of a track is not a position");
    assert!(
        format!("{error}").contains("0-100"),
        "the caller should be told what was wrong, got {error}"
    );

    service
        .sink
        .read(|recorded| assert!(recorded.posted.is_empty(), "nothing should have been filed"));
}

#[test]
fn every_fire_and_forget_request_lands_on_the_sink() {
    let service = Service::start();
    let mut client = service.client();

    client.close_notification(7).expect("close");
    client.open_notification_center().expect("open");
    client.close_notification_center().expect("close center");
    client.toggle_notification_center().expect("toggle");
    client.set_do_not_disturb(true).expect("dnd on");
    client.set_do_not_disturb(false).expect("dnd off");
    client.dismiss_all().expect("dismiss all");
    client.invoke_action(7, "archive".into()).expect("invoke");

    // Notifications carry no reply, so a request behind them is what proves
    // they have all been dispatched: one connection, one order.
    let barrier = client
        .send_general_notification("Mail".into(), String::new(), String::new(), 0, vec![])
        .expect("request");
    client.wait(barrier, TIMEOUT).expect("reply");

    service.sink.read(|recorded| {
        assert_eq!(recorded.closed, vec![(7, CloseReason::Closed)]);
        assert_eq!(
            recorded.center,
            vec![Some(true), Some(false), None],
            "None is the toggle"
        );
        assert_eq!(recorded.do_not_disturb, vec![true, false]);
        assert_eq!(recorded.dismissed_all, 1);
        assert_eq!(recorded.invoked, vec![(7, "archive".to_string())]);
    });
}

#[test]
fn a_subscribed_client_receives_the_events_the_service_broadcasts() {
    let service = Service::start();
    let mut client = service.client();

    client
        .subscribe_blocking::<proto::NotificationClosed>(TIMEOUT)
        .expect("subscribe to closures");
    client
        .subscribe_blocking::<proto::ReplySubmitted>(TIMEOUT)
        .expect("subscribe to replies");
    client
        .subscribe_blocking::<proto::DoNotDisturbChanged>(TIMEOUT)
        .expect("subscribe to do not disturb");

    service.emit(Emit::Closed(11, CloseReason::Dismissed as u8));
    service.emit(Emit::Replied(12, "on my way".into()));
    service.emit(Emit::DoNotDisturbChanged(true));

    let mut closed = None;
    let mut replied = None;
    let mut do_not_disturb = None;
    poll_until(
        &mut client,
        "NotificationClosed, ReplySubmitted and DoNotDisturbChanged",
        |client| {
            while let Some(event) = client.next_event() {
                match event {
                    proto::Event::NotificationClosed(event) => closed = Some(event),
                    proto::Event::ReplySubmitted(event) => replied = Some(event),
                    proto::Event::DoNotDisturbChanged(event) => do_not_disturb = Some(event),
                    _ => {}
                }
            }
            closed.is_some() && replied.is_some() && do_not_disturb.is_some()
        },
    );

    let closed = closed.expect("a closure event");
    assert_eq!(closed.id, 11);
    assert_eq!(
        closed.reason,
        CloseReason::Dismissed as u8,
        "the reason travels as the freedesktop wire value"
    );

    let replied = replied.expect("a reply event");
    assert_eq!(replied.id, 12);
    assert_eq!(replied.text, "on my way");

    assert!(do_not_disturb.expect("a do-not-disturb event").enabled);
}

#[test]
fn two_clients_are_served_at_once() {
    let service = Service::start();
    let mut first = service.client();
    let mut second = service.client();

    let one = first
        .send_general_notification("Mail".into(), "one".into(), String::new(), 0, vec![])
        .expect("request");
    let two = second
        .send_general_notification("Chat".into(), "two".into(), String::new(), 0, vec![])
        .expect("request");

    let one = first.wait(one, TIMEOUT).expect("reply");
    let two = second.wait(two, TIMEOUT).expect("reply");
    assert_ne!(one, two, "two notifications must not share an id");

    service
        .sink
        .read(|recorded| assert_eq!(recorded.posted.len(), 2));
}
