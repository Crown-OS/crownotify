//! The design tokens every card is drawn from.
//!
//! There are no literal colours or paddings anywhere else in [`ui`](crate::ui):
//! a card asks the [`Theme`] for them. That is what makes a light/dark switch a
//! single call to [`Theme::for_appearance`] rather than a sweep through every
//! widget, and it is what keeps the two appearances from drifting apart as the
//! cards grow.
//!
//! The target is the macOS Tahoe notification: a heavily translucent, blurred,
//! rounded card floating over the wallpaper. The card fill is therefore *not*
//! opaque — it is meant to sit on top of the compositor's backdrop blur (see
//! [`Theme::material`]). It is opaque enough, though, to stay readable on a
//! compositor with no background effect, where crownui falls back to an in-app
//! blur and the fill is most of what stands between the text and the wallpaper.

use crownui::prelude::{Color, Material, MaterialBlur, MaterialShadow, Styled, Vector, WHITE};

use crate::config::AppearancePreference;

/// Which of the two themes to build.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Appearance {
    #[default]
    Light,
    Dark,
}

impl Appearance {
    /// The appearance the user's [`AppearancePreference`] resolves to.
    pub fn from_prefers_dark(prefers_dark: bool) -> Self {
        if prefers_dark {
            Self::Dark
        } else {
            Self::Light
        }
    }

    pub fn is_dark(self) -> bool {
        matches!(self, Self::Dark)
    }
}

impl From<AppearancePreference> for Appearance {
    fn from(preference: AppearancePreference) -> Self {
        Self::from_prefers_dark(preference.prefers_dark())
    }
}

/// Every token the UI draws with, for one appearance. `Copy`, so a view
/// closure takes it by value.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Theme {
    pub glass: Glass,
    pub palette: Palette,
    pub metrics: Metrics,
    pub text: Typography,
}

impl Theme {
    pub fn light() -> Self {
        let palette = Palette::light();
        Self {
            text: Typography::new(&palette),
            metrics: Metrics::default(),
            glass: Glass::light(),
            palette,
        }
    }

    pub fn dark() -> Self {
        let palette = Palette::dark();
        Self {
            text: Typography::new(&palette),
            metrics: Metrics::default(),
            glass: Glass::dark(),
            palette,
        }
    }

    pub fn for_appearance(appearance: Appearance) -> Self {
        match appearance {
            Appearance::Light => Self::light(),
            Appearance::Dark => Self::dark(),
        }
    }

    /// What the compositor is asked to render behind and around a card.
    pub fn material(&self) -> Material {
        let glass = &self.glass;
        Material {
            corner_radius: self.metrics.card_radius,
            border_width: glass.border,
            blur: Some(MaterialBlur {
                radius: glass.blur_radius,
                tint: glass.tint,
                saturation: glass.saturation,
            }),
            shadow: Some(MaterialShadow {
                radius: glass.shadow_radius,
                offset: Vector::new(0.0, glass.shadow_dy),
                color: glass.shadow_color,
            }),
        }
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self::light()
    }
}

/// What the compositor is asked to draw behind and around a card.
///
/// These are not paint: they cross the wire to `crownpositor`, which evaluates
/// the card's rounded rectangle as a signed distance field and blurs, tints,
/// rims and shadows it in one fragment shader pass.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Glass {
    /// Backdrop blur radius in logical pixels.
    pub blur_radius: f32,
    /// Mixed over the blurred backdrop. Its alpha is how much of it shows, so
    /// this is far more transparent than [`Palette::card`], which has to stand
    /// in for the whole material when there is no blur behind it.
    pub tint: Color,
    /// Chroma multiplier about luma. Above `1.0` is the vibrancy that makes a
    /// colour show through frosted glass.
    pub saturation: f32,
    /// Width of the refractive rim, in logical pixels.
    pub border: f32,
    pub shadow_radius: f32,
    pub shadow_dy: f32,
    pub shadow_color: Color,
}

impl Glass {
    pub fn light() -> Self {
        Self {
            blur_radius: 32.0,
            tint: Color::from_rgba8(246, 248, 250, 92),
            saturation: 1.8,
            border: 1.0,
            shadow_radius: 18.0,
            shadow_dy: 6.0,
            shadow_color: Color::from_rgba8(0, 0, 0, 46),
        }
    }

    pub fn dark() -> Self {
        Self {
            tint: Color::from_rgba8(30, 30, 32, 110),
            // Deeper than the light shadow for the same reason the palette's
            // is: a dark card on a dark wallpaper has almost no edge contrast.
            shadow_color: Color::from_rgba8(0, 0, 0, 97),
            ..Self::light()
        }
    }
}

