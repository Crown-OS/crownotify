//! The incoming-call card: avatar, contact, and the two buttons that matter.
//!
//! This is the one card where a mis-click is expensive, so the answer and
//! decline buttons are drawn as large, saturated, unmistakably coloured discs
//! well clear of each other and of the card body — and the body's own
//! [`CardAction::Activate`] region is left underneath them rather than being
//! suppressed, because [`HitMap`](crate::ui::hit::HitMap) resolves the topmost
//! push and these are pushed last.
//!
//! The card is a remote control for a phone that crownconnect is relaying, so it
//! never assumes a call is local: everything it draws comes out of the
//! [`Call`](crate::model::Call) payload, and it degrades to the notification's
//! own summary and body when a profile forced it here without one.

use crownshell::{Text, TextContext, kurbo::Point, kurbo::Rect};

use crate::{
    model::{Content, DeviceKind, Icon, Notification, NotificationId},
    profile::ContentHint,
    ui::{
        card::{
            CONTROL_SPACING, LINE_GAP, MIN_COLUMN, MeasureCtx, RenderCtx, Row, content_rect,
            draw_icon, set_text, view::CardView,
        },
        hit::CardAction,
        theme::Theme,
        widget::{IconButton, sync_style},
    },
};

/// How much bigger the contact avatar is than an app icon. A call is the
/// loudest card in the stack and the face on it is the whole point.
const AVATAR_SCALE: f64 = 1.15;

/// The call card's retained state.
pub struct CallView {
    avatar: Option<Icon>,
    device: Option<DeviceKind>,
    contact: Text,
    number: Text,
}

impl CallView {
    pub fn new(notification: &Notification) -> Self {
        let mut view = Self {
            avatar: None,
            device: None,
            contact: Text::new(""),
            number: Text::new(""),
        };
        view.sync(notification);
        view
    }

    /// Width left for the text between the avatar and the two buttons.
    fn column(&self, width: f64, theme: &Theme, avatar: f64) -> f64 {
        let metrics = &theme.metrics;
        let buttons = 2.0 * metrics.button + CONTROL_SPACING;
        (width - 2.0 * metrics.pad_x - avatar - metrics.icon_gap - buttons - CONTROL_SPACING)
            .max(MIN_COLUMN)
    }

    fn rows(&mut self, column: f64, tcx: &mut TextContext) -> Column {
        self.contact.set_max_width(Some(column));
        self.contact.set_max_lines(Some(1));
        self.number.set_max_width(Some(column));
        self.number.set_max_lines(Some(1));

        let contact = Row {
            top: 0.0,
            height: self.contact.height(tcx),
        };
        let mut y = contact.height;
        let number = if self.number.text().is_empty() {
            Row::default()
        } else {
            y += LINE_GAP;
            let row = Row {
                top: y,
                height: self.number.height(tcx),
            };
            y += row.height;
            row
        };

        Column {
            contact,
            number,
            height: y,
        }
    }
}

impl CardView for CallView {
    fn kind(&self) -> ContentHint {
        ContentHint::Call
    }

