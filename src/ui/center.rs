//! The notification center: a panel that slides in over everything else and
//! shows the history the toasts left behind.
//!
//! The surface covers the whole output, which is what makes a click *outside*
//! the panel an ordinary pointer event on the same surface, and so the obvious
//! way to dismiss it. While the center is closed the panel is hidden and the
//! root takes no input, so the surface swallows nothing and the compositor
//! blurs nothing.

use crownui::{
    ext::{Key, Kind, NamedKey},
    motion,
    prelude::{
        Anchor, AppConfig, Cx, Element, KeyboardInteractivity, Layer, LayerOptions, Listeners,
        Signal, Size, Store, Styled, TRANSPARENT, View, bind, component, for_each, hstack, text,
        vstack, zstack,
    },
};

use crate::{
    daemon::Request,
    model::Notification,
    ui::{
        board::{BoardHandle, DeckHead, NOTCH, Side},
        env::Env,
        feed,
        head::head,
        tile::tile,
        widget::{ButtonStyle, measured, pill, reaction, takes_input, tap},
        window::Window,
    },
    util::spring::Spring,
};

/// Width of the panel, matching the toast strip so a notification does not
/// change size when it moves from one to the other.
const PANEL_WIDTH: f32 = 400.0;
/// Padding inside the panel's edges.
const PAD: f32 = 12.0;
/// The title row plus the Do Not Disturb row.
const HEADER_HEIGHT: f32 = 96.0;
/// Below this the panel is off-screen and the surface is treated as closed.
const CLOSED: f32 = 0.001;
/// Size and weight of the panel's own title, which is louder than anything a
/// card carries.
const TITLE_SIZE: f32 = 17.0;
const TITLE_WEIGHT: u16 = 700;
/// What the surface asks for before the compositor stretches it over the output.
const FALLBACK: (u32, u32) = (1280, 800);

/// The window the notification center lives in.
pub fn config() -> AppConfig {
    Window::config("crownotify-center", FALLBACK.0, FALLBACK.1).layer(
        // All four anchors with no exclusive zone: the compositor sizes the
        // surface to the usable area, so a click anywhere outside the panel
        // arrives here and closes it.
        LayerOptions::new("crownotify-center", Anchor::ALL).layer(Layer::Overlay),
    )
}

/// The panel's slide, 0 = fully off-screen right, 1 = resting on the edge.
struct Slide {
    spring: Spring,
    scheduled: bool,
}

#[derive(Clone, Copy)]
struct Panel {
    slide: Store<Slide>,
    /// The slide as of the last animation frame.
    position: Signal<f32>,
    open: Signal<bool>,
    /// Open, or still sliding shut: anything of the panel on screen.
    present: Signal<bool>,
    quiet: Signal<bool>,
}

pub fn center(window: Window) -> impl View {
    component(move |cx| {
        let Window {
            theme,
            profiles,
            requests,
            snapshots,
        } = window;
        let env = Env::new(cx, theme, profiles, requests);
        let board = BoardHandle::new(cx, Side::History, theme.metrics);
        let panel = Panel {
            slide: cx.store(Slide {
                spring: Spring::new(0.0, motion::Spring::smooth()),
                scheduled: false,
            }),
            position: cx.signal(0.0),
            open: cx.signal(false),
            present: cx.signal(false),
            quiet: cx.signal(false),
        };
        let surface = cx.signal(Size::ZERO);
        let viewport = cx.signal(Size::ZERO);
        feed::follow(cx, snapshots, move |cx, snapshot| {
            panel.quiet.set(cx, snapshot.do_not_disturb);
            if panel.open.get(cx) != snapshot.center_open {
                panel.open.set(cx, snapshot.center_open);
                slide(cx, panel, board);
            }
            if panel.open.get(cx) {
                board.show(cx, &snapshot.history);
            }
        });
        feed::keep_time(cx, env);
        let close = move |cx: &mut Cx| env.send(cx, Request::SetCenterOpen(false));

        zstack((
            measured((), surface).absolute().inset(0.0),
            reaction(
                move |runtime| viewport.get(runtime),
                move |cx, size| board.resize(cx, size),
            ),
            reaction(
                move |runtime| panel.present.get(runtime),
                |cx, present| {
                    cx.set_keyboard_interactivity(if present {
                        KeyboardInteractivity::OnDemand
                    } else {
                        KeyboardInteractivity::None
                    })
                },
            ),
            hstack(())
                .absolute()
                .inset(0.0)
                .modifier(takes_input(move |runtime| panel.present.get(runtime)))
                .on_click(move |cx, _| close(cx)),
            panel_view(env, board, panel, viewport)
                .left(bind(move |runtime| {
                    let width = surface.get(runtime).width;
                    width - PANEL_WIDTH.min(width) * panel.position.get(runtime).clamp(0.0, 1.0)
                }))
                .hidden(bind(move |runtime| !panel.present.get(runtime))),
        ))
        .bg(TRANSPARENT)
        .pointer_events_none()
        .on_key_down(move |cx, event| {
            if event.detail().key == Key::Named(NamedKey::Escape) {
                close(cx);
            }
        })
    })
}

