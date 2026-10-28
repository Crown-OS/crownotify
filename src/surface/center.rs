//! The notification center: a panel that slides in over everything else and
//! shows the history the toasts left behind.
//!
//! Like the toast strip, this is one layer surface that is created at start-up
//! and never unmapped. It covers the whole output — which is what makes a click
//! *outside* the panel an ordinary pointer event on the same surface, and so the
//! obvious way to dismiss it — and the panel is animated inside it rather than
//! by resizing anything.
//!
//! While the center is closed the surface draws nothing and sets an empty input
//! *and* blur region. Both matter: without the first, the surface would swallow
//! every click on the desktop; without the second, the compositor would spend
//! every frame blurring a rectangle nobody can see.

use crate::{
    model::NotificationId,
    profile::table::Profiles,
    surface::{
        CardOutcome, Shared, SharedState,
        cards::{Decks, reconcile},
    },
    ui::{
        Card, CardAction, HitMap, MeasureCtx, RenderCtx, Stack, Theme, card_width,
        deck::content_alpha,
        resolve_hit,
        widget::{ButtonState, ButtonStyle, Painted, PillButton, draw_card, sync_style},
    },
};
use crownshell::{
    Anchor, Clock, KeyPress, KeyboardInteractivity, Keysym, Layer, MaterialSpec, Scene,
    ScrollDelta, Spring, SpringProfile, SurfaceCtx, SurfaceHandler, Text, WindowConfig,
    kurbo::{Affine, Point, Rect, Size},
    peniko::{Fill, Mix},
};

/// Width of the panel, matching the toast strip so a notification does not
/// change size when it moves from one to the other.
const PANEL_WIDTH: f64 = 400.0;
/// Padding inside the panel's edges.
const PAD: f64 = 12.0;
/// The title row plus the Do Not Disturb row.
const HEADER_HEIGHT: f64 = 96.0;
/// How far one wheel notch scrolls.
const NOTCH: f64 = 60.0;
/// Below this the panel is off-screen and the surface is treated as closed.
const CLOSED: f32 = 0.001;
/// Size and weight of the panel's own title, which is louder than anything a
/// card carries.
const TITLE_SIZE: f32 = 17.0;
const TITLE_WEIGHT: f32 = 700.0;

/// The window the notification center lives in.
pub fn config() -> WindowConfig {
    WindowConfig {
        namespace: "crownotify-center".to_string(),
        layer: Layer::Overlay,
        // All four anchors with no exclusive zone: the compositor sizes the
        // surface to the usable area, so a click anywhere outside the panel
        // arrives here and closes it.
        anchor: Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT,
        size: (0, 0),
        exclusive_zone: 0,
        keyboard_interactivity: KeyboardInteractivity::None,
        blur: true,
        auto_blur_region: false,
        tick_interval: None,
    }
}

/// A part of the panel that is not a card.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Chrome {
    ClearAll,
    DoNotDisturb,
    /// Index into the group keys captured by the last paint.
    Group(usize),
}

/// The retained text one group's header row draws.
struct GroupChrome {
    name: Text,
}

impl GroupChrome {
    fn new() -> Self {
        Self {
            name: Text::new(""),
        }
    }
}

/// The notification center panel.
pub struct Center {
    shared: SharedState,
    /// The [`Shared::version`] this surface last painted.
    seen: u64,
    /// 0 = fully off-screen right, 1 = resting against the output's edge.
    slide: Spring,
    /// Content offset in logical pixels, sprung so a flick coasts to a stop.
    scroll: Spring,
    clock: Clock,
    cards: Vec<Card>,
    /// Reused when the visible set is rebuilt, so scrolling allocates nothing.
    scratch: Vec<Card>,
    headers: Vec<GroupChrome>,
    /// The group keys the last paint drew, so a click can name a group after
    /// the borrowed [`Group`](crate::store::Group) values are gone.
    group_keys: Vec<String>,
    decks: Decks,
    /// How many of `cards` are still in the store.
    live: usize,
    stack: Stack,
    title: Text,
    clear: PillButton,
    quiet: PillButton,
    hits: HitMap,
    chrome: Vec<(Rect, Chrome)>,
    hovered: Option<(NotificationId, CardAction)>,
    /// The chrome rectangle the pointer is over, identified by the rectangle
    /// itself so the highlight survives the panel being re-laid out.
    chrome_hover: Option<Rect>,
    /// The panel as the last paint placed it, for hit-testing a click outside.
    panel: Rect,
    content_height: f64,
    viewport_height: f64,
    keyboard: bool,
}

