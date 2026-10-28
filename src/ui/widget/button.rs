//! The two button shapes the cards use: a text pill and a round glyph button.

use crownui::{
    ext::Kind,
    prelude::{Color, CursorIcon, Element, Role, Runtime, Semantic, Styled, bind, button, zstack},
};

use crate::ui::{
    theme::Theme,
    widget::glyph::{Symbol, glyph},
};

/// How far a hovered button's fill moves toward its foreground.
const HOVER_MIX: f32 = 0.10;
/// How far a pressed button's fill moves toward its foreground.
const PRESSED_MIX: f32 = 0.20;
/// Horizontal padding inside a pill, either side of its label.
const PILL_PAD_X: f32 = 12.0;

/// What a pill is filled with.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ButtonStyle {
    /// The neutral field fill — an ordinary action.
    Plain,
    /// A wash of the accent with accent text — the action a card wants taken.
    Tinted,
    /// Solid colour with white text — a state that is on.
    Filled(Color),
}

impl ButtonStyle {
    /// Background and foreground.
    pub fn colors(self, theme: &Theme, accent: Color) -> (Color, Color) {
        let palette = &theme.palette;
        match self {
            Self::Plain => (palette.field, palette.primary),
            Self::Tinted => (accent.with_alpha_factor(0.16), accent),
            Self::Filled(fill) => (fill, palette.on_fill),
        }
    }
}

/// `under` moved `amount` of the way toward `over`, keeping `under`'s alpha:
/// a hover tint must not make a translucent button opaque.
pub fn mix(under: Color, over: Color, amount: f32) -> Color {
    Color::new(
        under.r + (over.r - under.r) * amount,
        under.g + (over.g - under.g) * amount,
        under.b + (over.b - under.b) * amount,
        under.a,
    )
}

/// A text pill whose style follows `style`, accented with `accent`.
pub fn pill(
    label: impl Into<String>,
    theme: Theme,
    accent: Color,
    style: impl Fn(&mut Runtime) -> ButtonStyle + Copy + 'static,
) -> Element<impl Kind> {
    let colors = move |runtime: &mut Runtime| style(runtime).colors(&theme, accent);
    let shade = move |amount: f32| {
        bind(move |runtime: &mut Runtime| {
            let (background, foreground) = colors(runtime);
            mix(background, foreground, amount)
        })
    };
    let role = theme.text.app;
    button(label.into())
        .h(theme.metrics.button)
        .px(PILL_PAD_X)
        .py(0.0)
        .rounded(theme.metrics.button / 2.0)
        .text_size(role.size)
        .font_weight(role.weight)
        .bg(bind(move |runtime| colors(runtime).0))
        .text_color(bind(move |runtime| colors(runtime).1))
        .hover(move |variant| variant.bg(shade(HOVER_MIX)))
        .active(move |variant| variant.bg(shade(PRESSED_MIX)))
        .shrink(0.0)
}

/// A round button showing `symbol`, `diameter` across, announced as `label`.
pub fn round_button(
    symbol: Symbol,
    fill: Color,
    foreground: Color,
    diameter: f32,
    label: &'static str,
) -> Element<impl Kind> {
    zstack((glyph(symbol, foreground).size(diameter),))
        .size(diameter)
        .shrink(0.0)
        .rounded(diameter / 2.0)
        .bg(fill)
        .hover(move |variant| variant.bg(mix(fill, foreground, HOVER_MIX)))
        .active(move |variant| variant.bg(mix(fill, foreground, PRESSED_MIX)))
        .cursor(CursorIcon::Pointer)
        .role(Role::Button)
        .label(label)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_state_tint_keeps_the_background_translucent() {
        let under = Color::from_rgba8(255, 255, 255, 100);
        let tinted = mix(under, Color::new(0.0, 0.0, 0.0, 1.0), PRESSED_MIX);
        assert_eq!(tinted.a, under.a);
        assert!(tinted.r < under.r);
    }

    #[test]
    fn no_mix_is_the_colour_itself() {
        let color = Color::from_rgba8(10, 20, 30, 40);
        assert_eq!(mix(color, Color::new(1.0, 1.0, 1.0, 1.0), 0.0), color);
    }

    #[test]
    fn a_filled_pill_carries_white_text_on_its_fill() {
        let theme = Theme::light();
        let fill = theme.palette.accent;
        assert_eq!(
            ButtonStyle::Filled(fill).colors(&theme, fill),
            (fill, theme.palette.on_fill)
        );
    }
}
