//! The toast strip: a full-height layer surface down the right-hand edge with
//! the live notifications animating inside it.
//!
//! # Why the surface never resizes
//!
//! The obvious way to build a toast stack is to size the surface to its
//! contents. It is also the wrong one: every size change is a commit the
//! compositor has to configure back, and a card that enters, re-flows or leaves
//! changes the contents on almost every frame of an animation. The surface is
//! therefore anchored top-to-bottom at a fixed width and the cards move *inside*
//! it, exactly as `crownshell/examples/menu_popup.rs` animates its panel. The
//! parts of the strip that hold no card are made click-through by setting the
//! input region to the union of the card rectangles each frame, and the blur
//! region is set to the same rectangles so the compositor never blurs a strip
//! of wallpaper nobody can see.
//!
//! # What drives a frame
//!
//! [`SurfaceHandler::on_frame`] returns `true` for as long as any spring is
//! moving, so the animation is paced by the compositor's frame clock rather than
//! by a timer of our own. The 500 ms tick is only for keeping the relative
//! timestamps ("now", "5m") honest and for pumping the phone link; it never
//! invalidates a measurement.

use std::time::{Duration, Instant};

use chrono::Local;
use crownshell::{
    Anchor, Capability, Clock, KeyPress, KeyboardInteractivity, Layer, MAX_SHAPE_PRIMITIVES,
    MaterialSpec, Scene, SurfaceCtx, SurfaceHandler, WindowConfig,
    kurbo::{Point, Rect, RoundedRect, Size},
};

use crate::{
    model::{CloseReason, NotificationId},
    profile::{default_profile, table::Profiles},
    store::Store,
    surface::{
        CardOutcome, Shared, SharedState,
        cards::{Decks, displaced, reconcile},
    },
    ui::{
        Card, CardAction, HitMap, MeasureCtx, RenderCtx, Stack, Theme, card_left, card_width,
        deck::{self, content_alpha},
        resolve_hit,
        widget::{Painted, draw_card, rounded_bands},
    },
};

/// Width of the strip. Wider than a card by two margins, so the card's shadow
/// has somewhere to fall.
const STRIP_WIDTH: u32 = 400;
/// How often the surface wakes to re-render timestamps and pump the phone link.
const TICK: Duration = Duration::from_millis(500);
/// How far right a card has to be dragged before it is dismissed.
pub(crate) const SWIPE_THRESHOLD: f64 = 40.0;
/// Ticks between icon-cache sweeps. Sixty ticks is half a minute, which is far
/// longer than any burst of notifications and short enough that an app that has
/// stopped notifying does not keep its artwork resident forever.
const ICON_SWEEP_TICKS: u32 = 60;

/// The window the toast strip lives in.
pub fn config() -> WindowConfig {
    WindowConfig {
        namespace: "crownotify".to_string(),
        layer: Layer::Top,
        // Anchoring top *and* bottom is what makes the compositor hand back a
        // full-height surface, which is what lets a card animate without the
        // surface ever being resized.
        anchor: Anchor::TOP | Anchor::RIGHT | Anchor::BOTTOM,
        size: (STRIP_WIDTH, 0),
        exclusive_zone: 0,
        keyboard_interactivity: KeyboardInteractivity::None,
        blur: true,
        auto_blur_region: false,
        tick_interval: Some(TICK),
    }
}

/// A pointer button held down on a card, and how far it has travelled.
struct Press {
    id: NotificationId,
    /// What the press resolved to, performed on release unless the press turns
    /// into a swipe first.
    action: CardAction,
    origin: Point,
    last_x: f64,
    last_at: Instant,
    /// Horizontal pointer speed in logical pixels per second, from the most
    /// recent motion event.
    velocity: f64,
}

