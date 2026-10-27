//! Pill buttons and circular icon buttons.
//!
//! Both are drawn immediately — there is no widget tree — so a button carries
//! no position of its own: the card lays it out, hands it a rectangle and
//! pushes the matching [`CardAction`] into the [`HitMap`]. What a button *does*
//! own is its label's [`Text`], because that holds a shaped layout that would
//! otherwise be rebuilt on every frame.
//!
//! [`CardAction`]: crate::ui::hit::CardAction
//! [`HitMap`]: crate::ui::hit::HitMap

use std::f64::consts::PI;

use crownshell::{
    Scene, Text, TextContext,
    kurbo::{Affine, Arc, BezPath, Cap, Circle, Point, Rect, RoundedRect, Shape, Size, Stroke},
    peniko::{Color, Fill},
};

use crate::ui::{theme::Theme, widget::surface::multiply_alpha};

/// Horizontal padding either side of a pill button's label.
const PILL_PAD_X: f64 = 12.0;
/// How far a hovered surface moves towards its own foreground colour.
const HOVER_MIX: f32 = 0.10;
/// Ditto while pressed. Deeper, so a press is unmistakable on a touchpad where
/// there is no cursor to watch.
const PRESSED_MIX: f32 = 0.20;
/// Fraction of an icon button's diameter its glyph occupies.
const GLYPH_RATIO: f64 = 0.5;

/// How a button is currently being pointed at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ButtonState {
    #[default]
    Normal,
    Hover,
    Pressed,
}

impl ButtonState {
    /// The state implied by the pointer, with a press winning over a hover.
    pub fn from_pointer(hovered: bool, pressed: bool) -> Self {
        match (hovered, pressed) {
            (_, true) => Self::Pressed,
            (true, false) => Self::Hover,
            (false, false) => Self::Normal,
        }
    }

    /// How far this state blends the foreground into the background.
    fn mix(self) -> f32 {
        match self {
            Self::Normal => 0.0,
            Self::Hover => HOVER_MIX,
            Self::Pressed => PRESSED_MIX,
        }
    }
}

/// How a pill button is filled.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ButtonStyle {
    /// A neutral chip on the card material — the usual freedesktop action.
    Plain,
    /// Accent-coloured label on a faint accent wash, for the one action a card
    /// wants to lead with.
    Tinted,
    /// Solid, with a white label. Reserved for the loud ones: Answer, Decline.
    Filled(Color),
}

/// A rounded-rect button with a text label.
///
/// Retained by the card that owns it. `set_label` only invalidates the layout
/// when the text actually changes, so a button whose label never moves costs
/// nothing to draw again.
pub struct PillButton {
    label: Text,
    style: ButtonStyle,
}

impl PillButton {
    pub fn new(label: impl Into<String>, style: ButtonStyle) -> Self {
        Self {
            label: Text::new(label),
            style,
        }
    }

    pub fn label(&self) -> &str {
        self.label.text()
    }

    pub fn set_label(&mut self, label: &str) {
        self.label.set_text(label);
    }

    pub fn style(&self) -> ButtonStyle {
        self.style
    }

    pub fn set_style(&mut self, style: ButtonStyle) {
        self.style = style;
    }

    /// The size this button wants: its label plus the pill's padding, at
    /// `Metrics::button` tall.
    ///
    /// Laying the label out is what makes this accurate, and it is also what
    /// [`render`](Self::render) needs done, so measuring first costs nothing.
    pub fn measure(&mut self, tcx: &mut TextContext, theme: &Theme) -> Size {
        self.apply_style(theme);
        Size::new(
            self.label.width(tcx) + 2.0 * PILL_PAD_X,
            theme.metrics.button,
        )
    }

    /// Draws the button filling `bounds`, faded by `alpha`.
    pub fn render(
        &mut self,
        scene: &mut Scene,
        bounds: Rect,
        theme: &Theme,
        state: ButtonState,
        alpha: f32,
        tcx: &mut TextContext,
    ) {
        self.apply_style(theme);
        let (background, foreground) = self.colors(theme);
        let fill = mix(background, foreground, state.mix());

        scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            multiply_alpha(fill, alpha),
            None,
            &RoundedRect::from_rect(bounds, bounds.height() / 2.0),
        );

