//! The ordinary notification card: icon, text column, timestamp, actions.
//!
//! This is the shape every app that never heard of CrownOS gets, so it has to
//! survive whatever it is handed — an empty summary, a body of five thousand
//! characters, a dozen actions — without ever growing past the four lines of
//! body copy the design allows it. The clamp ellipsises rather than simply
//! cutting, because a body that stops mid-word with no mark reads as a bug in
//! the sender.

use crownui::prelude::{
    Listeners, Runtime, Signal, Size, Styled, View, bind, component, hstack, keyed, show, text,
    vstack,
};

use crate::{
    daemon::Request,
    model::{Action, ActionKind, Content, Notification, NotificationId},
    ui::{
        action::CardAction,
        card::{CONTROL_GAP, CONTROL_SPACING, LINE_GAP, card_picture, own_icon, relative_time},
        env::Env,
        widget::{
            ButtonStyle, Symbol, bar, measured, menu, percent_fraction, pill, round_button, tap,
        },
    },
};

/// How many lines the summary may occupy before it is ellipsised.
const SUMMARY_LINES: u32 = 2;
/// How many lines the body may occupy. The number the design fixes: past four
/// lines a toast stops being a glance and starts being a document.
const BODY_LINES: u32 = 4;
/// Gap above a progress bar.
const PROGRESS_GAP: f32 = 8.0;
/// Height of the band a progress bar is centred in.
const PROGRESS_BAND: f32 = 10.0;
/// Share of a round button the chevron's circle takes, against a pill's height.
const CHEVRON_SCALE: f32 = 0.8;

pub fn general(env: Env, notification: Signal<Notification>, hovered: Signal<bool>) -> impl View {
    let theme = env.theme;
    let metrics = theme.metrics;
    let read = move |runtime: &mut Runtime, field: fn(&Notification) -> String| {
        notification.with(runtime, field)
    };
    let column = vstack((
        show(
            bind(move |runtime| notification.with(runtime, |n| n.level.eyebrow().is_some())),
            move || {
                theme.text.eyebrow.apply(text(bind(move |runtime| {
                    read(runtime, |n| {
                        n.level.eyebrow().unwrap_or_default().to_string()
                    })
                })))
            },
            || hstack(()),
        ),
        hstack((
            theme
                .text
                .app
                .apply(text(bind(move |runtime| {
                    read(runtime, |n| n.app.name.clone())
                })))
                .truncate()
                .grow(1.0)
                .shrink(1.0),
            stamp(env, notification, hovered),
        ))
        .gap(8.0)
        .items_baseline(),
        show(
            bind(move |runtime| notification.with(runtime, |n| !n.summary.is_empty())),
            move || {
                theme
                    .text
                    .summary
                    .apply(text(bind(move |runtime| {
                        read(runtime, |n| n.summary.clone())
                    })))
                    .line_clamp(SUMMARY_LINES)
            },
            || hstack(()),
        ),
        show(
            bind(move |runtime| notification.with(runtime, |n| !body(n).is_empty())),
            move || {
                theme
                    .text
                    .body
                    .apply(text(bind(move |runtime| {
                        read(runtime, |n| body(n).to_string())
                    })))
                    .line_clamp(BODY_LINES)
            },
            || hstack(()),
        ),
        show(
            bind(move |runtime| notification.with(runtime, |n| progress(n).is_some())),
            move || {
                bar(
                    move |runtime| {
                        notification.with(runtime, |n| percent_fraction(progress(n).unwrap_or(0)))
                    },
                    theme.palette.separator,
                    theme.palette.accent,
                )
                .h(PROGRESS_BAND)
                .mt(PROGRESS_GAP)
            },
            || hstack(()),
        ),
    ))
    .gap(LINE_GAP)
    .grow(1.0)
    .shrink(1.0)
    .min_w(0.0);

    vstack((
        hstack((
            card_picture(
                env,
                notification,
                metrics.icon,
                metrics.icon_radius,
                own_icon,
            ),
            column,
        ))
        .gap(metrics.icon_gap)
        .items_start(),
        keyed(
            move |runtime| {
                notification.with(runtime, |n| (n.id, buttons(n).cloned().collect::<Vec<_>>()))
            },
            move |_, (id, actions)| actions_row(env, id, actions),
        ),
    ))
}

/// The relative timestamp in the card's top-right corner. Hidden while the
/// pointer is over the card, because the close affordance takes that corner.
pub fn stamp(env: Env, notification: Signal<Notification>, hovered: Signal<bool>) -> impl View {
    env.theme
        .text
        .meta
        .apply(text(bind(move |runtime| {
            let now = env.now.get(runtime);
            let posted = notification.with(runtime, |n| n.posted);
            relative_time(now, posted)
        })))
        .shrink(0.0)
        .opacity(bind(
            move |runtime| if hovered.get(runtime) { 0.0 } else { 1.0 },
        ))
}

/// The body copy, or a progress notification's label when it sent no body.
fn body(notification: &Notification) -> &str {
    match &notification.content {
        Content::Progress(progress) if notification.body.is_empty() => &progress.label,
        _ => &notification.body,
    }
}

fn progress(notification: &Notification) -> Option<u8> {
    match &notification.content {
        Content::Progress(progress) => Some(progress.percent),
        _ => None,
    }
}

