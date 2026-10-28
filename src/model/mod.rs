//! What a notification *is*, independent of where it arrived from and how it
//! is drawn.
//!
//! Everything here is plain owned data with no knowledge of D-Bus, Wayland or
//! the store. The transports decode their wire formats into these types (see
//! [`Timeout::from_spec`], [`Urgency::from_hint`], [`Action::from_spec_pairs`]
//! and [`ImageData::from_hint`]), which is deliberate: those decoders are the
//! only place hostile input from another process is interpreted, so they are
//! also the only place that has to be careful, and they can be unit-tested
//! without a bus.

pub mod action;
pub mod app;
pub mod content;
pub mod device;
pub mod icon;
pub mod level;

use std::time::Duration;

use chrono::{DateTime, Local};

pub use action::{Action, ActionKind};
pub use app::AppIdentity;
pub use content::{Call, Chat, Content, Music, Progress};
pub use device::{DeviceKind, RelayDevice};
pub use icon::{Icon, ImageData};
pub use level::InterruptionLevel;

/// Identifies a notification for its whole life, across replacement.
///
/// Zero is reserved: the freedesktop `Notify` call uses `replaces_id == 0` to
/// mean "this is a new notification", so no live notification may ever carry
/// it.
pub type NotificationId = u32;

/// How loudly a notification asks for attention, in the freedesktop spec's
/// vocabulary.
///
/// Kept in ascending order because it maps onto [`InterruptionLevel`], which is
/// what the daemon actually files and presents on.
///
/// [`InterruptionLevel`]: crate::model::InterruptionLevel
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Hash)]
pub enum Urgency {
    Low,
    #[default]
    Normal,
    Critical,
}

impl Urgency {
    /// Decodes the freedesktop `urgency` hint, which is a byte.
    ///
    /// Values outside 0..=2 are not in the spec but do turn up in the wild;
    /// they degrade to [`Urgency::Normal`] rather than being rejected, because
    /// losing a notification is worse than showing it at the wrong weight.
    pub fn from_hint(value: u8) -> Self {
        match value {
            0 => Self::Low,
            2 => Self::Critical,
            _ => Self::Normal,
        }
    }
}

/// How long a notification stays on screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Timeout {
    /// Let the server decide, from the user's configured default.
    #[default]
    Default,
    /// Stay until the user does something about it.
    Never,
    Millis(u32),
}

impl Timeout {
    /// `expire_timeout` as the freedesktop spec encodes it: -1 default, 0 never.
    ///
    /// Any other negative value is malformed — the sender is a different
    /// process, so it may well be — and falls back to [`Timeout::Default`]
    /// instead of being cast into an absurdly large duration.
    pub fn from_spec(millis: i32) -> Self {
        match millis {
            0 => Self::Never,
            n if n > 0 => Self::Millis(n as u32),
            _ => Self::Default,
        }
    }

    /// Resolved against the config's per-urgency defaults.
    ///
    /// `None` means the notification never expires on its own.
    pub fn resolve(self, default: Duration) -> Option<Duration> {
        match self {
            Self::Default => Some(default),
            Self::Never => None,
            Self::Millis(millis) => Some(Duration::from_millis(u64::from(millis))),
        }
    }
}

/// Why a notification stopped being shown. Values match the freedesktop spec.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum CloseReason {
    Expired = 1,
    Dismissed = 2,
    Closed = 3,
    Undefined = 4,
}

impl CloseReason {
    /// The wire value carried by the `NotificationClosed` signal.
    pub const fn as_u32(self) -> u32 {
        self as u32
    }
}

/// One notification, as the daemon understands it.
#[derive(Debug, Clone, PartialEq)]
pub struct Notification {
    pub id: NotificationId,
    pub app: AppIdentity,
    pub summary: String,
    pub body: String,
    pub posted: DateTime<Local>,
    pub urgency: Urgency,
    /// What the daemon presents and files on. Derived from `urgency` unless
    /// the sender named a level itself.
    pub level: InterruptionLevel,
    pub timeout: Timeout,
    pub actions: Vec<Action>,
    pub content: Content,
    /// Stays in the center after the toast expires (freedesktop `resident`).
    pub resident: bool,
    /// Never enters the center (freedesktop `transient`).
    pub transient: bool,
    pub image: Option<Icon>,
    /// The paired device this was mirrored from, when it was not raised here.
    /// `None` is a notification from this machine.
    pub relayed_from: Option<RelayDevice>,
}

