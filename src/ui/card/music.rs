//! The now-playing card: a remote control for the phone's media session.
//!
//! The card holds the transport state the phone last reported rather than
//! deriving it, and every button sends a command back through crownconnect;
//! the card stays up after a press, because the whole point of a remote is to
//! be pressed again.

use crownui::prelude::{
    Color, Listeners, Runtime, Signal, Size, Styled, View, bind, component, hstack, keyed, text,
    vstack,
};

use crate::{
    daemon::Request,
    model::{Content, Icon, Notification},
    ui::{
        action::{CardAction, MediaAction},
        card::{CONTROL_GAP, CONTROL_SPACING, LINE_GAP, card_picture},
        env::Env,
        widget::{Symbol, measured, percent_fraction, round_button, scrubber, seek_percent, tap},
    },
};

/// How much bigger the album art is than an app icon. Cover art is the card's
/// only real ornament, so it earns the extra pixels the transport row costs.
const ART_SCALE: f32 = 1.3;
/// Corner radius of the album art, as a fraction of its side. Softer than an
/// app icon's, because a square photograph with sharp corners reads as an
/// un-styled image rather than as part of the card.
const ART_RADIUS_RATIO: f32 = 0.16;
/// Gap above the scrubber.
const SCRUB_GAP: f32 = 10.0;
/// Height of the band the scrubber's track is centred in. The band, not the
/// track, is the pointer target — a 4 px seek target is unusable.
const SCRUB_BAND: f32 = 14.0;

pub fn music(env: Env, notification: Signal<Notification>) -> impl View {
    component(move |cx| {
        let theme = env.theme;
        let metrics = theme.metrics;
        let art = metrics.icon * ART_SCALE;
        let band = cx.signal(Size::ZERO);
        let seeking = cx.signal(false);
        let media = move |action: MediaAction| {
            tap(move |cx| {
                let id = notification.with(cx, |n| n.id);
                env.send(cx, Request::Perform(id, CardAction::Media(action)));
            })
        };
        let state = move |runtime: &mut Runtime| {
            notification.with(runtime, |n| match &n.content {
                Content::Music(music) => (music.playing, music.percent),
                _ => (false, 0),
            })
        };
        let accent = env.accent(cx, notification);
        let neutral = (theme.palette.field, theme.palette.primary);
        let transport = move |symbol: Symbol,
                              action: MediaAction,
                              (fill, glyph): (Color, Color),
                              label: &'static str| {
            round_button(symbol, fill, glyph, metrics.button, label).on_click(media(action))
        };
        vstack((
            hstack((
                card_picture(env, notification, art, art * ART_RADIUS_RATIO, picture),
                vstack((
                    theme
                        .text
                        .summary
                        .apply(text(bind(move |runtime| {
                            notification.with(runtime, |n| title(n).to_string())
                        })))
                        .truncate(),
                    theme
                        .text
                        .body
                        .apply(text(bind(move |runtime| {
                            notification.with(runtime, |n| artist(n).to_string())
                        })))
                        .truncate(),
                    measured(
                        (scrubber(
                            move |runtime| percent_fraction(state(runtime).1),
                            move |runtime| seeking.get(runtime),
                            theme.palette.separator,
                            theme.palette.primary,
                        )
                        .w_full()
                        .h(SCRUB_BAND),),
                        band,
                    )
                    .mt(SCRUB_GAP - LINE_GAP)
                    .on_pointer_enter(move |cx, _| seeking.set(cx, true))
                    .on_pointer_leave(move |cx, _| seeking.set(cx, false))
                    .on_click(move |cx, event| {
                        event.stop_propagation();
                        let width = band.get(cx).width;
                        let percent = seek_percent(event.detail().position.x, width);
                        let id = notification.with(cx, |n| n.id);
                        env.send(
                            cx,
                            Request::Perform(id, CardAction::Media(MediaAction::Seek(percent))),
                        );
                    }),
                ))
                .gap(LINE_GAP)
                .grow(1.0)
                .shrink(1.0)
                .min_w(0.0),
            ))
            .gap(metrics.icon_gap)
            .items_start(),
            hstack((
                transport(Symbol::Previous, MediaAction::Previous, neutral, "Previous"),
                keyed(
                    move |runtime| state(runtime).0,
                    move |_, playing| {
                        let (symbol, action, label) = if playing {
                            (Symbol::Pause, MediaAction::Pause, "Pause")
                        } else {
                            (Symbol::Play, MediaAction::Play, "Play")
                        };
                        transport(symbol, action, (accent, theme.palette.on_fill), label)
                    },
                ),
                transport(Symbol::Next, MediaAction::Next, neutral, "Next"),
            ))
            .gap(2.0 * CONTROL_SPACING)
            .justify_center()
            .mt(CONTROL_GAP),
        ))
    })
}

fn title(notification: &Notification) -> &str {
    match &notification.content {
        Content::Music(music) => &music.title,
        _ => &notification.summary,
    }
}

fn artist(notification: &Notification) -> &str {
    match &notification.content {
        Content::Music(music) => &music.artist,
        _ => &notification.body,
    }
}

fn picture(notification: &Notification) -> Option<&Icon> {
    match &notification.content {
        Content::Music(music) => music.album_art.as_ref(),
        _ => None,
    }
    .or(notification.image.as_ref())
    .or(notification.app.icon.as_ref())
}
