use parley::{FontContext, LayoutContext};
use vello::{
    kurbo::{Affine, Circle, Point, RoundedRect, Stroke},
    peniko::{Brush, Color, Fill},
    Scene,
};

use crate::models::Notification;
use crate::ui::text::{build_layout, draw_layout};

pub const CARD_PAD_X: f32 = 16.0;
pub const CARD_PAD_Y: f32 = 14.0;
pub const CARD_GAP: f32 = 10.0;
pub const CARD_MARGIN: f32 = 12.0;
pub const CARD_RADIUS: f64 = 14.0;
pub const CALL_BTN_SIZE: f32 = 44.0;

const BG: Color = Color::from_rgba8(24, 24, 26, 235);
const RIM: Color = Color::from_rgba8(255, 255, 255, 32);
const FG: Color = Color::from_rgba8(245, 245, 247, 255);
const MUTED: Color = Color::from_rgba8(180, 180, 190, 255);
const ACCENT_GREEN: Color = Color::from_rgba8(46, 199, 89, 255);
const ACCENT_RED: Color = Color::from_rgba8(230, 62, 62, 255);
const AVATAR_STROKE: Color = Color::from_rgba8(255, 255, 255, 128);

pub enum HitAction {
    Pickup(String),
    Decline(String),
    Dismiss(usize),
}

pub struct HitRegion {
    pub rect: (f32, f32, f32, f32),
    pub action: HitAction,
}

pub struct Painter {
    font_ctx: FontContext,
    layout_ctx: LayoutContext<Brush>,
}

impl Painter {
    pub fn new() -> Self {
        Self {
            font_ctx: FontContext::new(),
            layout_ctx: LayoutContext::new(),
        }
    }

    pub fn paint(
        &mut self,
        scene: &mut Scene,
        notifications: &[Notification],
        size: (u32, u32),
        hits: &mut Vec<HitRegion>,
    ) {
        let (surface_w, _surface_h) = size;
        let card_w = surface_w as f32 - 2.0 * CARD_MARGIN;
        if card_w <= 0.0 {
            return;
        }
        let card_x = CARD_MARGIN;
        let mut y = CARD_MARGIN;

        for (idx, notif) in notifications.iter().enumerate() {
            let card_h = self.measure_card(notif, card_w);
            self.draw_card_bg(scene, card_x, y, card_w, card_h);
            self.draw_card(scene, notif, idx, card_x, y, card_w, card_h, hits);
            y += card_h + CARD_GAP;
        }
    }

    fn measure_card(&mut self, notif: &Notification, card_w: f32) -> f32 {
        let inner_w = card_w - 2.0 * CARD_PAD_X;
        match notif {
            Notification::General(g) => {
                let title = build_layout(
                    &mut self.font_ctx,
                    &mut self.layout_ctx,
                    &g.summary,
                    16.0,
                    Some(inner_w),
                );
                let body = build_layout(
                    &mut self.font_ctx,
                    &mut self.layout_ctx,
                    &g.body,
                    13.0,
                    Some(inner_w),
                );
                let app = build_layout(
                    &mut self.font_ctx,
                    &mut self.layout_ctx,
                    &g.app_name,
                    12.0,
                    Some(inner_w),
                );
                let h = app.height() + 4.0 + title.height() + 2.0 + body.height();
                h + 2.0 * CARD_PAD_Y
            }
            Notification::Call(c) => {
                // Avatar column + text column + two buttons row = fixed height
                let name = build_layout(
                    &mut self.font_ctx,
                    &mut self.layout_ctx,
                    &c.contact_name,
                    18.0,
                    Some(inner_w),
                );
                let phone = build_layout(
                    &mut self.font_ctx,
                    &mut self.layout_ctx,
                    &c.phone_number,
                    13.0,
                    Some(inner_w),
                );
                let top = 56.0f32.max(name.height() + phone.height() + 6.0);
                top + CALL_BTN_SIZE + 12.0 + 2.0 * CARD_PAD_Y
            }
            Notification::Music(m) => {
                let now = build_layout(
                    &mut self.font_ctx,
                    &mut self.layout_ctx,
                    "Now playing",
                    12.0,
                    Some(inner_w),
                );
                let song = build_layout(
                    &mut self.font_ctx,
                    &mut self.layout_ctx,
                    &m.song_name,
                    16.0,
                    Some(inner_w),
                );
                let h = now.height() + 4.0 + song.height() + 6.0 + 6.0;
                h + 2.0 * CARD_PAD_Y
            }
            Notification::Chat(c) => {
                let title = build_layout(
                    &mut self.font_ctx,
                    &mut self.layout_ctx,
                    &c.title,
                    16.0,
                    Some(inner_w),
                );
                let body = build_layout(
                    &mut self.font_ctx,
                    &mut self.layout_ctx,
                    &c.body,
                    13.0,
                    Some(inner_w),
                );
                let meta = build_layout(
                    &mut self.font_ctx,
                    &mut self.layout_ctx,
                    &format!("{} · {}", c.app_name, c.timestamp),
                    12.0,
                    Some(inner_w),
                );
                let h = meta.height() + 4.0 + title.height() + 2.0 + body.height();
                h + 2.0 * CARD_PAD_Y
            }
            Notification::Audio(_) | Notification::Display(_) => 48.0 + 2.0 * CARD_PAD_Y,
        }
    }

