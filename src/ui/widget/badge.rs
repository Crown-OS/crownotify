//! The small device silhouette a relayed notification's picture wears.

use crownui::{
    ext::{DrawList, Kind},
    prelude::{Color, CornerRadii, Element, Rect, Size, canvas},
};

use crate::model::DeviceKind;

/// Height of the badge. The design's number; anything smaller stops reading
/// as a device at a glance.
pub const BADGE_HEIGHT: f32 = 14.0;
/// How far the badge hangs past the picture's bottom-right corner. Enough that
/// it reads as sitting *on* the picture rather than inside it.
pub const BADGE_OVERHANG: f32 = 2.0;
/// Width of the halo separating the badge from whatever is underneath it.
const BADGE_HALO: f32 = 1.25;

/// Width over height of each silhouette.
pub fn badge_aspect(kind: DeviceKind) -> f32 {
    match kind {
        DeviceKind::Phone => 0.62,
        DeviceKind::Tablet => 0.80,
        DeviceKind::Watch => 0.62,
        DeviceKind::Computer => 1.15,
    }
}

/// The badge for `kind`, drawn in `body` with a `halo` around it; size it
/// `BADGE_HEIGHT * badge_aspect(kind)` × `BADGE_HEIGHT`.
pub fn badge(kind: DeviceKind, body: Color, halo: Color) -> Element<impl Kind> {
    canvas(move |_, size, list| draw(list, kind, size, body, halo))
}

/// A silhouette is one or two rounded body parts and a screen cut out of them.
struct Shape {
    parts: [Option<(Rect, f32)>; 2],
    screen: (Rect, f32),
}

fn draw(list: &mut DrawList, kind: DeviceKind, size: Size, body: Color, halo: Color) {
    let shape = shape(kind, size);
    let parts = || shape.parts.iter().flatten();
    for (part, radius) in parts() {
        list.fill(
            inflate(*part, BADGE_HALO),
            CornerRadii::uniform(radius + BADGE_HALO),
            halo,
        );
    }
    for (part, radius) in parts() {
        list.fill(*part, CornerRadii::uniform(*radius), body);
    }
    let (screen, radius) = shape.screen;
    list.fill(screen, CornerRadii::uniform(radius), halo);
}

fn inflate(rect: Rect, by: f32) -> Rect {
    Rect::from_xywh(
        rect.min_x() - by,
        rect.min_y() - by,
        rect.size.width + 2.0 * by,
        rect.size.height + 2.0 * by,
    )
}

fn shape(kind: DeviceKind, size: Size) -> Shape {
    let height = size.height;
    let inset = |dx: f32, dy: f32| {
        Rect::from_xywh(
            dx * height,
            dy * height,
            size.width - 2.0 * dx * height,
            height - 2.0 * dy * height,
        )
    };
    match kind {
        DeviceKind::Phone | DeviceKind::Tablet => {
            let corner = if kind == DeviceKind::Tablet {
                0.16
            } else {
                0.22
            };
            let radius = corner * height;
            Shape {
                parts: [Some((Rect::new(Default::default(), size), radius)), None],
                screen: (inset(0.11, 0.13), radius * 0.5),
            }
        }
        DeviceKind::Watch => Shape {
            parts: [
                Some((inset(0.17, 0.0), 0.06 * height)),
                Some((inset(0.0, 0.20), 0.24 * height)),
            ],
            screen: (inset(0.13, 0.32), 0.12 * height),
        },
        DeviceKind::Computer => {
            let screen = Rect::from_xywh(0.0, 0.0, size.width, height - 0.24 * height);
            let base = Rect::from_xywh(
                size.width / 2.0 - 0.26 * height,
                height - 0.10 * height,
                0.52 * height,
                0.10 * height,
            );
            Shape {
                parts: [Some((screen, 0.10 * height)), Some((base, 0.05 * height))],
                screen: (
                    Rect::from_xywh(
                        0.10 * height,
                        0.11 * height,
                        screen.size.width - 0.20 * height,
                        screen.size.height - 0.22 * height,
                    ),
                    0.05 * height,
                ),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_screen_sits_inside_its_silhouette() {
        for kind in [
            DeviceKind::Phone,
            DeviceKind::Tablet,
            DeviceKind::Watch,
            DeviceKind::Computer,
        ] {
            let size = Size::new(BADGE_HEIGHT * badge_aspect(kind), BADGE_HEIGHT);
            let (screen, _) = shape(kind, size).screen;
            assert!(screen.min_x() > 0.0 && screen.min_y() > 0.0, "{kind:?}");
            assert!(
                screen.max_x() < size.width && screen.max_y() < size.height,
                "{kind:?}"
            );
        }
    }

    #[test]
    fn a_computer_is_wider_than_it_is_tall_and_a_phone_is_not() {
        assert!(badge_aspect(DeviceKind::Computer) > 1.0);
        assert!(badge_aspect(DeviceKind::Phone) < 1.0);
    }
}
