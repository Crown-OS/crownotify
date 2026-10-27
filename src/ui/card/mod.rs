//! The notification cards, and the state each one retains between frames.
//!
//! A [`Card`] lives exactly as long as the notification it draws. That is the
//! whole design: every [`Text`] a card owns holds a shaped Parley layout, and
//! rebuilding those in `paint` — which runs at up to 144 Hz on the compositor's
//! frame clock — is the one cost in this daemon that would actually be felt. So
//! a card is built once, [`sync`](Card::sync)ed only when the store says its
//! notification changed, and [`measure`](Card::measure)d only when its content
//! or its width moved. `paint` after that is arithmetic over cached layouts.
//!
//! Which of the four views a card wears is decided once, from the
//! notification's [`Content`] and the sending app's
//! [`AppProfile`](crate::profile::AppProfile) — a profile may force a card kind
//! for an app that does not know how to ask for one. The views themselves are
//! deliberately forgiving about the payload they get: a music card whose
//! notification carries no [`Music`](crate::model::Music) still draws, because
//! the profile that forced it there knew something the sender did not.

pub mod call;
pub mod chat;
pub mod chrome;
pub mod general;
pub mod music;
pub mod time;
pub mod view;

use chrono::{DateTime, Local};
use crownshell::{
    KeyPress, Scene, Text, TextContext,
    kurbo::{Affine, Point, Rect},
    peniko::{Color, Fill, Mix},
};

use crate::{
    icons::IconLoader,
    model::{Content, Notification, NotificationId},
    profile::{AppProfile, ContentHint, profile_for},
    store::Entry,
    ui::{
        animation::CardAnimation,
        hit::{CardAction, HitMap, HitRegion, MediaAction},
        theme::Theme,
        widget::{ButtonState, Painted, draw_card, seek_percent},
    },
};

pub use call::CallView;
pub use chat::ChatView;
pub(crate) use chrome::{draw_close, draw_icon};
pub use general::GeneralView;
pub use music::MusicView;
pub use time::{relative_time, write_relative_time};
pub use view::CardView;

/// Vertical gap between two lines of a card's text column.
pub(crate) const LINE_GAP: f64 = 2.0;
/// Gap above a row of controls — action buttons, the reply field, transport.
pub(crate) const CONTROL_GAP: f64 = 10.0;
/// Gap between two side-by-side controls.
pub(crate) const CONTROL_SPACING: f64 = 8.0;
/// Gap between the text column and the timestamp sitting to its right.
pub(crate) const STAMP_GAP: f64 = 8.0;
/// Narrowest text column a view will lay text into. A card squeezed below this
/// draws overlapping nonsense; clamping keeps it merely cramped.
pub(crate) const MIN_COLUMN: f64 = 40.0;
/// Inset of the hover-revealed close affordance from the card's top-right
/// corner. Chosen so the whole button stays inside the card, because the hit
/// region it pushes has to be inside the surface's input region.
const CLOSE_INSET: f64 = 6.0;
/// Opacity of the placeholder tile drawn when an icon cannot be loaded.
const PLACEHOLDER_ALPHA: f32 = 0.22;
/// Height of the device badge on a relayed notification's icon. The design's
/// number; anything smaller stops reading as a device at a glance.
const BADGE_HEIGHT: f64 = 14.0;
/// How far the badge hangs past the icon's bottom-right corner. Enough that it
/// reads as sitting *on* the icon rather than inside it, and far short of both
/// the text column and the edge of the card's padding.
const BADGE_OVERHANG: f64 = 2.0;
/// Width of the halo separating the badge from whatever is underneath it.
const BADGE_HALO: f64 = 1.25;
/// Below this the card is faded through a group layer rather than drawn
/// opaquely; at 1.0 exactly the layer would be a pure cost.
const OPAQUE: f32 = 0.999;

/// What a card needs to work out how tall it is: text shaping and the tokens
/// that decide the spacing. No scene, no GPU — which is what lets the whole
/// stack be laid out in a unit test.
pub struct MeasureCtx<'a> {
    pub text: &'a mut TextContext,
    pub theme: &'a Theme,
}