/// The toast stack.
pub struct Popup {
    shared: SharedState,
    /// The [`Shared::version`] this surface last painted.
    seen: u64,
    /// Live cards first, in stack order, then the ones on their way out.
    cards: Vec<Card>,
    /// How many of `cards` are still in the store.
    live: usize,
    /// Reused by [`reconcile`] so re-ordering the stack allocates nothing.
    scratch: Vec<Card>,
    decks: Decks,
    stack: Stack,
    hits: HitMap,
    /// Where a leaving card's controls go: it is drawn, but it must not be
    /// clickable, and it is outside the input region anyway.
    discarded_hits: HitMap,
    /// Input and blur rectangles, rebuilt every frame and reused.
    regions: Vec<Rect>,
    /// The card rectangles the compositor renders the material for, rebuilt
    /// every frame and reused.
    shapes: Vec<RoundedRect>,
    /// The capability set last logged, so the line is printed once rather than
    /// on every frame.
    capabilities: Option<Capability>,
    clock: Clock,
    hovered: Option<(NotificationId, CardAction)>,
    press: Option<Press>,
    /// Whether the surface currently asks the compositor for keyboard focus.
    keyboard: bool,
    ticks: u32,
}

impl Popup {
    pub fn new(shared: SharedState) -> Self {
        Self {
            shared,
            seen: 0,
            cards: Vec::new(),
            live: 0,
            scratch: Vec::new(),
            decks: Decks::default(),
            stack: Stack::new(),
            hits: HitMap::new(),
            discarded_hits: HitMap::new(),
            regions: Vec::new(),
            shapes: Vec::new(),
            capabilities: None,
            clock: Clock::new(),
            hovered: None,
            press: None,
            keyboard: false,
            ticks: 0,
        }
    }

    /// Carries out `action` on the card for `id`, doing the part that belongs
    /// to the card here and handing the rest to [`Shared::perform`].
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
                self.shared.borrow_mut().dismiss_group(key);
                return;
            }
            _ => {}
        }
        let outcome = self.shared.borrow_mut().perform(id, action);
        match outcome {
            CardOutcome::Handled => {}
            CardOutcome::ToggleMenu => {
                if let Some(card) = self.card_mut(id) {
                    card.toggle_menu();
                }
            }
            CardOutcome::FocusReply => {
                if let Some(card) = self.card_mut(id) {
                    card.focus_reply();
                }
            }
            CardOutcome::SubmitReply => {
                let text = self.card_mut(id).and_then(Card::take_reply);
                if let Some(text) = text {
                    self.shared.borrow_mut().submit_reply(id, text);
                }
            }
        }
    }

    fn card_mut(&mut self, id: NotificationId) -> Option<&mut Card> {
        self.cards.iter_mut().find(|card| card.id == id)
    }

    /// Updates which control the pointer is over, reporting whether it moved.
    ///
    /// The region's own action is used rather than the resolved one: a scrubber
    /// resolves to a different `Seek` percentage for every pixel the pointer
    /// crosses, which would repaint the whole strip on every motion event.
    fn update_hover(&mut self, point: Point) -> bool {
        let hovered = self
            .hits
            .hit(point)
            .map(|region| (region.id, region.action.clone()));
        let changed = hovered != self.hovered;
        self.hovered = hovered;
        changed
    }
}

