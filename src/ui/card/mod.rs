//! The notification cards.
//!
//! Which of the four views a card wears is decided from the notification's
//! [`Content`] and the sending app's [`AppProfile`](crate::profile::AppProfile)
//! — a profile may force a card kind for an app that does not know how to ask
//! for one. The views themselves are deliberately forgiving about the payload
//! they get: a music card whose notification carries no
//! [`Music`](crate::model::Music) still draws, because the profile that forced
//! it there knew something the sender did not.
//!
//! A view is built once per card and bound to the notification's signal, so a
//! replacement in place updates its text without rebuilding it; only a change
//! of kind does that.

pub mod call;
pub mod chat;
pub mod general;
pub mod music;
pub mod time;

use crownui::prelude::{Runtime, Signal, View, keyed};

use crate::{
    model::{Content, DeviceKind, Icon, Notification},
    profile::{ContentHint, profile_for},
    ui::{
        env::Env,
        widget::{Picture, picture},
    },
};

pub use time::relative_time;

/// Vertical gap between two lines of a card's text column.
pub const LINE_GAP: f32 = 2.0;
/// Gap above a row of controls — action buttons, the reply field, transport.
pub const CONTROL_GAP: f32 = 10.0;
/// Gap between two side-by-side controls.
pub const CONTROL_SPACING: f32 = 8.0;

/// The contents of the card for `notification`, inside the card's padding.
pub fn card_content(
    env: Env,
    notification: Signal<Notification>,
    hovered: Signal<bool>,
) -> impl View {
    keyed(
        move |runtime| notification.with(runtime, view_kind),
        move |_, kind| match kind {
            ContentHint::General => general::general(env, notification, hovered).boxed(),
            ContentHint::Call => call::call(env, notification).boxed(),
            ContentHint::Chat => chat::chat(env, notification, hovered).boxed(),
            ContentHint::Music => music::music(env, notification).boxed(),
        },
    )
}

/// Which view a notification is drawn with.
///
/// The app profile wins over the notification's own [`Content`]: an app that
/// posts its now-playing state as a plain notification has no way to ask for a
/// music card, and the registry is where that knowledge lives.
pub fn view_kind(notification: &Notification) -> ContentHint {
    if let Some(hint) = profile_for(&notification.app.key()).content {
        return hint;
    }
    match &notification.content {
        Content::Call(_) => ContentHint::Call,
        Content::Chat(_) => ContentHint::Chat,
        Content::Music(_) => ContentHint::Music,
        // A progress notification is a general card with a bar in it, not a
        // shape of its own.
        Content::General | Content::Progress(_) => ContentHint::General,
    }
}

/// The card's picture, rebuilt only when what it shows changes. `icon` picks
/// the picture out of the notification; the device badge comes along with it.
pub fn card_picture(
    env: Env,
    notification: Signal<Notification>,
    size: f32,
    radius: f32,
    icon: fn(&Notification) -> Option<&Icon>,
) -> impl View {
    keyed(
        move |runtime: &mut Runtime| {
            let (shown, device) = notification.with(runtime, |notification| {
                (icon(notification).cloned(), device_of(notification))
            });
            let accent = env.accent(runtime, notification);
            Picture {
                icon: shown,
                size,
                radius,
                accent,
                device,
            }
        },
        move |_, shown| picture(env, shown),
    )
}

fn device_of(notification: &Notification) -> Option<DeviceKind> {
    notification.relayed_from.as_ref().map(|device| device.kind)
}

/// A notification's own picture: its image if it sent one, its app's icon
/// otherwise.
pub fn own_icon(notification: &Notification) -> Option<&Icon> {
    notification
        .image
        .as_ref()
        .or(notification.app.icon.as_ref())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Chat, Music};
    use chrono::Local;

    #[test]
    fn a_card_takes_its_view_from_the_notification_content() {
        assert_eq!(
            view_kind(&Notification::builder("Mail").build()),
            ContentHint::General
        );
        let chat = Notification::builder("Signal")
            .content(Content::Chat(Chat {
                conversation_id: "t1".into(),
                sender: "Ada".into(),
                avatar: None,
                sent_at: Local::now(),
            }))
            .build();
        assert_eq!(view_kind(&chat), ContentHint::Chat);
        let music = Notification::builder("Player")
            .content(Content::Music(Music {
                title: "Blue Monday".into(),
                artist: "New Order".into(),
                album_art: None,
                playing: true,
                percent: 10,
            }))
            .build();
        assert_eq!(view_kind(&music), ContentHint::Music);
    }

    /// The registry is what turns Spotify's plain notification into a remote
    /// control; if the profile stopped being consulted, nothing else would.
    #[test]
    fn an_app_profile_overrides_the_content_the_sender_asked_for() {
        let spotify = Notification::builder("Spotify")
            .summary("Blue Monday")
            .body("New Order")
            .build();
        assert_eq!(view_kind(&spotify), ContentHint::Music);
    }

    #[test]
    fn a_progress_notification_is_a_general_card_with_a_bar() {
        let progress = Notification::builder("Files")
            .content(Content::Progress(crate::model::Progress {
                label: "Copying".into(),
                percent: 40,
            }))
            .build();
        assert_eq!(view_kind(&progress), ContentHint::General);
    }
}