    fn draw_card_bg(&self, scene: &mut Scene, x: f32, y: f32, w: f32, h: f32) {
        let rect = RoundedRect::new(
            x as f64,
            y as f64,
            (x + w) as f64,
            (y + h) as f64,
            CARD_RADIUS,
        );
        scene.fill(Fill::NonZero, Affine::IDENTITY, BG, None, &rect);
        scene.stroke(&Stroke::new(1.0), Affine::IDENTITY, RIM, None, &rect);
    }

    fn draw_card(
        &mut self,
        scene: &mut Scene,
        notif: &Notification,
        idx: usize,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        hits: &mut Vec<HitRegion>,
    ) {
        let inner_x = x + CARD_PAD_X;
        let inner_y = y + CARD_PAD_Y;
        let inner_w = w - 2.0 * CARD_PAD_X;

        match notif {
            Notification::General(g) => {
                let app = build_layout(
                    &mut self.font_ctx,
                    &mut self.layout_ctx,
                    &g.app_name,
                    12.0,
                    Some(inner_w),
                );
                draw_layout(scene, &app, inner_x, inner_y, MUTED);
                let title_y = inner_y + app.height() + 4.0;
                let title = build_layout(
                    &mut self.font_ctx,
                    &mut self.layout_ctx,
                    &g.summary,
                    16.0,
                    Some(inner_w),
                );
                draw_layout(scene, &title, inner_x, title_y, FG);
                let body_y = title_y + title.height() + 2.0;
                let body = build_layout(
                    &mut self.font_ctx,
                    &mut self.layout_ctx,
                    &g.body,
                    13.0,
                    Some(inner_w),
                );
                draw_layout(scene, &body, inner_x, body_y, MUTED);
                hits.push(HitRegion {
                    rect: (x, y, w, h),
                    action: HitAction::Dismiss(idx),
                });
            }
            Notification::Call(c) => {
                let avatar_r = 28.0f32;
                let avatar_cx = inner_x + avatar_r;
                let avatar_cy = inner_y + avatar_r;
                let circle =
                    Circle::new(Point::new(avatar_cx as f64, avatar_cy as f64), avatar_r as f64);
                scene.stroke(&Stroke::new(2.0), Affine::IDENTITY, AVATAR_STROKE, None, &circle);

                let text_x = avatar_cx + avatar_r + 14.0;
                let text_w = inner_w - (text_x - inner_x);
                let name = build_layout(
                    &mut self.font_ctx,
                    &mut self.layout_ctx,
                    &c.contact_name,
                    18.0,
                    Some(text_w),
                );
                draw_layout(scene, &name, text_x, inner_y + 4.0, FG);
                let phone_y = inner_y + 4.0 + name.height() + 4.0;
                let phone = build_layout(
                    &mut self.font_ctx,
                    &mut self.layout_ctx,
                    &c.phone_number,
                    13.0,
                    Some(text_w),
                );
                draw_layout(scene, &phone, text_x, phone_y, MUTED);

                let btn_y = y + h - CARD_PAD_Y - CALL_BTN_SIZE;
                let decline_x = inner_x;
                let pickup_x = x + w - CARD_PAD_X - CALL_BTN_SIZE;
                self.draw_call_button(scene, decline_x, btn_y, CALL_BTN_SIZE, ACCENT_RED);
                self.draw_call_button(scene, pickup_x, btn_y, CALL_BTN_SIZE, ACCENT_GREEN);
                hits.push(HitRegion {
                    rect: (decline_x, btn_y, CALL_BTN_SIZE, CALL_BTN_SIZE),
                    action: HitAction::Decline(c.call_id.clone()),
                });
                hits.push(HitRegion {
                    rect: (pickup_x, btn_y, CALL_BTN_SIZE, CALL_BTN_SIZE),
                    action: HitAction::Pickup(c.call_id.clone()),
                });
            }
            Notification::Music(m) => {
                let now = build_layout(
                    &mut self.font_ctx,
                    &mut self.layout_ctx,
                    "Now playing",
                    12.0,
                    Some(inner_w),
                );
                draw_layout(scene, &now, inner_x, inner_y, MUTED);
                let song_y = inner_y + now.height() + 4.0;
                let song = build_layout(
                    &mut self.font_ctx,
                    &mut self.layout_ctx,
                    &m.song_name,
                    16.0,
                    Some(inner_w),
                );
                draw_layout(scene, &song, inner_x, song_y, FG);
                let bar_y = song_y + song.height() + 8.0;
                self.draw_progress(scene, inner_x, bar_y, inner_w, 4.0, m.percentage as f32 / 100.0);
                hits.push(HitRegion {
                    rect: (x, y, w, h),
                    action: HitAction::Dismiss(idx),
                });
            }
            Notification::Chat(c) => {
                let meta = build_layout(
                    &mut self.font_ctx,
                    &mut self.layout_ctx,
                    &format!("{} · {}", c.app_name, c.timestamp),
                    12.0,
                    Some(inner_w),
                );
                draw_layout(scene, &meta, inner_x, inner_y, MUTED);
                let title_y = inner_y + meta.height() + 4.0;
                let title = build_layout(
                    &mut self.font_ctx,
                    &mut self.layout_ctx,
                    &c.title,
                    16.0,
                    Some(inner_w),
                );
                draw_layout(scene, &title, inner_x, title_y, FG);
                let body_y = title_y + title.height() + 2.0;
                let body = build_layout(
                    &mut self.font_ctx,
                    &mut self.layout_ctx,
                    &c.body,
                    13.0,
                    Some(inner_w),
                );
                draw_layout(scene, &body, inner_x, body_y, MUTED);
                hits.push(HitRegion {
                    rect: (x, y, w, h),
                    action: HitAction::Dismiss(idx),
                });
            }
            Notification::Audio(_) | Notification::Display(_) => {
                let label = build_layout(
                    &mut self.font_ctx,
                    &mut self.layout_ctx,
                    "Notification",
                    14.0,
                    Some(inner_w),
                );
                draw_layout(scene, &label, inner_x, inner_y, FG);
                hits.push(HitRegion {
                    rect: (x, y, w, h),
                    action: HitAction::Dismiss(idx),
                });
            }
        }
    }

    fn draw_call_button(&self, scene: &mut Scene, x: f32, y: f32, size: f32, color: Color) {
        let cx = (x + size * 0.5) as f64;
        let cy = (y + size * 0.5) as f64;
        let circle = Circle::new(Point::new(cx, cy), (size * 0.5) as f64);
        scene.fill(Fill::NonZero, Affine::IDENTITY, color, None, &circle);
    }

    fn draw_progress(&self, scene: &mut Scene, x: f32, y: f32, w: f32, h: f32, pct: f32) {
        let bg = RoundedRect::new(x as f64, y as f64, (x + w) as f64, (y + h) as f64, (h * 0.5) as f64);
        scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            Color::from_rgba8(255, 255, 255, 40),
            None,
            &bg,
        );
        let filled_w = (w * pct.clamp(0.0, 1.0)).max(0.0);
        if filled_w > 0.0 {
            let fill = RoundedRect::new(
                x as f64,
                y as f64,
                (x + filled_w) as f64,
                (y + h) as f64,
                (h * 0.5) as f64,
            );
            scene.fill(Fill::NonZero, Affine::IDENTITY, FG, None, &fill);
        }
    }
}

impl Default for Painter {
    fn default() -> Self {
        Self::new()
    }
}
