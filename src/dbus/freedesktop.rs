//! `org.freedesktop.Notifications` — the interface every other application on
//! the system already speaks.
//!
//! crownos-ipc is how the *shell* talks to the notification daemon; this is how
//! everything else does, and it is the one transport crownotify cannot choose
//! not to implement. The implementation follows spec 1.2, plus the de-facto
//! `inline-reply` capability and its `NotificationReplied` signal that KDE and
//! Fedora's servers established.
//!
//! # Threading and ids
//!
//! The interface lives on its own thread with an async executor, and it reaches
//! the UI thread only through a `calloop::channel`. That leaves one problem:
//! `Notify` has to return the notification's id to its caller *now*, long
//! before the UI thread wakes up and files it. So this side allocates the id
//! itself, from the `Arc<AtomicU32>` the [`Store`](crate::store::Store) hands
//! out through [`Store::id_counter`](crate::store::Store::id_counter), and
//! sends the notification with the id already set — which
//! [`Store::post`](crate::store::Store::post) honours. One counter, two
//! allocators, no collisions and no round trip.
//!
//! # Hostile input
//!
//! Every argument here comes from another process, and a notification daemon
//! that a malformed hint can crash takes every notification on the system down
//! with it. Nothing in this module returns an error for a hint it cannot make
//! sense of: it logs, ignores that hint, and delivers the notification.

// The freedesktop `Notify` signature is fixed by the specification; it has
// eight arguments whether or not clippy approves.
#![allow(clippy::too_many_arguments)]

use std::{
    collections::HashMap,
    sync::{Arc, Mutex, atomic::AtomicU32},
};

use anyhow::Context;
use chrono::Local;
use zbus::{
    Connection, connection, fdo, interface,
    object_server::SignalEmitter,
    zvariant::{Structure, Value},
};

use crate::{
    error::Error,
    ipc::{Incoming, IncomingSender, Outgoing, OutgoingReceiver, parse_icon},
    model::{
        Action, Call, Chat, CloseReason, Content, Icon, ImageData, Music, Notification,
        NotificationId, Progress, RelayDevice, Timeout, Urgency,
    },
    store::allocate_id,
};

/// The well-known name a notification server owns. Exactly one process on a
/// session may hold it, which is why the tests serialise around it.
pub const BUS_NAME: &str = "org.freedesktop.Notifications";
/// Where the interface is served. The path is part of the spec, not a choice.
pub const OBJECT_PATH: &str = "/org/freedesktop/Notifications";

/// The level a CrownOS-aware sender asks for directly, overriding whatever its
/// `urgency` would otherwise imply.
const LEVEL_HINT: &str = "x-crownos.interruption-level";
/// The version of the freedesktop specification this implements.
const SPEC_VERSION: &str = "1.2";

/// The `image-data` hint under each of the three names it has been given.
///
/// `image_data` was the 1.1 spelling and `icon_data` the 1.0 one; both are
/// deprecated and both are still sent by applications shipping today, so all
/// three are accepted in the order the spec prescribes.
const IMAGE_DATA_HINTS: [&str; 3] = ["image-data", "image_data", "icon_data"];
/// The `image-path` hint, and its deprecated 1.1 spelling.
const IMAGE_PATH_HINTS: [&str; 2] = ["image-path", "image_path"];
/// The hints a relaying process marks a mirrored notification with. Neither is
/// required: the badge is drawn from whichever of them arrived.
const DEVICE_KIND_HINT: &str = "x-crownos.device-kind";
const DEVICE_NAME_HINT: &str = "x-crownos.device-name";
/// Categories that select one of CrownOS's richer cards.
const CALL_CATEGORY: &str = "x-crownos.call";
const CHAT_CATEGORY: &str = "x-crownos.chat";
const MUSIC_CATEGORY: &str = "x-crownos.music";

/// The freedesktop notification server.
///
/// Holds nothing but a way to reach the UI thread and the shared id counter;
/// all the state a notification has lives in the store on the other side of
/// the channel.
pub struct Notifications {
    /// `calloop::channel::Sender` is `Send` but not `Sync`, and zbus may call
    /// an interface from any executor thread, so the channel needs a lock.
    /// It is held for the duration of a `send` into an unbounded queue, which
    /// never blocks.
    incoming: Mutex<IncomingSender>,
    ids: Arc<AtomicU32>,
}

