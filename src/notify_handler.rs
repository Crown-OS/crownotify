use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    time::Instant,
};

use crownshell::{Scene, SurfaceCtx, SurfaceHandler};

use crate::{
    models::{call::CallNotification, Notification},
    ui::painter::{HitAction, HitRegion, Painter},
};

pub type Inbox = Arc<Mutex<VecDeque<Notification>>>;

struct NotifItem {
    notif: Notification,
    added: Instant,
}

pub struct NotifyHandler {
    inbox: Inbox,
    notifications: Vec<NotifItem>,
    painter: Painter,
    hit_regions: Vec<HitRegion>,
    do_not_disturb: bool,
}

impl NotifyHandler {
    pub fn new(inbox: Inbox) -> Self {
        Self {
            inbox,
            notifications: Vec::new(),
            painter: Painter::new(),
            hit_regions: Vec::new(),
            do_not_disturb: false,
        }
    }

    pub fn toggle_dnd(&mut self) {
        self.do_not_disturb = !self.do_not_disturb;
    }

    fn drain_inbox(&mut self) -> bool {
        let mut added = false;
        if let Ok(mut inbox) = self.inbox.lock() {
            while let Some(n) = inbox.pop_front() {
                if self.do_not_disturb {
                    continue;
                }
                self.notifications.push(NotifItem {
                    notif: n,
                    added: Instant::now(),
                });
                added = true;
            }
        }
        added
    }

    /// Returns true if any notification was removed.
    fn expire(&mut self) -> bool {
        let before = self.notifications.len();
        self.notifications.retain(|item| {
            let timeout_ms = match &item.notif {
                Notification::General(g) => g.expire_timeout,
                _ => 0,
            };
            if timeout_ms == 0 {
                return true;
            }
            item.added.elapsed().as_millis() < timeout_ms as u128
        });
        before != self.notifications.len()
    }

    fn notifications_snapshot(&self) -> Vec<Notification> {
        self.notifications.iter().map(|i| i.notif.clone()).collect()
    }
}

fn in_rect(rect: (f32, f32, f32, f32), x: f32, y: f32) -> bool {
    let (rx, ry, rw, rh) = rect;
    x >= rx && x <= rx + rw && y >= ry && y <= ry + rh
}

impl SurfaceHandler for NotifyHandler {
    fn paint(&mut self, scene: &mut Scene, ctx: SurfaceCtx<'_>) {
        self.drain_inbox();
        self.expire();
        self.hit_regions.clear();
        let notifs = self.notifications_snapshot();
        self.painter
            .paint(scene, &notifs, ctx.size, &mut self.hit_regions);
    }

    fn on_pointer_press(&mut self, x: f64, y: f64, _ctx: SurfaceCtx<'_>) -> bool {
        let hx = x as f32;
        let hy = y as f32;
        let action = self
            .hit_regions
            .iter()
            .find(|r| in_rect(r.rect, hx, hy))
            .map(|r| match &r.action {
                HitAction::Pickup(id) => HitAction::Pickup(id.clone()),
                HitAction::Decline(id) => HitAction::Decline(id.clone()),
                HitAction::Dismiss(idx) => HitAction::Dismiss(*idx),
            });
        match action {
            Some(HitAction::Pickup(id)) => {
                smol::spawn(async move {
                    if let Err(e) = CallNotification::pickup(&id).await {
                        log::warn!("pickup call {id}: {e}");
                    }
                })
                .detach();
                true
            }
            Some(HitAction::Decline(id)) => {
                let id_bg = id.clone();
                smol::spawn(async move {
                    if let Err(e) = CallNotification::decline(&id_bg).await {
                        log::warn!("decline call {id_bg}: {e}");
                    }
                })
                .detach();
                if let Some(pos) = self.notifications.iter().position(|item| {
                    matches!(&item.notif, Notification::Call(c) if c.call_id == id)
                }) {
                    self.notifications.remove(pos);
                }
                true
            }
            Some(HitAction::Dismiss(idx)) => {
                if idx < self.notifications.len() {
                    self.notifications.remove(idx);
                }
                true
            }
            None => false,
        }
    }

    fn on_tick(&mut self, _ctx: SurfaceCtx<'_>) -> bool {
        self.expire()
    }
}
