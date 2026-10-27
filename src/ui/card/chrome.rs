//! The parts of a card every view draws the same way.
//!
//! The icon column, the badge that marks a notification relayed from another
//! device, and the close affordance a hovered card reveals. They live here
//! rather than in each view because a card that drew its icon a little
//! differently from the one above it would read as a rendering fault, and
//! because none of it depends on which shape the notification took.

use crownshell::{
    kurbo::{Affine, Point, Rect, RoundedRect},
    peniko::{Fill, Mix},
};

use crate::{
    model::{DeviceKind, Icon, NotificationId},
    ui::{
        card::{
            BADGE_HALO, BADGE_HEIGHT, BADGE_OVERHANG, CLOSE_INSET, PLACEHOLDER_ALPHA, RenderCtx,
        },
        hit::CardAction,
        widget::{IconButton, fill_rounded, surface::multiply_alpha},
    },
};

/// Draws `icon` into `rect`, rounded to `radius`, with a tinted placeholder
/// when it cannot be loaded, and a badge for `device` when the notification was
/// relayed from one.
///
/// The placeholder matters more than it looks: an app with a broken or missing
/// icon otherwise leaves a hole in the card's left column, and the eye reads
/// that as a rendering bug rather than as a missing icon.
pub(crate) fn draw_icon(
    ctx: &mut RenderCtx<'_>,
    icon: Option<&Icon>,
    rect: Rect,
    radius: f64,
    device: Option<DeviceKind>,
) {
    let radius = radius.min(rect.width() / 2.0).min(rect.height() / 2.0);
    let size = rect.width().round().max(1.0) as u32;
    let placeholder = multiply_alpha(ctx.accent(), PLACEHOLDER_ALPHA);
    let image = icon.and_then(|icon| ctx.icons.load(icon, size));
    match image {
        Some(image) => {
            let (width, height) = (f64::from(image.width), f64::from(image.height));
            if width > 0.0 && height > 0.0 {
                ctx.scene.push_layer(
                    Fill::NonZero,
                    Mix::Normal,
                    1.0,
                    Affine::IDENTITY,
                    &RoundedRect::from_rect(rect, radius),
                );
                let transform = Affine::translate((rect.x0, rect.y0))
                    * Affine::scale_non_uniform(rect.width() / width, rect.height() / height);
                ctx.scene.draw_image(&image, transform);
                ctx.scene.pop_layer();
            }
        }
        None => fill_rounded(ctx.scene, rect, radius, placeholder, 1.0),
    }

    if let Some(device) = device {
        draw_device_badge(ctx, rect, device);
    }
}

/// Draws the badge that marks a notification mirrored from another device, over
/// the bottom-right corner of the icon in `rect`.
///
/// Built from kurbo geometry rather than an icon-theme lookup, for the same
/// reason the [`Symbol`](crate::ui::widget::Symbol)s are: this is the only mark
/// on the card that says the message is also sitting on the user's phone, and
/// it has to be there on a system with no icon theme installed.
///
/// It costs the card no height. The badge hangs [`BADGE_OVERHANG`] past an icon
/// that every card already reserves a full padding's worth of room below, and
/// nothing here feeds back into [`Card::measure`] — a relayed card and a local
/// one are exactly the same size. It also draws at full opacity on purpose: a
/// fading card is drawn through a single group layer (see [`Card::render`]), so
/// fading the badge here as well would make it disappear before the icon it
/// sits on.
fn draw_device_badge(ctx: &mut RenderCtx<'_>, rect: Rect, kind: DeviceKind) {
    let bounds = badge_bounds(rect, kind);
    if bounds.width() <= 0.0 || bounds.height() <= 0.0 {
        return;
    }
    // Both colours are forced opaque: the badge sits on top of an arbitrary app
    // icon, and a translucent halo would let that icon through exactly where
    // the badge needs to be separated from it.
    let body = ctx.theme.palette.primary.with_alpha(1.0);
    let contrast = ctx.theme.palette.card.with_alpha(1.0);
    let shape = badge_shape(kind, bounds);

    // Every halo before every fill, so the seam where two parts of a device meet
    // is not drawn over by the second part's halo.
    for (part, radius) in shape.parts.iter().flatten() {
        fill_rounded(
            ctx.scene,
            part.inset(BADGE_HALO),
            radius + BADGE_HALO,
            contrast,
            1.0,
        );
    }
    for (part, radius) in shape.parts.iter().flatten() {
        fill_rounded(ctx.scene, *part, *radius, body, 1.0);
    }
    if let Some((screen, radius)) = shape.screen {
        fill_rounded(ctx.scene, screen, radius, contrast, 1.0);
    }
}

