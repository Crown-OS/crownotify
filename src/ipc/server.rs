//! The crownos-ipc `crownotify` service.
//!
//! This is the transport the rest of the shell uses: the bar's notification
//! applet, crownconnect relaying a call from the phone, a keybinding helper
//! toggling Do Not Disturb. It runs **on crownshell's calloop loop** rather
//! than a thread of its own, so a request reaches the store without a channel
//! hop and without a lock.
//!
//! Everything here is generic over [`NotificationSink`], which is what lets
//! `tests/ipc_service.rs` drive the genuine service — the real socket, the
//! real generated client, the real dispatcher — against a recording sink with
//! no compositor and no GPU in the process.

use std::path::Path;

use anyhow::Context;
use calloop::LoopHandle;
use chrono::{DateTime, Local};
use crownos_ipc::{RemoteError, Server, ServiceBuilder, adapter::calloop::CalloopService};

use crate::proto::protocol as crownotify;
use crate::{
    ipc::{NotificationSink, parse_icon},
    model::{
        Action, Call, Chat, CloseReason, Content, DeviceKind, Music, Notification,
        NotificationBuilder, NotificationId, Progress, RelayDevice, Timeout, Urgency,
    },
};

/// The de-facto action key that makes a card grow a reply field.
const INLINE_REPLY_KEY: &str = "inline-reply";

/// Adapter implementing the generated `crownotify::Handler` over any sink.
pub struct NotifyHandler<S: NotificationSink> {
    sink: S,
}

impl<S: NotificationSink> NotifyHandler<S> {
    pub fn new(sink: S) -> Self {
        Self { sink }
    }

    /// The sink every protocol method ends up in.
    pub fn sink(&self) -> &S {
        &self.sink
    }

    pub fn sink_mut(&mut self) -> &mut S {
        &mut self.sink
    }
}

/// Binds `$XDG_RUNTIME_DIR/crownos/crownotify.sock` and drives it on `handle`.
///
/// `D` is crownshell's `App`, so the service can share the loop the compositor
/// connection is already driven by. The returned [`CalloopService`]
/// deregisters every source it owns when dropped, so it has to be kept alive
/// for as long as the daemon should answer requests — park it in the shared
/// state rather than letting it fall off the end of the setup closure.
pub fn serve<D: 'static, S: NotificationSink + 'static>(
    handle: &LoopHandle<'static, D>,
    sink: S,
) -> anyhow::Result<CalloopService> {
    let server = ServiceBuilder::new(crownotify::SERVICE)
        .build()
        .context("binding the crownotify IPC socket")?;
    insert(handle, server, sink)
}

/// [`serve`], but bound inside `directory` instead of the runtime directory.
///
/// Exists for the integration tests, which must not collide with a crownotify
/// the developer already has running, and for anything that wants to serve a
/// private instance (a nested session, a fixture).
pub fn serve_in<D: 'static, S: NotificationSink + 'static>(
    handle: &LoopHandle<'static, D>,
    sink: S,
    directory: &Path,
) -> anyhow::Result<CalloopService> {
    let server = ServiceBuilder::new(crownotify::SERVICE)
        .directory(directory)
        .build()
        .with_context(|| {
            format!(
                "binding the crownotify IPC socket in {}",
                directory.display()
            )
        })?;
    insert(handle, server, sink)
}

fn insert<D: 'static, S: NotificationSink + 'static>(
    handle: &LoopHandle<'static, D>,
    server: Server,
    sink: S,
) -> anyhow::Result<CalloopService> {
    let mut handler = NotifyHandler::new(sink);
    // The handler is emitted through the `&mut Server` calloop hands the
    // closure, never through the `CalloopService` this returns: the service
    // holds the same borrow `dispatch` is running under.
    CalloopService::insert(handle, server, move |_data, server, peer, message| {
        crownotify::dispatch(&mut handler, server, peer, message)
    })
    .context("registering the crownotify IPC service with the event loop")
}

impl<S: NotificationSink> crownotify::Handler for NotifyHandler<S> {
    fn open_notification_center(&mut self) {
        self.sink.set_center_open(Some(true));
    }

    fn close_notification_center(&mut self) {
        self.sink.set_center_open(Some(false));
    }

    fn toggle_notification_center(&mut self) {
        self.sink.set_center_open(None);
    }