        let size = self.label.size(tcx);
        let origin = (
            (bounds.x0 + (bounds.width() - size.width) / 2.0).round(),
            (bounds.y0 + (bounds.height() - size.height) / 2.0).round(),
        );
        self.label.draw(tcx, scene, origin);
    }

    /// The pill's fill and label colours for its style.
    fn colors(&self, theme: &Theme) -> (Color, Color) {
        match self.style {
            ButtonStyle::Plain => (theme.palette.field, theme.palette.primary),
            ButtonStyle::Tinted => (theme.palette.accent.with_alpha(0.16), theme.palette.accent),
            ButtonStyle::Filled(fill) => (fill, Color::WHITE),
        }
    }

    /// Re-syncs the label's style with the theme.
    ///
    /// The label wears the app-name role in the button's own foreground colour.
    /// Building that style clones the family stack, so the colour is checked
    /// first: this runs from both `measure` and `render`, every frame.
    fn apply_style(&mut self, theme: &Theme) {
        let color = self.colors(theme).1;
        let current = self.label.style();
        if current.color == color && current.size == theme.text.app.size {
            return;
        }
        self.label
            .set_style(theme.text.app.clone().with_color(color));
    }
}

/// A circular button carrying one of the [`Symbol`]s.
///
/// Unlike [`PillButton`] this holds no layout, so it is cheap to build inline
/// in a paint; the card keeps the colours in it rather than the button.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IconButton {
    pub symbol: Symbol,
    /// Disc colour, before the hover/press tint.
    pub fill: Color,
    /// Colour of the symbol drawn on the disc.
    pub glyph: Color,
    pub diameter: f64,
}

impl IconButton {
    pub fn new(symbol: Symbol, fill: Color, glyph: Color, diameter: f64) -> Self {
        Self {
            symbol,
            fill,
            glyph,
            diameter,
        }
    }

    /// The green answer button of a call card.
    pub fn answer(theme: &Theme) -> Self {
        Self::new(
            Symbol::Phone,
            theme.palette.positive,
            Color::WHITE,
            theme.metrics.button,
        )
    }

    /// The red decline button of a call card.
    pub fn decline(theme: &Theme) -> Self {
        Self::new(
            Symbol::PhoneDown,
            theme.palette.negative,
            Color::WHITE,
            theme.metrics.button,
        )
    }

    /// The close affordance a hovered card reveals.
    /// The chevron that opens a card's overflow menu.
    pub fn chevron(theme: &Theme) -> Self {
        Self::new(
            Symbol::ChevronDown,
            theme.palette.field,
            theme.palette.secondary,
            theme.metrics.button * 0.8,
        )
    }

    pub fn close(theme: &Theme) -> Self {
        Self::new(
            Symbol::Close,
            theme.palette.separator,
            theme.palette.secondary,
            theme.metrics.button * 0.6,
        )
    }

    /// The square this button occupies when centred on `center` — the rect to
    /// push into the [`HitMap`](crate::ui::hit::HitMap).
    pub fn bounds(&self, center: Point) -> Rect {
        Rect::from_center_size(center, Size::new(self.diameter, self.diameter))
    }

    pub fn render(&self, scene: &mut Scene, center: Point, state: ButtonState, alpha: f32) {
        let fill = mix(self.fill, self.glyph, state.mix());
        scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            multiply_alpha(fill, alpha),
            None,
            &Circle::new(center, self.diameter / 2.0),
        );
        draw_symbol(
            scene,
            self.symbol,
            center,
            self.diameter * GLYPH_RATIO,
            multiply_alpha(self.glyph, alpha),
        );
    }
}

/// The small set of glyphs the cards draw themselves.
///
/// Anything richer — an app icon, an album cover — comes from `IconLoader`.
/// These are here because they are controls rather than content: they must be
/// present and correctly coloured even on a system with no icon theme
/// installed, which is exactly what a missing "hang up" button would ruin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Symbol {
    /// Handset, tilted to answer.
    Phone,
    /// The same handset rotated, which is the universal hang-up.
    PhoneDown,
    Play,
    Pause,
    Next,
    Previous,
    Close,
    /// Arrow for submitting an inline reply.
    Send,
    /// Chevron revealing the actions that did not fit on the card.
    ChevronDown,
}