    /// Re-reads `notification`, reporting whether the card's height could have
    /// moved as a result.
    fn sync(&mut self, notification: &Notification) -> bool {
        let call = match &notification.content {
            Content::Call(call) => Some(call),
            _ => None,
        };

        let contact = call.map_or(notification.summary.as_str(), |call| call.contact.as_str());
        let number = call.map_or(notification.body.as_str(), |call| call.number.as_str());
        let changed = set_text(&mut self.contact, contact) | set_text(&mut self.number, number);

        let avatar = call
            .and_then(|call| call.avatar.as_ref())
            .or(notification.image.as_ref())
            .or(notification.app.icon.as_ref());
        if self.avatar.as_ref() != avatar {
            self.avatar = avatar.cloned();
        }
        self.device = notification.relayed_from.as_ref().map(|device| device.kind);
        changed
    }
    fn apply_theme(&mut self, theme: &Theme) {
        sync_style(&mut self.contact, &theme.text.summary);
        sync_style(&mut self.number, &theme.text.body);
    }
    fn measure(&mut self, width: f64, ctx: &mut MeasureCtx<'_>) -> f64 {
        let metrics = &ctx.theme.metrics;
        let avatar = metrics.icon * AVATAR_SCALE;
        let column = self.column(width, ctx.theme, avatar);
        let rows = self.rows(column, ctx.text);
        2.0 * metrics.pad_y + rows.height.max(avatar).max(metrics.button)
    }
    fn render(&mut self, id: NotificationId, ctx: &mut RenderCtx<'_>) {
        let metrics = ctx.theme.metrics;
        let inner = content_rect(ctx.bounds, ctx.theme);
        let avatar = metrics.icon * AVATAR_SCALE;

        let avatar_rect = Rect::new(
            inner.x0,
            inner.y0 + (inner.height() - avatar) / 2.0,
            inner.x0 + avatar,
            inner.y0 + (inner.height() + avatar) / 2.0,
        );
        draw_icon(
            ctx,
            self.avatar.as_ref(),
            avatar_rect,
            avatar / 2.0,
            self.device,
        );

        // Answer sits furthest right, where the thumb lands; decline is inboard
        // of it, so a fumbled reach hangs up rather than picks up in error.
        let answer = IconButton::answer(ctx.theme);
        let decline = IconButton::decline(ctx.theme);
        let middle = inner.center().y;
        let answer_center = Point::new(inner.x1 - answer.diameter / 2.0, middle);
        let decline_center = Point::new(
            answer_center.x - answer.diameter / 2.0 - CONTROL_SPACING - decline.diameter / 2.0,
            middle,
        );

        for (button, center, action) in [
            (decline, decline_center, CardAction::Decline),
            (answer, answer_center, CardAction::Answer),
        ] {
            let state = ctx.state(&action);
            button.render(ctx.scene, center, state, 1.0);
            ctx.hits.push(button.bounds(center), id, action);
        }

        let left = avatar_rect.x1 + metrics.icon_gap;
        let column =
            (decline_center.x - decline.diameter / 2.0 - CONTROL_SPACING - left).max(MIN_COLUMN);
        let rows = self.rows(column, ctx.text);
        let top = inner.y0 + (inner.height() - rows.height) / 2.0;

        for (row, text) in [
            (rows.contact, &mut self.contact),
            (rows.number, &mut self.number),
        ] {
            if row.shown() {
                text.draw(ctx.text, ctx.scene, (left.round(), (top + row.top).round()));
            }
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct Column {
    contact: Row,
    number: Row,
    height: f64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        model::Call,
        ui::card::tests::{card, entry, measure, paint},
    };
    use crownshell::TextContext;

    fn incoming() -> crate::store::Entry {
        entry(
            Notification::builder("Phone")
                .id(21)
                .summary("Incoming call")
                .content(Content::Call(Call {
                    call_id: "call-42".into(),
                    contact: "Ada Lovelace".into(),
                    number: "+44 7700 900000".into(),
                    avatar: None,
                }))
                .build(),
        )
    }

    #[test]
    fn the_call_card_shows_the_contact_and_the_number_from_the_payload() {
        let mut view = CallView::new(&incoming().notification);
        assert_eq!(view.contact.text(), "Ada Lovelace");
        assert_eq!(view.number.text(), "+44 7700 900000");
        assert!(
            !view.sync(&incoming().notification),
            "an identical sync claimed the card changed"
        );
    }

    /// A profile can force a call card onto a notification that carries no call
    /// payload; it has to draw something rather than a blank strip.
    #[test]
    fn a_call_card_without_a_payload_falls_back_to_the_summary_and_body() {
        let mut view = CallView::new(
            &Notification::builder("Phone")
                .summary("Unknown caller")
                .body("+44 7700 900001")
                .build(),
        );
        assert_eq!(view.contact.text(), "Unknown caller");
        assert_eq!(view.number.text(), "+44 7700 900001");
        assert!(
            view.sync(&incoming().notification),
            "gaining a payload should have changed the card"
        );
        assert_eq!(view.contact.text(), "Ada Lovelace");
    }

    #[test]
    fn answering_and_declining_are_two_separate_regions_inside_the_card() {
        let theme = Theme::light();
        let mut tcx = TextContext::new();
        let mut card = card(&incoming());

        let hits = paint(&mut card, &mut tcx, &theme);
        let bounds = Rect::new(
            12.0,
            20.0,
            12.0 + theme.metrics.card_width,
            20.0 + card.height(),
        );

        let middle = bounds.center().y;
        let answer = Point::new(
            bounds.x1 - theme.metrics.pad_x - theme.metrics.button / 2.0,
            middle,
        );
        let decline = Point::new(answer.x - theme.metrics.button - CONTROL_SPACING, middle);

        assert_eq!(
            hits.hit(answer).map(|hit| &hit.action),
            Some(&CardAction::Answer),
            "the answer button was not where the card drew it"
        );
        assert_eq!(
            hits.hit(decline).map(|hit| &hit.action),
            Some(&CardAction::Decline)
        );
        for rect in hits.rects() {
            assert!(
                bounds.union(rect) == bounds,
                "{rect:?} escaped the card bounds {bounds:?}"
            );
        }
    }

    #[test]
    fn a_call_card_is_at_least_as_tall_as_its_own_controls() {
        let theme = Theme::light();
        let mut tcx = TextContext::new();
        let mut card = card(&incoming());
        let height = measure(&mut card, &mut tcx, &theme);

        assert!(
            height >= 2.0 * theme.metrics.pad_y + theme.metrics.icon * AVATAR_SCALE,
            "the card {height} is shorter than the avatar it draws"
        );
        assert!(height >= 2.0 * theme.metrics.pad_y + theme.metrics.button);
    }

    #[test]
    fn a_call_card_never_grows_a_reply_field() {
        let mut card = card(&incoming());
        assert!(!card.wants_keyboard());
        card.focus_reply();
        assert!(
            !card.wants_keyboard(),
            "a call card accepted keyboard focus"
        );
        assert_eq!(card.take_reply(), None);
    }
}