impl Notifications {
    pub fn new(incoming: IncomingSender, ids: Arc<AtomicU32>) -> Self {
        Self {
            incoming: Mutex::new(incoming),
            ids,
        }
    }

    /// Hands `incoming` to the UI thread, waking its event loop.
    ///
    /// A poisoned lock is recovered from rather than propagated: it only means
    /// some other caller panicked while holding it, the channel behind it is
    /// unaffected, and refusing every subsequent notification would be a far
    /// worse outcome than carrying on.
    fn deliver(&self, incoming: Incoming) -> fdo::Result<()> {
        let sender = self
            .incoming
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        sender.send(incoming).map_err(|_| {
            fdo::Error::Failed("the notification daemon's UI thread has gone away".to_string())
        })
    }
}

#[interface(name = "org.freedesktop.Notifications")]
impl Notifications {
    /// What this server can do, as the spec's fixed vocabulary.
    ///
    /// `inline-reply` is not in 1.2; it is the key KDE and Fedora's servers
    /// agreed on, and advertising it is how a chat client learns it may send
    /// an `inline-reply` action instead of a plain button.
    fn get_capabilities(&self) -> Vec<String> {
        [
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
        ]
        .iter()
        .map(|capability| (*capability).to_string())
        .collect()
    }

    fn get_server_information(&self) -> (String, String, String, String) {
        (
            env!("CARGO_PKG_NAME").to_string(),
            env!("CARGO_PKG_AUTHORS").to_string(),
            env!("CARGO_PKG_VERSION").to_string(),
            SPEC_VERSION.to_string(),
        )
    }

    /// Posts a notification and returns the id it was filed under.
    ///
    /// `replaces_id` is honoured verbatim: the store replaces in place, so an
    /// application editing its own notification does not make it jump to the
    /// front of the stack and shove the others down.
    fn notify(
        &self,
        app_name: String,
        replaces_id: u32,
        app_icon: String,
        summary: String,
        body: String,
        actions: Vec<String>,
        mut hints: HashMap<String, Value<'_>>,
        expire_timeout: i32,
    ) -> fdo::Result<u32> {
        let id = if replaces_id != 0 {
            replaces_id
        } else {
            allocate_id(&self.ids)
        };

        // Taken first because it is the one hint that has to be moved out of
        // the map rather than borrowed: the pixel buffer is the only part of a
        // notification large enough for a copy to be worth avoiding.
        let image = take_image_data(&mut hints)
            .map(|data| Icon::Data(Arc::new(data)))
            .or_else(|| {
                hint(&hints, &IMAGE_PATH_HINTS)
                    .and_then(hint_text)
                    .and_then(parse_icon)
            });

        let urgency = hint(&hints, &["urgency"])
            .and_then(hint_integer)
            .map(|value| Urgency::from_hint(value.clamp(0, i64::from(u8::MAX)) as u8))
            .unwrap_or_default();
        // A CrownOS-aware sender names its level directly; everything else is
        // judged at whatever its urgency implies.
        let level = hint(&hints, &[LEVEL_HINT])
            .and_then(hint_text)
            .and_then(|raw| match raw.parse() {
                Ok(level) => Some(level),
                Err(error) => {
                    log::warn!("ignoring {LEVEL_HINT} on notification {id}: {error}");
                    None
                }
            });
        let percent = hint(&hints, &["value"])
            .and_then(hint_integer)
            .map(|value| value.clamp(0, 100) as u8);
        let content = content_for(&hints, id, &summary, &body, image.as_ref(), percent);

        if hint(&hints, &["suppress-sound"])
            .and_then(hint_bool)
            .unwrap_or(false)
        {
            log::trace!("notification {id} asked for silence; crownotify plays no sound anyway");
        }

        let mut builder = Notification::builder(app_name)
            .id(id)
            .summary(summary)
            .body(body)
            .urgency(urgency)
            .timeout(Timeout::from_spec(expire_timeout))
            .actions(Action::from_spec_pairs(&actions))
            .content(content)
            .transient(
                hint(&hints, &["transient"])
                    .and_then(hint_bool)
                    .unwrap_or(false),
            )
            .resident(
                hint(&hints, &["resident"])
                    .and_then(hint_bool)
                    .unwrap_or(false),
            );
        if let Some(icon) = parse_icon(&app_icon) {
            builder = builder.icon(icon);
        }
        if let Some(image) = image {
            builder = builder.image(image);
        }
        if let Some(level) = level {
            builder = builder.level(level);
        }
        if let Some(entry) = hint(&hints, &["desktop-entry", "desktop_entry"]).and_then(hint_text) {
            builder = builder.desktop_entry(entry);
        }
        if let Some(device) = relay_device(&hints) {
            builder = builder.relayed_from(device);
        }

        self.deliver(Incoming::Post(Box::new(builder.build())))?;
        Ok(id)
    }

