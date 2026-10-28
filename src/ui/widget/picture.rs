//! The square or round picture at the head of a card: app icon, avatar or
//! album art, with a placeholder when there is none.

use crownui::prelude::{Color, Point, Size, Styled, View, component, cover_image, hstack, zstack};

use crate::{
    model::{DeviceKind, Icon},
    ui::{
        env::Env,
        widget::badge::{BADGE_HEIGHT, BADGE_OVERHANG, badge, badge_aspect},
    },
};

/// Opacity of the accent tile drawn when a picture cannot be loaded.
const PLACEHOLDER_ALPHA: f32 = 0.22;

/// What one picture shows and how.
#[derive(Debug, Clone, PartialEq)]
pub struct Picture {
    pub icon: Option<Icon>,
    pub size: f32,
    pub radius: f32,
    pub accent: Color,
    /// The device a relayed notification came from, badged in the corner.
    pub device: Option<DeviceKind>,
}

/// `picture`, decoded through the window's icon cache at the screen's scale.
pub fn picture(env: Env, picture: Picture) -> impl View {
    component(move |cx| {
        let Picture {
            icon,
            size,
            radius,
            accent,
            device,
        } = picture;
        let scale = cx.scale_factor().get(cx).max(1.0);
        let pixels = icon.and_then(|icon| {
            env.icons
                .update(cx, |icons| icons.load(&icon, (size * scale).ceil() as u32))
        });
        let image = match pixels {
            Some(pixels) => {
                let natural = Size::new(pixels.width as f32, pixels.height as f32);
                let id = cx.register_image(pixels.width, pixels.height, pixels.rgba.clone());
                cx.release_image_on_cleanup(id);
                cover_image(id, natural, radius).size(size).boxed()
            }
            None => hstack(())
                .size(size)
                .rounded(radius)
                .bg(accent.with_alpha_factor(PLACEHOLDER_ALPHA))
                .boxed(),
        };
        let device = device.map(|kind| {
            let width = BADGE_HEIGHT * badge_aspect(kind);
            let corner = Point::new(size + BADGE_OVERHANG, size + BADGE_OVERHANG);
            let palette = env.theme.palette;
            badge(kind, opaque(palette.primary), opaque(palette.card))
                .absolute()
                .left(corner.x - width)
                .top(corner.y - BADGE_HEIGHT)
                .w(width)
                .h(BADGE_HEIGHT)
        });
        zstack((image, device)).size(size).shrink(0.0)
    })
}

/// `color` with its alpha discarded: the badge must stand out against
/// whatever is behind the picture, whatever the card's translucency.
fn opaque(color: Color) -> Color {
    Color::new(color.r, color.g, color.b, 1.0)
}