fn buttons(notification: &Notification) -> impl Iterator<Item = &Action> {
    notification
        .actions
        .iter()
        .filter(|action| action.kind == ActionKind::Button)
}

/// How many of `widths` fit side by side in `available`, leaving room for the
/// chevron when some do not.
pub fn fitting(widths: &[f32], available: f32, chevron: f32) -> usize {
    let all: f32 =
        widths.iter().sum::<f32>() + CONTROL_SPACING * widths.len().saturating_sub(1) as f32;
    if all <= available + 0.01 {
        return widths.len();
    }
    let budget = available - chevron - CONTROL_SPACING;
    let mut used = 0.0;
    for (index, width) in widths.iter().enumerate() {
        let needed = if index == 0 {
            *width
        } else {
            used + CONTROL_SPACING + width
        };
        if needed > budget + 0.01 {
            return index;
        }
        used = needed;
    }
    widths.len()
}

/// The action pills along the bottom of the card, aligned with the text column,
/// with whatever does not fit behind a chevron.
fn actions_row(env: Env, id: NotificationId, actions: Vec<Action>) -> impl View {
    component(move |cx| {
        if actions.is_empty() {
            return hstack(()).boxed();
        }
        let theme = env.theme;
        let metrics = theme.metrics;
        let available = metrics.card_width - 2.0 * metrics.pad_x - metrics.icon - metrics.icon_gap;
        let chevron = metrics.button * CHEVRON_SCALE;
        let accent = theme.palette.accent;
        let widths: Vec<Signal<Size>> = actions.iter().map(|_| cx.signal(Size::ZERO)).collect();
        let open = cx.signal(false);
        let observed = widths.clone();
        let shown = cx.memo(move |runtime| {
            let measured: Vec<f32> = observed
                .iter()
                .map(|size| size.get(runtime).width)
                .collect();
            fitting(&measured, available, chevron)
        });
        let invoke = move |key: String| {
            tap(move |cx| env.send(cx, Request::Perform(id, CardAction::Invoke(key.clone()))))
        };
        // Every pill is laid out once off to the side, so the row knows how
        // wide each is before it decides how many it has room for.
        let probe = hstack(
            actions
                .iter()
                .zip(&widths)
                .map(|(action, width)| {
                    measured(
                        pill(action.label.clone(), theme, accent, |_| ButtonStyle::Plain),
                        *width,
                    )
                })
                .collect::<Vec<_>>(),
        )
        .absolute()
        .hidden(true)
        .pointer_events_none();
        let pills = actions
            .iter()
            .enumerate()
            .map(|(index, action)| {
                let (label, key) = (action.label.clone(), action.key.clone());
                show(
                    bind(move |runtime| index < shown.get(runtime)),
                    move || {
                        pill(label.clone(), theme, accent, |_| ButtonStyle::Plain)
                            .on_click(invoke(key.clone()))
                    },
                    || hstack(()),
                )
            })
            .collect::<Vec<_>>();
        let total = actions.len();
        let spills = move |runtime: &mut Runtime| shown.get(runtime) < total;
        let overflow = actions.clone();
        vstack((
            hstack((
                hstack(pills).gap(CONTROL_SPACING).grow(1.0),
                // Always mounted, and only as wide as it needs to be: the row
                // learns whether anything spills once the pills are measured.
                round_button(
                    Symbol::ChevronDown,
                    theme.palette.field,
                    theme.palette.secondary,
                    chevron,
                    "More actions",
                )
                .w(bind(
                    move |runtime| if spills(runtime) { chevron } else { 0.0 },
                ))
                .hidden(bind(move |runtime| !spills(runtime)))
                .on_click(tap(move |cx| open.update(cx, |open| *open = !*open))),
                probe,
            ))
            .items_center()
            .h(metrics.button),
            keyed(
                move |runtime| (open.get(runtime), shown.get(runtime)),
                move |_, (open, shown)| {
                    let spilled = overflow[shown.min(overflow.len())..].to_vec();
                    show(
                        open && !spilled.is_empty(),
                        move || menu(env, id, spilled.clone()).mt(CONTROL_SPACING),
                        || hstack(()),
                    )
                },
            ),
        ))
        .mt(CONTROL_GAP)
        .pl(metrics.icon + metrics.icon_gap)
        .boxed()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn actions_that_fit_all_get_a_pill_and_no_chevron() {
        assert_eq!(fitting(&[60.0, 60.0], 200.0, 26.0), 2);
        assert_eq!(fitting(&[], 200.0, 26.0), 0);
    }

    #[test]
    fn the_actions_that_do_not_fit_leave_room_for_the_chevron() {
        // 3 × 80 + 2 gaps is 256, past 200; with the chevron's 26 + 8 reserved
        // only two pills fit in the 166 left.
        assert_eq!(fitting(&[80.0, 80.0, 80.0], 200.0, 26.0), 1);
        assert_eq!(fitting(&[70.0, 70.0, 70.0], 200.0, 26.0), 2);
    }

    #[test]
    fn a_single_action_too_wide_for_the_row_goes_behind_the_chevron() {
        assert_eq!(fitting(&[400.0], 200.0, 26.0), 0);
    }
}