    /// Closes a notification on the sender's behalf.
    ///
    /// The spec's `CloseNotification` is reason 3, "closed by a call to
    /// CloseNotification", which is exactly [`CloseReason::Closed`].
    fn close_notification(&self, id: u32) -> fdo::Result<()> {
        self.deliver(Incoming::Close(id, CloseReason::Closed))
    }

    #[zbus(signal)]
    async fn notification_closed(
        emitter: &SignalEmitter<'_>,
        id: u32,
        reason: u32,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn action_invoked(
        emitter: &SignalEmitter<'_>,
        id: u32,
        action_key: String,
    ) -> zbus::Result<()>;

    /// The de-facto inline-reply signal: the user typed into the card's reply
    /// field and submitted it.
    #[zbus(signal)]
    async fn notification_replied(
        emitter: &SignalEmitter<'_>,
        id: u32,
        text: String,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn activation_token(
        emitter: &SignalEmitter<'_>,
        id: u32,
        activation_token: String,
    ) -> zbus::Result<()>;
}

/// Connects to the session bus, serves the interface and claims [`BUS_NAME`].
///
/// The returned [`Connection`] owns the name and the object; dropping it
/// releases both, so the caller has to keep it for as long as the daemon
/// should answer. Claiming the name fails when another notification server is
/// already running, which is the correct outcome — two servers fighting over
/// the same notifications is worse than one refusing to start.
pub async fn serve(incoming: IncomingSender, ids: Arc<AtomicU32>) -> anyhow::Result<Connection> {
    connection::Builder::session()
        .context("connecting to the session bus")?
        .name(BUS_NAME)
        .with_context(|| format!("requesting {BUS_NAME}"))?
        .serve_at(OBJECT_PATH, Notifications::new(incoming, ids))
        .with_context(|| format!("serving the notification interface at {OBJECT_PATH}"))?
        .build()
        .await
        .with_context(|| format!("claiming {BUS_NAME} on the session bus"))
}

/// Turns the UI thread's [`Outgoing`] stream into D-Bus signals until the
/// channel closes.
///
/// This is the half the previous implementation was missing entirely: the
/// signals were declared and never fired, so no application ever learned that
/// its notification had been dismissed or its action clicked. Spawn it on the
/// same thread as [`serve`]'s connection.
///
/// A signal that fails to go out is logged and skipped rather than ending the
/// loop: one unroutable signal must not stop every later one.
pub async fn emit_signals(
    connection: &Connection,
    outgoing: OutgoingReceiver,
) -> anyhow::Result<()> {
    let emitter = SignalEmitter::new(connection, OBJECT_PATH)
        .with_context(|| format!("building a signal emitter for {OBJECT_PATH}"))?;

    while let Ok(event) = outgoing.recv().await {
        let emitted = match event {
            Outgoing::Closed(id, reason) => {
                Notifications::notification_closed(&emitter, id, reason.as_u32()).await
            }
            Outgoing::ActionInvoked(id, key) => {
                Notifications::action_invoked(&emitter, id, key).await
            }
            Outgoing::Replied(id, text) => {
                Notifications::notification_replied(&emitter, id, text).await
            }
            // The notification center and focus modes are CrownOS concepts the
            // freedesktop interface has no signal for; crownos-ipc subscribers
            // are told about them instead.
            Outgoing::CenterVisibility(_)
            | Outgoing::DoNotDisturbChanged(_)
            | Outgoing::FocusModeChanged(_) => Ok(()),
        };
        if let Err(error) = emitted {
            log::warn!("emitting a notification signal failed: {error}");
        }
    }
    Ok(())
}

/// Emits `ActivationToken` for `id`.
///
/// Separate from [`emit_signals`] because a token is not produced by the
/// notification's own lifecycle: it comes from the compositor when the daemon
/// is about to launch or focus the sending application on the user's behalf,
/// and it must precede the `ActionInvoked` that causes the launch, or the app
/// loses its focus request to the shell's focus-stealing prevention.
pub async fn emit_activation_token(
    connection: &Connection,
    id: NotificationId,
    token: impl Into<String>,
) -> zbus::Result<()> {
    let emitter = SignalEmitter::new(connection, OBJECT_PATH)?;
    Notifications::activation_token(&emitter, id, token.into()).await
}

/// The first of `keys` the sender actually supplied.
///
/// Hints have accumulated alternative spellings across three revisions of the
/// spec, and applications in the wild send all of them.
fn hint<'h, 'v>(hints: &'h HashMap<String, Value<'v>>, keys: &[&str]) -> Option<&'h Value<'v>> {
    keys.iter().find_map(|key| hints.get(*key))
}

/// Reads a hint that should be a number, whatever width the sender chose.
///
/// The spec says `urgency` is a byte and `value` an int; senders disagree, and
/// a progress bar that silently does nothing because the sender used a `u32`
/// is a worse outcome than a lenient decoder.
fn hint_integer(value: &Value<'_>) -> Option<i64> {
    match value {
        Value::U8(value) => Some(i64::from(*value)),
        Value::I16(value) => Some(i64::from(*value)),
        Value::U16(value) => Some(i64::from(*value)),
        Value::I32(value) => Some(i64::from(*value)),
        Value::U32(value) => Some(i64::from(*value)),
        Value::I64(value) => Some(*value),
        Value::U64(value) => i64::try_from(*value).ok(),
        Value::F64(value) => Some(*value as i64),
        _ => None,
    }
}

/// Reads a hint that should be a boolean, tolerating the senders that use 0/1.
fn hint_bool(value: &Value<'_>) -> Option<bool> {
    match value {
        Value::Bool(value) => Some(*value),
        other => hint_integer(other).map(|value| value != 0),
    }
}

fn hint_text<'a>(value: &'a Value<'_>) -> Option<&'a str> {
    match value {
        Value::Str(text) => Some(text.as_str()),
        _ => None,
    }
}

/// Decodes the raw-pixel image hint, under whichever of its names it arrived.
///
/// Returns `None` — not an error — when the hint is absent or malformed. A
/// notification with a broken icon is still a notification the user needs to
/// read, and rejecting the call would let any application on the session
/// silence itself by accident.
fn take_image_data(hints: &mut HashMap<String, Value<'_>>) -> Option<ImageData> {
    for key in IMAGE_DATA_HINTS {
        let Some(value) = hints.remove(key) else {
            continue;
        };
        match decode_image_hint(value) {
            Ok(image) => return Some(image),
            Err(error) => log::warn!("ignoring malformed {key} hint: {error}"),
        }
    }
    None
}

/// Unpacks the `(iiibiiay)` structure into [`ImageData::from_hint`].
fn decode_image_hint(value: Value<'_>) -> Result<ImageData, Error> {
    let fields = match value {
        Value::Structure(fields) => structure_fields(fields),
        other => {
            return Err(Error::IconDecode(format!(
                "image hint has signature {}, expected (iiibiiay)",
                other.value_signature()
            )));
        }
    };
    if fields.len() != 7 {
        return Err(Error::IconDecode(format!(
            "image hint has {} fields, expected 7",
            fields.len()
        )));
    }

    let mut fields = fields.into_iter();
    let width = next_integer(&mut fields, "width")?;
    let height = next_integer(&mut fields, "height")?;
    let rowstride = next_integer(&mut fields, "rowstride")?;
    let has_alpha = fields
        .next()
        .as_ref()
        .and_then(hint_bool)
        .ok_or_else(|| Error::IconDecode("has_alpha is not a boolean".to_string()))?;
    let bits_per_sample = next_integer(&mut fields, "bits_per_sample")?;
    let channels = next_integer(&mut fields, "channels")?;
    let data = match fields.next() {
        Some(Value::Array(array)) => Vec::<u8>::try_from(array)
            .map_err(|error| Error::IconDecode(format!("pixel array is not bytes: {error}")))?,
        _ => return Err(Error::IconDecode("pixel data is not an array".to_string())),
    };

    ImageData::from_hint(
        width,
        height,
        rowstride,
        has_alpha,
        bits_per_sample,
        channels,
        data,
    )
}

fn structure_fields(structure: Structure<'_>) -> Vec<Value<'_>> {
    structure.into_fields()
}

fn next_integer<'v>(
    fields: &mut impl Iterator<Item = Value<'v>>,
    name: &str,
) -> Result<i32, Error> {
    let value = fields
        .next()
        .as_ref()
        .and_then(hint_integer)
        .ok_or_else(|| Error::IconDecode(format!("{name} is not a number")))?;
    i32::try_from(value).map_err(|_| Error::IconDecode(format!("{name} is out of range: {value}")))
}

/// The device a relaying process says the notification came from.
///
/// `None` for the overwhelming majority of notifications, which were raised on
/// this machine and say nothing about a device at all.
fn relay_device(hints: &HashMap<String, Value<'_>>) -> Option<RelayDevice> {
    RelayDevice::from_parts(
        hint(hints, &[DEVICE_KIND_HINT]).and_then(hint_text),
        hint(hints, &[DEVICE_NAME_HINT]).and_then(hint_text),
    )
}

/// Chooses the card from the `category` hint, falling back to progress and
/// then to a plain notification.
///
/// The `x-crownos.*` categories are how an application opts in to a richer
/// card over the freedesktop interface, which has no other room to say so.
/// Neither carries the identifiers the card needs to act — the call to hang up,
/// the conversation to reply to — so those come from optional hints and fall
/// back to the notification's own id, which is at least stable for its
/// lifetime.
fn content_for(
    hints: &HashMap<String, Value<'_>>,
    id: NotificationId,
    summary: &str,
    body: &str,
    image: Option<&Icon>,
    percent: Option<u8>,
) -> Content {
    let category = hint(hints, &["category"]).and_then(hint_text).unwrap_or("");

    if category.starts_with(CALL_CATEGORY) {
        return Content::Call(Call {
            call_id: hint(hints, &["x-crownos.call-id"])
                .and_then(hint_text)
                .map_or_else(|| id.to_string(), str::to_string),
            contact: summary.to_string(),
            number: body.to_string(),
            avatar: image.cloned(),
        });
    }
    if category.starts_with(CHAT_CATEGORY) {
        return Content::Chat(Chat {
            conversation_id: hint(hints, &["x-crownos.conversation-id"])
                .and_then(hint_text)
                .map_or_else(|| id.to_string(), str::to_string),
            sender: summary.to_string(),
            avatar: image.cloned(),
            sent_at: Local::now(),
        });
    }
    if category.starts_with(MUSIC_CATEGORY) {
        return Content::Music(Music {
            title: summary.to_string(),
            artist: body.to_string(),
            album_art: image.cloned(),
            playing: hint(hints, &["x-crownos.playing"])
                .and_then(hint_bool)
                .unwrap_or(true),
            percent: percent.unwrap_or(0),
        });
    }
    match percent {
        Some(percent) => Content::Progress(Progress {
            label: summary.to_string(),
            percent,
        }),
        None => Content::General,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::DeviceKind;
    use zbus::zvariant::Array;

    fn hints<'v>(pairs: Vec<(&str, Value<'v>)>) -> HashMap<String, Value<'v>> {
        pairs
            .into_iter()
            .map(|(key, value)| (key.to_string(), value))
            .collect()
    }