impl SurfaceHandler for Popup {
    fn paint(&mut self, scene: &mut Scene, ctx: SurfaceCtx<'_>) {
        let size = Size::new(f64::from(ctx.size.0), f64::from(ctx.size.1));
        let mut shared = self.shared.borrow_mut();
        self.seen = shared.version;
        let Shared {
            store,
            theme,
            profiles,
            icons,
            ..
        } = &mut *shared;
        let profiles: &Profiles = profiles;
        let store: &Store = store;
        let theme: &Theme = theme;

        // A card that has just arrived enters from below the stack rather than
        // from the top of the surface, so it slides the short way to its place.
        let entering_at = self
            .stack
            .placements()
            .last()
            .map_or(theme.metrics.card_margin, |placement| {
                placement.rect.y1 + theme.metrics.card_gap
            });
        // Newest deck first is deck order; the strip wants the oldest at the
        // top, with the newest card still on top *within* each deck.
        let mut groups = store.toast_groups();
        groups.reverse();
        self.live = reconcile(
            &mut self.cards,
            &mut self.scratch,
            &mut self.decks,
            &groups,
            theme,
            entering_at as f32,
        );

        self.stack.layout(
            &mut self.cards[..self.live],
            self.decks.as_slice(),
            size,
            &mut MeasureCtx {
                text: &mut *ctx.text,
                theme,
            },
        );

        let width = card_width(size, theme);
        let radius = theme.metrics.card_radius;
        self.hits.clear();
        self.discarded_hits.clear();
        self.regions.clear();
        self.shapes.clear();
        let capabilities = ctx.material_capabilities();
        if self.capabilities.replace(capabilities) != Some(capabilities) {
            // A withheld effect and a theme that is merely too transparent look
            // identical on screen, so say which one this is.
            log::info!("compositor renders {capabilities:?} for notifications");
        }
        let painted = Painted::after(capabilities);

        for index in 0..self.stack.placements().len() {
            let placement = self.stack.placements()[index];
            let deck = self
                .stack
                .decks()
                .iter()
                .find(|deck| deck.cards.contains(&index));
            let key = deck
                .and_then(|deck| groups.get(deck.index))
                .map(|group| group.key.to_string());
            // Collapsed, the whole deck is one target: a click on the top card
            // pulls it apart rather than activating the notification.
            let body = match (deck.map(|deck| deck.unfurl), key.clone()) {
                (Some(unfurl), Some(key)) if unfurl < deck::OPEN => CardAction::ToggleStack(key),
                _ => CardAction::Activate,
            };
            let card = &mut self.cards[placement.index];
            let bounds = displaced(&placement, card, width);
            let reveal = card.animation.alpha();
            // The plate and the card are cross-faded over the same rectangle
            // rather than switched, so the material never blinks out between
            // them; a sliver is bare material with no contents at all.
            let content = content_alpha(placement.depth, placement.unfurl);
            let plate = placement.alpha * reveal * (1.0 - content);
            let alpha = placement.alpha * reveal * content;
            if plate > 0.0 {
                draw_card(scene, bounds, theme, plate, painted);
            }
            if alpha <= 0.0 {
                // A sliver is bare material: it is drawn and blurred, and it
                // belongs to the deck's single click target rather than
                // carrying one of its own.
                self.shapes.push(bounds.to_rounded_rect(radius));
                self.regions.extend(rounded_bands(bounds, radius));
                if let CardAction::ToggleStack(_) = &body {
                    self.hits.push(bounds, card.id, body.clone());
                }
                continue;
            }
            let hovered = hovered_action(&self.hovered, card.id);
            // A `match` rather than `map_or_else`: the latter would unify both
            // arms on `default_profile`'s `'static`, and the merged table's
            // profiles live only as long as the borrow of `Shared`.
            let profile = match store.get(card.id) {
                Some(entry) => profiles.get(&entry.notification.app.key()),
                None => default_profile(),
            };
            card.render(&mut RenderCtx {
                painted,
                body,
                scene: &mut *scene,
                text: &mut *ctx.text,
                icons: &mut *icons,
                theme,
                profile,
                hits: &mut self.hits,
                bounds,
                alpha,
                hovered,
            });
            self.regions.extend(rounded_bands(bounds, radius));
            self.shapes.push(bounds.to_rounded_rect(radius));
        }
        // Everything after this point is a card that is leaving: it still gets
        // drawn and still gets blurred, but it must not swallow a click meant
        // for the card sliding up underneath it.
        let clickable = self.regions.len();

        let left = card_left(size, theme);
        for card in self.cards[self.live..].iter_mut() {
            // A leaving card has no deck any more: it slides out from wherever
            // its own springs left it, at full width.
            let top = card.animation.top();
            let dx = card.animation.translate_x(width);
            let bounds = Rect::new(left + dx, top, left + dx + width, top + card.height());
            let alpha = card.animation.alpha();
            card.render(&mut RenderCtx {
                scene: &mut *scene,
                text: &mut *ctx.text,
                icons: &mut *icons,
                theme,
                profile: default_profile(),
                hits: &mut self.discarded_hits,
                bounds,
                painted,
                body: CardAction::Activate,
                alpha,
                hovered: None,
            });
            self.shapes.push(bounds.to_rounded_rect(radius));
        }

        // Input is a rectangle list whatever happens — a `wl_region` cannot be
        // rounded — but the material follows the card's real corner.
        ctx.set_input_region(&self.regions[..clickable]);
        if self.shapes.len() > MAX_SHAPE_PRIMITIVES {
            log::debug!(
                "{} cards is past the compositor's {MAX_SHAPE_PRIMITIVES}-shape limit",
                self.shapes.len()
            );
        }
        // No shapes means the whole surface to the compositor, so an empty
        // strip asks for no material rather than one full-height pane of glass.
        let material = if self.shapes.is_empty() {
            MaterialSpec::default()
        } else {
            theme.material_spec()
        };
        ctx.set_material(&self.shapes, &material);

        let wants_keyboard = self.cards.iter().any(Card::wants_keyboard);
        if wants_keyboard != self.keyboard {
            self.keyboard = wants_keyboard;
            ctx.set_keyboard_interactivity(if wants_keyboard {
                KeyboardInteractivity::OnDemand
            } else {
                KeyboardInteractivity::None
            });
        }
    }