    fn set_do_not_disturb(&mut self, enabled: bool) {
        self.sink.set_do_not_disturb(enabled);
    }

    fn set_focus_mode(&mut self, name: String) {
        self.sink
            .set_focus_mode(crate::ipc::parse_focus_mode(&name));
    }

    fn dismiss_all(&mut self) {
        self.sink.dismiss_all();
    }

    fn invoke_action(&mut self, id: u32, action: String) {
        self.sink.invoke_action(id, &action);
    }

    fn close_notification(&mut self, id: u32) {
        self.sink.close(id, CloseReason::Closed);
    }

    fn send_general_notification(
        &mut self,
        app_name: String,
        summary: String,
        body: String,
        expire_timeout: u32,
        actions: Vec<String>,
    ) -> Result<NotificationId, RemoteError> {
        Ok(self.post(
            Notification::builder(app_name)
                .summary(summary)
                .body(body)
                .timeout(timeout_from_millis(expire_timeout))
                .actions(actions_from_keys(&actions)),
        ))
    }

    fn send_call_notification(
        &mut self,
        call_id: String,
        app_name: String,
        contact_avatar: String,
        contact_name: String,
        phone_number: String,
    ) -> Result<NotificationId, RemoteError> {
        let avatar = parse_icon(&contact_avatar);
        let mut builder = Notification::builder(app_name)
            .summary(contact_name.clone())
            .body(phone_number.clone())
            // A ringing phone is the canonical critical notification: it must
            // survive Do Not Disturb and it must not vanish before it is
            // answered.
            .urgency(Urgency::Critical)
            .timeout(Timeout::Never)
            .content(Content::Call(Call {
                call_id,
                contact: contact_name,
                number: phone_number,
                avatar: avatar.clone(),
            }));
        if let Some(avatar) = avatar {
            builder = builder.image(avatar);
        }
        Ok(self.post(builder))
    }

    fn send_music_notification(
        &mut self,
        song_name: String,
        percentage: u8,
    ) -> Result<NotificationId, RemoteError> {
        if percentage > 100 {
            return Err(RemoteError::handler(
                "percentage is a position in the track, 0-100",
            ));
        }
        Ok(self.post(
            Notification::builder("Music")
                .summary(song_name.clone())
                .content(Content::Music(Music {
                    title: song_name,
                    artist: String::new(),
                    album_art: None,
                    playing: true,
                    percent: percentage,
                })),
        ))
    }

    fn send_chat_notification(
        &mut self,
        app_name: String,
        title: String,
        body: String,
        timestamp: String,
    ) -> Result<NotificationId, RemoteError> {
        Ok(self.post(
            Notification::builder(app_name)
                .summary(title.clone())
                .body(body)
                .action(Action {
                    key: INLINE_REPLY_KEY.to_string(),
                    label: "Reply".to_string(),
                    kind: crate::model::ActionKind::InlineReply {
                        placeholder: "Reply".to_string(),
                    },
                })
                .content(Content::Chat(Chat {
                    // The protocol carries no conversation handle, so the
                    // thread is addressed by the only stable thing it does
                    // carry. A sender that wants replies routed properly
                    // should be posting through its app profile instead.
                    conversation_id: title.clone(),
                    sender: title,
                    avatar: None,
                    sent_at: parse_timestamp(&timestamp),
                })),
        ))
    }

    fn send_app_notification(
        &mut self,
        app_id: String,
        kind: String,
        payload: Vec<u8>,
    ) -> Result<NotificationId, RemoteError> {
        let Some(content) = content_for_kind(&kind) else {
            return Err(RemoteError::handler(format!(
                "unknown app notification kind {kind:?}"
            )));
        };
        // `payload` is the profile's own encoding and there is no profile
        // registry decoding it yet. It is deliberately left uninterpreted
        // rather than guessed at: a wrong guess would put another app's bytes
        // on screen as text.
        log::debug!(
            "app notification from {app_id} of kind {kind} carries {} undecoded bytes",
            payload.len()
        );
        Ok(self.post(
            Notification::builder(app_id.clone())
                .desktop_entry(app_id)
                .content(content),
        ))
    }

