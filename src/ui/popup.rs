//! The toast strip: a full-height layer surface down the right-hand edge with
//! the live notifications animating inside it.
//!
//! # Why the surface never resizes
//!
//! Every size change is a commit the compositor has to configure back, and a
//! card that enters, re-flows or leaves changes the contents on almost every
//! frame of an animation. The surface is therefore anchored top-to-bottom at a
//! fixed width and the cards move *inside* it. The root takes no pointer input,
//! so the window's input region is the cards alone and the rest of the strip
//! clicks through to whatever is below it.

use crownui::prelude::{
    Anchor, AppConfig, KeyboardInteractivity, Layer, LayerExtent, LayerOptions, Size, Styled,
    TRANSPARENT, View, component, for_each, zstack,
};

use crate::{
    model::Notification,
    ui::{
        board::{BoardHandle, DeckHead, Side},
        env::Env,
        feed,
        head::head,
        tile::tile,
        widget::{measured, reaction},
        window::Window,
    },
};

/// Width of the strip. Wider than a card by two margins, so the card's shadow
/// has somewhere to fall.
pub const STRIP_WIDTH: u32 = 400;
/// The height asked for until the compositor says how tall the output is.
const FALLBACK_HEIGHT: u32 = 900;

/// The window the toast strip lives in.
pub fn config() -> AppConfig {
    Window::config("crownotify", STRIP_WIDTH, FALLBACK_HEIGHT).layer(
        LayerOptions::new("crownotify", Anchor::TOP | Anchor::RIGHT | Anchor::BOTTOM)
            .layer(Layer::Top)
            .width(LayerExtent::Fixed(STRIP_WIDTH)),
    )
}

pub fn popup(window: Window) -> impl View {
    component(move |cx| {
        let Window {
            theme,
            profiles,
            requests,
            snapshots,
        } = window;
        let env = Env::new(cx, theme, profiles, requests);
        let board = BoardHandle::new(cx, Side::Toasts, theme.metrics);
        let size = cx.signal(Size::ZERO);
        feed::follow(cx, snapshots, move |cx, snapshot| {
            board.show(cx, &snapshot.toasts)
        });
        feed::keep_time(cx, env);
        zstack((
            measured((), size).absolute().inset(0.0),
            reaction(
                move |runtime| size.get(runtime),
                move |cx, size| board.resize(cx, size),
            ),
            // Only an open reply field takes the keyboard: a strip that held it
            // would steal every key the user meant for the window below.
            reaction(
                move |runtime| env.reply.get(runtime).is_some(),
                |cx, replying| {
                    cx.set_keyboard_interactivity(if replying {
                        KeyboardInteractivity::OnDemand
                    } else {
                        KeyboardInteractivity::None
                    })
                },
            ),
            for_each(
                board.heads,
                |deck: &DeckHead| deck.key.clone(),
                move |_, deck| head(env, board, deck, Side::Toasts),
            )
            .absolute()
            .inset(0.0),
            for_each(
                board.cards,
                |card: &Notification| card.id,
                move |_, card| tile(env, board, card, Side::Toasts),
            )
            .absolute()
            .inset(0.0),
        ))
        .bg(TRANSPARENT)
        .pointer_events_none()
    })
}