/// Everything a card needs to paint one frame of itself.
pub struct RenderCtx<'a> {
    pub scene: &'a mut Scene,
    pub text: &'a mut TextContext,
    pub icons: &'a mut IconLoader,
    pub theme: &'a Theme,
    pub profile: &'a AppProfile,
    pub hits: &'a mut HitMap,
    /// Card bounds in surface-local logical pixels, animation already applied.
    pub bounds: Rect,
    /// What a bare click on the card's body means.
    ///
    /// [`CardAction::Activate`] everywhere except the top card of a collapsed
    /// deck, where a click pulls the deck apart instead. The card's own
    /// controls are pushed after this and so still win over it.
    pub body: CardAction,
    /// Which of the card material the compositor is not drawing this frame.
    pub painted: Painted,
    pub alpha: f32,
    /// The action the pointer is currently over, if it is over this card at
    /// all. `Some` therefore doubles as "this card is hovered", which is what
    /// reveals the close affordance.
    pub hovered: Option<CardAction>,
}

impl RenderCtx<'_> {
    /// The accent to draw with: the app's, when it has one, otherwise the
    /// theme's.
    pub fn accent(&self) -> Color {
        self.profile.accent_or(self.theme.palette.accent)
    }

    /// How to draw the control that would produce `action`.
    pub fn state(&self, action: &CardAction) -> ButtonState {
        if self.hovered.as_ref() == Some(action) {
            ButtonState::Hover
        } else {
            ButtonState::Normal
        }
    }

    /// Whether the pointer is over this card.
    pub fn is_hovered(&self) -> bool {
        self.hovered.is_some()
    }
}

/// One notification's retained view state.
pub struct Card {
    pub id: NotificationId,
    pub animation: CardAnimation,
    view: Box<dyn CardView>,
    height: f64,
    /// The width [`height`](Self::height) was measured for.
    measured_width: f64,
    /// Set when the content changed under the cached measurement.
    dirty: bool,
    #[cfg(test)]
    measurements: u32,
}

impl Card {
    /// Builds the card for `entry`, entering the stack at `offset`.
    pub fn new(entry: &Entry, theme: &Theme, offset: f32) -> Self {
        let notification = &entry.notification;
        let mut view = view_for(view_kind(notification), notification);
        view.apply_theme(theme);
        view.refresh_time(Local::now());
        Self {
            id: notification.id,
            animation: CardAnimation::entering(offset),
            view,
            height: 0.0,
            measured_width: f64::NAN,
            dirty: true,
            #[cfg(test)]
            measurements: 0,
        }
    }

    /// Re-syncs from the store when the notification is replaced in place.
    ///
    /// Cheap to call on an unchanged entry: every field is compared before it
    /// is written, so an unchanged card keeps both its shaped text and its
    /// cached height. Only a change that could move the card's height clears
    /// the measurement.
    pub fn sync(&mut self, entry: &Entry, theme: &Theme) {
        let notification = &entry.notification;
        self.id = notification.id;
        let kind = view_kind(notification);
        if self.view.kind() != kind {
            self.view = view_for(kind, notification);
            self.dirty = true;
        } else if self.view.sync(notification) {
            self.dirty = true;
        }
        self.view.apply_theme(theme);
        self.view.refresh_time(Local::now());
    }

    /// Re-renders the relative timestamps against `now`.
    ///
    /// Split out of [`sync`](Self::sync) because a timestamp goes stale on the
    /// surface's tick rather than on a store change, and rolling "now" over to
    /// "1m" must not cost a re-measure — it cannot change the card's height.
    pub fn refresh_time(&mut self, now: DateTime<Local>) {
        self.view.refresh_time(now);
    }

    /// Height for `width`, cached until the content or the width changes.
    pub fn measure(&mut self, width: f64, ctx: &mut MeasureCtx<'_>) -> f64 {
        if !self.dirty && self.measured_width == width {
            return self.height;
        }
        self.height = self.view.measure(width, ctx);
        self.measured_width = width;
        self.dirty = false;
        #[cfg(test)]
        {
            self.measurements += 1;
        }
        self.height
    }

    /// The height the last [`measure`](Self::measure) produced. Zero until the
    /// card has been measured once.
    pub fn height(&self) -> f64 {
        self.height
    }

