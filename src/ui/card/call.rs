//! The incoming-call card: avatar, contact, and the two buttons that matter.
//!
//! This is the one card where a mis-click is expensive, so the answer and
//! decline buttons are large, saturated, unmistakably coloured discs well clear
//! of each other. The card is a remote control for a phone that crownconnect is
//! relaying, so everything it draws comes out of the
//! [`Call`](crate::model::Call) payload, and it degrades to the notification's
//! own summary and body when a profile forced it here without one.

use crownui::prelude::{Listeners, Signal, Styled, View, bind, hstack, text, vstack};

use crate::{
    daemon::Request,
    model::{Content, Icon, Notification},
    ui::{
        action::CardAction,
        card::{CONTROL_SPACING, LINE_GAP, card_picture},
        env::Env,
        widget::{Symbol, round_button, tap},
    },
};

/// How much bigger the contact avatar is than an app icon. A call is the
/// loudest card in the stack and the face on it is the whole point.
const AVATAR_SCALE: f32 = 1.15;

pub fn call(env: Env, notification: Signal<Notification>) -> impl View {
    let theme = env.theme;
    let metrics = theme.metrics;
    let avatar = metrics.icon * AVATAR_SCALE;
    let perform = move |action: CardAction| {
        tap(move |cx| {
            let id = notification.with(cx, |n| n.id);
            env.send(cx, Request::Perform(id, action.clone()));
        })
    };
    hstack((
        card_picture(env, notification, avatar, avatar / 2.0, picture),
        vstack((
            theme
                .text
                .summary
                .apply(text(bind(move |runtime| {
                    notification.with(runtime, |n| contact(n).to_string())
                })))
                .truncate(),
            theme
                .text
                .body
                .apply(text(bind(move |runtime| {
                    notification.with(runtime, |n| number(n).to_string())
                })))
                .truncate(),
        ))
        .gap(LINE_GAP)
        .grow(1.0)
        .shrink(1.0)
        .min_w(0.0)
        .ml(metrics.icon_gap - CONTROL_SPACING),
        round_button(
            Symbol::PhoneDown,
            theme.palette.negative,
            theme.palette.on_fill,
            metrics.button,
            "Decline",
        )
        .on_click(perform(CardAction::Decline)),
        round_button(
            Symbol::Phone,
            theme.palette.positive,
            theme.palette.on_fill,
            metrics.button,
            "Answer",
        )
        .on_click(perform(CardAction::Answer)),
    ))
    .gap(CONTROL_SPACING)
    .items_center()
    .min_h(avatar.max(metrics.button))
}

fn contact(notification: &Notification) -> &str {
    match &notification.content {
        Content::Call(call) => &call.contact,
        _ => &notification.summary,
    }
}

fn number(notification: &Notification) -> &str {
    match &notification.content {
        Content::Call(call) => &call.number,
        _ => &notification.body,
    }
}

fn picture(notification: &Notification) -> Option<&Icon> {
    match &notification.content {
        Content::Call(call) => call.avatar.as_ref(),
        _ => None,
    }
    .or(notification.image.as_ref())
    .or(notification.app.icon.as_ref())
}