impl Center {
    pub fn new(shared: SharedState) -> Self {
        // Styled once rather than per paint: a `TextStyle` owns its family
        // stack, so building one every frame is an allocation per frame, and
        // the theme is fixed for the life of the process.
        let title = Text::new("Notifications").with_style(
            shared
                .borrow()
                .theme
                .text
                .app
                .clone()
                .with_size(TITLE_SIZE)
                .with_weight(TITLE_WEIGHT),
        );
        Self {
            shared,
            seen: 0,
            slide: Spring::with_profile(0.0, SpringProfile::SMOOTH),
            scroll: Spring::with_profile(0.0, SpringProfile::SMOOTH),
            clock: Clock::new(),
            cards: Vec::new(),
            scratch: Vec::new(),
            headers: Vec::new(),
            group_keys: Vec::new(),
            decks: Decks::default(),
            live: 0,
            stack: Stack::new(),
            title,
            clear: PillButton::new("Clear All", ButtonStyle::Tinted),
            quiet: PillButton::new("Do Not Disturb", ButtonStyle::Plain),
            hits: HitMap::new(),
            chrome: Vec::new(),
            hovered: None,
            chrome_hover: None,
            panel: Rect::ZERO,
            content_height: 0.0,
            viewport_height: 0.0,
            keyboard: false,
        }
    }

    /// Puts the panel back to how it opens: at the top, with nothing expanded.
    fn reset(&mut self) {
        self.cards.clear();
        self.chrome.clear();
        self.hits.clear();
        self.hovered = None;
        self.chrome_hover = None;
        // Nothing to animate for a panel that has slid off screen.
        self.decks.collapse_all();
        self.scroll.set_target(0.0);
        self.scroll.snap_to_target();
    }

    /// Carries out `action` on the card for `id`.
    fn perform(&mut self, id: NotificationId, action: &CardAction) {
        // Both are the surface's own business: only it knows how far this
        // deck's unfurl spring has got.
        match action {
            CardAction::ToggleStack(key) => {
                self.decks.toggle(key);
                self.shared.borrow_mut().bump();
                return;
            }
            CardAction::ClearStack(key) => {
                self.shared.borrow_mut().clear_group(key);
                return;
            }
            _ => {}
        }
        let outcome = self.shared.borrow_mut().perform(id, action);
        match outcome {
            CardOutcome::Handled => {}
            CardOutcome::ToggleMenu => {
                if let Some(card) = self.cards.iter_mut().find(|card| card.id == id) {
                    card.toggle_menu();
                }
            }
            CardOutcome::FocusReply => {
                if let Some(card) = self.cards.iter_mut().find(|card| card.id == id) {
                    card.focus_reply();
                }
            }
            CardOutcome::SubmitReply => {
                let text = self
                    .cards
                    .iter_mut()
                    .find(|card| card.id == id)
                    .and_then(Card::take_reply);
                if let Some(text) = text {
                    self.shared.borrow_mut().submit_reply(id, text);
                }
            }
        }
    }

    /// Handles a click on something that is not a card.
    fn press_chrome(&mut self, chrome: Chrome) {
        match chrome {
            Chrome::ClearAll => self.shared.borrow_mut().clear_history(),
            Chrome::DoNotDisturb => {
                let mut shared = self.shared.borrow_mut();
                let enabled = shared.store.do_not_disturb();
                shared.set_do_not_disturb(!enabled);
            }
            Chrome::Group(index) => {
                let Some(key) = self.group_keys.get(index).cloned() else {
                    return;
                };
                self.decks.toggle(&key);
                self.shared.bump();
            }
        }
    }
}