    fn on_frame(&mut self, _ctx: SurfaceCtx<'_>) -> bool {
        if self.cards.iter().all(|card| card.animation.at_rest()) && self.decks.at_rest() {
            // Nothing is moving, so the next frame would draw the same pixels.
            // Resetting the clock keeps the first frame after the pause from
            // integrating the whole idle period in one step.
            self.clock.reset();
            return false;
        }
        let dt = self.clock.tick();
        self.decks.step(dt);
        self.cards.retain_mut(|card| card.animation.step(dt));
        true
    }

    fn on_pointer_enter(&mut self, x: f64, y: f64, ctx: SurfaceCtx<'_>) -> bool {
        self.on_pointer_motion(x, y, ctx)
    }

    fn on_pointer_leave(&mut self, _ctx: SurfaceCtx<'_>) -> bool {
        self.press = None;
        let changed = self.hovered.is_some();
        self.hovered = None;
        changed
    }

    fn on_pointer_motion(&mut self, x: f64, y: f64, ctx: SurfaceCtx<'_>) -> bool {
        let mut redraw = self.update_hover(Point::new(x, y));

        let Some(press) = self.press.as_mut() else {
            return redraw;
        };
        let now = Instant::now();
        let elapsed = now.duration_since(press.last_at).as_secs_f64();
        if elapsed > 0.0 {
            press.velocity = (x - press.last_x) / elapsed;
        }
        press.last_x = x;
        press.last_at = now;
        let (id, velocity, travelled) = (press.id, press.velocity, x - press.origin.x);

        let width = {
            let shared = self.shared.borrow();
            card_width(
                Size::new(f64::from(ctx.size.0), f64::from(ctx.size.1)),
                &shared.theme,
            )
        };
        let Some(card) = self.cards.iter_mut().find(|card| card.id == id) else {
            self.press = None;
            return redraw;
        };

        // Only a rightward drag follows the pointer; dragging a card back onto
        // the screen has nowhere to go.
        card.animation.drag = travelled.max(0.0) as f32;
        redraw = true;
        if is_swipe(travelled) {
            card.animation
                .leave_with_velocity(dismiss_velocity(velocity, width));
            self.press = None;
            self.shared.borrow_mut().close(id, CloseReason::Dismissed);
        }
        redraw
    }

