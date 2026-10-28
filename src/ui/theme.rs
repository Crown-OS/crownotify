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
//! `WindowConfig::blur`). It is opaque enough, though, to stay readable on a
//! compositor with no `ext-background-effect`, where the blur silently does
//! nothing and the fill is all that stands between the text and the wallpaper.

use crownshell::{Blur, INTER, MaterialSpec, Shadow, TextStyle, kurbo::Vec2, peniko::Color};

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

/// Every token the UI draws with, for one appearance.
///
/// Cheap to clone but not free — it owns a family string per text style — so
/// build it once and share it through `surface::Shared` rather than per frame.
#[derive(Debug, Clone, PartialEq)]
pub struct Theme {
    /// What the compositor is asked to render behind and around a card.
    pub material: Material,
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
            material: Material::light(),
            palette,
        }
    }

    pub fn dark() -> Self {
        let palette = Palette::dark();
        Self {
            text: Typography::new(&palette),
            metrics: Metrics::default(),
            material: Material::dark(),
            palette,
        }
    }

    pub fn for_appearance(appearance: Appearance) -> Self {
        match appearance {
            Appearance::Light => Self::light(),
            Appearance::Dark => Self::dark(),
        }
    }
}

impl Theme {
    /// What the compositor is asked to render for a card's shapes.
    ///
    /// Built fresh rather than stored because it is a handful of `f64`s and
    /// building it keeps [`Material`] the single place the numbers live.
    pub fn material_spec(&self) -> MaterialSpec {
        MaterialSpec {
            blur: Some(Blur {
                radius: self.material.blur_radius,
                tint: self.material.tint,
                saturation: self.material.saturation,
            }),
            shadow: Some(Shadow {
                radius: self.material.shadow_radius,
                offset: Vec2::new(0.0, self.material.shadow_dy),
                color: self.material.shadow_color,
            }),
            border: self.material.border,
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
/// rims and shadows it in one fragment shader pass. What the daemon still
/// paints for itself is whatever the compositor says it will not do — see
/// [`Palette::card`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Material {
    /// Backdrop blur radius in logical pixels.
    pub blur_radius: f64,
    /// Mixed over the blurred backdrop. Its alpha is how much of it shows, so
    /// this is far more transparent than [`Palette::card`], which has to stand
    /// in for the whole material when there is no blur behind it.
    pub tint: Color,
    /// Chroma multiplier about luma. Above `1.0` is the vibrancy that makes a
    /// colour show through frosted glass.
    pub saturation: f64,
    /// Width of the refractive rim, in logical pixels.
    pub border: f64,
    pub shadow_radius: f64,
    pub shadow_dy: f64,
    pub shadow_color: Color,
}

impl Material {
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

/// Colours, all premultiplied by nothing — alpha is meaningful and is what
/// makes the card material work, so widgets must fade by *multiplying* alpha
/// (see `widget::surface::multiply_alpha`) rather than by replacing it.
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
        }
    }
}

/// Sizes and spacing, in logical pixels.
///
/// Shared by both appearances: a light/dark switch must never re-flow the
/// stack, because the surface would then have to re-measure every card to
/// change a colour.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Metrics {
    pub card_width: f64,
    pub card_radius: f64,
    /// Vertical gap between two cards in the stack.
    pub card_gap: f64,
    /// How far one card of a collapsed deck peeks out below the card in front.
    pub deck_peek: f64,
    /// How much narrower each card of a collapsed deck is than the one in
    /// front of it, per side.
    pub deck_inset: f64,
    /// Gap between the card and the edge of the output.
    pub card_margin: f64,
    pub pad_x: f64,
    pub pad_y: f64,
    /// Side of the square app icon.
    pub icon: f64,
    pub icon_radius: f64,
    /// Gap between the icon column and the text column.
    pub icon_gap: f64,
    pub shadow_dy: f64,
    /// Shadow softness as a CSS-style blur radius. Vello wants a Gaussian
    /// standard deviation, which is half of this; `widget::surface` converts.
    pub shadow_blur: f64,
    /// Height of a pill button and diameter of a circular icon button.
    pub button: f64,
    pub field_height: f64,
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
            shadow_dy: 6.0,
            shadow_blur: 18.0,
            button: 32.0,
            field_height: 30.0,
        }
    }
}

