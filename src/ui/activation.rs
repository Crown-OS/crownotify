//! Acting on a notification from its card: the app it came from comes
//! forward, then the daemon hears what the user chose.
//!
//! Raising goes through `crownos_app_icon_v1`, crownui's app-icons API, the
//! way a click on a dock icon does. The card stands in for the icon for the
//! length of the click, so the compositor brings the app's windows forward —
//! Android ones included, whose app id is their desktop entry — and grows a
//! window it has to start out of the card.

use crownui::prelude::{Cx, Rect, Signal};

use crate::{
    daemon::Request,
    model::Notification,
    ui::{action::CardAction, env::Env, widget::NodeRef},
};

/// One card's way to act on its notification.
#[derive(Clone, Copy)]
pub struct Activation {
    pub env: Env,
    pub notification: Signal<Notification>,
    /// The card's plate, which is where the app's windows come from.
    pub card: NodeRef,
}

impl Activation {
    /// Raises the app the notification's desktop entry names, then hands
    /// `action` to the daemon, which publishes `ActionInvoked` as it always
    /// has. A notification without a desktop entry only does the latter.
    pub fn perform(self, cx: &mut Cx, action: CardAction) {
        let (id, entry) = self
            .notification
            .with_untracked(cx, |n| (n.id, n.app.desktop_entry.clone()));
        if let (Some(entry), Some(card)) = (entry, self.card.window_rect(cx)) {
            raise(cx, &entry, card, self.env.theme.metrics.card_radius);
        }
        self.env.send(cx, Request::Perform(id, action));
    }
}

/// Hands `app_id` to the compositor as though its icon were drawn at `card`
/// and had just been clicked.
///
/// The icon is put up for the click and taken down straight after it: left up,
/// it would make a card that is about to leave the screen the place the app's
/// windows minimize into.
fn raise(cx: &mut Cx, app_id: &str, card: Rect, corner_radius: f32) {
    cx.set_app_icon(app_id, card, corner_radius);
    if !cx.activate_app_icon(app_id) {
        log::debug!("the compositor offers no way to raise {app_id}");
    }
    cx.remove_app_icon(app_id);
}
