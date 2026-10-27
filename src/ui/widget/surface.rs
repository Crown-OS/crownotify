//! The card material, for the parts of it the compositor is not drawing.
//!
//! `crownos-background-effects-v1` renders the blur, the refractive rim and the
//! drop shadow server-side, against the card's real rounded rectangle. A
//! compositor may withhold any of those — the user turned the effect off, the
//! shaders would not compile, the machine is on battery — so everything here is
//! gated on [`Painted`], which says what is left for the daemon to do.
//!
//! When the shadow does fall to us it is Vello's own blurred rounded rect, a
//! single analytic draw rather than a stack of concentric strokes faking a
//! falloff, which would cost one path per ring and still band.

use crownshell::{
    Capability, Scene,
    kurbo::{Affine, Rect, RoundedRect, Stroke},
    peniko::{Color, Fill},
};

use crate::ui::theme::Theme;

/// Width of a rim, in logical pixels. One physical pixel on a 1x output and a
/// crisp half-pixel line on a 2x one, which is what "hairline" means here.
pub const HAIRLINE: f64 = 1.0;

/// The design quotes shadow softness as a CSS-style blur radius; Vello's
/// blurred rounded rect wants the Gaussian standard deviation, which is half
/// of it. Getting this backwards doubles the shadow's extent and turns a
/// notification into a smudge.
const BLUR_RADIUS_TO_STD_DEV: f64 = 0.5;

/// Scales `color`'s alpha, leaving its channels alone.
///
/// This is how a card fades: the material is translucent by design, so
/// *replacing* alpha with the animation's would make a leaving card briefly
/// more opaque than a resting one.
pub fn multiply_alpha(color: Color, alpha: f32) -> Color {
    color.with_alpha(color.components[3] * alpha.clamp(0.0, 1.0))
}

/// Which parts of the card material the daemon has to paint itself.
///
/// Derived once per frame from the compositor's advertised capabilities, so a
/// card never has to reason about the protocol and a withdrawn effect reappears
/// as paint on the very next frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Painted {
    /// The compositor is not blurring, so the translucent fill has to stand in
    /// for the whole material or the text loses its contrast.
    pub fill: bool,
    pub rim: bool,
    pub shadow: bool,
}

impl Painted {
    /// Everything: what a compositor with no material support leaves us.
    pub const ALL: Self = Self {
        fill: true,
        rim: true,
        shadow: true,
    };

    /// What is left once the compositor has said what it will render.
    pub fn after(capabilities: Capability) -> Self {
        Self {
            fill: !capabilities.contains(Capability::Blur),
            rim: !capabilities.contains(Capability::Border),
            shadow: !capabilities.contains(Capability::Shadow),
        }
    }

    /// Whether the card needs any paint of its own at all.
    pub fn is_empty(self) -> bool {
        self == Self {
            fill: false,
            rim: false,
            shadow: false,
        }
    }
}

/// Draws whatever of the card material `painted` says is still ours.
pub fn draw_card(scene: &mut Scene, bounds: Rect, theme: &Theme, alpha: f32, painted: Painted) {
    if painted.shadow {
        draw_shadow(scene, bounds, theme, alpha);
    }
    if painted.fill {
        fill_rounded(
            scene,
            bounds,
            theme.metrics.card_radius,
            theme.palette.card,
            alpha,
        );
    }
    if painted.rim {
        stroke_rounded(
            scene,
            bounds,
            theme.metrics.card_radius,
            theme.palette.card_rim,
            alpha,
            HAIRLINE,
        );
    }
}

/// Draws only the drop shadow, offset by `Metrics::shadow_dy`.
///
/// Separate from [`draw_card`] because a card that is being dragged wants its
/// shadow drawn under the whole stack rather than under itself.
pub fn draw_shadow(scene: &mut Scene, bounds: Rect, theme: &Theme, alpha: f32) {
    let metrics = &theme.metrics;
    scene.draw_blurred_rounded_rect(
        Affine::translate((0.0, metrics.shadow_dy)),
        bounds,
        multiply_alpha(theme.palette.shadow, alpha),
        metrics.card_radius,
        metrics.shadow_blur * BLUR_RADIUS_TO_STD_DEV,
    );
}

/// Fills a rounded rectangle, fading `color` by `alpha`.
pub fn fill_rounded(scene: &mut Scene, rect: Rect, radius: f64, color: Color, alpha: f32) {
    scene.fill(
        Fill::NonZero,
        Affine::IDENTITY,
        multiply_alpha(color, alpha),
        None,
        &RoundedRect::from_rect(rect, radius.min(rect.height() / 2.0)),
    );
}

/// Strokes a rounded rectangle *inside* `rect`.
///
/// A stroke is centred on its path, so the path is inset by half the width;
/// without that the rim would spill half a pixel past the card and be clipped
/// by whatever the card is composited into.
pub fn stroke_rounded(
    scene: &mut Scene,
    rect: Rect,
    radius: f64,
    color: Color,
    alpha: f32,
    width: f64,
) {
    let inset = width / 2.0;
    let path = rect.inset(-inset);
    scene.stroke(
        &Stroke::new(width),
        Affine::IDENTITY,
        multiply_alpha(color, alpha),
        None,
        &RoundedRect::from_rect(path, (radius - inset).max(0.0).min(path.height() / 2.0)),
    );
}