    fn on_pointer_press(&mut self, x: f64, y: f64, ctx: SurfaceCtx<'_>) -> bool {
        let point = Point::new(x, y);
        let Some(region) = self.hits.hit(point) else {
            self.press = None;
            return false;
        };
        // Resolved here rather than on release so that a scrubber seeks to
        // where the pointer went *down*, which is where the user was looking.
        let id = region.id;
        let action = resolve_hit(region, point);
        let reply = action == CardAction::FocusReply;
        self.press = Some(Press {
            id,
            action,
            origin: point,
            last_x: x,
            last_at: Instant::now(),
            velocity: 0.0,
        });

        if !reply {
            return false;
        }
        // The reply region is both the collapsed chip and the open field, and
        // the card knows which it is currently wearing: an open field takes the
        // click and puts its caret where the pointer went down, which is what a
        // text field does everywhere else, while the chip ignores it and waits
        // for the release to open the field.
        if let Some(card) = self.card_mut(id) {
            card.on_click(point, ctx.text);
        }
        true
    }

    fn on_pointer_release(&mut self, x: f64, _y: f64, _ctx: SurfaceCtx<'_>) -> bool {
        let Some(press) = self.press.take() else {
            return false;
        };
        if let Some(card) = self.card_mut(press.id) {
            card.animation.drag = 0.0;
        }
        // A press that turned into a swipe took the card down on the motion
        // event that crossed the threshold, so anything still here is a click.
        if (x - press.origin.x).abs() < SWIPE_THRESHOLD {
            self.perform(press.id, &press.action);
        }
        true
    }

    fn on_key_press(&mut self, key: KeyPress<'_>, _ctx: SurfaceCtx<'_>) -> bool {
        let Some(index) = self.cards.iter().position(Card::wants_keyboard) else {
            return false;
        };
        let id = self.cards[index].id;
        let action = self.cards[index].on_key(&key);
        if let Some(action) = action {
            self.perform(id, &action);
        }
        // Repaint unconditionally: the caret moved, or the field closed, and
        // `paint` is where the keyboard interactivity is re-checked.
        true
    }

    fn on_tick(&mut self, _ctx: SurfaceCtx<'_>) -> bool {
        let now = Local::now();
        for card in &mut self.cards {
            card.refresh_time(now);
        }

        self.ticks = self.ticks.wrapping_add(1);
        let mut shared = self.shared.borrow_mut();
        shared.phone.pump();
        if self.ticks.is_multiple_of(ICON_SWEEP_TICKS) {
            shared.icons.sweep();
        }
        !self.cards.is_empty()
    }

    fn needs_redraw(&self) -> bool {
        self.shared.borrow().version != self.seen
    }
}

/// Whether a horizontal drag is far enough to dismiss the card it is on.
///
/// Only rightwards counts: the strip is on the right edge, so that is the
/// direction a card has anywhere to go.
pub(crate) fn is_swipe(travelled: f64) -> bool {
    travelled >= SWIPE_THRESHOLD
}

/// Converts a pointer velocity into the units the leave spring wants.
///
/// [`CardAnimation::leave_with_velocity`] measures velocity in *reveal* units
/// per second — one card width per second is `1.0` — and takes off-screen as
/// negative, so a flick to the right at `p` pixels per second is
/// `-(p / card_width)`. Getting the sign wrong makes a flicked card lunge back
/// onto the screen before it leaves.
///
/// [`CardAnimation::leave_with_velocity`]: crate::ui::CardAnimation::leave_with_velocity
pub(crate) fn dismiss_velocity(pixels_per_second: f64, card_width: f64) -> f32 {
    if card_width <= 0.0 {
        return 0.0;
    }
    -(pixels_per_second / card_width) as f32
}

/// The hovered action, but only for the card the pointer is actually over.
///
/// `Some` doubles as "this card is hovered" in [`RenderCtx`], which is what
/// reveals the close affordance.
fn hovered_action(
    hovered: &Option<(NotificationId, CardAction)>,
    id: NotificationId,
) -> Option<CardAction> {
    match hovered {
        Some((hovered_id, action)) if *hovered_id == id => Some(action.clone()),
        _ => None,
    }
}
