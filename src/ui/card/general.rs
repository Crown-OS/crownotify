//! The ordinary notification card: icon, text column, timestamp, actions.
//!
//! This is the shape every app that never heard of CrownOS gets, so it has to
//! survive whatever it is handed — an empty summary, a body of five thousand
//! characters, a dozen actions — without ever growing past the four lines of
//! body copy the design allows it. The clamp is [`Text::set_max_lines`], which
//! ellipsises rather than simply cutting, because a body that stops mid-word
//! with no mark reads as a bug in the sender.

use chrono::{DateTime, Local};
use crownshell::{Text, TextContext, kurbo::Rect};

use crate::{
    model::{
        Action, ActionKind, Content, DeviceKind, Icon, InterruptionLevel, Notification,
        NotificationId,
    },
    profile::ContentHint,
    ui::{
        card::{
            CONTROL_GAP, CONTROL_SPACING, LINE_GAP, MIN_COLUMN, MeasureCtx, RenderCtx, Row,
            STAMP_GAP, column_width, content_rect, draw_icon, set_text, view::CardView,
            write_relative_time,
        },
        hit::CardAction,
        theme::Theme,
        widget::{
            ButtonState, ButtonStyle, IconButton, Menu, PillButton, chevron_state, draw_bar,
            percent_fraction, sync_style,
        },
    },
};

/// How many lines the summary may occupy before it is ellipsised.
const SUMMARY_LINES: usize = 2;
/// How many lines the body may occupy. The number the design fixes: past four
/// lines a toast stops being a glance and starts being a document.
const BODY_LINES: usize = 4;
/// Gap above a progress bar.
const PROGRESS_GAP: f64 = 8.0;
/// Height of the band a progress bar is centred in. Taller than the track so
/// the band is also a comfortable pointer target.
const PROGRESS_BAND: f64 = 10.0;

/// The general card's retained state.
pub struct GeneralView {
    icon: Option<Icon>,
    device: Option<DeviceKind>,
    eyebrow: Text,
    app: Text,
    summary: Text,
    body: Text,
    stamp: Text,
    /// Buffer the relative timestamp is rendered into, kept so that a tick
    /// which does not change the timestamp costs no allocation at all.
    stamp_buffer: String,
    posted: DateTime<Local>,
    level: InterruptionLevel,
    /// Completion of a [`Content::Progress`] notification, if it is one.
    progress: Option<u8>,
    /// Action key paired with the button that invokes it. The key is kept
    /// separate because it is what goes back over the wire, while the button
    /// only ever knows its label.
    actions: Vec<(String, PillButton)>,
    /// The actions that did not fit, and the control that reveals them.
    menu: Menu,
    chevron: IconButton,
}

impl GeneralView {
    pub fn new(notification: &Notification) -> Self {
        let mut view = Self {
            icon: None,
            device: None,
            eyebrow: Text::new(""),
            app: Text::new(""),
            summary: Text::new(""),
            body: Text::new(""),
            stamp: Text::new(""),
            stamp_buffer: String::new(),
            posted: notification.posted,
            level: InterruptionLevel::default(),
            progress: None,
            actions: Vec::new(),
            menu: Menu::new(),
            chevron: IconButton::chevron(&Theme::light()),
        };
        view.sync(notification);
        view
    }

    /// Rebuilds the action buttons when the notification's actions differ.
    fn sync_actions(&mut self, notification: &Notification) -> bool {
        let wanted = || {
            notification
                .actions
                .iter()
                .filter(|action| action.kind == ActionKind::Button)
        };
        let unchanged = wanted().count() == self.actions.len()
            && wanted().zip(&self.actions).all(|(action, (key, button))| {
                action.key == *key && action.label == button.label()
            });
        if unchanged {
            return false;
        }
        self.actions = wanted().map(button_for).collect();
        true
    }