/// Draws `symbol` centred on `center`, fitted to a `size`-wide square.
pub fn draw_symbol(scene: &mut Scene, symbol: Symbol, center: Point, size: f64, color: Color) {
    match symbol {
        Symbol::Phone => draw_handset(scene, center, size, color, -PI / 4.0),
        Symbol::PhoneDown => draw_handset(scene, center, size, color, PI * 0.75),
        Symbol::Play => fill_polygon(scene, center, size, color, &PLAY),
        Symbol::Pause => {
            for band in [(-0.34, -0.10), (0.10, 0.34)] {
                let rect = Rect::new(
                    center.x + band.0 * size,
                    center.y - 0.36 * size,
                    center.x + band.1 * size,
                    center.y + 0.36 * size,
                );
                scene.fill(
                    Fill::NonZero,
                    Affine::IDENTITY,
                    color,
                    None,
                    &RoundedRect::from_rect(rect, 0.06 * size),
                );
            }
        }
        Symbol::Next => {
            fill_polygon(scene, center, size, color, &NEXT);
            fill_bar(scene, center, size, color, 0.30);
        }
        Symbol::Previous => {
            fill_polygon(scene, center, size, color, &PREVIOUS);
            fill_bar(scene, center, size, color, -0.38);
        }
        Symbol::Close => {
            for (x0, y0, x1, y1) in [(-0.3, -0.3, 0.3, 0.3), (-0.3, 0.3, 0.3, -0.3)] {
                let mut path = BezPath::new();
                path.move_to((center.x + x0 * size, center.y + y0 * size));
                path.line_to((center.x + x1 * size, center.y + y1 * size));
                stroke_glyph(scene, &path, color, size * 0.14, Affine::IDENTITY);
            }
        }
        Symbol::ChevronDown => {
            let mut path = BezPath::new();
            path.move_to((center.x - 0.28 * size, center.y - 0.12 * size));
            path.line_to((center.x, center.y + 0.16 * size));
            path.line_to((center.x + 0.28 * size, center.y - 0.12 * size));
            stroke_glyph(scene, &path, color, size * 0.14, Affine::IDENTITY);
        }
        Symbol::Send => {
            let mut path = BezPath::new();
            path.move_to((center.x, center.y + 0.38 * size));
            path.line_to((center.x, center.y - 0.38 * size));
            path.move_to((center.x - 0.28 * size, center.y - 0.10 * size));
            path.line_to((center.x, center.y - 0.38 * size));
            path.line_to((center.x + 0.28 * size, center.y - 0.10 * size));
            stroke_glyph(scene, &path, color, size * 0.14, Affine::IDENTITY);
        }
    }
}

/// Triangles as (x, y) fractions of the glyph box.
const PLAY: [(f64, f64); 3] = [(-0.26, -0.38), (0.38, 0.0), (-0.26, 0.38)];
const NEXT: [(f64, f64); 3] = [(-0.38, -0.34), (0.20, 0.0), (-0.38, 0.34)];
const PREVIOUS: [(f64, f64); 3] = [(0.38, -0.34), (-0.20, 0.0), (0.38, 0.34)];

fn fill_polygon(scene: &mut Scene, center: Point, size: f64, color: Color, points: &[(f64, f64)]) {
    let mut path = BezPath::new();
    for (index, (x, y)) in points.iter().enumerate() {
        let point = (center.x + x * size, center.y + y * size);
        if index == 0 {
            path.move_to(point);
        } else {
            path.line_to(point);
        }
    }
    path.close_path();
    scene.fill(Fill::NonZero, Affine::IDENTITY, color, None, &path);
}

/// The end stop of a skip-track glyph, at `x` in glyph-box fractions.
fn fill_bar(scene: &mut Scene, center: Point, size: f64, color: Color, x: f64) {
    let rect = Rect::new(
        center.x + x * size,
        center.y - 0.34 * size,
        center.x + (x + 0.08) * size,
        center.y + 0.34 * size,
    );
    scene.fill(
        Fill::NonZero,
        Affine::IDENTITY,
        color,
        None,
        &RoundedRect::from_rect(rect, 0.04 * size),
    );
}

/// A telephone handset: a fat, round-capped arc. `rotation` turns it about its
/// centre, which is the whole difference between answering and hanging up.
fn draw_handset(scene: &mut Scene, center: Point, size: f64, color: Color, rotation: f64) {
    let arc = Arc::new(center, (size * 0.3, size * 0.3), PI * 0.28, PI * 1.44, 0.0);
    let path = arc.to_path(0.05);
    stroke_glyph(
        scene,
        &path,
        color,
        size * 0.26,
        Affine::rotate_about(rotation, center),
    );
}

fn stroke_glyph(scene: &mut Scene, path: &BezPath, color: Color, width: f64, transform: Affine) {
    scene.stroke(
        &Stroke::new(width).with_caps(Cap::Round),
        transform,
        color,
        None,
        path,
    );
}

