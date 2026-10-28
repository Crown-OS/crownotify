//! The handful of vector symbols the cards draw on their buttons.
//!
//! Drawn from paths rather than an icon font, so they scale with the button and
//! take the button's own colour with nothing to load.

use std::f32::consts::PI;

use crownui::{
    ext::{DrawCommand, DrawList, Paint, Path, Stroke},
    prelude::{Color, CornerRadii, Element, Point, Rect, Size, canvas},
};

/// Share of the button a glyph spans.
pub const GLYPH_RATIO: f32 = 0.5;
/// Segments a handset's arc is flattened into. Plenty at button size.
const ARC_SEGMENTS: usize = 12;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Symbol {
    Phone,
    PhoneDown,
    Play,
    Pause,
    Next,
    Previous,
    Close,
    Send,
    ChevronDown,
}

const PLAY: [(f32, f32); 3] = [(-0.26, -0.38), (0.38, 0.0), (-0.26, 0.38)];
const NEXT: [(f32, f32); 3] = [(-0.38, -0.34), (0.20, 0.0), (-0.38, 0.34)];
const PREVIOUS: [(f32, f32); 3] = [(0.38, -0.34), (-0.20, 0.0), (0.38, 0.34)];

/// `symbol` in `color`, centred in whatever box it is given at
/// [`GLYPH_RATIO`] of its shorter side.
pub fn glyph(symbol: Symbol, color: Color) -> Element<impl crownui::ext::Kind> {
    canvas(move |_, size, list| draw(list, symbol, size, color))
}

fn draw(list: &mut DrawList, symbol: Symbol, size: Size, color: Color) {
    let center = Point::new(size.width / 2.0, size.height / 2.0);
    let extent = size.width.min(size.height) * GLYPH_RATIO;
    let at = |x: f32, y: f32| Point::new(center.x + x * extent, center.y + y * extent);
    let stroke = Stroke::rounded(extent * 0.14);
    match symbol {
        Symbol::Phone => handset(list, center, extent, color, -PI / 4.0),
        Symbol::PhoneDown => handset(list, center, extent, color, PI * 0.75),
        Symbol::Play => polygon(list, &PLAY, at, color),
        Symbol::Pause => {
            for (left, right) in [(-0.34, -0.10), (0.10, 0.34)] {
                bar(list, at(left, -0.36), at(right, 0.36), 0.06 * extent, color);
            }
        }
        Symbol::Next => {
            polygon(list, &NEXT, at, color);
            bar(list, at(0.30, -0.34), at(0.38, 0.34), 0.04 * extent, color);
        }
        Symbol::Previous => {
            polygon(list, &PREVIOUS, at, color);
            bar(
                list,
                at(-0.38, -0.34),
                at(-0.30, 0.34),
                0.04 * extent,
                color,
            );
        }
        Symbol::Close => {
            for (x0, y0, x1, y1) in [(-0.3, -0.3, 0.3, 0.3), (-0.3, 0.3, 0.3, -0.3)] {
                let path = Path::new().move_to(at(x0, y0)).line_to(at(x1, y1));
                list.push(DrawCommand::StrokePath(path, stroke, Paint::solid(color)));
            }
        }
        Symbol::ChevronDown => {
            let path = Path::new()
                .move_to(at(-0.28, -0.12))
                .line_to(at(0.0, 0.16))
                .line_to(at(0.28, -0.12));
            list.push(DrawCommand::StrokePath(path, stroke, Paint::solid(color)));
        }
        Symbol::Send => {
            let path = Path::new()
                .move_to(at(0.0, 0.38))
                .line_to(at(0.0, -0.38))
                .move_to(at(-0.28, -0.10))
                .line_to(at(0.0, -0.38))
                .line_to(at(0.28, -0.10));
            list.push(DrawCommand::StrokePath(path, stroke, Paint::solid(color)));
        }
    }
}

fn polygon(
    list: &mut DrawList,
    points: &[(f32, f32)],
    at: impl Fn(f32, f32) -> Point,
    color: Color,
) {
    let mut corners = points.iter().map(|&(x, y)| at(x, y));
    let Some(first) = corners.next() else {
        return;
    };
    let path = corners
        .fold(Path::new().move_to(first), Path::line_to)
        .close();
    list.push(DrawCommand::FillPath(path, Paint::solid(color)));
}

fn bar(list: &mut DrawList, top_left: Point, bottom_right: Point, radius: f32, color: Color) {
    let rect = Rect::from_xywh(
        top_left.x,
        top_left.y,
        bottom_right.x - top_left.x,
        bottom_right.y - top_left.y,
    );
    list.fill(rect, CornerRadii::uniform(radius), color);
}

/// A telephone handset: a thick arc, turned by `rotation` about `center`.
fn handset(list: &mut DrawList, center: Point, extent: f32, color: Color, rotation: f32) {
    let radius = extent * 0.3;
    let (start, sweep) = (PI * 0.28, PI * 1.44);
    let point = |step: usize| {
        let angle = start + sweep * step as f32 / ARC_SEGMENTS as f32 + rotation;
        Point::new(
            center.x + radius * angle.cos(),
            center.y + radius * angle.sin(),
        )
    };
    let path = (1..=ARC_SEGMENTS).fold(Path::new().move_to(point(0)), |path, step| {
        path.line_to(point(step))
    });
    list.push(DrawCommand::StrokePath(
        path,
        Stroke::rounded(extent * 0.26),
        Paint::solid(color),
    ));
}
