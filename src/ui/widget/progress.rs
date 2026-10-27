//! Progress bars and the media scrubber.
//!
//! The drawing is trivial; the part worth having in one place is
//! [`seek_percent`], which turns a click into the 0–100 the crownconnect
//! `media_seek` protocol speaks. Keeping the mapping here — rather than in the
//! music card, where it would be reachable only through a paint — is what makes
//! it testable, and a seek that is off by the track's own padding is otherwise
//! very hard to notice and very annoying to use.

use crownshell::{
    Scene,
    kurbo::{Affine, Circle, Point, Rect, RoundedRect},
    peniko::{Color, Fill},
};

use crate::ui::{theme::Theme, widget::surface::multiply_alpha};

/// Thickness of the track, in logical pixels.
pub const TRACK_HEIGHT: f64 = 4.0;
/// Diameter of the scrubber's knob.
pub const KNOB_DIAMETER: f64 = 10.0;

/// Draws a rounded progress bar, centred vertically in `bounds`.
///
/// `bounds` is the whole band the caller reserved, not the track: the track is
/// [`TRACK_HEIGHT`] tall inside it, so the band can be as tall as the pointer
/// target it also serves as. `fraction` is clamped to `0.0..=1.0`.
pub fn draw_bar(
    scene: &mut Scene,
    bounds: Rect,
    fraction: f64,
    track: Color,
    fill: Color,
    alpha: f32,
) {
    let rect = track_rect(bounds);
    let radius = rect.height() / 2.0;
    scene.fill(
        Fill::NonZero,
        Affine::IDENTITY,
        multiply_alpha(track, alpha),
        None,
        &RoundedRect::from_rect(rect, radius),
    );

    let width = rect.width() * fraction.clamp(0.0, 1.0);
    // A rounded rect narrower than its own diameter degenerates, and Vello
    // draws the degenerate case as a sliver that flickers as the track fills.
    if width < f64::EPSILON {
        return;
    }
    let filled = Rect::new(
        rect.x0,
        rect.y0,
        rect.x0 + width.max(rect.height()),
        rect.y1,
    );
    scene.fill(
        Fill::NonZero,
        Affine::IDENTITY,
        multiply_alpha(fill, alpha),
        None,
        &RoundedRect::from_rect(filled, radius),
    );
}

/// Draws the media scrubber: a themed bar, plus a knob at the play head while
/// `active` — hovered or being dragged. The knob only appears then because a
/// resting music card is content, and content does not need a handle on it.
pub fn draw_scrubber(
    scene: &mut Scene,
    bounds: Rect,
    fraction: f64,
    theme: &Theme,
    alpha: f32,
    active: bool,
) {
    draw_bar(
        scene,
        bounds,
        fraction,
        theme.palette.separator,
        theme.palette.primary,
        alpha,
    );
    if !active {
        return;
    }
    let rect = track_rect(bounds);
    let center = Point::new(
        rect.x0 + rect.width() * fraction.clamp(0.0, 1.0),
        rect.center().y,
    );
    scene.fill(
        Fill::NonZero,
        Affine::IDENTITY,
        multiply_alpha(theme.palette.primary, alpha),
        None,
        &Circle::new(center, KNOB_DIAMETER / 2.0),
    );
}

/// Where in `bounds` a click at `x` falls, as `0.0..=1.0`.
pub fn seek_fraction(bounds: Rect, x: f64) -> f64 {
    let width = bounds.width();
    if width <= 0.0 {
        return 0.0;
    }
    ((x - bounds.x0) / width).clamp(0.0, 1.0)
}

/// The position a click at `x` seeks to, as the 0–100 percentage the crownconnect
/// `media_seek` protocol carries.
pub fn seek_percent(bounds: Rect, x: f64) -> u8 {
    (seek_fraction(bounds, x) * 100.0).round() as u8
}

/// The fraction a protocol percentage stands for, tolerating the out-of-range
/// values a phone that lost its place can send.
pub fn percent_fraction(percent: u8) -> f64 {
    f64::from(percent.min(100)) / 100.0
}

/// The track inside a band, centred vertically.
fn track_rect(bounds: Rect) -> Rect {
    let height = TRACK_HEIGHT.min(bounds.height());
    let top = bounds.y0 + (bounds.height() - height) / 2.0;
    Rect::new(bounds.x0, top, bounds.x1, top + height)
}

#[cfg(test)]
mod tests {
    use super::*;

    const BAND: Rect = Rect::new(20.0, 100.0, 220.0, 116.0);

    #[test]
    fn a_click_maps_to_the_position_under_it() {
        assert_eq!(seek_percent(BAND, 20.0), 0);
        assert_eq!(seek_percent(BAND, 120.0), 50);
        assert_eq!(seek_percent(BAND, 220.0), 100);
        assert_eq!(seek_percent(BAND, 70.0), 25);
    }

    #[test]
    fn a_click_outside_the_track_clamps_instead_of_wrapping() {
        assert_eq!(seek_percent(BAND, -400.0), 0);
        assert_eq!(seek_percent(BAND, 4000.0), 100);
        assert_eq!(
            seek_fraction(Rect::new(10.0, 0.0, 10.0, 4.0), 10.0),
            0.0,
            "a zero-width track must not divide by zero"
        );
    }

    #[test]
    fn protocol_percentages_round_trip_through_the_track() {
        for percent in [0_u8, 1, 37, 99, 100] {
            let fraction = percent_fraction(percent);
            let x = BAND.x0 + BAND.width() * fraction;
            assert_eq!(
                seek_percent(BAND, x),
                percent,
                "{percent}% did not survive the trip through the track"
            );
        }
        assert_eq!(
            percent_fraction(255),
            1.0,
            "an out-of-range percentage must clamp, not overflow the bar"
        );
    }

    #[test]
    fn the_track_sits_centred_in_the_band_it_was_given() {
        let track = track_rect(BAND);
        assert_eq!(track.height(), TRACK_HEIGHT);
        assert_eq!(track.center().y, BAND.center().y);
        assert_eq!(track.width(), BAND.width());

        // A band thinner than the track keeps the track inside it.
        let thin = track_rect(Rect::new(0.0, 0.0, 100.0, 2.0));
        assert_eq!(thin.height(), 2.0);
    }

    #[test]
    fn an_empty_bar_draws_only_its_track_and_a_partial_one_draws_both() {
        let theme = Theme::light();

        let mut empty = Scene::new();
        draw_bar(
            &mut empty,
            BAND,
            0.0,
            theme.palette.separator,
            theme.palette.accent,
            1.0,
        );
        assert_eq!(empty.encoding().n_paths, 1);

        let mut half = Scene::new();
        draw_bar(
            &mut half,
            BAND,
            0.5,
            theme.palette.separator,
            theme.palette.accent,
            1.0,
        );
        assert_eq!(half.encoding().n_paths, 2);
    }

    #[test]
    fn the_scrubber_only_grows_a_knob_when_it_is_being_used() {
        let theme = Theme::dark();

        let mut resting = Scene::new();
        draw_scrubber(&mut resting, BAND, 0.4, &theme, 1.0, false);

        let mut active = Scene::new();
        draw_scrubber(&mut active, BAND, 0.4, &theme, 1.0, true);

        assert_eq!(
            active.encoding().n_paths,
            resting.encoding().n_paths + 1,
            "the knob did not appear"
        );
    }
}