impl Notification {
    /// Starts a notification for `app_name`; see [`NotificationBuilder`].
    pub fn builder(app_name: impl Into<String>) -> NotificationBuilder {
        NotificationBuilder {
            level: None,
            notification: Self {
                id: 0,
                app: AppIdentity {
                    name: app_name.into(),
                    desktop_entry: None,
                    icon: None,
                },
                summary: String::new(),
                body: String::new(),
                posted: Local::now(),
                urgency: Urgency::default(),
                level: InterruptionLevel::default(),
                timeout: Timeout::default(),
                actions: Vec::new(),
                content: Content::default(),
                resident: false,
                transient: false,
                image: None,
                relayed_from: None,
            },
        }
    }

    /// The action a bare click on the card invokes, if any.
    pub fn default_action(&self) -> Option<&Action> {
        self.actions
            .iter()
            .find(|action| action.kind == ActionKind::Default)
    }

    /// The inline-reply action, if the sender offered one.
    pub fn reply_action(&self) -> Option<&Action> {
        self.actions
            .iter()
            .find(|action| matches!(action.kind, ActionKind::InlineReply { .. }))
    }
}

/// Owned builder for [`Notification`].
///
/// The transports fill notifications in field by field as they walk the hints
/// map, and the tests build them by hand; both want the same defaults, so they
/// live here rather than being repeated at every call site.
#[derive(Debug, Clone)]
pub struct NotificationBuilder {
    notification: Notification,
    /// `None` until a transport says otherwise, which is what lets `build`
    /// fall back to the urgency no matter which setter ran first.
    level: Option<InterruptionLevel>,
}

impl NotificationBuilder {
    /// Files the notification under a specific id, which is how a replacement
    /// keeps its place in the stack. Leaving it unset (or zero) lets the store
    /// allocate.
    pub fn id(mut self, id: NotificationId) -> Self {
        self.notification.id = id;
        self
    }

    pub fn summary(mut self, summary: impl Into<String>) -> Self {
        self.notification.summary = summary.into();
        self
    }

    pub fn body(mut self, body: impl Into<String>) -> Self {
        self.notification.body = body.into();
        self
    }

    pub fn urgency(mut self, urgency: Urgency) -> Self {
        self.notification.urgency = urgency;
        self
    }

    /// Overrides the level the urgency would otherwise imply.
    pub fn level(mut self, level: InterruptionLevel) -> Self {
        self.level = Some(level);
        self
    }

    pub fn timeout(mut self, timeout: Timeout) -> Self {
        self.notification.timeout = timeout;
        self
    }

    pub fn actions(mut self, actions: Vec<Action>) -> Self {
        self.notification.actions = actions;
        self
    }

    pub fn action(mut self, action: Action) -> Self {
        self.notification.actions.push(action);
        self
    }

    pub fn content(mut self, content: Content) -> Self {
        self.notification.content = content;
        self
    }

    /// The sending application's own icon, drawn in the card's left column.
    pub fn icon(mut self, icon: Icon) -> Self {
        self.notification.app.icon = Some(icon);
        self
    }

    /// The image the notification carries (the `image-data` / `image-path`
    /// hint), which outranks the app icon when both are present.
    pub fn image(mut self, image: Icon) -> Self {
        self.notification.image = Some(image);
        self
    }

    /// The device this notification was mirrored from, which is what makes the
    /// card wear a device badge on its icon.
    pub fn relayed_from(mut self, device: RelayDevice) -> Self {
        self.notification.relayed_from = Some(device);
        self
    }

    /// The `desktop-entry` hint. Worth setting whenever it is known: it is a
    /// far more stable grouping key than the app's display name.
    pub fn desktop_entry(mut self, entry: impl Into<String>) -> Self {
        self.notification.app.desktop_entry = Some(entry.into());
        self
    }

    pub fn resident(mut self, resident: bool) -> Self {
        self.notification.resident = resident;
        self
    }

    pub fn transient(mut self, transient: bool) -> Self {
        self.notification.transient = transient;
        self
    }

