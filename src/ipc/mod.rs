//! The seam between the transports and the daemon's state.
//!
//! Two transports post notifications — `org.freedesktop.Notifications` on the
//! session bus and the crownos-ipc `crownotify` service — and both of them run
//! somewhere other than the thread that owns the [`Store`](crate::store::Store).
//! Rather than let either of them reach into the UI, they are written against
//! [`NotificationSink`] and talk in [`Incoming`] and [`Outgoing`] values.
//!
//! That buys two things. The obvious one is that the store stays on the
//! compositor's thread with no locking. The less obvious one is that both
//! transports become testable: `tests/ipc_service.rs` drives the real service
//! with a recording sink and no compositor at all.
//!
//! # Direction of travel
//!
//! * **In**: a transport hands the UI thread an [`Incoming`] over a
//!   [`calloop::channel`], which wakes the event loop. The D-Bus side uses this
//!   because it has nothing but a channel to reach the UI with; the crownos-ipc
//!   side runs *on* the loop and calls the sink directly.
//! * **Out**: the UI thread queues an [`Outgoing`] and something else publishes
//!   it later — never inside the call that produced it. `CalloopService`
//!   forbids emitting through the service from inside its own `dispatch`, and a
//!   pointer click can produce an event just as easily as an IPC request can,
//!   so deferring every publication is the only scheme with one code path and
//!   no re-entrancy.

pub mod crownconnect;

pub use crownconnect_linux::ipc::proto as crownconnect_protocol;
pub mod server;

use std::path::PathBuf;

use crate::model::{CloseReason, Icon, Notification, NotificationId};

pub use crownconnect::{PhoneCommand, PhoneLink};
pub use server::{NotifyHandler, serve, serve_in};

/// Something a service thread wants the UI thread to do.
#[derive(Debug)]
pub enum Incoming {
    /// Boxed because this variant is an order of magnitude larger than the
    /// rest, and the channel's slot size is the size of the largest variant.
    Post(Box<Notification>),
    Close(NotificationId, CloseReason),
    OpenCenter,
    CloseCenter,
    ToggleCenter,
    SetDoNotDisturb(bool),
    SetFocusMode(Option<String>),
    DismissAll,
    InvokeAction(NotificationId, String),
}

/// Something the UI thread wants a service thread to publish.
#[derive(Debug, Clone)]
pub enum Outgoing {
    Closed(NotificationId, CloseReason),
    ActionInvoked(NotificationId, String),
    Replied(NotificationId, String),
    CenterVisibility(bool),
    DoNotDisturbChanged(bool),
    FocusModeChanged(Option<String>),
}

/// UI thread -> service threads. `Incoming` travels the other way over a
/// `calloop::channel`, so the UI never polls.
pub type OutgoingSender = async_channel::Sender<Outgoing>;
pub type OutgoingReceiver = async_channel::Receiver<Outgoing>;
pub type IncomingSender = calloop::channel::Sender<Incoming>;

/// What a transport does to the daemon's state.
///
/// The transports are written against this rather than against the surface
/// layer directly, which keeps the dependency pointing one way and lets both
/// of them be tested with a mock sink, without a compositor. `surface::Shared`
/// is the real implementation.
pub trait NotificationSink {
    /// Files a notification and returns the id it landed under. Called
    /// synchronously from a D-Bus `Notify`, which must return the id to its
    /// caller, so it cannot be deferred.
    fn post(&mut self, notification: Notification) -> NotificationId;
    fn close(&mut self, id: NotificationId, reason: CloseReason);
    /// `Some(open)` sets the notification center's state, `None` toggles it.
    fn set_center_open(&mut self, open: Option<bool>);
    fn set_do_not_disturb(&mut self, enabled: bool);
    /// `None` ends whichever mode is in force.
    fn set_focus_mode(&mut self, name: Option<&str>);
    fn dismiss_all(&mut self);
    fn invoke_action(&mut self, id: NotificationId, action: &str);
}

/// Interprets an icon string the way both transports receive one.
///
/// `app_icon` on `Notify` and `contact_avatar` over crownos-ipc are the same
/// kind of value: a sender-supplied string that is either an icon-theme name
/// or a location. Getting the two confused would send the theme lookup hunting
/// for an icon called `/home/ada/face.png`, so the discrimination lives in one
/// place. An empty string is not an icon, it is the absence of one.
/// An empty focus-mode name is not a name, it is the absence of one — the same
/// rule [`parse_icon`] applies to an empty icon string.
pub(crate) fn parse_focus_mode(raw: &str) -> Option<&str> {
    Some(raw).filter(|name| !name.is_empty())
}

pub(crate) fn parse_icon(raw: &str) -> Option<Icon> {
    if raw.is_empty() {
        return None;
    }
    if let Some(path) = raw.strip_prefix("file://") {
        return Some(Icon::Path(PathBuf::from(path)));
    }
    if raw.starts_with('/') {
        return Some(Icon::Path(PathBuf::from(raw)));
    }
    Some(Icon::Named(raw.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_focus_mode_name_ends_the_active_mode() {
        assert_eq!(parse_focus_mode(""), None);
        assert_eq!(parse_focus_mode("work"), Some("work"));
    }

    #[test]
    fn an_empty_icon_string_is_no_icon_at_all() {
        assert_eq!(parse_icon(""), None);
    }

    #[test]
    fn absolute_paths_and_file_urls_are_paths_not_theme_names() {
        assert_eq!(
            parse_icon("/usr/share/icons/mail.png"),
            Some(Icon::Path(PathBuf::from("/usr/share/icons/mail.png")))
        );
        assert_eq!(
            parse_icon("file:///home/ada/face.png"),
            Some(Icon::Path(PathBuf::from("/home/ada/face.png")))
        );
    }

    #[test]
    fn a_bare_name_is_resolved_through_the_icon_theme() {
        assert_eq!(
            parse_icon("mail-unread"),
            Some(Icon::Named("mail-unread".to_string()))
        );
    }
}
