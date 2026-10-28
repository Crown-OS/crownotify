//! The chat card, and the inline reply that makes it worth having.
//!
//! Reply has two states and the card is a different height in each, which is
//! the whole reason [`focus_reply`](ChatView::focus_reply) and
//! [`on_key`](ChatView::on_key) report back whether they resized it: collapsed
//! the affordance is a chip that only asks for the keyboard, expanded it is a
//! [`TextField`] plus a send button and the card has grown to hold them. A
//! resize that did not invalidate the cached height would leave the field drawn
//! outside the card it belongs to.
//!
//! Keyboard focus is the card's, not the surface's: the surface asks
//! [`wants_keyboard`](ChatView::wants_keyboard) and raises the layer surface's
//! `KeyboardInteractivity` to match. A toast strip that held the keyboard
//! permanently would steal every key the user meant for the window below.

use chrono::{DateTime, Local};
use crownshell::{
    KeyPress, Text, TextContext,
    kurbo::{Point, Rect},
    peniko::Color,
};

use crate::{
    model::{ActionKind, Content, DeviceKind, Icon, Notification, NotificationId},
    profile::ContentHint,
    ui::{
        card::{
            CONTROL_GAP, CONTROL_SPACING, LINE_GAP, MIN_COLUMN, MeasureCtx, RenderCtx, Row,
            STAMP_GAP, content_rect, draw_icon, set_text, view::CardView, write_relative_time,
        },
        hit::CardAction,
        theme::Theme,
        widget::{ButtonStyle, FieldEvent, IconButton, PillButton, Symbol, TextField, sync_style},
    },
};

/// How many lines of the message the card shows before ellipsising. Shorter
/// than a general card's body on purpose: a chat card's job is to say who said
/// something and roughly what, and then get out of the way of the reply.
const MESSAGE_LINES: usize = 3;
/// Label on the collapsed reply chip.
const REPLY_LABEL: &str = "Reply";

/// The chat card's retained state.
pub struct ChatView {
    avatar: Option<Icon>,
    device: Option<DeviceKind>,
    sender: Text,
    message: Text,
    stamp: Text,
    stamp_buffer: String,
    sent_at: DateTime<Local>,
    /// The collapsed affordance. Retained even while expanded, because the
    /// field can be cancelled back to it at any keystroke.
    chip: PillButton,
    field: TextField,
    /// Where the field was drawn last frame, which is what a click has to be
    /// made relative to. Empty while the field is collapsed, so a click can
    /// never be measured against a rectangle that is no longer on screen.
    field_bounds: Rect,
    /// Whether the notification offered an inline reply at all.
    repliable: bool,
    expanded: bool,
}

impl ChatView {
    pub fn new(notification: &Notification) -> Self {
        let mut view = Self {
            avatar: None,
            device: None,
            sender: Text::new(""),
            message: Text::new(""),
            stamp: Text::new(""),
            stamp_buffer: String::new(),
            sent_at: notification.posted,
            chip: PillButton::new(REPLY_LABEL, ButtonStyle::Tinted),
            field: TextField::new(REPLY_LABEL),
            field_bounds: Rect::ZERO,
            repliable: false,
            expanded: false,
        };
        view.sync(notification);
        view
    }

    /// Closes the reply field without sending anything.
    fn cancel_reply(&mut self) -> bool {
        let resized = self.expanded;
        self.expanded = false;
        self.field.set_focused(false);
        resized
    }

    /// Draws whichever half of the reply affordance is currently showing.
    fn render_reply(&mut self, id: NotificationId, ctx: &mut RenderCtx<'_>, left: f64) {
        if !self.repliable {
            self.field_bounds = Rect::ZERO;
            return;
        }
        let metrics = ctx.theme.metrics;
        let right = ctx.bounds.x1 - metrics.pad_x;

        if !self.expanded {
            self.field_bounds = Rect::ZERO;
            let width = self.chip.measure(ctx.text, ctx.theme).width;
            let top = ctx.bounds.y1 - metrics.pad_y - metrics.button;
            let bounds = Rect::new(left, top, (left + width).min(right), top + metrics.button);
            let state = ctx.state(&CardAction::FocusReply);
            let theme = ctx.theme;
            self.chip
                .render(ctx.scene, bounds, theme, state, 1.0, ctx.text);
            ctx.hits.push(bounds, id, CardAction::FocusReply);
            return;
        }

        let top = ctx.bounds.y1 - metrics.pad_y - metrics.field_height;
        let send = IconButton::new(
            Symbol::Send,
            ctx.accent(),
            Color::WHITE,
            metrics.field_height,
        );
        let center = Point::new(
            right - send.diameter / 2.0,
            top + metrics.field_height / 2.0,
        );
        let field = Rect::new(
            left,
            top,
            (center.x - send.diameter / 2.0 - CONTROL_SPACING).max(left + MIN_COLUMN),
            top + metrics.field_height,
        );

        let theme = ctx.theme;
        self.field.render(ctx.scene, field, theme, ctx.text);
        self.field_bounds = field;
        ctx.hits.push(field, id, CardAction::FocusReply);

        let state = ctx.state(&CardAction::SubmitReply);
        send.render(ctx.scene, center, state, 1.0);
        ctx.hits
            .push(send.bounds(center), id, CardAction::SubmitReply);
    }