/// Colours with meaningful alpha: it is what makes the card material work, so
/// widgets fade by multiplying alpha (`Color::with_alpha_factor`) rather than by
/// replacing it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Palette {
    /// Card fill. Translucent: the compositor's blur is the other half of it.
    pub card: Color,
    /// Hairline rim just inside the card edge, which is what lifts the card off
    /// the blurred wallpaper when the two happen to be the same brightness.
    pub card_rim: Color,
    pub shadow: Color,
    /// App name, summary, typed text.
    pub primary: Color,
    /// Body copy.
    pub secondary: Color,
    /// Timestamps, the "TIME SENSITIVE" eyebrow, placeholder text.
    pub tertiary: Color,
    /// Selection, links, tinted buttons. Overridden per app by `AppProfile`.
    pub accent: Color,
    /// Answer, playing, success.
    pub positive: Color,
    /// Decline, destructive.
    pub negative: Color,
    /// Fill of an inset control — the inline reply field, a neutral chip.
    pub field: Color,
    pub field_rim: Color,
    pub separator: Color,
    /// Glyphs on a filled button.
    pub on_fill: Color,
}

impl Palette {
    pub fn light() -> Self {
        Self {
            card: Color::from_rgba8(246, 248, 250, 184),
            card_rim: Color::from_rgba8(255, 255, 255, 140),
            shadow: Color::from_rgba8(0, 0, 0, 46),
            primary: Color::from_rgba8(0, 0, 0, 229),
            secondary: Color::from_rgba8(60, 60, 67, 153),
            tertiary: Color::from_rgba8(60, 60, 67, 110),
            accent: Color::from_rgba8(0, 122, 255, 255),
            positive: Color::from_rgba8(52, 199, 89, 255),
            negative: Color::from_rgba8(255, 59, 48, 255),
            field: Color::from_rgba8(255, 255, 255, 179),
            field_rim: Color::from_rgba8(0, 0, 0, 26),
            separator: Color::from_rgba8(0, 0, 0, 20),
            on_fill: WHITE,
        }
    }

    pub fn dark() -> Self {
        Self {
            card: Color::from_rgba8(30, 30, 32, 184),
            card_rim: Color::from_rgba8(255, 255, 255, 26),
            // Deeper than the light shadow on purpose: a dark card over a dark
            // wallpaper has almost no contrast at its edge, so the shadow is
            // the only thing separating the two.
            shadow: Color::from_rgba8(0, 0, 0, 97),
            primary: Color::from_rgba8(255, 255, 255, 240),
            secondary: Color::from_rgba8(235, 235, 245, 153),
            tertiary: Color::from_rgba8(235, 235, 245, 110),
            accent: Color::from_rgba8(10, 132, 255, 255),
            positive: Color::from_rgba8(48, 209, 88, 255),
            negative: Color::from_rgba8(255, 69, 58, 255),
            field: Color::from_rgba8(255, 255, 255, 26),
            field_rim: Color::from_rgba8(255, 255, 255, 36),
            separator: Color::from_rgba8(255, 255, 255, 31),
            on_fill: WHITE,
        }
    }
}

/// Sizes and spacing, in logical pixels.
///
/// Shared by both appearances: a light/dark switch must never re-flow the
/// stack.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Metrics {
    pub card_width: f32,
    pub card_radius: f32,
    /// Vertical gap between two cards in the stack.
    pub card_gap: f32,
    /// How far one card of a collapsed deck peeks out below the card in front.
    pub deck_peek: f32,
    /// How much narrower each card of a collapsed deck is than the one in
    /// front of it, per side.
    pub deck_inset: f32,
    /// Gap between the card and the edge of the output.
    pub card_margin: f32,
    pub pad_x: f32,
    pub pad_y: f32,
    /// Side of the square app icon.
    pub icon: f32,
    pub icon_radius: f32,
    /// Gap between the icon column and the text column.
    pub icon_gap: f32,
    /// Height of a pill button and diameter of a circular icon button.
    pub button: f32,
    pub field_height: f32,
}

impl Default for Metrics {
    fn default() -> Self {
        Self {
            card_width: 376.0,
            card_radius: 18.0,
            card_gap: 10.0,
            deck_peek: 8.0,
            deck_inset: 10.0,
            card_margin: 12.0,
            pad_x: 14.0,
            pad_y: 14.0,
            icon: 38.0,
            icon_radius: 9.0,
            icon_gap: 12.0,
            button: 32.0,
            field_height: 30.0,
        }
    }
}

/// One text role: what a run of text is drawn with.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextRole {
    pub size: f32,
    pub weight: u16,
    pub color: Color,
    /// As a multiple of the size; `None` is the font's own.
    pub line_height: Option<f32>,
    pub tracking: f32,
}

impl TextRole {
    const BASE: Self = Self {
        size: 13.0,
        weight: 400,
        color: WHITE,
        line_height: None,
        tracking: 0.0,
    };

    /// Styles `view` with this role.
    pub fn apply<S: Styled>(self, view: S) -> S {
        let view = view
            .text_size(self.size)
            .font_weight(self.weight)
            .text_color(self.color)
            .tracking(self.tracking);
        match self.line_height {
            Some(line_height) => view.leading(line_height),
            None => view,
        }
    }
}