    fn send_relayed_notification(
        &mut self,
        device_kind: String,
        device_name: String,
        app_name: String,
        summary: String,
        body: String,
        expire_timeout: u32,
        actions: Vec<String>,
    ) -> Result<NotificationId, RemoteError> {
        // An unreadable `device_kind` is not worth refusing the call over: the
        // card is the same card either way, and `DeviceKind::from_name` already
        // degrades to the device that does the relaying in practice.
        Ok(self.post(
            Notification::builder(app_name)
                .summary(summary)
                .body(body)
                .timeout(timeout_from_millis(expire_timeout))
                .actions(actions_from_keys(&actions))
                .relayed_from(RelayDevice::new(
                    DeviceKind::from_name(&device_kind),
                    device_name,
                )),
        ))
    }
}

impl<S: NotificationSink> NotifyHandler<S> {
    /// Files a notification and hands back the id the sink chose, which is the
    /// id the caller is waiting on the other end of the socket for.
    fn post(&mut self, builder: NotificationBuilder) -> NotificationId {
        self.sink.post(builder.build())
    }
}

/// Interprets the IPC `expire_timeout`, which is a `u32` and so has no way to
/// spell the freedesktop `-1`.
///
/// Zero therefore means "I did not set one" rather than the spec's "never
/// expire": a caller that genuinely wants a permanent card has the app-profile
/// route, whereas every caller that simply left the field alone would
/// otherwise pin a toast to the screen forever.
fn timeout_from_millis(millis: u32) -> Timeout {
    match millis {
        0 => Timeout::Default,
        millis => Timeout::Millis(millis),
    }
}

/// Decodes the IPC `actions` array, which is a flat list of action *keys*
/// rather than the freedesktop call's key/label pairs.
///
/// The keys are reused as labels — the caller supplied nothing better — but
/// they are still routed through [`Action::from_spec_pairs`] so that `default`
/// and `inline-reply` keep meaning the same thing on both transports.
fn actions_from_keys(keys: &[String]) -> Vec<Action> {
    let pairs: Vec<String> = keys
        .iter()
        .flat_map(|key| {
            let label = if key == INLINE_REPLY_KEY {
                "Reply"
            } else {
                key
            };
            [key.clone(), label.to_string()]
        })
        .collect();
    Action::from_spec_pairs(&pairs)
}

/// The card an app notification's `kind` asks for.
///
/// The payloads start empty because the profile that would fill them in has
/// not decoded `payload` yet; the kind still selects the presentation, which
/// is the part other surfaces already depend on.
fn content_for_kind(kind: &str) -> Option<Content> {
    match kind {
        "general" => Some(Content::General),
        "call" => Some(Content::Call(Call {
            call_id: String::new(),
            contact: String::new(),
            number: String::new(),
            avatar: None,
        })),
        "chat" => Some(Content::Chat(Chat {
            conversation_id: String::new(),
            sender: String::new(),
            avatar: None,
            sent_at: Local::now(),
        })),
        "music" => Some(Content::Music(Music {
            title: String::new(),
            artist: String::new(),
            album_art: None,
            playing: false,
            percent: 0,
        })),
        "progress" => Some(Content::Progress(Progress {
            label: String::new(),
            percent: 0,
        })),
        _ => None,
    }
}