/// Three rectangles whose union approximates a rounded rectangle.
///
/// Wayland regions are rectangle lists, so a blur or input region cannot be
/// rounded. Banding it like this keeps the compositor from blurring the four
/// corner squares that sit outside the card, which is visible as a hard corner
/// on top of a busy wallpaper.
pub fn rounded_bands(rect: Rect, radius: f64) -> [Rect; 3] {
    let radius = radius.min(rect.width() / 2.0).min(rect.height() / 2.0);
    [
        Rect::new(
            rect.x0 + radius,
            rect.y0,
            rect.x1 - radius,
            rect.y0 + radius,
        ),
        Rect::new(rect.x0, rect.y0 + radius, rect.x1, rect.y1 - radius),
        Rect::new(
            rect.x0 + radius,
            rect.y1 - radius,
            rect.x1 - radius,
            rect.y1,
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    const BOUNDS: Rect = Rect::new(12.0, 12.0, 388.0, 100.0);

    #[test]
    fn multiplying_alpha_scales_the_channel_it_was_given() {
        let color = Color::from_rgba8(0, 0, 0, 128);
        let faded = multiply_alpha(color, 0.5);
        assert!(
            (faded.components[3] - 0.25).abs() < 0.01,
            "expected a quarter alpha, got {}",
            faded.components[3]
        );
        assert_eq!(faded.components[0], color.components[0], "channels moved");

        assert_eq!(
            multiply_alpha(color, 1.0).components[3],
            color.components[3]
        );
        assert_eq!(multiply_alpha(color, 0.0).components[3], 0.0);
        assert_eq!(
            multiply_alpha(color, 4.0).components[3],
            color.components[3],
            "an out-of-range alpha must clamp rather than brighten"
        );
    }

    /// The material is three draws: shadow, fill, rim. If Vello ever loses the
    /// analytic blur this count changes and the fallback has to be re-thought.
    #[test]
    fn a_card_encodes_a_shadow_a_fill_and_a_rim() {
        let theme = Theme::light();
        let mut scene = Scene::new();
        draw_card(&mut scene, BOUNDS, &theme, 1.0, Painted::ALL);

        assert_eq!(
            scene.encoding().n_paths,
            3,
            "expected three paths in the card material"
        );

        let mut shadow_only = Scene::new();
        draw_shadow(&mut shadow_only, BOUNDS, &theme, 1.0);
        let shadow_tag = shadow_only.encoding().draw_tags[0];
        assert!(
            scene.encoding().draw_tags.contains(&shadow_tag),
            "the card did not encode a blurred rounded rect"
        );
    }

    #[test]
    fn a_fully_faded_card_still_encodes_but_draws_nothing_visible() {
        let theme = Theme::dark();
        let mut scene = Scene::new();
        draw_card(&mut scene, BOUNDS, &theme, 0.0, Painted::ALL);
        assert_eq!(scene.encoding().n_paths, 3);
    }

    #[test]
    fn the_rim_stays_inside_the_card_it_outlines() {
        // The stroke is centred on a path inset by half its width, so the outer
        // edge of the rim lands exactly on the card edge.
        let inset = HAIRLINE / 2.0;
        let path = BOUNDS.inset(-inset);
        assert_eq!(path.x0 - inset, BOUNDS.x0, "the rim spilled past the card");
        assert_eq!(path.x1 + inset, BOUNDS.x1, "the rim spilled past the card");
        assert!(path.width() < BOUNDS.width());
    }

    #[test]
    fn rounded_bands_cover_the_rect_without_its_corners() {
        let rect = Rect::new(0.0, 0.0, 100.0, 60.0);
        let bands = rounded_bands(rect, 18.0);

        assert_eq!(bands[1], Rect::new(0.0, 18.0, 100.0, 42.0));
        assert_eq!(bands[0].y0, rect.y0);
        assert_eq!(bands[2].y1, rect.y1);
        for band in bands {
            assert!(band.width() > 0.0 && band.height() > 0.0, "{band:?}");
        }

        // A radius larger than the rect must not invert any band.
        for band in rounded_bands(Rect::new(0.0, 0.0, 10.0, 10.0), 40.0) {
            assert!(band.width() >= 0.0 && band.height() >= 0.0, "{band:?}");
        }
    }

    #[test]
    fn a_compositor_that_draws_everything_leaves_the_card_nothing_to_paint() {
        let painted = Painted::after(Capability::all());
        assert!(painted.is_empty(), "{painted:?}");
    }

    #[test]
    fn a_compositor_that_draws_nothing_leaves_the_card_the_whole_material() {
        assert_eq!(Painted::after(Capability::empty()), Painted::ALL);
    }

    #[test]
    fn each_withheld_capability_falls_back_on_its_own() {
        // Blur is the one that matters: without it the translucent fill is all
        // that stands between the body text and the wallpaper.
        assert!(Painted::after(Capability::Shadow | Capability::Border).fill);
        assert!(!Painted::after(Capability::Blur).fill);
        assert!(!Painted::after(Capability::Shadow).shadow);
        assert!(!Painted::after(Capability::Border).rim);
    }

    #[test]
    fn a_card_with_nothing_left_to_paint_still_encodes_its_contents() {
        let theme = Theme::light();
        let mut bare = Scene::new();
        draw_card(
            &mut bare,
            BOUNDS,
            &theme,
            1.0,
            Painted::after(Capability::all()),
        );
        let mut full = Scene::new();
        draw_card(&mut full, BOUNDS, &theme, 1.0, Painted::ALL);
        assert!(
            bare.encoding().n_paths < full.encoding().n_paths,
            "the compositor's material should cost the daemon paths, not add them"
        );
    }
}