    /// Draws the card and records every clickable rectangle it contains.
    pub fn render(&mut self, ctx: &mut RenderCtx<'_>) {
        // The card fades as a group rather than per element. Text colour lives
        // inside a shaped layout, so fading it by restyling would re-shape
        // every glyph on every frame of an animation — exactly the cost the
        // retained `Text`s exist to avoid — and overlapping translucent
        // elements would blend wrongly against each other besides.
        let faded = ctx.alpha < OPAQUE;
        if faded {
            let metrics = &ctx.theme.metrics;
            let clip = ctx.bounds.inset(metrics.shadow_blur + metrics.shadow_dy);
            ctx.scene.push_layer(
                Fill::NonZero,
                Mix::Normal,
                ctx.alpha.clamp(0.0, 1.0),
                Affine::IDENTITY,
                &clip,
            );
        }

        draw_card(ctx.scene, ctx.bounds, ctx.theme, 1.0, ctx.painted);
        // Pushed first so every control the view adds afterwards wins the click
        // over the body it sits on.
        ctx.hits.push(ctx.bounds, self.id, ctx.body.clone());
        self.view.render(self.id, ctx);
        if ctx.is_hovered() {
            draw_close(self.id, ctx);
        }

        if faded {
            ctx.scene.pop_layer();
        }
    }

    /// Offers a key to the card. Only a card with a focused reply field takes
    /// one; everything else returns `None` so the surface can use it for its
    /// own shortcuts.
    ///
    /// The surface should repaint after any call and re-read
    /// [`wants_keyboard`](Self::wants_keyboard), which a cancelled reply turns
    /// off.
    pub fn on_key(&mut self, key: &KeyPress<'_>) -> Option<CardAction> {
        let (action, resized) = self.view.on_key(key);
        if resized {
            self.dirty = true;
        }
        action
    }

    /// Places the caret from a click at `point`, which is in the surface-local
    /// coordinates the card was last rendered in.
    ///
    /// Only an already-open reply field takes it, and only when the click landed
    /// inside the rectangle that field was drawn in — every other click on a card
    /// is an action, and putting a caret somewhere for it would move the one the
    /// user had already placed. Opening the field is
    /// [`focus_reply`](Self::focus_reply)'s job, not this one's.
    pub fn on_click(&mut self, point: Point, tcx: &mut TextContext) {
        self.view.on_click(point, tcx);
    }

    /// Opens or closes the card's overflow menu, if it has one.
    pub fn toggle_menu(&mut self) {
        if self.view.toggle_menu() {
            self.dirty = true;
        }
    }

    /// Opens and focuses the inline reply field, if this card has one.
    pub fn focus_reply(&mut self) {
        if self.view.focus_reply() {
            self.dirty = true;
        }
    }

    /// Whether the card is holding keyboard focus, which is what raises the
    /// layer surface's `KeyboardInteractivity` to `OnDemand`.
    pub fn wants_keyboard(&self) -> bool {
        self.view.wants_keyboard()
    }

    /// Drains the inline reply, for a [`CardAction::SubmitReply`].
    ///
    /// `None` when the card has no reply field or the field is empty — an
    /// empty reply is a mis-click, not a message.
    pub fn take_reply(&mut self) -> Option<String> {
        let taken = self.view.take_reply();
        if taken.is_some() {
            self.dirty = true;
        }
        taken
    }

    /// Whether the card would re-measure if it were measured now. Exposed for
    /// the surface's dirty tracking and for the caching tests.
    pub fn needs_measure(&self) -> bool {
        self.dirty
    }

    #[cfg(test)]
    fn measurements(&self) -> u32 {
        self.measurements
    }
}

/// Builds the retained view for `kind`.
///
/// Boxed because the kinds differ by more than a kilobyte — a chat card carries
/// a whole editable field — and this is stored per card, moved when the stack is
/// re-ordered and swapped wholesale when a replacement changes a notification's
/// kind. One allocation per notification is not a cost worth counting; carrying
/// the largest view's footprint in every card is.
fn view_for(kind: ContentHint, notification: &Notification) -> Box<dyn CardView> {
    match kind {
        ContentHint::General => Box::new(GeneralView::new(notification)),
        ContentHint::Call => Box::new(CallView::new(notification)),
        ContentHint::Chat => Box::new(ChatView::new(notification)),
        ContentHint::Music => Box::new(MusicView::new(notification)),
    }
}

