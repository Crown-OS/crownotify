//! The strip above a pulled-apart deck: the app's name, which puts the deck
//! back together, and its own Clear All.

use crownui::prelude::{
    CursorIcon, Listeners, Signal, Styled, View, bind, component, hstack, spacer, text,
};

use crate::{
    daemon::Request,
    ui::{
        board::{BoardHandle, DeckHead, Frame, Side},
        env::Env,
        widget::{ButtonStyle, pill, takes_input, tap},
    },
};

pub fn head(env: Env, board: BoardHandle, deck: Signal<DeckHead>, side: Side) -> impl View {
    component(move |cx| {
        let theme = env.theme;
        let DeckHead { key, name } = deck.get(cx);
        let frame = cx.signal(Frame::default());
        board.attach_head(cx, key.clone(), frame);
        let detached = key.clone();
        cx.on_cleanup(move |runtime| board.detach_head(runtime, &detached));
        let geometry =
            move |read: fn(&Frame) -> f32| bind(move |runtime| frame.with(runtime, read));
        let toggle_key = key.clone();
        let clear = match side {
            Side::Toasts => Request::DismissGroup(key),
            Side::History => Request::ClearGroup(key),
        };
        hstack((
            theme
                .text
                .eyebrow
                .apply(text(name.to_uppercase()))
                .truncate()
                .cursor(CursorIcon::Pointer)
                .on_click(tap(move |cx| board.toggle(cx, &toggle_key))),
            spacer(),
            pill("Clear All", theme, theme.palette.accent, |_| {
                ButtonStyle::Plain
            })
            .on_click(tap(move |cx| env.send(cx, clear.clone()))),
        ))
        .items_center()
        .absolute()
        .left(geometry(|frame| frame.rect.min_x()))
        .top(geometry(|frame| frame.rect.min_y()))
        .w(geometry(|frame| frame.rect.size.width))
        .h(theme.metrics.button)
        .opacity(geometry(|frame| frame.alpha))
        .hidden(bind(move |runtime| {
            !frame.with(runtime, |frame| frame.shown)
        }))
        .modifier(takes_input(move |runtime| {
            frame.with(runtime, |frame| frame.interactive)
        }))
    })
}