/// The six text roles a card uses, already carrying their colour.
///
/// Handing widgets a finished [`TextStyle`] rather than a size and a weight
/// means `Text::set_style` can compare it against what it already has and skip
/// the re-layout, which is the whole reason cards retain their `Text`s.
#[derive(Debug, Clone, PartialEq)]
pub struct Typography {
    /// App name.
    pub app: TextStyle,
    pub summary: TextStyle,
    /// Body copy. Wrapped and clamped to four lines by the card, not here.
    pub body: TextStyle,
    /// The uppercase "TIME SENSITIVE" label a critical notification carries.
    pub eyebrow: TextStyle,
    /// Timestamps and other small print.
    pub meta: TextStyle,
    /// Text typed into an inline reply.
    pub field: TextStyle,
}

impl Typography {
    /// Builds the roles against `palette`, so the colours cannot drift from the
    /// appearance the rest of the theme was built for.
    pub fn new(palette: &Palette) -> Self {
        let base = TextStyle::new(INTER, 13.0);
        Self {
            app: base.clone().with_weight(600.0).with_color(palette.primary),
            summary: base.clone().with_weight(600.0).with_color(palette.primary),
            body: base
                .clone()
                .with_color(palette.secondary)
                .with_line_height(1.35),
            eyebrow: base
                .clone()
                .with_size(11.0)
                .with_weight(600.0)
                .with_letter_spacing(0.6)
                .with_color(palette.tertiary),
            meta: base.clone().with_size(11.0).with_color(palette.tertiary),
            field: base.with_color(palette.primary),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Rough perceived brightness of a colour, ignoring alpha. Good enough to
    /// assert that one theme is the light one.
    fn luminance(color: Color) -> f32 {
        let [r, g, b, _] = color.components;
        0.2126 * r + 0.7152 * g + 0.0722 * b
    }

    fn alpha(color: Color) -> f32 {
        color.components[3]
    }

    #[test]
    fn the_light_theme_has_a_bright_card_and_dark_text_and_the_dark_theme_the_reverse() {
        let light = Theme::light();
        let dark = Theme::dark();

        assert!(
            luminance(light.palette.card) > luminance(dark.palette.card),
            "light card {} is not brighter than the dark card {}",
            luminance(light.palette.card),
            luminance(dark.palette.card)
        );
        assert!(
            luminance(light.palette.primary) < luminance(dark.palette.primary),
            "light primary text {} is not darker than the dark one {}",
            luminance(light.palette.primary),
            luminance(dark.palette.primary)
        );
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
            let card = alpha(theme.palette.card);
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
            assert!(
                alpha(palette.primary) > alpha(palette.secondary),
                "secondary text is not softer than primary: {} vs {}",
                alpha(palette.secondary),
                alpha(palette.primary)
            );
            assert!(
                alpha(palette.secondary) > alpha(palette.tertiary),
                "tertiary text is not softer than secondary: {} vs {}",
                alpha(palette.tertiary),
                alpha(palette.secondary)
            );
        }
    }

    #[test]
    fn typography_takes_its_colours_from_the_palette_it_was_built_with() {
        let dark = Theme::dark();
        assert_eq!(dark.text.summary.color, dark.palette.primary);
        assert_eq!(dark.text.body.color, dark.palette.secondary);
        assert_eq!(dark.text.meta.color, dark.palette.tertiary);
        assert_eq!(dark.text.field.color, dark.palette.primary);
        assert_eq!(dark.text.app.family, INTER);
    }

    #[test]
    fn the_eyebrow_is_smaller_and_wider_tracked_than_body_copy() {
        let theme = Theme::light();
        assert!(theme.text.eyebrow.size < theme.text.body.size);
        assert!(
            theme.text.eyebrow.letter_spacing > theme.text.body.letter_spacing,
            "the eyebrow needs the extra tracking to read as a label"
        );
    }

    /// A light/dark switch must not re-flow the stack; the surface layer relies
    /// on that to avoid re-measuring every card when only a colour changed.
    #[test]
    fn both_appearances_share_one_set_of_metrics() {
        assert_eq!(Theme::light().metrics, Theme::dark().metrics);
        assert_eq!(Theme::light().metrics.card_width, 376.0);
        assert_eq!(Theme::light().metrics.card_radius, 18.0);
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
        assert_eq!(
            Theme::for_appearance(Appearance::Dark).palette,
            Theme::dark().palette
        );
        assert_eq!(Theme::default().palette, Theme::light().palette);
    }
}