/// Reads the chat protocol's `timestamp`, which is when the message was *sent*
/// and so is not the same thing as when we were told about it.
///
/// An unparseable value falls back to now rather than failing the call: a chat
/// card with a slightly wrong time is worth far more than no chat card.
fn parse_timestamp(raw: &str) -> DateTime<Local> {
    if raw.is_empty() {
        return Local::now();
    }
    match DateTime::parse_from_rfc3339(raw) {
        Ok(parsed) => parsed.with_timezone(&Local),
        Err(error) => {
            log::debug!("chat timestamp {raw:?} is not RFC 3339 ({error}); using now");
            Local::now()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ActionKind;
    use crate::proto::protocol::Handler;

    /// Records what a transport did, which is all the sink contract promises.
    #[derive(Default)]
    struct Recorder {
        posted: Vec<Notification>,
        closed: Vec<(NotificationId, CloseReason)>,
        center: Vec<Option<bool>>,
        do_not_disturb: Vec<bool>,
        focus: Vec<Option<String>>,
        dismissed_all: usize,
        invoked: Vec<(NotificationId, String)>,
    }

    impl NotificationSink for Recorder {
        fn post(&mut self, notification: Notification) -> NotificationId {
            self.posted.push(notification);
            100 + self.posted.len() as NotificationId
        }
        fn close(&mut self, id: NotificationId, reason: CloseReason) {
            self.closed.push((id, reason));
        }
        fn set_center_open(&mut self, open: Option<bool>) {
            self.center.push(open);
        }
        fn set_do_not_disturb(&mut self, enabled: bool) {
            self.do_not_disturb.push(enabled);
        }
        fn set_focus_mode(&mut self, name: Option<&str>) {
            self.focus.push(name.map(str::to_string));
        }
        fn dismiss_all(&mut self) {
            self.dismissed_all += 1;
        }
        fn invoke_action(&mut self, id: NotificationId, action: &str) {
            self.invoked.push((id, action.to_string()));
        }
    }

    fn handler() -> NotifyHandler<Recorder> {
        NotifyHandler::new(Recorder::default())
    }

    #[test]
    fn a_general_notification_reaches_the_sink_and_returns_its_id() {
        let mut handler = handler();
        let id = handler
            .send_general_notification(
                "Mail".into(),
                "Ada".into(),
                "See you at 5".into(),
                4000,
                vec!["default".into(), "archive".into()],
            )
            .expect("the sink cannot refuse a general notification");

        assert_eq!(id, 101, "the id must be the one the sink filed it under");
        let posted = &handler.sink().posted[0];
        assert_eq!(posted.app.name, "Mail");
        assert_eq!(posted.summary, "Ada");
        assert_eq!(posted.body, "See you at 5");
        assert_eq!(posted.timeout, Timeout::Millis(4000));
        assert_eq!(
            posted.default_action().map(|a| a.key.as_str()),
            Some("default")
        );
        assert_eq!(posted.actions[1].kind, ActionKind::Button);
    }

    #[test]
    fn a_zero_expire_timeout_means_the_configured_default_not_never() {
        let mut handler = handler();
        handler
            .send_general_notification("Mail".into(), String::new(), String::new(), 0, vec![])
            .expect("posted");

        assert_eq!(
            handler.sink().posted[0].timeout,
            Timeout::Default,
            "a u32 has no -1, so 0 has to carry 'unset'"
        );
    }

    #[test]
    fn an_inline_reply_key_gets_a_readable_placeholder() {
        let mut handler = handler();
        handler
            .send_general_notification(
                "Chat".into(),
                String::new(),
                String::new(),
                0,
                vec!["inline-reply".into()],
            )
            .expect("posted");

        assert_eq!(
            handler.sink().posted[0]
                .reply_action()
                .map(|a| a.kind.clone()),
            Some(ActionKind::InlineReply {
                placeholder: "Reply".to_string()
            })
        );
    }

    #[test]
    fn a_call_notification_is_critical_and_never_expires() {
        let mut handler = handler();
        handler
            .send_call_notification(
                "call-42".into(),
                "Phone".into(),
                "/tmp/ada.png".into(),
                "Ada".into(),
                "+44 7700 900000".into(),
            )
            .expect("posted");

        let posted = &handler.sink().posted[0];
        assert_eq!(posted.urgency, Urgency::Critical);
        assert_eq!(posted.timeout, Timeout::Never);
        let Content::Call(call) = &posted.content else {
            panic!("expected a call card, got {:?}", posted.content);
        };
        assert_eq!(call.call_id, "call-42");
        assert_eq!(call.contact, "Ada");
        assert_eq!(call.number, "+44 7700 900000");
        assert!(call.avatar.is_some(), "an absolute path is an avatar");
    }

    #[test]
    fn a_music_position_outside_the_track_is_refused() {
        let mut handler = handler();
        let error = handler
            .send_music_notification("Blue Monday".into(), 101)
            .expect_err("101% of a track is not a position");
        assert!(
            format!("{error:?}").contains("0-100"),
            "the error should say what is wrong, got {error:?}"
        );
        assert!(handler.sink().posted.is_empty());
    }

    #[test]
    fn a_music_notification_carries_the_track_and_its_position() {
        let mut handler = handler();
        handler
            .send_music_notification("Blue Monday".into(), 33)
            .expect("posted");

        let Content::Music(music) = &handler.sink().posted[0].content else {
            panic!("expected a music card");
        };
        assert_eq!(music.title, "Blue Monday");
        assert_eq!(music.percent, 33);
        assert!(music.playing);
    }

    #[test]
    fn a_chat_notification_offers_an_inline_reply_and_keeps_the_sent_time() {
        let mut handler = handler();
        handler
            .send_chat_notification(
                "Signal".into(),
                "Ada".into(),
                "On my way".into(),
                "2026-08-09T10:30:00+00:00".into(),
            )
            .expect("posted");

        let posted = &handler.sink().posted[0];
        assert!(
            posted.reply_action().is_some(),
            "a chat card can be replied to"
        );
        let Content::Chat(chat) = &posted.content else {
            panic!("expected a chat card");
        };
        assert_eq!(chat.sender, "Ada");
        assert_eq!(chat.conversation_id, "Ada");
        assert_eq!(
            chat.sent_at.to_utc().to_rfc3339(),
            "2026-08-09T10:30:00+00:00"
        );
    }

    #[test]
    fn a_malformed_chat_timestamp_does_not_lose_the_message() {
        let mut handler = handler();
        handler
            .send_chat_notification(
                "Signal".into(),
                "Ada".into(),
                "hi".into(),
                "yesterday".into(),
            )
            .expect("posted");
        assert_eq!(handler.sink().posted.len(), 1);
    }

    #[test]
    fn an_app_notifications_kind_selects_the_card() {
        for (kind, expected) in [
            ("general", "General"),
            ("call", "Call"),
            ("chat", "Chat"),
            ("music", "Music"),
            ("progress", "Progress"),
        ] {
            let mut handler = handler();
            handler
                .send_app_notification("com.example.widget".into(), kind.into(), vec![1, 2, 3])
                .expect("posted");
            let content = format!("{:?}", handler.sink().posted[0].content);
            assert!(
                content.starts_with(expected),
                "kind {kind} should select {expected}, got {content}"
            );
        }
    }

    #[test]
    fn an_unknown_app_notification_kind_is_refused_rather_than_guessed_at() {
        let mut handler = handler();
        let error = handler
            .send_app_notification("com.example.widget".into(), "hologram".into(), vec![])
            .expect_err("an unknown kind has no card to draw");
        assert!(
            format!("{error:?}").contains("hologram"),
            "the error should name the kind, got {error:?}"
        );
        assert!(handler.sink().posted.is_empty());
    }

    #[test]
    fn a_relayed_notification_carries_the_device_that_mirrored_it() {
        let mut handler = handler();
        handler
            .send_relayed_notification(
                "Watch".into(),
                "Ada's watch".into(),
                "Signal".into(),
                "Ada".into(),
                "On my way".into(),
                4000,
                vec!["inline-reply".into()],
            )
            .expect("posted");

        let posted = &handler.sink().posted[0];
        assert_eq!(
            posted.relayed_from,
            Some(RelayDevice::new(DeviceKind::Watch, "Ada's watch"))
        );
        assert_eq!(posted.app.name, "Signal");
        assert_eq!(posted.timeout, Timeout::Millis(4000));
        assert!(
            posted.reply_action().is_some(),
            "a relayed card takes the same actions as a general one"
        );
    }

    #[test]
    fn an_unreadable_device_kind_still_badges_the_relayed_card() {
        let mut handler = handler();
        handler
            .send_relayed_notification(
                "hologram".into(),
                String::new(),
                "Signal".into(),
                String::new(),
                String::new(),
                0,
                vec![],
            )
            .expect("an unknown device is not a reason to lose the notification");

        let device = handler.sink().posted[0]
            .relayed_from
            .as_ref()
            .expect("the card is still relayed");
        assert_eq!(device.kind, DeviceKind::Phone);
        assert!(device.name.is_empty());
    }

    #[test]
    fn the_center_methods_set_and_toggle_the_same_state() {
        let mut handler = handler();
        handler.open_notification_center();
        handler.close_notification_center();
        handler.toggle_notification_center();

        assert_eq!(
            handler.sink().center,
            vec![Some(true), Some(false), None],
            "None is the toggle"
        );
    }

    #[test]
    fn the_remaining_notifications_land_on_the_sink_verbatim() {
        let mut handler = handler();
        handler.close_notification(7);
        handler.set_do_not_disturb(true);
        handler.dismiss_all();
        handler.invoke_action(7, "archive".into());

        let sink = handler.sink();
        assert_eq!(sink.closed, vec![(7, CloseReason::Closed)]);
        assert_eq!(sink.do_not_disturb, vec![true]);
        assert_eq!(sink.dismissed_all, 1);
        assert_eq!(sink.invoked, vec![(7, "archive".to_string())]);
    }
}