fn panel_view(
    env: Env,
    board: BoardHandle,
    panel: Panel,
    viewport: Signal<Size>,
) -> Element<impl Kind> {
    let theme = env.theme;
    let palette = theme.palette;
    let accent = palette.accent;
    let quiet = panel.quiet;
    vstack((
        hstack((
            theme
                .text
                .app
                .apply(text("Notifications"))
                .text_size(TITLE_SIZE)
                .font_weight(TITLE_WEIGHT)
                .grow(1.0),
            pill("Clear All", theme, accent, |_| ButtonStyle::Tinted)
                .on_click(tap(move |cx| env.send(cx, Request::ClearHistory))),
        ))
        .items_center()
        .h(theme.metrics.button),
        hstack((pill("Do Not Disturb", theme, accent, move |runtime| {
            if quiet.get(runtime) {
                ButtonStyle::Filled(accent)
            } else {
                ButtonStyle::Plain
            }
        })
        .on_click(tap(move |cx| {
            let enabled = quiet.get(cx);
            env.send(cx, Request::SetDoNotDisturb(!enabled));
        })),))
        .mt(8.0),
        measured(
            (
                for_each(
                    board.heads,
                    |deck: &DeckHead| deck.key.clone(),
                    move |_, deck| head(env, board, deck, Side::History),
                )
                .absolute()
                .inset(0.0),
                for_each(
                    board.cards,
                    |card: &Notification| card.id,
                    move |_, card| tile(env, board, card, Side::History),
                )
                .absolute()
                .inset(0.0),
            ),
            viewport,
        )
        .absolute()
        .top(HEADER_HEIGHT)
        .left(0.0)
        .right(0.0)
        .bottom(PAD)
        .overflow_hidden()
        // A notch scrolls a fixed distance and a touchpad its own pixels, so a
        // mouse and a touchpad do not scroll at wildly different speeds.
        .on_wheel(move |cx, event| {
            board.scroll_by(cx, event.detail().delta.to_pixels(NOTCH).y);
        }),
        // The hairline down the panel's leading edge.
        hstack(())
            .absolute()
            .top(0.0)
            .bottom(0.0)
            .left(0.0)
            .w(1.0)
            .bg(palette.separator),
    ))
    .absolute()
    .top(0.0)
    .bottom(0.0)
    .w(PANEL_WIDTH)
    .p(PAD)
    .bg(palette.card)
    .material(theme.material())
    .modifier(takes_input(move |runtime| panel.present.get(runtime)))
}

/// Springs the panel toward the open state, one display refresh at a time,
/// and puts it back the way it opens once it is fully off screen.
fn slide(cx: &mut Cx, panel: Panel, board: BoardHandle) {
    if panel.open.get(cx) {
        panel.present.set(cx, true);
    }
    if panel
        .slide
        .update(cx, |slide| std::mem::replace(&mut slide.scheduled, true))
    {
        return;
    }
    cx.request_animation_frame(move |cx, now| {
        let target = if panel.open.get(cx) { 1.0 } else { 0.0 };
        let (position, at_rest) = panel.slide.update(cx, |slide| {
            slide.scheduled = false;
            slide.spring.set_target(now, target);
            (slide.spring.value(now), slide.spring.at_rest(now))
        });
        panel.position.set(cx, position);
        if target == 0.0 && position <= CLOSED {
            panel.present.set(cx, false);
            board.clear(cx);
        } else if !at_rest {
            slide(cx, panel, board);
        }
    });
}