    /// Resolves the level here rather than in a setter, so a sender that sets
    /// the urgency after the level does not silently overwrite it.
    pub fn build(mut self) -> Notification {
        self.notification.level = self
            .level
            .unwrap_or(InterruptionLevel::from_urgency(self.notification.urgency));
        self.notification
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urgency_hints_outside_the_spec_are_treated_as_normal() {
        assert_eq!(Urgency::from_hint(0), Urgency::Low);
        assert_eq!(Urgency::from_hint(1), Urgency::Normal);
        assert_eq!(Urgency::from_hint(2), Urgency::Critical);
        for value in [3u8, 7, 255] {
            assert_eq!(
                Urgency::from_hint(value),
                Urgency::Normal,
                "hint {value} should degrade to Normal"
            );
        }
    }

    #[test]
    fn urgency_orders_low_below_critical() {
        assert!(
            Urgency::Low < Urgency::Normal,
            "Low should sort below Normal"
        );
        assert!(
            Urgency::Normal < Urgency::Critical,
            "Normal should sort below Critical"
        );
    }

    #[test]
    fn timeout_decodes_the_freedesktop_encoding() {
        assert_eq!(Timeout::from_spec(-1), Timeout::Default);
        assert_eq!(Timeout::from_spec(0), Timeout::Never);
        assert_eq!(Timeout::from_spec(5000), Timeout::Millis(5000));
        assert_eq!(
            Timeout::from_spec(i32::MAX),
            Timeout::Millis(i32::MAX as u32)
        );
    }

    #[test]
    fn malformed_negative_timeouts_fall_back_to_the_default() {
        for millis in [-2, -1000, i32::MIN] {
            assert_eq!(
                Timeout::from_spec(millis),
                Timeout::Default,
                "expire_timeout {millis} is malformed and should use the default"
            );
        }
    }

    #[test]
    fn timeout_resolves_against_the_configured_default() {
        let default = Duration::from_secs(5);
        assert_eq!(Timeout::Default.resolve(default), Some(default));
        assert_eq!(Timeout::Never.resolve(default), None);
        assert_eq!(
            Timeout::Millis(250).resolve(default),
            Some(Duration::from_millis(250)),
            "an explicit timeout should ignore the default"
        );
    }

    #[test]
    fn close_reasons_use_the_spec_wire_values() {
        assert_eq!(CloseReason::Expired.as_u32(), 1);
        assert_eq!(CloseReason::Dismissed.as_u32(), 2);
        assert_eq!(CloseReason::Closed.as_u32(), 3);
        assert_eq!(CloseReason::Undefined.as_u32(), 4);
    }

    #[test]
    fn a_built_notification_defaults_to_a_plain_general_card() {
        let notification = Notification::builder("Mail").summary("Hello").build();
        assert_eq!(
            notification.id, 0,
            "an unset id means 'allocate one for me'"
        );
        assert_eq!(notification.app.name, "Mail");
        assert!(notification.body.is_empty());
        assert_eq!(notification.urgency, Urgency::Normal);
        assert_eq!(notification.timeout, Timeout::Default);
        assert!(matches!(notification.content, Content::General));
        assert!(!notification.transient);
        assert!(!notification.resident);
        assert_eq!(
            notification.relayed_from, None,
            "a notification is local unless a transport says otherwise"
        );
    }

    #[test]
    fn the_builder_records_every_field_it_is_given() {
        let notification = Notification::builder("Signal")
            .id(9)
            .summary("Ada")
            .body("See you at 5")
            .urgency(Urgency::Critical)
            .timeout(Timeout::Never)
            .desktop_entry("org.signal.Signal")
            .icon(Icon::Named("signal".into()))
            .image(Icon::Named("avatar".into()))
            .resident(true)
            .transient(true)
            .relayed_from(RelayDevice::new(DeviceKind::Phone, "Ada's phone"))
            .build();

        assert_eq!(notification.id, 9);
        assert_eq!(notification.summary, "Ada");
        assert_eq!(notification.body, "See you at 5");
        assert_eq!(notification.urgency, Urgency::Critical);
        assert_eq!(notification.timeout, Timeout::Never);
        assert_eq!(
            notification.app.desktop_entry.as_deref(),
            Some("org.signal.Signal")
        );
        assert_eq!(notification.app.icon, Some(Icon::Named("signal".into())));
        assert_eq!(notification.image, Some(Icon::Named("avatar".into())));
        assert!(notification.resident);
        assert!(notification.transient);
        assert_eq!(
            notification.relayed_from,
            Some(RelayDevice::new(DeviceKind::Phone, "Ada's phone"))
        );
    }

    #[test]
    fn default_and_reply_actions_are_found_by_kind_not_position() {
        let notification = Notification::builder("Chat")
            .action(Action {
                key: "archive".into(),
                label: "Archive".into(),
                kind: ActionKind::Button,
            })
            .action(Action {
                key: "inline-reply".into(),
                label: "Reply".into(),
                kind: ActionKind::InlineReply {
                    placeholder: "Reply".into(),
                },
            })
            .action(Action {
                key: "default".into(),
                label: "Open".into(),
                kind: ActionKind::Default,
            })
            .build();

        assert_eq!(
            notification.default_action().map(|a| a.key.as_str()),
            Some("default")
        );
        assert_eq!(
            notification.reply_action().map(|a| a.key.as_str()),
            Some("inline-reply")
        );
    }

    #[test]
    fn a_notification_without_actions_has_neither_default_nor_reply() {
        let notification = Notification::builder("Mail").build();
        assert!(notification.default_action().is_none());
        assert!(notification.reply_action().is_none());
    }
}