    /// Lays the action row out along the bottom of the card, left to right.
    fn render_actions(&mut self, id: NotificationId, ctx: &mut RenderCtx<'_>, left: f64) {
        if self.actions.is_empty() {
            return;
        }
        let metrics = ctx.theme.metrics;
        let right = ctx.bounds.x1 - metrics.pad_x;
        let top = ctx.bounds.y1 - metrics.pad_y - metrics.button;
        let mut x = left;

        // A sender can attach more actions than the card is wide. The ones
        // that fit get a pill; the rest go behind the chevron, because dropping
        // them silently is how an app's "Snooze" stops existing.
        let chevron = metrics.button;
        let overflow = self.menu.len();
        let budget = if overflow > 0 {
            right - chevron - CONTROL_SPACING
        } else {
            right
        };

        let shown = self.actions.len() - overflow;
        for (key, button) in self.actions.iter_mut().take(shown) {
            let width = button.measure(ctx.text, ctx.theme).width;
            if x + width > budget + 0.01 {
                break;
            }
            let bounds = Rect::new(x, top, x + width, top + metrics.button);
            let action = CardAction::Invoke(key.clone());
            let state = ctx.state(&action);
            let theme = ctx.theme;
            button.render(ctx.scene, bounds, theme, state, 1.0, ctx.text);
            ctx.hits.push(bounds, id, action);
            x = bounds.x1 + CONTROL_SPACING;
        }

        if overflow == 0 {
            return;
        }
        let chevron_rect = Rect::new(right - chevron, top, right, top + chevron);
        let state = chevron_state(
            self.menu.is_open(),
            ctx.state(&CardAction::ToggleMenu) != ButtonState::Normal,
        );
        self.chevron
            .render(ctx.scene, chevron_rect.center(), state, 1.0);
        ctx.hits.push(chevron_rect, id, CardAction::ToggleMenu);

        if !self.menu.is_open() {
            return;
        }
        let width = self.menu.width(ctx.text, ctx.theme).max(chevron * 4.0);
        let panel = Rect::new(
            right - width,
            chevron_rect.y1 + CONTROL_SPACING,
            right,
            chevron_rect.y1 + CONTROL_SPACING + self.menu.height(),
        );
        let hovered = match ctx.hovered.clone() {
            Some(CardAction::Invoke(key)) => Some(key),
            _ => None,
        };
        let theme = ctx.theme;
        let rows = self
            .menu
            .render(ctx.scene, panel, theme, hovered.as_deref(), 1.0, ctx.text);
        for (row, key) in rows {
            ctx.hits.push(row, id, CardAction::Invoke(key));
        }
    }

    /// Puts the actions that do not fit into the menu.
    fn sync_menu(&mut self, width: f64, tcx: &mut TextContext, theme: &Theme) {
        let available = self.action_row_width(width, theme);
        let overflow = self.overflow(tcx, theme, available);
        let shown = self.actions.len() - overflow;
        let spilled: Vec<_> = self.actions[shown..]
            .iter()
            .map(|(key, button)| (key.clone(), button.label().to_string()))
            .collect();
        self.menu.sync(
            spilled
                .iter()
                .map(|(key, label)| (key.as_str(), label.as_str())),
        );
    }

    /// How much room the action row has, chevron included when there is one.
    fn action_row_width(&self, width: f64, theme: &Theme) -> f64 {
        width - 2.0 * theme.metrics.pad_x
    }

    /// How many trailing actions do not fit in `available`.
    fn overflow(&mut self, tcx: &mut TextContext, theme: &Theme, available: f64) -> usize {
        let mut used = 0.0;
        for (index, (_, button)) in self.actions.iter_mut().enumerate() {
            let width = button.measure(tcx, theme).width;
            let needed = if index == 0 {
                width
            } else {
                used + CONTROL_SPACING + width
            };
            if needed > available + 0.01 {
                return self.actions.len() - index;
            }
            used = needed;
        }
        0
    }

    fn actions_height(&self, theme: &Theme) -> f64 {
        if self.actions.is_empty() {
            return 0.0;
        }
        // An open menu makes the card taller, which re-flows the stack — and
        // that is already a spring, so everything below it slides.
        let menu = if self.menu.is_open() {
            CONTROL_SPACING + self.menu.height()
        } else {
            0.0
        };
        CONTROL_GAP + theme.metrics.button + menu
    }

