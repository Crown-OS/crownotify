//! The contract every card kind satisfies.
//!
//! A card is drawn immediately rather than retained as a widget tree, so the
//! surface needs exactly one vocabulary for "sync this, measure it, paint it"
//! whatever shape the notification turned out to be. Before this trait that
//! vocabulary was an enum whose every method named all four kinds, so adding a
//! shape meant editing ten `match` blocks that had nothing to say about it.
//!
//! Most kinds care about only the first five methods. The rest have bodies here
//! because an inline reply is a chat card's business and nobody else's, and a
//! card kind should not have to write `false` six times to say so.

use chrono::{DateTime, Local};
use crownshell::{KeyPress, TextContext, kurbo::Point};

use crate::{
    model::{Notification, NotificationId},
    profile::ContentHint,
    ui::{
        card::{MeasureCtx, RenderCtx},
        hit::CardAction,
        theme::Theme,
    },
};

pub trait CardView {
    /// Which shape this is, so [`Card::sync`] can tell "the notification
    /// changed" from "the notification became a different kind of thing".
    ///
    /// [`Card::sync`]: crate::ui::Card::sync
    fn kind(&self) -> ContentHint;

    /// Takes the notification's new contents, reporting whether anything that
    /// could change the card's height moved.
    fn sync(&mut self, notification: &Notification) -> bool;

    fn apply_theme(&mut self, theme: &Theme);

    /// Height for `width`. Called only when the content or the width changed.
    fn measure(&mut self, width: f64, ctx: &mut MeasureCtx<'_>) -> f64;

    fn render(&mut self, id: NotificationId, ctx: &mut RenderCtx<'_>);

    /// Re-renders relative timestamps against `now`. A card with no timestamp
    /// has nothing to re-render.
    fn refresh_time(&mut self, _now: DateTime<Local>) {}

    /// The action a key produced, and whether it changed the card's height.
    fn on_key(&mut self, _key: &KeyPress<'_>) -> (Option<CardAction>, bool) {
        (None, false)
    }

    /// Places the caret from a click inside an open reply field.
    fn on_click(&mut self, _point: Point, _tcx: &mut TextContext) {}

    /// Opens or closes the overflow menu, reporting whether the card resized.
    fn toggle_menu(&mut self) -> bool {
        false
    }

    /// Opens the inline reply field, reporting whether the card grew.
    fn focus_reply(&mut self) -> bool {
        false
    }

    fn wants_keyboard(&self) -> bool {
        false
    }

    fn take_reply(&mut self) -> Option<String> {
        None
    }
}