    /// A 1x1 opaque red pixel as the `(iiibiiay)` hint carries it.
    fn image_hint(data: Vec<u8>) -> Value<'static> {
        Value::from((1i32, 1i32, 4i32, true, 8i32, 4i32, Array::from(data)))
    }

    #[test]
    fn integer_hints_are_read_whatever_width_the_sender_used() {
        assert_eq!(hint_integer(&Value::U8(2)), Some(2));
        assert_eq!(hint_integer(&Value::I32(2)), Some(2));
        assert_eq!(hint_integer(&Value::U32(2)), Some(2));
        assert_eq!(hint_integer(&Value::F64(2.7)), Some(2));
        assert_eq!(
            hint_integer(&Value::from("2")),
            None,
            "a string is not a number"
        );
    }

    #[test]
    fn boolean_hints_tolerate_the_senders_that_use_zero_and_one() {
        assert_eq!(hint_bool(&Value::Bool(true)), Some(true));
        assert_eq!(hint_bool(&Value::I32(0)), Some(false));
        assert_eq!(hint_bool(&Value::I32(1)), Some(true));
        assert_eq!(hint_bool(&Value::from("yes")), None);
    }

    #[test]
    fn a_hint_is_found_under_any_of_its_historical_spellings() {
        let map = hints(vec![("image_path", Value::from("/tmp/a.png"))]);
        assert_eq!(
            hint(&map, &IMAGE_PATH_HINTS).and_then(hint_text),
            Some("/tmp/a.png"),
            "the 1.1 spelling is still sent today"
        );
    }

    #[test]
    fn a_well_formed_image_hint_decodes_to_pixels() {
        let image = decode_image_hint(image_hint(vec![255, 0, 0, 255])).expect("decodes");
        assert_eq!(image.width, 1);
        assert_eq!(image.height, 1);
        assert_eq!(image.rgba, vec![255, 0, 0, 255]);
    }

    #[test]
    fn a_malformed_image_hint_is_dropped_rather_than_failing_the_notification() {
        let mut map = hints(vec![("image-data", image_hint(vec![0, 0]))]);
        assert!(
            take_image_data(&mut map).is_none(),
            "two bytes cannot hold a 1x1 RGBA pixel"
        );

        let mut map = hints(vec![("image-data", Value::from("not a struct"))]);
        assert!(take_image_data(&mut map).is_none());
    }

    #[test]
    fn the_image_hint_is_removed_so_it_is_never_decoded_twice() {
        let mut map = hints(vec![("image-data", image_hint(vec![1, 2, 3, 255]))]);
        assert!(take_image_data(&mut map).is_some());
        assert!(map.is_empty(), "the pixel buffer must not be left behind");
    }

    #[test]
    fn the_crownos_categories_select_the_richer_cards() {
        let map = hints(vec![("category", Value::from("x-crownos.call.incoming"))]);
        let content = content_for(&map, 7, "Ada", "+44 7700 900000", None, None);
        let Content::Call(call) = content else {
            panic!("expected a call card");
        };
        assert_eq!(call.contact, "Ada");
        assert_eq!(call.number, "+44 7700 900000");
        assert_eq!(call.call_id, "7", "the id is the fallback handle");

        let map = hints(vec![
            ("category", Value::from("x-crownos.chat")),
            ("x-crownos.conversation-id", Value::from("thread-3")),
        ]);
        let Content::Chat(chat) = content_for(&map, 7, "Ada", "hi", None, None) else {
            panic!("expected a chat card");
        };
        assert_eq!(chat.conversation_id, "thread-3");

        let map = hints(vec![("category", Value::from("x-crownos.music"))]);
        let Content::Music(music) =
            content_for(&map, 7, "Blue Monday", "New Order", None, Some(33))
        else {
            panic!("expected a music card");
        };
        assert_eq!(music.percent, 33);
        assert!(music.playing, "playing unless the sender says otherwise");
    }

    #[test]
    fn a_value_hint_without_a_category_is_a_progress_bar() {
        let map = hints(vec![("value", Value::I32(64))]);
        let Content::Progress(progress) = content_for(&map, 7, "Copying", "", None, Some(64))
        else {
            panic!("expected a progress card");
        };
        assert_eq!(progress.percent, 64);
        assert_eq!(progress.label, "Copying");
    }

    #[test]
    fn the_device_hints_mark_a_notification_as_relayed() {
        let map = hints(vec![
            (DEVICE_KIND_HINT, Value::from("Watch")),
            (DEVICE_NAME_HINT, Value::from("Ada's watch")),
        ]);
        assert_eq!(
            relay_device(&map),
            Some(RelayDevice::new(DeviceKind::Watch, "Ada's watch"))
        );

        let map = hints(vec![(DEVICE_NAME_HINT, Value::from("Ada's phone"))]);
        let device = relay_device(&map).expect("a name alone still badges the card");
        assert_eq!(device.kind, DeviceKind::Phone);

        let map = hints(vec![(DEVICE_KIND_HINT, Value::from("tablet"))]);
        let device = relay_device(&map).expect("a kind alone still badges the card");
        assert_eq!(device.kind, DeviceKind::Tablet);
        assert!(device.name.is_empty());
    }

    #[test]
    fn a_notification_with_no_device_hints_is_not_relayed() {
        let map = hints(vec![("category", Value::from("email.arrived"))]);
        assert_eq!(relay_device(&map), None);

        // A sender that puts something other than a string in the hint has told
        // us nothing, and a badge invented from that would be a lie.
        let map = hints(vec![(DEVICE_KIND_HINT, Value::I32(2))]);
        assert_eq!(relay_device(&map), None);
    }

    #[test]
    fn an_ordinary_notification_is_a_general_card() {
        let map = hints(vec![("category", Value::from("email.arrived"))]);
        assert!(matches!(
            content_for(&map, 7, "Ada", "hi", None, None),
            Content::General
        ));
    }
}