    /// Sets the wrap widths and works out where every row of the text column
    /// goes. Called by both `measure` and `render`, which is what keeps them
    /// agreeing; the `Text`s cache their layouts, so the second call is
    /// arithmetic.
    fn rows(&mut self, column: f64, tcx: &mut TextContext) -> Column {
        // The first row shares its line with the timestamp, so it gets a
        // narrower wrap width. Kept fixed regardless of whether the timestamp
        // is currently drawn: making it depend on hover would re-lay out the
        // app name every time the pointer crossed the card.
        let head = (column - self.stamp.width(tcx) - STAMP_GAP).max(MIN_COLUMN);
        self.eyebrow.set_max_width(Some(head));
        self.eyebrow.set_max_lines(Some(1));
        self.app.set_max_width(Some(head));
        self.app.set_max_lines(Some(1));
        self.summary.set_max_width(Some(column));
        self.summary.set_max_lines(Some(SUMMARY_LINES));
        self.body.set_max_width(Some(column));
        self.body.set_max_lines(Some(BODY_LINES));

        let mut y = 0.0;
        let eyebrow = if self.level.eyebrow().is_some() {
            let row = Row {
                top: y,
                height: self.eyebrow.height(tcx),
            };
            y += row.height + LINE_GAP;
            row
        } else {
            Row::default()
        };

        let app = Row {
            top: y,
            height: self.app.height(tcx),
        };
        y += app.height;

        // An empty summary or body must take no room at all, not the height of
        // an empty line, or a bodyless notification would sit lopsided against
        // its own icon.
        let mut stacked = [Row::default(); 2];
        for (row, text) in stacked
            .iter_mut()
            .zip([&mut self.summary, &mut self.body])
            .filter(|(_, text)| !text.text().is_empty())
        {
            y += LINE_GAP;
            *row = Row {
                top: y,
                height: text.height(tcx),
            };
            y += row.height;
        }
        let [summary, body] = stacked;

        let progress = if self.progress.is_some() {
            y += PROGRESS_GAP;
            let row = Row {
                top: y,
                height: PROGRESS_BAND,
            };
            y += row.height;
            row
        } else {
            Row::default()
        };

        Column {
            eyebrow,
            app,
            summary,
            body,
            progress,
            height: y,
        }
    }
}

impl CardView for GeneralView {
    fn toggle_menu(&mut self) -> bool {
        if self.menu.is_empty() {
            return false;
        }
        self.menu.toggle();
        true
    }

    fn kind(&self) -> ContentHint {
        ContentHint::General
    }

    /// Re-reads `notification`, reporting whether the card's height could have
    /// moved as a result.
    fn sync(&mut self, notification: &Notification) -> bool {
        let mut changed = set_text(&mut self.app, &notification.app.name);
        changed |= set_text(&mut self.summary, &notification.summary);

        let (body, progress) = match &notification.content {
            // A progress notification's label stands in for a body it usually
            // does not have; showing neither would leave a bare bar.
            Content::Progress(progress) if notification.body.is_empty() => {
                (progress.label.as_str(), Some(progress.percent))
            }
            Content::Progress(progress) => (notification.body.as_str(), Some(progress.percent)),
            _ => (notification.body.as_str(), None),
        };
        changed |= set_text(&mut self.body, body);
        // Only the bar's presence changes the height; its position does not.
        changed |= self.progress.is_some() != progress.is_some();
        self.progress = progress;

        changed |= notification.level != self.level;
        self.level = notification.level;
        changed |= set_text(&mut self.eyebrow, self.level.eyebrow().unwrap_or_default());

        let icon = notification
            .image
            .as_ref()
            .or(notification.app.icon.as_ref());
        if self.icon.as_ref() != icon {
            self.icon = icon.cloned();
        }
        self.device = notification.relayed_from.as_ref().map(|device| device.kind);
        self.posted = notification.posted;
        changed | self.sync_actions(notification)
    }
    fn apply_theme(&mut self, theme: &Theme) {
        sync_style(&mut self.eyebrow, &theme.text.eyebrow);
        sync_style(&mut self.app, &theme.text.app);
        sync_style(&mut self.summary, &theme.text.summary);
        sync_style(&mut self.body, &theme.text.body);
        sync_style(&mut self.stamp, &theme.text.meta);
    }
    fn refresh_time(&mut self, now: DateTime<Local>) {
        write_relative_time(&mut self.stamp_buffer, now, self.posted);
        self.stamp.set_text(&self.stamp_buffer);
    }
    fn measure(&mut self, width: f64, ctx: &mut MeasureCtx<'_>) -> f64 {
        let metrics = &ctx.theme.metrics;
        let column = column_width(width, ctx.theme, metrics.icon);
        let rows = self.rows(column, ctx.text);
        // Which actions overflow depends on how wide they shape up, so the menu
        // can only be filled where there is a `TextContext` — and it has to be
        // filled before the height that includes it is worked out.
        self.sync_menu(width, ctx.text, ctx.theme);
        2.0 * metrics.pad_y + rows.height.max(metrics.icon) + self.actions_height(ctx.theme)
    }
    fn render(&mut self, id: NotificationId, ctx: &mut RenderCtx<'_>) {
        let metrics = ctx.theme.metrics;
        let inner = content_rect(ctx.bounds, ctx.theme);

        let icon_rect = Rect::new(
            inner.x0,
            inner.y0,
            inner.x0 + metrics.icon,
            inner.y0 + metrics.icon,
        );
        draw_icon(
            ctx,
            self.icon.as_ref(),
            icon_rect,
            metrics.icon_radius,
            self.device,
        );

        let left = icon_rect.x1 + metrics.icon_gap;
        let column = (inner.x1 - left).max(MIN_COLUMN);
        let rows = self.rows(column, ctx.text);
        let top = inner.y0;

        // Hidden while the pointer is over the card, because the close
        // affordance takes the same corner. Losing a timestamp on hover is far
        // less confusing than two things sitting on top of each other.
        if !ctx.is_hovered() {
            let stamp = self.stamp.width(ctx.text);
            self.stamp.draw(
                ctx.text,
                ctx.scene,
                ((inner.x1 - stamp).round(), top.round()),
            );
        }

        for (row, text) in [
            (rows.eyebrow, &mut self.eyebrow),
            (rows.app, &mut self.app),
            (rows.summary, &mut self.summary),
            (rows.body, &mut self.body),
        ] {
            if row.shown() {
                text.draw(ctx.text, ctx.scene, (left.round(), (top + row.top).round()));
            }
        }

        if let Some(percent) = self.progress.filter(|_| rows.progress.shown()) {
            let band = Rect::new(
                left,
                top + rows.progress.top,
                inner.x1,
                top + rows.progress.top + rows.progress.height,
            );
            let (track, fill) = (ctx.theme.palette.separator, ctx.accent());
            draw_bar(ctx.scene, band, percent_fraction(percent), track, fill, 1.0);
        }

        self.render_actions(id, ctx, left);
    }
}

