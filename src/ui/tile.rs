//! One card on a board: the glass plate, the hover close button and the
//! gestures, around whichever card view the notification wears.

use crownui::prelude::{
    Listeners, Role, Semantic, Signal, Size, Styled, View, bind, component, hstack, show, zstack,
};

use crate::{
    daemon::Request,
    model::{CloseReason, Notification},
    ui::{
        action::CardAction,
        board::{BoardHandle, Frame, Side},
        card::card_content,
        env::Env,
        widget::{Symbol, measured, reaction, round_button, takes_input, tap},
    },
};

/// Inset of the hover-revealed close button from the card's top-right corner.
const CLOSE_INSET: f32 = 6.0;
/// Share of a pill's height the close button's circle takes.
const CLOSE_SCALE: f32 = 0.6;
/// Pointer events per second a drag delta is assumed to be apart, to turn the
/// last move of a swipe into a fling speed.
const DRAG_RATE: f32 = 60.0;

pub fn tile(
    env: Env,
    board: BoardHandle,
    notification: Signal<Notification>,
    side: Side,
) -> impl View {
    component(move |cx| {
        let theme = env.theme;
        let metrics = theme.metrics;
        let palette = theme.palette;
        let id = notification.with(cx, |n| n.id);
        let frame = cx.signal(Frame::default());
        board.attach(cx, id, frame);
        let hovered = cx.signal(false);
        let swiped = cx.signal(false);
        let size = cx.signal(Size::ZERO);
        let geometry =
            move |read: fn(&Frame) -> f32| bind(move |runtime| frame.with(runtime, read));

        let content = measured((card_content(env, notification, hovered),), size)
            .absolute()
            .top(0.0)
            .left(0.0)
            .w(metrics.card_width)
            .px(metrics.pad_x)
            .py(metrics.pad_y)
            .opacity(geometry(|frame| frame.content));
        let close = show(
            bind(move |runtime| {
                hovered.get(runtime) && frame.with(runtime, |frame| frame.content > 0.5)
            }),
            move || {
                round_button(
                    Symbol::Close,
                    palette.separator,
                    palette.secondary,
                    metrics.button * CLOSE_SCALE,
                    "Dismiss",
                )
                .absolute()
                .top(CLOSE_INSET)
                .right(CLOSE_INSET)
                .on_click(tap(move |cx| {
                    env.send(cx, Request::Perform(id, CardAction::Dismiss))
                }))
            },
            || hstack(()),
        );
        let shell = zstack((
            content,
            close,
            reaction(
                move |runtime| size.get(runtime).height,
                move |cx, height| board.measured(cx, id, height),
            ),
        ))
        .absolute()
        .left(geometry(|frame| frame.rect.min_x()))
        .top(geometry(|frame| frame.rect.min_y()))
        .w(geometry(|frame| frame.rect.size.width))
        .h(geometry(|frame| frame.rect.size.height))
        .opacity(geometry(|frame| frame.alpha))
        .hidden(bind(move |runtime| {
            !frame.with(runtime, |frame| frame.shown)
        }))
        .rounded(metrics.card_radius)
        .bg(palette.card)
        .border(1.0)
        .border_color(palette.card_rim)
        .overflow_hidden()
        .modifier(takes_input(move |runtime| {
            frame.with(runtime, |frame| frame.interactive)
        }))
        .role(Role::ListItem)
        .label(bind(move |runtime| {
            notification.with(runtime, |n| format!("{}: {}", n.app.name, n.summary))
        }))
        .on_pointer_enter(move |cx, _| hovered.set(cx, true))
        .on_pointer_leave(move |cx, _| hovered.set(cx, false))
        .on_click(move |cx, _| match board.collapsed(cx, id) {
            Some(key) => board.toggle(cx, &key),
            None => env.send(cx, Request::Perform(id, CardAction::Activate)),
        });
        match side {
            Side::History => shell,
            Side::Toasts => shell
                .material(theme.material())
                .on_drag(move |cx, event| {
                    if swiped.get(cx) {
                        return;
                    }
                    let drag = *event.detail();
                    if board.drag(cx, id, drag.total.x) {
                        swiped.set(cx, true);
                        board.fling(cx, id, drag.delta.x * DRAG_RATE);
                        env.send(cx, Request::Close(id, CloseReason::Dismissed));
                    }
                })
                .on_drag_end(move |cx, _| {
                    if !swiped.get(cx) {
                        board.release(cx, id);
                    }
                }),
        }
    })
}