/// Which view a notification is drawn with.
///
/// The app profile wins over the notification's own [`Content`]: an app that
/// posts its now-playing state as a plain notification has no way to ask for a
/// music card, and the registry is where that knowledge lives. Going through
/// [`profile_for`] rather than reading a table means a config-driven profile
/// will change this with no edit here.
fn view_kind(notification: &Notification) -> ContentHint {
    if let Some(hint) = profile_for(&notification.app.key()).content {
        return hint;
    }
    match &notification.content {
        Content::Call(_) => ContentHint::Call,
        Content::Chat(_) => ContentHint::Chat,
        Content::Music(_) => ContentHint::Music,
        // A progress notification is a general card with a bar in it, not a
        // shape of its own.
        Content::General | Content::Progress(_) => ContentHint::General,
    }
}

/// Turns a hit on a card into the action to perform.
///
/// Every action is fully determined by the region it was pushed under except a
/// seek, whose percentage is *where in the scrubber* the click landed and so
/// cannot be baked into a [`HitRegion`]. Routing every click through here means
/// the surface never has to know which is which.
pub fn resolve_hit(region: &HitRegion, point: Point) -> CardAction {
    match region.action {
        CardAction::Media(MediaAction::Seek(_)) => {
            CardAction::Media(MediaAction::Seek(seek_percent(region.rect, point.x)))
        }
        _ => region.action.clone(),
    }
}

/// One row of a card's text column: where its top edge sits relative to the top
/// of the column, and how tall it is.
///
/// A zero height means the row is not drawn. Views compute the whole column as
/// a set of these once, so `measure` and `render` walk exactly the same list
/// and cannot drift apart into a card whose contents do not fit it.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct Row {
    pub top: f64,
    pub height: f64,
}

impl Row {
    pub fn shown(self) -> bool {
        self.height > 0.0
    }
}

/// Replaces `text`'s content, reporting whether it actually differed.
///
/// [`Text::set_text`] already compares before invalidating; this returns the
/// answer as well, which is what lets a card know its cached height is stale.
pub(crate) fn set_text(text: &mut Text, value: &str) -> bool {
    if text.text() == value {
        return false;
    }
    text.set_text(value);
    true
}

/// The rectangle inside a card's padding.
pub(crate) fn content_rect(bounds: Rect, theme: &Theme) -> Rect {
    let metrics = &theme.metrics;
    Rect::new(
        bounds.x0 + metrics.pad_x,
        bounds.y0 + metrics.pad_y,
        bounds.x1 - metrics.pad_x,
        bounds.y1 - metrics.pad_y,
    )
}