/// Blends `over` into `under` by `t`, keeping `under`'s alpha.
///
/// The alpha matters: button backgrounds are translucent so the blurred card
/// shows through them, and a hover that quietly made them opaque would punch a
/// hole in the material. Mixing towards the *foreground* colour also means the
/// tint moves in the right direction for both a dark label on a light chip and
/// a white glyph on a saturated disc, with no per-appearance special case.
fn mix(under: Color, over: Color, t: f32) -> Color {
    if t == 0.0 {
        return under;
    }
    let [ur, ug, ub, ua] = under.components;
    let [or, og, ob, _] = over.components;
    Color::new([
        ur + (or - ur) * t,
        ug + (og - ug) * t,
        ub + (ob - ub) * t,
        ua,
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_press_wins_over_a_hover() {
        assert_eq!(ButtonState::from_pointer(false, false), ButtonState::Normal);
        assert_eq!(ButtonState::from_pointer(true, false), ButtonState::Hover);
        assert_eq!(ButtonState::from_pointer(true, true), ButtonState::Pressed);
        assert!(ButtonState::Pressed.mix() > ButtonState::Hover.mix());
        assert_eq!(ButtonState::Normal.mix(), 0.0);
    }

    #[test]
    fn a_state_tint_keeps_the_background_translucent() {
        let under = Color::from_rgba8(255, 255, 255, 100);
        let tinted = mix(under, Color::BLACK, PRESSED_MIX);
        assert_eq!(
            tinted.components[3], under.components[3],
            "the tint changed the alpha, which would break the material"
        );
        assert!(
            tinted.components[0] < under.components[0],
            "mixing black in did not darken: {} -> {}",
            under.components[0],
            tinted.components[0]
        );

        // And the same rule lightens a dark chip with a light label on it.
        let dark = Color::from_rgba8(30, 30, 32, 200);
        assert!(mix(dark, Color::WHITE, HOVER_MIX).components[0] > dark.components[0]);
        assert_eq!(mix(dark, Color::WHITE, 0.0), dark);
    }

    #[test]
    fn a_pill_is_as_wide_as_its_label_plus_its_padding() {
        let theme = Theme::light();
        let mut tcx = TextContext::new();

        let mut short = PillButton::new("Ok", ButtonStyle::Plain);
        let mut long = PillButton::new("Mark as read", ButtonStyle::Plain);

        let short_size = short.measure(&mut tcx, &theme);
        let long_size = long.measure(&mut tcx, &theme);

        assert_eq!(short_size.height, theme.metrics.button);
        assert_eq!(long_size.height, theme.metrics.button);
        assert!(
            long_size.width > short_size.width,
            "a longer label did not make a wider pill: {} vs {}",
            long_size.width,
            short_size.width
        );
        assert!(
            short_size.width > 2.0 * PILL_PAD_X,
            "the pill lost its padding: {}",
            short_size.width
        );
    }

    #[test]
    fn a_pill_encodes_its_background_and_every_glyph_of_its_label() {
        let theme = Theme::light();
        let mut tcx = TextContext::new();
        let mut button = PillButton::new("Reply", ButtonStyle::Tinted);
        let mut scene = Scene::new();

        button.render(
            &mut scene,
            Rect::new(0.0, 0.0, 80.0, theme.metrics.button),
            &theme,
            ButtonState::Hover,
            1.0,
            &mut tcx,
        );

        assert_eq!(scene.encoding().n_paths, 1, "expected one background path");
        assert_eq!(
            scene.encoding().resources.glyphs.len(),
            5,
            "expected one glyph per character of the label"
        );
    }

    #[test]
    fn the_label_takes_its_colour_from_the_style() {
        let theme = Theme::dark();
        let mut plain = PillButton::new("Ok", ButtonStyle::Plain);
        let mut tinted = PillButton::new("Ok", ButtonStyle::Tinted);
        let mut filled = PillButton::new("Ok", ButtonStyle::Filled(theme.palette.negative));

        plain.apply_style(&theme);
        tinted.apply_style(&theme);
        filled.apply_style(&theme);

        assert_eq!(plain.label.style().color, theme.palette.primary);
        assert_eq!(tinted.label.style().color, theme.palette.accent);
        assert_eq!(filled.label.style().color, Color::WHITE);
    }

    #[test]
    fn the_call_buttons_are_the_theme_s_positive_and_negative() {
        let theme = Theme::light();
        let answer = IconButton::answer(&theme);
        let decline = IconButton::decline(&theme);

        assert_eq!(answer.fill, theme.palette.positive);
        assert_eq!(decline.fill, theme.palette.negative);
        assert_eq!(answer.symbol, Symbol::Phone);
        assert_eq!(decline.symbol, Symbol::PhoneDown);

        let bounds = answer.bounds(Point::new(100.0, 50.0));
        assert_eq!(bounds.width(), theme.metrics.button);
        assert_eq!(bounds.center(), Point::new(100.0, 50.0));
    }

    #[test]
    fn every_symbol_encodes_something_on_top_of_its_disc() {
        let theme = Theme::light();
        for symbol in [
            Symbol::Phone,
            Symbol::PhoneDown,
            Symbol::Play,
            Symbol::Pause,
            Symbol::Next,
            Symbol::Previous,
            Symbol::Close,
            Symbol::Send,
        ] {
            let button = IconButton::new(
                symbol,
                theme.palette.accent,
                Color::WHITE,
                theme.metrics.button,
            );
            let mut scene = Scene::new();
            button.render(&mut scene, Point::new(20.0, 20.0), ButtonState::Normal, 1.0);
            assert!(
                scene.encoding().n_paths >= 2,
                "{symbol:?} drew only its disc"
            );
        }
    }
}