/// Where each row of the text column sits, top to bottom.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct Column {
    eyebrow: Row,
    app: Row,
    summary: Row,
    body: Row,
    progress: Row,
    /// Total height of the column.
    height: f64,
}

fn button_for(action: &Action) -> (String, PillButton) {
    (
        action.key.clone(),
        PillButton::new(action.label.clone(), ButtonStyle::Plain),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        model::Urgency,
        ui::card::tests::{LONG_BODY, card, entry, general, measure, paint},
    };
    use crownshell::TextContext;

    #[test]
    fn the_body_is_clamped_to_four_lines_however_long_it_is() {
        let theme = Theme::light();
        let mut tcx = TextContext::new();
        let mut view = GeneralView::new(&general(LONG_BODY).notification);
        view.apply_theme(&theme);
        view.refresh_time(Local::now());

        let column = column_width(theme.metrics.card_width, &theme, theme.metrics.icon);
        view.rows(column, &mut tcx);
        assert_eq!(
            view.body.line_count(&mut tcx),
            BODY_LINES,
            "the body was not clamped to {BODY_LINES} lines"
        );
        assert_eq!(view.body.max_width(), Some(column), "the body did not wrap");
    }

    #[test]
    fn an_empty_body_costs_the_card_no_height_at_all() {
        let theme = Theme::light();
        let mut tcx = TextContext::new();

        let mut empty = card(&general(""));
        let mut short = card(&general("Hi"));
        assert!(measure(&mut short, &mut tcx, &theme) >= measure(&mut empty, &mut tcx, &theme));

        // With no body, the icon is what sets the card's height.
        let height = measure(&mut empty, &mut tcx, &theme);
        assert!(
            height >= 2.0 * theme.metrics.pad_y + theme.metrics.icon,
            "a bodyless card {height} is shorter than its own icon column"
        );
    }

    #[test]
    fn the_eyebrow_names_the_level_that_earned_it() {
        let theme = Theme::light();
        let mut tcx = TextContext::new();

        for (level, expected) in [
            (InterruptionLevel::Passive, None),
            (InterruptionLevel::Active, None),
            (InterruptionLevel::TimeSensitive, Some("TIME SENSITIVE")),
            (InterruptionLevel::Critical, Some("CRITICAL")),
        ] {
            let posted = entry(
                Notification::builder("Mail")
                    .summary("Ada")
                    .body("Body")
                    .level(level)
                    .build(),
            );
            let mut view = GeneralView::new(&posted.notification);
            view.apply_theme(&theme);
            assert_eq!(
                view.rows(280.0, &mut tcx).eyebrow.shown(),
                expected.is_some(),
                "{level:?}"
            );
            if let Some(expected) = expected {
                assert_eq!(view.eyebrow.text(), expected, "{level:?}");
            }
        }
    }

    #[test]
    fn a_critical_urgency_still_earns_its_eyebrow_without_a_level_hint() {
        let theme = Theme::light();
        let mut tcx = TextContext::new();
        let critical = entry(
            Notification::builder("Mail")
                .summary("Ada")
                .urgency(Urgency::Critical)
                .build(),
        );
        let mut view = GeneralView::new(&critical.notification);
        view.apply_theme(&theme);
        assert!(view.rows(280.0, &mut tcx).eyebrow.shown());
    }

    #[test]
    fn every_button_action_gets_a_pill_and_a_hit_region_carrying_its_key() {
        let theme = Theme::light();
        let mut tcx = TextContext::new();
        let mut card = card(&entry(
            Notification::builder("Mail")
                .id(11)
                .summary("Ada")
                .body("Ready when you are.")
                .action(Action {
                    key: "default".into(),
                    label: "Open".into(),
                    kind: ActionKind::Default,
                })
                .action(Action {
                    key: "archive".into(),
                    label: "Archive".into(),
                    kind: ActionKind::Button,
                })
                .action(Action {
                    key: "trash".into(),
                    label: "Delete".into(),
                    kind: ActionKind::Button,
                })
                .build(),
        ));

        let hits = paint(&mut card, &mut tcx, &theme);
        // The two button actions are the only ones that get a pill; the
        // default action is the card body itself, and the pills are pushed
        // after it, so a probe at a pill's centre resolves to the pill.
        let rects: Vec<_> = hits.rects().collect();
        let mut keys: Vec<String> = rects
            .iter()
            .filter_map(
                |rect| match hits.hit(rect.center()).map(|hit| &hit.action) {
                    Some(CardAction::Invoke(key)) => Some(key.clone()),
                    _ => None,
                },
            )
            .collect();
        keys.sort();
        keys.dedup();
        assert_eq!(keys, vec!["archive".to_string(), "trash".to_string()]);
    }

    #[test]
    fn a_progress_notification_draws_a_bar_and_borrows_its_label_as_a_body() {
        let theme = Theme::light();
        let mut tcx = TextContext::new();

        let with_bar = entry(
            Notification::builder("Files")
                .summary("Copying")
                .content(Content::Progress(crate::model::Progress {
                    label: "12 of 40 files".into(),
                    percent: 30,
                }))
                .build(),
        );
        let mut view = GeneralView::new(&with_bar.notification);
        view.apply_theme(&theme);
        assert_eq!(view.body.text(), "12 of 40 files");
        assert!(view.rows(280.0, &mut tcx).progress.shown());

        let mut bar = card(&with_bar);
        let mut plain = card(&entry(
            Notification::builder("Files")
                .summary("Copying")
                .body("12 of 40 files")
                .build(),
        ));
        assert!(
            measure(&mut bar, &mut tcx, &theme) > measure(&mut plain, &mut tcx, &theme),
            "the progress bar took no room"
        );
    }

    /// Re-syncing with the same notification must not throw the shaped text
    /// away; that is the whole reason cards are retained.
    #[test]
    fn syncing_identical_content_reports_no_change() {
        let source = general("See you at five.");
        let mut view = GeneralView::new(&source.notification);
        assert!(
            !view.sync(&source.notification),
            "an identical sync claimed the card changed"
        );
        assert!(view.sync(&general("Something else").notification));
    }

    #[test]
    fn changing_only_the_actions_still_invalidates_the_height() {
        let source = general("Body");
        let mut view = GeneralView::new(&source.notification);
        let with_action = entry(
            Notification::builder("Mail")
                .summary("Ada Lovelace")
                .body("Body")
                .action(Action {
                    key: "archive".into(),
                    label: "Archive".into(),
                    kind: ActionKind::Button,
                })
                .build(),
        );
        assert!(view.sync(&with_action.notification));
        assert_eq!(view.actions.len(), 1);
        assert!(!view.sync(&with_action.notification));
    }
}