/// Width of the text column of a card `width` wide, with a leading icon column.
pub(crate) fn column_width(width: f64, theme: &Theme, icon: f64) -> f64 {
    let metrics = &theme.metrics;
    (width - 2.0 * metrics.pad_x - icon - metrics.icon_gap).max(MIN_COLUMN)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::{
        model::{Action, ActionKind, Chat, DeviceKind, Music, RelayDevice, Urgency},
        ui::card::chrome::badge_bounds,
        ui::hit::HitMap,
    };
    use chrono::{Duration as ChronoDuration, TimeZone};
    use std::time::Instant;

    /// A long body, so a wrapped column needs more than four lines of it.
    pub(crate) const LONG_BODY: &str = "The quick brown fox jumps over the lazy dog while the \
         compositor waits for a frame, and then it does it again, and again, and once more for \
         luck, until there is very obviously more text here than any notification should carry.";

    pub(crate) fn entry(notification: Notification) -> Entry {
        Entry {
            notification,
            received: Instant::now(),
            dismissed: false,
        }
    }

    pub(crate) fn general(body: &str) -> Entry {
        entry(
            Notification::builder("Mail")
                .id(1)
                .summary("Ada Lovelace")
                .body(body)
                .build(),
        )
    }

    pub(crate) fn card(entry: &Entry) -> Card {
        Card::new(entry, &Theme::light(), 0.0)
    }

    pub(crate) fn measure(card: &mut Card, tcx: &mut TextContext, theme: &Theme) -> f64 {
        card.measure(
            theme.metrics.card_width,
            &mut MeasureCtx { text: tcx, theme },
        )
    }

    /// Paints `card` into a throwaway scene and returns the hits it recorded.
    pub(crate) fn paint(card: &mut Card, tcx: &mut TextContext, theme: &Theme) -> HitMap {
        paint_hovered(card, tcx, theme, None)
    }

    pub(crate) fn paint_hovered(
        card: &mut Card,
        tcx: &mut TextContext,
        theme: &Theme,
        hovered: Option<CardAction>,
    ) -> HitMap {
        paint_into(&mut Scene::new(), card, tcx, theme, hovered)
    }

    /// [`paint`], into a scene the caller can then inspect.
    pub(crate) fn paint_into(
        scene: &mut Scene,
        card: &mut Card,
        tcx: &mut TextContext,
        theme: &Theme,
        hovered: Option<CardAction>,
    ) -> HitMap {
        let height = measure(card, tcx, theme);
        let bounds = Rect::new(12.0, 20.0, 12.0 + theme.metrics.card_width, 20.0 + height);
        let mut icons = IconLoader::new();
        let mut hits = HitMap::new();
        card.render(&mut RenderCtx {
            painted: Painted::ALL,
            body: CardAction::Activate,
            scene,
            text: tcx,
            icons: &mut icons,
            theme,
            profile: crate::profile::default_profile(),
            hits: &mut hits,
            bounds,
            alpha: 1.0,
            hovered,
        });
        hits
    }

    #[test]
    fn a_card_takes_its_view_from_the_notification_content() {
        let theme = Theme::light();
        let plain = Card::new(&general(""), &theme, 0.0);
        assert_eq!(plain.view.kind(), ContentHint::General);

        let chat = Card::new(
            &entry(
                Notification::builder("Signal")
                    .content(Content::Chat(Chat {
                        conversation_id: "t1".into(),
                        sender: "Ada".into(),
                        avatar: None,
                        sent_at: Local::now(),
                    }))
                    .build(),
            ),
            &theme,
            0.0,
        );
        assert_eq!(chat.view.kind(), ContentHint::Chat);
    }

    /// The registry is what turns Spotify's plain notification into a remote
    /// control; if the profile stopped being consulted, nothing else would.
    #[test]
    fn an_app_profile_overrides_the_content_the_sender_asked_for() {
        let card = Card::new(
            &entry(
                Notification::builder("Spotify")
                    .summary("Blue Monday")
                    .body("New Order")
                    .build(),
            ),
            &Theme::light(),
            0.0,
        );
        assert_eq!(
            card.view.kind(),
            ContentHint::Music,
            "the Spotify profile did not force a music card"
        );
    }

    #[test]
    fn a_replacement_that_changes_the_content_swaps_the_view() {
        let theme = Theme::light();
        let mut card = card(&general("hello"));
        assert_eq!(card.view.kind(), ContentHint::General);

        card.sync(
            &entry(
                Notification::builder("Player")
                    .id(1)
                    .content(Content::Music(Music {
                        title: "Blue Monday".into(),
                        artist: "New Order".into(),
                        album_art: None,
                        playing: true,
                        percent: 40,
                    }))
                    .build(),
            ),
            &theme,
        );
        assert_eq!(card.view.kind(), ContentHint::Music);
        assert!(
            card.needs_measure(),
            "a new view must invalidate the height"
        );
    }

    #[test]
    fn a_longer_body_makes_a_taller_card() {
        let theme = Theme::light();
        let mut tcx = TextContext::new();

        let mut short = card(&general("See you at five."));
        let mut long = card(&general(
            "See you at five, by the fountain, the one with the bronze fish on it.",
        ));

        let short_height = measure(&mut short, &mut tcx, &theme);
        let long_height = measure(&mut long, &mut tcx, &theme);
        assert!(
            long_height > short_height,
            "a body that wraps onto a second line did not grow the card: {long_height} vs \
             {short_height}"
        );
    }

    #[test]
    fn the_four_line_body_clamp_holds_however_much_more_text_arrives() {
        let theme = Theme::light();
        let mut tcx = TextContext::new();

        let mut clamped = card(&general(LONG_BODY));
        let mut longer = card(&general(&format!("{LONG_BODY} {LONG_BODY} {LONG_BODY}")));

        let clamped_height = measure(&mut clamped, &mut tcx, &theme);
        let longer_height = measure(&mut longer, &mut tcx, &theme);
        assert_eq!(
            clamped_height, longer_height,
            "tripling an already-clamped body changed the card height"
        );

        let mut two_lines = card(&general(
            "See you at five, by the fountain, the one with the bronze fish on it.",
        ));
        assert!(
            clamped_height > measure(&mut two_lines, &mut tcx, &theme),
            "the clamped card is not taller than a two-line one"
        );
    }

    #[test]
    fn a_card_with_actions_is_taller_than_the_same_card_without() {
        let theme = Theme::light();
        let mut tcx = TextContext::new();

        let mut bare = card(&general("Ready when you are."));
        let mut with_actions = card(&entry(
            Notification::builder("Mail")
                .id(2)
                .summary("Ada Lovelace")
                .body("Ready when you are.")
                .action(Action {
                    key: "archive".into(),
                    label: "Archive".into(),
                    kind: ActionKind::Button,
                })
                .build(),
        ));

        let bare_height = measure(&mut bare, &mut tcx, &theme);
        let action_height = measure(&mut with_actions, &mut tcx, &theme);
        assert!(
            action_height > bare_height + theme.metrics.button,
            "an action row added only {} px",
            action_height - bare_height
        );
    }

    /// A critical notification wears an eyebrow, which costs it a line.
    #[test]
    fn a_critical_notification_is_taller_because_of_its_eyebrow() {
        let theme = Theme::light();
        let mut tcx = TextContext::new();

        let mut normal = card(&general(LONG_BODY));
        let mut critical = card(&entry(
            Notification::builder("Mail")
                .id(3)
                .summary("Ada Lovelace")
                .body(LONG_BODY)
                .urgency(Urgency::Critical)
                .build(),
        ));

        assert!(
            measure(&mut critical, &mut tcx, &theme) > measure(&mut normal, &mut tcx, &theme),
            "the eyebrow did not make room for itself"
        );
    }

    #[test]
    fn measurement_is_cached_until_the_content_or_the_width_moves() {
        let theme = Theme::light();
        let mut tcx = TextContext::new();
        let source = general("See you at five.");
        let mut card = card(&source);

        let height = measure(&mut card, &mut tcx, &theme);
        assert_eq!(card.measurements(), 1);

        for _ in 0..8 {
            assert_eq!(measure(&mut card, &mut tcx, &theme), height);
        }
        assert_eq!(
            card.measurements(),
            1,
            "re-measuring an unchanged card re-laid it out"
        );

        card.sync(&source, &theme);
        assert_eq!(measure(&mut card, &mut tcx, &theme), height);
        assert_eq!(
            card.measurements(),
            1,
            "syncing an unchanged notification invalidated the measurement"
        );

        let narrower = theme.metrics.card_width - 60.0;
        card.measure(
            narrower,
            &mut MeasureCtx {
                text: &mut tcx,
                theme: &theme,
            },
        );
        assert_eq!(card.measurements(), 2, "a new width must re-measure");

        card.sync(&general("Something else entirely, and longer too."), &theme);
        assert!(card.needs_measure());
        measure(&mut card, &mut tcx, &theme);
        assert_eq!(card.measurements(), 3);
    }

    #[test]
    fn every_hit_region_lands_inside_the_card_that_pushed_it() {
        let theme = Theme::light();
        let mut tcx = TextContext::new();
        let mut card = card(&entry(
            Notification::builder("Mail")
                .id(4)
                .summary("Ada Lovelace")
                .body(LONG_BODY)
                .action(Action {
                    key: "archive".into(),
                    label: "Archive".into(),
                    kind: ActionKind::Button,
                })
                .action(Action {
                    key: "trash".into(),
                    label: "Delete".into(),
                    kind: ActionKind::Button,
                })
                .build(),
        ));

        let hits = paint_hovered(&mut card, &mut tcx, &theme, Some(CardAction::Activate));
        let height = card.height();
        let bounds = Rect::new(12.0, 20.0, 12.0 + theme.metrics.card_width, 20.0 + height);

        assert!(hits.len() >= 4, "expected body, close and two actions");
        for rect in hits.rects() {
            assert!(
                rect.x0 >= bounds.x0 - 0.01
                    && rect.x1 <= bounds.x1 + 0.01
                    && rect.y0 >= bounds.y0 - 0.01
                    && rect.y1 <= bounds.y1 + 0.01,
                "{rect:?} escaped the card bounds {bounds:?}"
            );
        }
    }

    #[test]
    fn a_click_on_the_body_activates_and_a_click_on_the_close_dismisses() {
        let theme = Theme::light();
        let mut tcx = TextContext::new();
        let mut card = card(&general("See you at five."));

        let hits = paint_hovered(&mut card, &mut tcx, &theme, Some(CardAction::Activate));
        let bounds = Rect::new(
            12.0,
            20.0,
            12.0 + theme.metrics.card_width,
            20.0 + card.height(),
        );

        assert_eq!(
            hits.hit(bounds.center()).map(|hit| &hit.action),
            Some(&CardAction::Activate)
        );
        let close = Point::new(bounds.x1 - CLOSE_INSET - 6.0, bounds.y0 + CLOSE_INSET + 6.0);
        assert_eq!(
            hits.hit(close).map(|hit| &hit.action),
            Some(&CardAction::Dismiss),
            "the close affordance did not win over the card body"
        );
    }

    /// A resting card is drawn straight into the scene; a fading one goes
    /// through one group layer so its text fades with everything else without
    /// being re-shaped.
    #[test]
    fn a_fading_card_is_drawn_through_a_single_group_layer() {
        let theme = Theme::light();
        let mut tcx = TextContext::new();
        let mut card = card(&general("See you at five."));
        let height = measure(&mut card, &mut tcx, &theme);
        let bounds = Rect::new(0.0, 0.0, theme.metrics.card_width, height);

        let mut opaque = Scene::new();
        let mut faded = Scene::new();
        for (scene, alpha) in [(&mut opaque, 1.0_f32), (&mut faded, 0.4_f32)] {
            let mut icons = IconLoader::new();
            let mut hits = HitMap::new();
            card.render(&mut RenderCtx {
                painted: Painted::ALL,
                body: CardAction::Activate,
                scene,
                text: &mut tcx,
                icons: &mut icons,
                theme: &theme,
                profile: crate::profile::default_profile(),
                hits: &mut hits,
                bounds,
                alpha,
                hovered: None,
            });
        }

        assert_eq!(
            faded.encoding().n_clips,
            opaque.encoding().n_clips + 2,
            "expected exactly one extra layer (a begin and an end clip) when fading"
        );
    }

    fn relayed(kind: DeviceKind) -> Entry {
        entry(
            Notification::builder("Mail")
                .id(5)
                .summary("Ada Lovelace")
                .body("See you at five.")
                .relayed_from(RelayDevice::new(kind, "Ada's phone"))
                .build(),
        )
    }

    /// The badge is the only thing that says a notification is also sitting on
    /// the user's phone, and it is drawn into padding the card already has — so
    /// it must show up in the scene and must not move the stack underneath it.
    #[test]
    fn a_relayed_card_wears_a_device_badge_and_is_no_taller_for_it() {
        let theme = Theme::light();
        let mut tcx = TextContext::new();
        let mut local = card(&general("See you at five."));
        let mut phone = card(&relayed(DeviceKind::Phone));

        assert_eq!(
            measure(&mut phone, &mut tcx, &theme),
            measure(&mut local, &mut tcx, &theme),
            "the badge changed the card's height"
        );

        let (mut plain, mut badged) = (Scene::new(), Scene::new());
        paint_into(&mut plain, &mut local, &mut tcx, &theme, None);
        paint_into(&mut badged, &mut phone, &mut tcx, &theme, None);
        assert!(
            badged.encoding().n_paths > plain.encoding().n_paths,
            "the relayed card drew no badge: {} paths against {}",
            badged.encoding().n_paths,
            plain.encoding().n_paths
        );
    }

    #[test]
    fn every_device_kind_draws_a_badge_of_its_own_shape() {
        let theme = Theme::light();
        let mut tcx = TextContext::new();
        let mut local = card(&general("See you at five."));
        let mut plain = Scene::new();
        paint_into(&mut plain, &mut local, &mut tcx, &theme, None);

        let mut widths = Vec::new();
        for kind in [
            DeviceKind::Phone,
            DeviceKind::Tablet,
            DeviceKind::Watch,
            DeviceKind::Computer,
        ] {
            let mut card = card(&relayed(kind));
            let mut scene = Scene::new();
            paint_into(&mut scene, &mut card, &mut tcx, &theme, None);
            assert!(
                scene.encoding().n_paths > plain.encoding().n_paths,
                "{kind:?} drew nothing"
            );
            widths.push(badge_bounds(Rect::new(0.0, 0.0, 38.0, 38.0), kind).width());
        }
        assert!(
            widths.iter().any(|width| *width != widths[0]),
            "every device came out the same width: {widths:?}"
        );
    }

    #[test]
    fn a_badge_overlaps_the_icons_corner_without_leaving_the_card() {
        let metrics = Theme::light().metrics;
        let bounds = Rect::new(
            0.0,
            0.0,
            metrics.card_width,
            2.0 * metrics.pad_y + metrics.icon,
        );
        let icon = Rect::new(
            metrics.pad_x,
            metrics.pad_y,
            metrics.pad_x + metrics.icon,
            metrics.pad_y + metrics.icon,
        );

        for kind in [
            DeviceKind::Phone,
            DeviceKind::Tablet,
            DeviceKind::Watch,
            DeviceKind::Computer,
        ] {
            let badge = badge_bounds(icon, kind);
            assert!(
                badge.x1 > icon.x1 && badge.y1 > icon.y1,
                "{kind:?} does not overlap the icon's corner: {badge:?}"
            );
            assert!(
                icon.contains(badge.center()),
                "{kind:?} sits beside the icon rather than on it: {badge:?}"
            );
            assert_eq!(
                bounds.union(badge),
                bounds,
                "{kind:?} escaped the card: {badge:?}"
            );
            assert!(
                badge.x1 < icon.x1 + metrics.icon_gap,
                "{kind:?} reaches into the text column: {badge:?}"
            );
        }

        // An icon smaller than the badge is overlapped, not swamped by it.
        let tiny = Rect::new(0.0, 0.0, 8.0, 8.0);
        let badge = badge_bounds(tiny, DeviceKind::Phone);
        assert!(badge.height() <= tiny.height(), "{badge:?}");
    }

    #[test]
    fn a_seek_is_resolved_from_where_the_click_landed_and_nothing_else_is() {
        let region = HitRegion {
            rect: Rect::new(20.0, 100.0, 220.0, 116.0),
            id: 7,
            action: CardAction::Media(MediaAction::Seek(0)),
        };
        assert_eq!(
            resolve_hit(&region, Point::new(120.0, 108.0)),
            CardAction::Media(MediaAction::Seek(50))
        );
        assert_eq!(
            resolve_hit(&region, Point::new(-40.0, 108.0)),
            CardAction::Media(MediaAction::Seek(0))
        );

        let play = HitRegion {
            rect: region.rect,
            id: 7,
            action: CardAction::Media(MediaAction::Play),
        };
        assert_eq!(
            resolve_hit(&play, Point::new(120.0, 108.0)),
            CardAction::Media(MediaAction::Play),
            "a transport button must not be rewritten into a seek"
        );
    }

    fn at(hour: u32, minute: u32) -> DateTime<Local> {
        Local
            .with_ymd_and_hms(2026, 8, 9, hour, minute, 0)
            .single()
            .expect("2026-08-09 is not an ambiguous local time")
    }

    #[test]
    fn relative_times_walk_from_now_through_yesterday_to_a_date() {
        let now = at(21, 30);

        assert_eq!(relative_time(now, now), "now");
        assert_eq!(relative_time(now, now - ChronoDuration::seconds(59)), "now");
        assert_eq!(
            relative_time(now, now + ChronoDuration::minutes(5)),
            "now",
            "a clock that jumped backwards must not print a negative age"
        );

        assert_eq!(relative_time(now, now - ChronoDuration::seconds(60)), "1m");
        assert_eq!(relative_time(now, now - ChronoDuration::minutes(5)), "5m");
        assert_eq!(relative_time(now, now - ChronoDuration::minutes(59)), "59m");

        assert_eq!(
            relative_time(now, at(9, 41)),
            "9:41 AM",
            "anything older than an hour but from today shows a wall clock"
        );
        assert_eq!(relative_time(now, at(13, 5)), "1:05 PM");

        assert_eq!(
            relative_time(at(0, 30), at(23, 0) - ChronoDuration::days(1)),
            "Yesterday",
            "an hour and a half ago across midnight is yesterday, not a time"
        );
        assert_eq!(
            relative_time(now, now - ChronoDuration::days(1)),
            "Yesterday"
        );
        assert_eq!(relative_time(now, now - ChronoDuration::days(6)), "3 Aug");
    }

    #[test]
    fn writing_a_relative_time_reuses_the_buffer_it_was_given() {
        let now = at(21, 30);
        let mut buffer = String::from("Yesterday");
        let capacity = buffer.capacity();

        write_relative_time(&mut buffer, now, now);
        assert_eq!(buffer, "now", "the previous value was not cleared");
        assert_eq!(buffer.capacity(), capacity, "the buffer was re-allocated");
    }
}