impl SurfaceHandler for Center {
    fn paint(&mut self, scene: &mut Scene, ctx: SurfaceCtx<'_>) {
        let size = Size::new(f64::from(ctx.size.0), f64::from(ctx.size.1));
        // Cloning the handle first keeps the `RefMut` from borrowing `self`, so
        // the rest of the paint can still reach the surface's own methods.
        let state = self.shared.clone();
        let mut shared = state.borrow_mut();
        self.seen = shared.version;

        let open = shared.center_open;
        self.slide.set_target(if open { 1.0 } else { 0.0 });
        if !open && self.slide.position <= CLOSED {
            self.reset();
            ctx.set_input_region(&[]);
            ctx.set_material(&[], &MaterialSpec::default());
            if self.keyboard {
                self.keyboard = false;
                ctx.set_keyboard_interactivity(KeyboardInteractivity::None);
            }
            return;
        }

        let Shared {
            store,
            theme,
            profiles,
            icons,
            ..
        } = &mut *shared;
        let profiles: &Profiles = &*profiles;
        let theme: &Theme = theme;

        let painted = Painted::after(ctx.material_capabilities());
        let width = PANEL_WIDTH.min(size.width);
        let left = size.width - width * f64::from(self.slide.position.clamp(0.0, 1.0));
        let panel = Rect::new(left, 0.0, left + width, size.height);
        self.panel = panel;

        // Open in any degree takes the whole surface, so that a click outside
        // the panel is delivered here and dismisses it.
        ctx.set_input_region(&[Rect::new(0.0, 0.0, size.width, size.height)]);
        ctx.set_material(
            &[panel.to_rounded_rect(theme.metrics.card_radius)],
            &theme.material_spec(),
        );
        if !self.keyboard {
            self.keyboard = true;
            ctx.set_keyboard_interactivity(KeyboardInteractivity::OnDemand);
        }

        scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            theme.palette.card,
            None,
            &panel,
        );
        scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            theme.palette.separator,
            None,
            &Rect::new(panel.x0, panel.y0, panel.x0 + 1.0, panel.y1),
        );

        self.hits.clear();
        self.chrome.clear();

        // --- header -------------------------------------------------------
        let title_size = self.title.size(ctx.text);
        self.title.draw(
            ctx.text,
            scene,
            (
                (panel.x0 + PAD).round(),
                (panel.y0 + PAD + (theme.metrics.button - title_size.height) / 2.0).round(),
            ),
        );

        let clear_size = self.clear.measure(ctx.text, theme);
        let clear_rect = Rect::new(
            panel.x1 - PAD - clear_size.width,
            panel.y0 + PAD,
            panel.x1 - PAD,
            panel.y0 + PAD + clear_size.height,
        );
        self.clear.render(
            scene,
            clear_rect,
            theme,
            button_state(&self.chrome_hover, clear_rect),
            1.0,
            ctx.text,
        );
        self.chrome.push((clear_rect, Chrome::ClearAll));

        self.quiet.set_style(if store.do_not_disturb() {
            ButtonStyle::Filled(theme.palette.accent)
        } else {
            ButtonStyle::Plain
        });
        let quiet_size = self.quiet.measure(ctx.text, theme);
        let quiet_rect = Rect::new(
            panel.x0 + PAD,
            clear_rect.y1 + 8.0,
            panel.x0 + PAD + quiet_size.width,
            clear_rect.y1 + 8.0 + quiet_size.height,
        );
        self.quiet.render(
            scene,
            quiet_rect,
            theme,
            button_state(&self.chrome_hover, quiet_rect),
            1.0,
            ctx.text,
        );
        self.chrome.push((quiet_rect, Chrome::DoNotDisturb));

        // --- content ------------------------------------------------------
        let viewport = Rect::new(panel.x0, panel.y0 + HEADER_HEIGHT, panel.x1, panel.y1 - PAD);
        self.viewport_height = viewport.height().max(0.0);

        let card_width = card_width(Size::new(width, size.height), theme);
        let card_left = panel.x0 + (width - card_width) / 2.0;
        let scroll = f64::from(self.scroll.position);

        let groups = store.groups();
        self.group_keys.clear();
        self.group_keys
            .extend(groups.iter().map(|group| group.key.to_string()));
        while self.headers.len() < groups.len() {
            self.headers.push(GroupChrome::new());
        }
        self.live = reconcile(
            &mut self.cards,
            &mut self.scratch,
            &mut self.decks,
            &groups,
            theme,
            0.0,
        );
        self.stack.layout(
            &mut self.cards[..self.live],
            self.decks.as_slice(),
            Size::new(width, f64::INFINITY),
            &mut MeasureCtx {
                text: &mut *ctx.text,
                theme,
            },
        );

        scene.push_layer(Fill::NonZero, Mix::Normal, 1.0, Affine::IDENTITY, &viewport);

        let origin = viewport.y0 - scroll;
        for deck in self.stack.decks() {
            let Some(group) = groups.get(deck.index) else {
                continue;
            };
            let header = shifted(deck.header, card_left - deck.header.x0, origin);
            if header.height() > 0.0 && header.y1 > viewport.y0 && header.y0 < viewport.y1 {
                let chrome = &mut self.headers[deck.index];
                sync_style(&mut chrome.name, &theme.text.eyebrow);
                chrome.name.set_text(&group.app.name);
                let size = chrome.name.size(ctx.text);
                chrome.name.draw(
                    ctx.text,
                    scene,
                    (
                        header.x0.round(),
                        (header.center().y - size.height / 2.0).round(),
                    ),
                );
            }
            self.chrome.push((header, Chrome::Group(deck.index)));

            for index in deck.cards.clone() {
                let placement = self.stack.placements()[index];
                let card = &mut self.cards[placement.index];
                let bounds = shifted(placement.rect, card_left - deck.header.x0, origin);
                if bounds.y1 <= viewport.y0 || bounds.y0 >= viewport.y1 {
                    continue;
                }
                // Collapsed, the whole deck is one target: a click on the top
                // card pulls it apart rather than activating the notification.
                let body = if deck.unfurl >= crate::ui::deck::OPEN {
                    CardAction::Activate
                } else {
                    CardAction::ToggleStack(group.key.to_string())
                };
                let content = content_alpha(placement.depth, placement.unfurl);
                if content < 1.0 {
                    draw_card(
                        scene,
                        bounds,
                        theme,
                        placement.alpha * (1.0 - content),
                        painted,
                    );
                }
                if content <= 0.0 {
                    continue;
                }
                let hovered = match &self.hovered {
                    Some((id, action)) if *id == card.id => Some(action.clone()),
                    _ => None,
                };
                let key = store
                    .get(card.id)
                    .map(|entry| entry.notification.app.key().into_owned())
                    .unwrap_or_default();
                card.render(&mut RenderCtx {
                    painted,
                    body: body.clone(),
                    scene,
                    text: &mut *ctx.text,
                    icons: &mut *icons,
                    theme,
                    profile: profiles.get(&key),
                    hits: &mut self.hits,
                    bounds,
                    alpha: content,
                    hovered,
                });
            }
        }
        let y = self.stack.height() + 2.0 * theme.metrics.card_margin;

        scene.pop_layer();

        self.content_height = y;
        // A group that collapsed under the pointer can leave the panel scrolled
        // past the end of what is left.
        let clamped = clamp_scroll(
            f64::from(self.scroll.target),
            self.content_height,
            self.viewport_height,
        );
        self.scroll.set_target(clamped as f32);
    }

    fn on_frame(&mut self, _ctx: SurfaceCtx<'_>) -> bool {
        if self.slide.at_rest() && self.scroll.at_rest() && self.decks.at_rest() {
            self.clock.reset();
            return false;
        }
        let dt = self.clock.tick();
        self.slide.step(dt);
        self.scroll.step(dt);
        self.decks.step(dt);
        true
    }

    fn on_pointer_enter(&mut self, x: f64, y: f64, ctx: SurfaceCtx<'_>) -> bool {
        self.on_pointer_motion(x, y, ctx)
    }

    fn on_pointer_leave(&mut self, _ctx: SurfaceCtx<'_>) -> bool {
        let changed = self.hovered.is_some() || self.chrome_hover.is_some();
        self.hovered = None;
        self.chrome_hover = None;
        changed
    }

    fn on_pointer_motion(&mut self, x: f64, y: f64, _ctx: SurfaceCtx<'_>) -> bool {
        let point = Point::new(x, y);
        let hovered = self
            .hits
            .hit(point)
            .map(|region| (region.id, region.action.clone()));
        let chrome_hover = self
            .chrome
            .iter()
            .rev()
            .find(|(rect, _)| rect.contains(point))
            .map(|(rect, _)| *rect);
        let changed = hovered != self.hovered || chrome_hover != self.chrome_hover;
        self.hovered = hovered;
        self.chrome_hover = chrome_hover;
        changed
    }

    fn on_pointer_press(&mut self, x: f64, y: f64, _ctx: SurfaceCtx<'_>) -> bool {
        let point = Point::new(x, y);
        if !self.panel.contains(point) {
            self.shared.borrow_mut().set_center_open(Some(false));
            return true;
        }

        if let Some(region) = self.hits.hit(point) {
            let (id, action) = (region.id, resolve_hit(region, point));
            self.perform(id, &action);
            return true;
        }
        let chrome = self
            .chrome
            .iter()
            .rev()
            .find(|(rect, _)| rect.contains(point))
            .map(|(_, chrome)| *chrome);
        if let Some(chrome) = chrome {
            self.press_chrome(chrome);
        }
        true
    }

    fn on_pointer_scroll(
        &mut self,
        _x: f64,
        _y: f64,
        delta: ScrollDelta,
        _ctx: SurfaceCtx<'_>,
    ) -> bool {
        // A wheel reports both a notch count and a synthesised pixel delta;
        // taking the notch when there is one keeps a mouse and a touchpad from
        // scrolling at wildly different speeds.
        let step = if delta.discrete_y != 0 {
            f64::from(delta.discrete_y) * NOTCH
        } else {
            delta.y
        };
        if step == 0.0 {
            return false;
        }
        let target = clamp_scroll(
            f64::from(self.scroll.target) + step,
            self.content_height,
            self.viewport_height,
        );
        self.scroll.set_target(target as f32);
        true
    }

    fn on_key_press(&mut self, key: KeyPress<'_>, _ctx: SurfaceCtx<'_>) -> bool {
        if let Some(index) = self.cards.iter().position(Card::wants_keyboard) {
            let id = self.cards[index].id;
            let action = self.cards[index].on_key(&key);
            if let Some(action) = action {
                self.perform(id, &action);
            }
            return true;
        }
        if key.keysym == Keysym::Escape {
            self.shared.borrow_mut().set_center_open(Some(false));
            return true;
        }
        false
    }

    fn needs_redraw(&self) -> bool {
        self.shared.borrow().version != self.seen
    }
}

/// Whether the pointer is resting on `rect`.
fn button_state(hover: &Option<Rect>, rect: Rect) -> ButtonState {
    match hover {
        Some(hovered) if *hovered == rect => ButtonState::Hover,
        _ => ButtonState::Normal,
    }
}

/// Moves a rectangle from the stack's own coordinates into the panel's
/// scrolled ones.
fn shifted(rect: Rect, dx: f64, origin: f64) -> Rect {
    Rect::new(
        rect.x0 + dx,
        origin + rect.y0,
        rect.x1 + dx,
        origin + rect.y1,
    )
}

/// Clamps a scroll offset to the range the content actually occupies.
///
/// Content shorter than the viewport does not scroll at all, and a group that
/// collapsed under the pointer must not leave the panel scrolled past the end
/// of what is left.
pub(crate) fn clamp_scroll(offset: f64, content: f64, viewport: f64) -> f64 {
    offset.clamp(0.0, (content - viewport).max(0.0))
}
