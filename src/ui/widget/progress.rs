//! The progress bar, and the scrubber the music card seeks with.

use crownui::{
    ext::{DrawList, Kind},
    prelude::{Color, CornerRadii, Element, Point, Rect, Runtime, Size, canvas},
};

/// Height of the track, centred in whatever band it is given.
pub const TRACK_HEIGHT: f32 = 4.0;
/// Diameter of the knob a hovered scrubber shows.
pub const KNOB_DIAMETER: f32 = 10.0;

/// A rounded track filled `fraction` of the way with `fill`.
pub fn bar(
    fraction: impl Fn(&mut Runtime) -> f32 + 'static,
    track: Color,
    fill: Color,
) -> Element<impl Kind> {
    canvas(move |runtime, size, list| draw_bar(list, size, fraction(runtime), track, fill))
}

/// A [`bar`] with a knob at the fill's end while `active` says so.
pub fn scrubber(
    fraction: impl Fn(&mut Runtime) -> f32 + 'static,
    active: impl Fn(&mut Runtime) -> bool + 'static,
    track: Color,
    fill: Color,
) -> Element<impl Kind> {
    canvas(move |runtime, size, list| {
        let fraction = fraction(runtime);
        draw_bar(list, size, fraction, track, fill);
        if active(runtime) {
            let rect = track_rect(size);
            let center = Point::new(
                rect.min_x() + rect.size.width * fraction.clamp(0.0, 1.0),
                size.height / 2.0,
            );
            let radius = KNOB_DIAMETER / 2.0;
            list.fill(
                Rect::from_xywh(
                    center.x - radius,
                    center.y - radius,
                    KNOB_DIAMETER,
                    KNOB_DIAMETER,
                ),
                CornerRadii::uniform(radius),
                fill,
            );
        }
    })
}

fn track_rect(size: Size) -> Rect {
    Rect::from_xywh(
        0.0,
        (size.height - TRACK_HEIGHT) / 2.0,
        size.width,
        TRACK_HEIGHT,
    )
}

fn draw_bar(list: &mut DrawList, size: Size, fraction: f32, track: Color, fill: Color) {
    let rect = track_rect(size);
    let radius = CornerRadii::uniform(TRACK_HEIGHT / 2.0);
    list.fill(rect, radius, track);
    let width = rect.size.width * fraction.clamp(0.0, 1.0);
    // An empty bar shows no fill at all rather than a round dot at its start.
    if width < f32::EPSILON {
        return;
    }
    let filled = Rect::new(
        rect.origin,
        Size::new(width.max(TRACK_HEIGHT), TRACK_HEIGHT),
    );
    list.fill(filled, radius, fill);
}

/// A percentage as a fraction, clamped so a sender's 250 % is a full bar.
pub fn percent_fraction(percent: u8) -> f32 {
    f32::from(percent.min(100)) / 100.0
}

/// The percentage a click `x` pixels into a `width`-wide scrubber seeks to.
pub fn seek_percent(x: f32, width: f32) -> u8 {
    if width <= 0.0 {
        return 0;
    }
    ((x / width).clamp(0.0, 1.0) * 100.0).round() as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentages_clamp_to_a_full_bar() {
        assert_eq!(percent_fraction(0), 0.0);
        assert_eq!(percent_fraction(50), 0.5);
        assert_eq!(percent_fraction(250), 1.0);
    }

    #[test]
    fn a_click_seeks_to_where_it_landed_and_never_past_either_end() {
        assert_eq!(seek_percent(0.0, 200.0), 0);
        assert_eq!(seek_percent(100.0, 200.0), 50);
        assert_eq!(seek_percent(-10.0, 200.0), 0);
        assert_eq!(seek_percent(500.0, 200.0), 100);
        assert_eq!(seek_percent(10.0, 0.0), 0);
    }
}
