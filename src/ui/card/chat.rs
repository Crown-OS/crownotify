//! The chat card, and the inline reply that makes it worth having.
//!
//! Reply has two states and the card is a different height in each: collapsed
//! the affordance is a chip that only asks for the keyboard, expanded it is a
//! field plus a send button. Keyboard focus belongs to the open field: the
//! window raises its keyboard interactivity only while [`Env::reply`] names a
//! card, so a toast strip never steals keys meant for the window below.

use crownui::{
    ext::{Key, NamedKey},
    prelude::{
        Cx, Listeners, Runtime, Signal, Styled, View, bind, component, hstack, show, text,
        text_field, vstack,
    },
};

use crate::{
    daemon::Request,
    model::{ActionKind, Content, Icon, Notification, NotificationId},
    ui::{
        card::{CONTROL_GAP, CONTROL_SPACING, LINE_GAP, card_picture, general::stamp},
        env::Env,
        widget::{ButtonStyle, Symbol, autofocus, pill, round_button, tap},
    },
};

/// How many lines of the message the card shows before ellipsising. Shorter
/// than a general card's body on purpose: a chat card's job is to say who said
/// something and roughly what, and then get out of the way of the reply.
const MESSAGE_LINES: u32 = 3;
const REPLY_LABEL: &str = "Reply";
/// Horizontal padding inside the reply field.
const FIELD_PAD_X: f32 = 10.0;

pub fn chat(env: Env, notification: Signal<Notification>, hovered: Signal<bool>) -> impl View {
    let theme = env.theme;
    let metrics = theme.metrics;
    vstack((
        hstack((
            card_picture(env, notification, metrics.icon, metrics.icon / 2.0, avatar),
            vstack((
                hstack((
                    theme
                        .text
                        .summary
                        .apply(text(bind(move |runtime| {
                            notification.with(runtime, |n| sender(n).to_string())
                        })))
                        .truncate()
                        .grow(1.0)
                        .shrink(1.0),
                    stamp(env, notification, hovered),
                ))
                .gap(8.0)
                .items_baseline(),
                show(
                    bind(move |runtime| notification.with(runtime, |n| !n.body.is_empty())),
                    move || {
                        theme
                            .text
                            .body
                            .apply(text(bind(move |runtime| {
                                notification.with(runtime, |n| n.body.clone())
                            })))
                            .line_clamp(MESSAGE_LINES)
                    },
                    || hstack(()),
                ),
            ))
            .gap(LINE_GAP)
            .grow(1.0)
            .shrink(1.0)
            .min_w(0.0),
        ))
        .gap(metrics.icon_gap)
        .items_start(),
        show(
            bind(move |runtime| notification.with(runtime, |n| placeholder(n).is_some())),
            move || reply(env, notification),
            || hstack(()),
        ),
    ))
}

/// The chip, or the open field and its send button.
fn reply(env: Env, notification: Signal<Notification>) -> impl View {
    component(move |cx| {
        let theme = env.theme;
        let metrics = theme.metrics;
        let id = notification.with(cx, |n| n.id);
        let draft = cx.signal(String::new());
        let accent = env.accent(cx, notification);
        let hint = notification.with(cx, |n| placeholder(n).unwrap_or(REPLY_LABEL).to_string());
        let open = move |runtime: &mut Runtime| env.reply.get(runtime) == Some(id);
        let submit = move |cx: &mut Cx| send(cx, env, id, draft);
        show(
            bind(open),
            move || {
                hstack((
                    autofocus(
                        theme
                            .text
                            .field
                            .apply(text_field(draft, hint.clone()))
                            .on_submit(move |cx, _| submit(cx)),
                    )
                    .h(metrics.field_height)
                    .px(FIELD_PAD_X)
                    .grow(1.0)
                    .items_center()
                    .rounded(metrics.field_height / 2.0)
                    .bg(theme.palette.field)
                    .border(1.0)
                    .border_color(accent),
                    round_button(
                        Symbol::Send,
                        accent,
                        theme.palette.on_fill,
                        metrics.field_height,
                        "Send",
                    )
                    .on_click(tap(submit)),
                ))
                .gap(CONTROL_SPACING)
                .items_center()
                .on_key_down(move |cx, event| {
                    if event.detail().key == Key::Named(NamedKey::Escape) {
                        event.stop_propagation();
                        env.reply.set(cx, None);
                    }
                })
                .on_click(tap(|_| {}))
            },
            move || {
                hstack((pill(REPLY_LABEL, theme, accent, |_| ButtonStyle::Tinted)
                    .on_click(tap(move |cx| env.reply.set(cx, Some(id)))),))
            },
        )
        .mt(CONTROL_GAP)
    })
}

/// Sends the draft, unless it is blank: an empty reply is a mis-click, not a
/// message, and leaves the field open with whatever was typed.
fn send(cx: &mut Cx, env: Env, id: NotificationId, draft: Signal<String>) {
    let text = draft.get(cx);
    if text.trim().is_empty() {
        return;
    }
    env.send(cx, Request::Reply(id, text));
    draft.set(cx, String::new());
    env.reply.set(cx, None);
}

fn sender(notification: &Notification) -> &str {
    match &notification.content {
        Content::Chat(chat) if !chat.sender.is_empty() => &chat.sender,
        _ => &notification.summary,
    }
}

fn avatar(notification: &Notification) -> Option<&Icon> {
    match &notification.content {
        Content::Chat(chat) => chat.avatar.as_ref(),
        _ => None,
    }
    .or(notification.image.as_ref())
    .or(notification.app.icon.as_ref())
}

/// The reply field's placeholder, when the sender offered an inline reply.
pub fn placeholder(notification: &Notification) -> Option<&str> {
    notification
        .reply_action()
        .map(|action| match &action.kind {
            ActionKind::InlineReply { placeholder } if !placeholder.is_empty() => {
                placeholder.as_str()
            }
            _ => REPLY_LABEL,
        })
}