/// The six text roles a card uses, already carrying their colour.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Typography {
    /// App name.
    pub app: TextRole,
    pub summary: TextRole,
    /// Body copy. Wrapped and clamped by the card, not here.
    pub body: TextRole,
    /// The uppercase "TIME SENSITIVE" label a critical notification carries.
    pub eyebrow: TextRole,
    /// Timestamps and other small print.
    pub meta: TextRole,
    /// Text typed into an inline reply.
    pub field: TextRole,
}

impl Typography {
    /// Builds the roles against `palette`, so the colours cannot drift from the
    /// appearance the rest of the theme was built for.
    pub fn new(palette: &Palette) -> Self {
        let base = TextRole::BASE;
        Self {
            app: TextRole {
                weight: 600,
                color: palette.primary,
                ..base
            },
            summary: TextRole {
                weight: 600,
                color: palette.primary,
                ..base
            },
            body: TextRole {
                color: palette.secondary,
                line_height: Some(1.35),
                ..base
            },
            eyebrow: TextRole {
                size: 11.0,
                weight: 600,
                tracking: 0.6,
                color: palette.tertiary,
                ..base
            },
            meta: TextRole {
                size: 11.0,
                color: palette.tertiary,
                ..base
            },
            field: TextRole {
                color: palette.primary,
                ..base
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Rough perceived brightness of a colour, ignoring alpha. Good enough to
    /// assert that one theme is the light one.
    fn luminance(color: Color) -> f32 {
        0.2126 * color.r + 0.7152 * color.g + 0.0722 * color.b
    }

    #[test]
    fn the_light_theme_has_a_bright_card_and_dark_text_and_the_dark_theme_the_reverse() {
        let light = Theme::light();
        let dark = Theme::dark();

        assert!(luminance(light.palette.card) > luminance(dark.palette.card));
        assert!(luminance(light.palette.primary) < luminance(dark.palette.primary));
        assert!(
            luminance(light.palette.card) > luminance(light.palette.primary),
            "light text would be invisible on its own card"
        );
        assert!(
            luminance(dark.palette.primary) > luminance(dark.palette.card),
            "dark text would be invisible on its own card"
        );
    }

    /// The blur behind the card only shows through if the fill lets it. A fully
    /// opaque card is the single easiest way to lose the whole material.
    #[test]
    fn both_cards_stay_translucent_enough_for_the_backdrop_blur() {
        for theme in [Theme::light(), Theme::dark()] {
            let card = theme.palette.card.a;
            assert!(
                (0.5..0.85).contains(&card),
                "card alpha {card} is outside the translucent range"
            );
        }
    }

    #[test]
    fn text_roles_fade_from_primary_to_tertiary() {
        for theme in [Theme::light(), Theme::dark()] {
            let palette = &theme.palette;
            assert!(palette.primary.a > palette.secondary.a);
            assert!(palette.secondary.a > palette.tertiary.a);
        }
    }

    #[test]
    fn typography_takes_its_colours_from_the_palette_it_was_built_with() {
        let dark = Theme::dark();
        assert_eq!(dark.text.summary.color, dark.palette.primary);
        assert_eq!(dark.text.body.color, dark.palette.secondary);
        assert_eq!(dark.text.meta.color, dark.palette.tertiary);
        assert_eq!(dark.text.field.color, dark.palette.primary);
    }

    #[test]
    fn the_eyebrow_is_smaller_and_wider_tracked_than_body_copy() {
        let theme = Theme::light();
        assert!(theme.text.eyebrow.size < theme.text.body.size);
        assert!(theme.text.eyebrow.tracking > theme.text.body.tracking);
    }

    /// A light/dark switch must not re-flow the stack.
    #[test]
    fn both_appearances_share_one_set_of_metrics() {
        assert_eq!(Theme::light().metrics, Theme::dark().metrics);
        assert_eq!(Theme::light().metrics.card_width, 376.0);
        assert_eq!(Theme::light().metrics.card_radius, 18.0);
    }

    #[test]
    fn the_material_follows_the_card_shape_and_the_glass_tokens() {
        let theme = Theme::dark();
        let material = theme.material();
        assert_eq!(material.corner_radius, theme.metrics.card_radius);
        assert_eq!(material.blur.map(|blur| blur.tint), Some(theme.glass.tint));
    }

    #[test]
    fn appearance_follows_the_configured_preference() {
        assert_eq!(
            Appearance::from(AppearancePreference::Dark),
            Appearance::Dark
        );
        assert_eq!(
            Appearance::from(AppearancePreference::Light),
            Appearance::Light
        );
        assert_eq!(
            Appearance::from(AppearancePreference::Auto),
            Appearance::Light,
            "auto resolves to light until CrownOS has an appearance daemon"
        );
        assert_eq!(Theme::default().palette, Theme::light().palette);
    }
}