/// Where a `kind` badge lands on an icon occupying `rect`.
///
/// Anchored to the icon's bottom-right corner and clamped to the icon's own
/// height, so an icon smaller than the badge is overlapped rather than swamped.
pub(crate) fn badge_bounds(rect: Rect, kind: DeviceKind) -> Rect {
    let height = BADGE_HEIGHT.min(rect.height());
    let width = height * badge_aspect(kind);
    let (x1, y1) = (rect.x1 + BADGE_OVERHANG, rect.y1 + BADGE_OVERHANG);
    Rect::new(x1 - width, y1 - height, x1, y1)
}

/// Width of a badge as a multiple of its height. This is the whole difference
/// between the four devices at a glance: nothing else about them survives being
/// drawn 14 pixels tall.
fn badge_aspect(kind: DeviceKind) -> f64 {
    match kind {
        DeviceKind::Phone => 0.62,
        DeviceKind::Tablet => 0.80,
        DeviceKind::Watch => 0.62,
        DeviceKind::Computer => 1.15,
    }
}

/// The rounded rectangles one badge is drawn from, as (rect, radius) pairs.
///
/// Fixed-size because this runs on the frame path and no device here is more
/// than two pieces; a `None` is simply a kind that does not need the second one.
struct BadgeShape {
    /// The device itself, drawn in the foreground colour.
    parts: [Option<(Rect, f64)>; 2],
    /// The screen, drawn back in the contrasting colour so that the body reads
    /// as an outlined device rather than as a blob.
    screen: Option<(Rect, f64)>,
}

fn badge_shape(kind: DeviceKind, bounds: Rect) -> BadgeShape {
    let height = bounds.height();
    let inset = |dx: f64, dy: f64| {
        Rect::new(
            bounds.x0 + dx * height,
            bounds.y0 + dy * height,
            bounds.x1 - dx * height,
            bounds.y1 - dy * height,
        )
    };
    match kind {
        DeviceKind::Phone | DeviceKind::Tablet => {
            // A tablet's corners are proportionally squarer than a phone's, and
            // at this size that is most of what tells the two apart.
            let corner = match kind {
                DeviceKind::Tablet => 0.16,
                _ => 0.22,
            };
            let radius = corner * height;
            BadgeShape {
                parts: [Some((bounds, radius)), None],
                screen: Some((inset(0.11, 0.13), radius * 0.5)),
            }
        }
        DeviceKind::Watch => {
            // A single strap behind a wide case: two straps and a buckle are
            // indistinguishable from noise at this size.
            let strap = inset(0.17, 0.0);
            let case = inset(0.0, 0.20);
            BadgeShape {
                parts: [Some((strap, 0.06 * height)), Some((case, 0.24 * height))],
                screen: Some((inset(0.13, 0.32), 0.12 * height)),
            }
        }
        DeviceKind::Computer => {
            let screen = Rect::new(bounds.x0, bounds.y0, bounds.x1, bounds.y1 - 0.24 * height);
            let base = Rect::new(
                bounds.center().x - 0.26 * height,
                bounds.y1 - 0.10 * height,
                bounds.center().x + 0.26 * height,
                bounds.y1,
            );
            BadgeShape {
                parts: [Some((screen, 0.10 * height)), Some((base, 0.05 * height))],
                screen: Some((
                    Rect::new(
                        screen.x0 + 0.10 * height,
                        screen.y0 + 0.11 * height,
                        screen.x1 - 0.10 * height,
                        screen.y1 - 0.11 * height,
                    ),
                    0.05 * height,
                )),
            }
        }
    }
}

/// The close affordance a hovered card reveals, in its top-right corner.
pub(crate) fn draw_close(id: NotificationId, ctx: &mut RenderCtx<'_>) {
    let button = IconButton::close(ctx.theme);
    let center = Point::new(
        ctx.bounds.x1 - CLOSE_INSET - button.diameter / 2.0,
        ctx.bounds.y0 + CLOSE_INSET + button.diameter / 2.0,
    );
    let state = ctx.state(&CardAction::Dismiss);
    button.render(ctx.scene, center, state, 1.0);
    ctx.hits
        .push(button.bounds(center), id, CardAction::Dismiss);
}