    /// Height the reply affordance adds below the text column.
    fn reply_height(&self, theme: &Theme) -> f64 {
        if !self.repliable {
            0.0
        } else if self.expanded {
            CONTROL_GAP + theme.metrics.field_height
        } else {
            CONTROL_GAP + theme.metrics.button
        }
    }

    fn rows(&mut self, column: f64, tcx: &mut TextContext) -> Column {
        let head = (column - self.stamp.width(tcx) - STAMP_GAP).max(MIN_COLUMN);
        self.sender.set_max_width(Some(head));
        self.sender.set_max_lines(Some(1));
        self.message.set_max_width(Some(column));
        self.message.set_max_lines(Some(MESSAGE_LINES));

        let sender = Row {
            top: 0.0,
            height: self.sender.height(tcx),
        };
        let mut y = sender.height;
        let message = if self.message.text().is_empty() {
            Row::default()
        } else {
            y += LINE_GAP;
            let row = Row {
                top: y,
                height: self.message.height(tcx),
            };
            y += row.height;
            row
        };

        Column {
            sender,
            message,
            height: y,
        }
    }
}

impl CardView for ChatView {
    fn kind(&self) -> ContentHint {
        ContentHint::Chat
    }

    /// Re-reads `notification`, reporting whether the card's height could have
    /// moved as a result.
    fn sync(&mut self, notification: &Notification) -> bool {
        let chat = match &notification.content {
            Content::Chat(chat) => Some(chat),
            _ => None,
        };

        let sender = chat
            .map(|chat| chat.sender.as_str())
            .filter(|sender| !sender.is_empty())
            .unwrap_or(&notification.summary);
        let mut changed = set_text(&mut self.sender, sender);
        changed |= set_text(&mut self.message, &notification.body);

        let avatar = chat
            .and_then(|chat| chat.avatar.as_ref())
            .or(notification.image.as_ref())
            .or(notification.app.icon.as_ref());
        if self.avatar.as_ref() != avatar {
            self.avatar = avatar.cloned();
        }
        self.device = notification.relayed_from.as_ref().map(|device| device.kind);
        self.sent_at = chat.map_or(notification.posted, |chat| chat.sent_at);

        let reply = notification.reply_action();
        if let Some(ActionKind::InlineReply { placeholder }) = reply.map(|action| &action.kind)
            && self.field.placeholder() != placeholder.as_str()
        {
            // The placeholder is the sender's own wording ("Reply to Ada"), so
            // it changes when the conversation does.
            self.field = TextField::new(placeholder.clone());
        }

        let repliable = reply.is_some();
        changed |= repliable != self.repliable;
        self.repliable = repliable;
        if !repliable {
            self.expanded = false;
        }
        changed
    }
    fn apply_theme(&mut self, theme: &Theme) {
        sync_style(&mut self.sender, &theme.text.summary);
        sync_style(&mut self.message, &theme.text.body);
        sync_style(&mut self.stamp, &theme.text.meta);
    }
    fn refresh_time(&mut self, now: DateTime<Local>) {
        write_relative_time(&mut self.stamp_buffer, now, self.sent_at);
        self.stamp.set_text(&self.stamp_buffer);
    }
    /// Opens and focuses the reply field. Returns whether the card resized.
    fn focus_reply(&mut self) -> bool {
        if !self.repliable {
            return false;
        }
        let resized = !self.expanded;
        self.expanded = true;
        self.field.set_focused(true);
        resized
    }
    /// Places the caret from a click at `point`, in the coordinates the card
    /// was rendered in.
    ///
    /// Ignored unless the click landed in the open field: the collapsed chip
    /// holds no text, and a click anywhere else on the card is an action rather
    /// than a caret. The rectangle comes from the last frame, which is the only
    /// place it exists — the card lays its controls out as it draws them — and
    /// is exactly the one the click was hit-tested against.
    fn on_click(&mut self, point: Point, tcx: &mut TextContext) {
        if !self.field_bounds.contains(point) {
            return;
        }
        self.field
            .on_click(point - self.field_bounds.origin().to_vec2(), tcx);
    }
    fn wants_keyboard(&self) -> bool {
        self.expanded && self.field.is_focused()
    }
    /// Drains the field. `None` for an empty reply, which is a mis-click rather
    /// than a message.
    fn take_reply(&mut self) -> Option<String> {
        if self.field.is_empty() {
            return None;
        }
        let text = self.field.take();
        self.cancel_reply();
        Some(text)
    }
    /// The action the key produced, and whether it resized the card.
    fn on_key(&mut self, key: &KeyPress<'_>) -> (Option<CardAction>, bool) {
        match self.field.on_key(key) {
            FieldEvent::Submit if !self.field.is_empty() => (Some(CardAction::SubmitReply), false),
            // Return on an empty field is the user changing their mind, not an
            // empty message; treat it as a cancel rather than sending nothing.
            FieldEvent::Submit | FieldEvent::Cancel => (None, self.cancel_reply()),
            FieldEvent::Changed | FieldEvent::Ignored => (None, false),
        }
    }
    fn measure(&mut self, width: f64, ctx: &mut MeasureCtx<'_>) -> f64 {
        let metrics = &ctx.theme.metrics;
        let column =
            (width - 2.0 * metrics.pad_x - metrics.icon - metrics.icon_gap).max(MIN_COLUMN);
        let rows = self.rows(column, ctx.text);
        2.0 * metrics.pad_y + rows.height.max(metrics.icon) + self.reply_height(ctx.theme)
    }
    fn render(&mut self, id: NotificationId, ctx: &mut RenderCtx<'_>) {
        let metrics = ctx.theme.metrics;
        let inner = content_rect(ctx.bounds, ctx.theme);

        let avatar = Rect::new(
            inner.x0,
            inner.y0,
            inner.x0 + metrics.icon,
            inner.y0 + metrics.icon,
        );
        draw_icon(
            ctx,
            self.avatar.as_ref(),
            avatar,
            metrics.icon / 2.0,
            self.device,
        );

        let left = avatar.x1 + metrics.icon_gap;
        let column = (inner.x1 - left).max(MIN_COLUMN);
        let rows = self.rows(column, ctx.text);
        let top = inner.y0;

        if !ctx.is_hovered() {
            let stamp = self.stamp.width(ctx.text);
            self.stamp.draw(
                ctx.text,
                ctx.scene,
                ((inner.x1 - stamp).round(), top.round()),
            );
        }
        for (row, text) in [
            (rows.sender, &mut self.sender),
            (rows.message, &mut self.message),
        ] {
            if row.shown() {
                text.draw(ctx.text, ctx.scene, (left.round(), (top + row.top).round()));
            }
        }

        self.render_reply(id, ctx, left);
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct Column {
    sender: Row,
    message: Row,
    height: f64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        model::{Action, Chat},
        store::Entry,
        ui::card::tests::{card, entry, measure, paint},
    };
    use crownshell::{Keysym, Modifiers, TextContext};

    fn message(repliable: bool) -> Entry {
        let mut builder = Notification::builder("Signal")
            .id(31)
            .summary("Ada Lovelace")
            .body("Are we still on for five?")
            .content(Content::Chat(Chat {
                conversation_id: "thread-7".into(),
                sender: "Ada Lovelace".into(),
                avatar: None,
                sent_at: Local::now(),
            }));
        if repliable {
            builder = builder.action(Action {
                key: "inline-reply".into(),
                label: "Reply to Ada".into(),
                kind: ActionKind::InlineReply {
                    placeholder: "Reply to Ada".into(),
                },
            });
        }
        entry(builder.build())
    }

    fn key(
        card: &mut crate::ui::card::Card,
        keysym: Keysym,
        text: Option<&str>,
    ) -> Option<CardAction> {
        card.on_key(&KeyPress {
            keysym,
            raw_code: 0,
            text,
            modifiers: Modifiers::default(),
            repeat: false,
        })
    }

    fn type_text(card: &mut crate::ui::card::Card, text: &str) {
        for character in text.chars() {
            let mut buffer = [0_u8; 4];
            key(
                card,
                Keysym::from(0),
                Some(character.encode_utf8(&mut buffer)),
            );
        }
    }

    #[test]
    fn the_reply_affordance_only_exists_when_the_sender_offered_one() {
        let mut without = ChatView::new(&message(false).notification);
        assert!(!without.repliable);
        assert!(
            !without.focus_reply(),
            "a card with no reply action expanded"
        );
        assert!(!without.wants_keyboard());

        let mut with = ChatView::new(&message(true).notification);
        assert!(with.repliable);
        assert_eq!(with.field.placeholder(), "Reply to Ada");
        assert!(with.focus_reply());
        assert!(with.wants_keyboard());
    }

    #[test]
    fn a_repliable_card_is_taller_and_grows_again_when_the_field_opens() {
        let theme = Theme::light();
        let mut tcx = TextContext::new();

        let mut plain = card(&message(false));
        let mut repliable = card(&message(true));

        let plain_height = measure(&mut plain, &mut tcx, &theme);
        let collapsed = measure(&mut repliable, &mut tcx, &theme);
        assert!(
            collapsed > plain_height,
            "the reply chip took no room: {collapsed} vs {plain_height}"
        );

        repliable.focus_reply();
        assert!(
            repliable.needs_measure(),
            "expanding the field did not invalidate the cached height"
        );
        let expanded = measure(&mut repliable, &mut tcx, &theme);
        assert!(
            (expanded - collapsed).abs() > f64::EPSILON,
            "expanding the field left the card exactly {expanded} px tall"
        );
    }

    #[test]
    fn the_collapsed_chip_asks_for_focus_and_the_open_field_submits() {
        let theme = Theme::light();
        let mut tcx = TextContext::new();
        let mut card = card(&message(true));

        let hits = paint(&mut card, &mut tcx, &theme);
        let bounds = Rect::new(
            12.0,
            20.0,
            12.0 + theme.metrics.card_width,
            20.0 + card.height(),
        );
        let chip = Point::new(
            bounds.x0 + theme.metrics.pad_x + theme.metrics.icon + theme.metrics.icon_gap + 8.0,
            bounds.y1 - theme.metrics.pad_y - theme.metrics.button / 2.0,
        );
        assert_eq!(
            hits.hit(chip).map(|hit| &hit.action),
            Some(&CardAction::FocusReply),
            "the reply chip was not where the card drew it"
        );

        card.focus_reply();
        let hits = paint(&mut card, &mut tcx, &theme);
        let bounds = Rect::new(
            12.0,
            20.0,
            12.0 + theme.metrics.card_width,
            20.0 + card.height(),
        );
        let send = Point::new(
            bounds.x1 - theme.metrics.pad_x - theme.metrics.field_height / 2.0,
            bounds.y1 - theme.metrics.pad_y - theme.metrics.field_height / 2.0,
        );
        assert_eq!(
            hits.hit(send).map(|hit| &hit.action),
            Some(&CardAction::SubmitReply)
        );
        for rect in hits.rects() {
            assert!(
                bounds.union(rect) == bounds,
                "{rect:?} escaped the card bounds {bounds:?}"
            );
        }
    }

    #[test]
    fn typing_into_a_focused_card_and_pressing_return_submits_the_reply() {
        let mut card = card(&message(true));
        assert_eq!(
            key(&mut card, Keysym::Return, None),
            None,
            "an unfocused card must not swallow Return"
        );

        card.focus_reply();
        type_text(&mut card, "on my way");
        assert_eq!(
            key(&mut card, Keysym::Return, None),
            Some(CardAction::SubmitReply)
        );
        assert_eq!(card.take_reply().as_deref(), Some("on my way"));
        assert!(
            !card.wants_keyboard(),
            "the card kept the keyboard after sending"
        );
    }

    #[test]
    fn escape_closes_the_field_without_sending_and_gives_the_keyboard_back() {
        let mut card = card(&message(true));
        card.focus_reply();
        type_text(&mut card, "never mind");

        assert_eq!(key(&mut card, Keysym::Escape, None), None);
        assert!(
            !card.wants_keyboard(),
            "escape did not release the keyboard"
        );
        assert!(
            card.needs_measure(),
            "collapsing the field did not invalidate the height"
        );
    }

    #[test]
    fn return_on_an_empty_field_cancels_rather_than_sending_nothing() {
        let mut card = card(&message(true));
        card.focus_reply();
        assert_eq!(key(&mut card, Keysym::Return, None), None);
        assert!(!card.wants_keyboard());
        assert_eq!(card.take_reply(), None);
    }

    #[test]
    fn losing_the_reply_action_in_a_replacement_closes_the_field() {
        let theme = Theme::light();
        let mut card = card(&message(true));
        card.focus_reply();
        assert!(card.wants_keyboard());

        card.sync(&message(false), &theme);
        assert!(
            !card.wants_keyboard(),
            "a replacement that dropped the reply action kept the keyboard"
        );
        assert!(card.needs_measure());
    }

    /// The field's rectangle, as [`paint`] leaves it on screen.
    fn field_rect(card: &crate::ui::card::Card, theme: &Theme) -> Rect {
        let metrics = &theme.metrics;
        let bounds = Rect::new(12.0, 20.0, 12.0 + metrics.card_width, 20.0 + card.height());
        Rect::new(
            bounds.x0 + metrics.pad_x + metrics.icon + metrics.icon_gap,
            bounds.y1 - metrics.pad_y - metrics.field_height,
            bounds.x1 - metrics.pad_x,
            bounds.y1 - metrics.pad_y,
        )
    }

    #[test]
    fn clicking_in_an_open_field_puts_the_caret_where_the_pointer_was() {
        let theme = Theme::light();
        let mut tcx = TextContext::new();
        let mut card = card(&message(true));
        card.focus_reply();
        type_text(&mut card, "hello");
        paint(&mut card, &mut tcx, &theme);

        let field = field_rect(&card, &theme);
        card.on_click(Point::new(field.x0 + 1.0, field.center().y), &mut tcx);
        type_text(&mut card, "X");
        assert_eq!(
            card.take_reply().as_deref(),
            Some("Xhello"),
            "the click did not reach the start of the field"
        );

        card.focus_reply();
        type_text(&mut card, "hello");
        paint(&mut card, &mut tcx, &theme);
        card.on_click(Point::new(field.x1 - 1.0, field.center().y), &mut tcx);
        type_text(&mut card, "X");
        assert_eq!(
            card.take_reply().as_deref(),
            Some("helloX"),
            "a click past the end of the text should land after it"
        );
    }

    #[test]
    fn a_click_that_missed_the_field_leaves_the_caret_alone() {
        let theme = Theme::light();
        let mut tcx = TextContext::new();
        let mut card = card(&message(true));
        card.focus_reply();
        type_text(&mut card, "hello");
        paint(&mut card, &mut tcx, &theme);

        let field = field_rect(&card, &theme);
        for point in [
            Point::new(field.x0 - 20.0, field.center().y),
            Point::new(field.center().x, field.y0 - 30.0),
        ] {
            card.on_click(point, &mut tcx);
        }
        type_text(&mut card, "X");
        assert_eq!(
            card.take_reply().as_deref(),
            Some("helloX"),
            "a click outside the field moved the caret"
        );
    }

    /// Clicking the collapsed chip is what *opens* the field; there is no text
    /// under it to put a caret in, and it must not panic on the way past.
    #[test]
    fn clicking_a_collapsed_chip_or_a_card_without_a_field_does_nothing() {
        let theme = Theme::light();
        let mut tcx = TextContext::new();

        let mut collapsed = card(&message(true));
        paint(&mut collapsed, &mut tcx, &theme);
        let field = field_rect(&collapsed, &theme);
        collapsed.on_click(field.center(), &mut tcx);
        assert!(!collapsed.wants_keyboard(), "a click focused the field");

        let mut plain = card(&crate::ui::card::tests::general("See you at five."));
        paint(&mut plain, &mut tcx, &theme);
        plain.on_click(field.center(), &mut tcx);
    }

    #[test]
    fn the_message_is_clamped_and_the_card_stops_growing_with_it() {
        let theme = Theme::light();
        let mut tcx = TextContext::new();
        let long = crate::ui::card::tests::LONG_BODY;

        let mut clamped = card(&entry(
            Notification::builder("Signal")
                .summary("Ada")
                .body(long)
                .content(Content::Chat(Chat {
                    conversation_id: "t".into(),
                    sender: "Ada".into(),
                    avatar: None,
                    sent_at: Local::now(),
                }))
                .build(),
        ));
        let mut longer = card(&entry(
            Notification::builder("Signal")
                .summary("Ada")
                .body(format!("{long} {long}"))
                .content(Content::Chat(Chat {
                    conversation_id: "t".into(),
                    sender: "Ada".into(),
                    avatar: None,
                    sent_at: Local::now(),
                }))
                .build(),
        ));

        assert_eq!(
            measure(&mut clamped, &mut tcx, &theme),
            measure(&mut longer, &mut tcx, &theme),
            "the {MESSAGE_LINES}-line message clamp did not hold"
        );
    }
}
