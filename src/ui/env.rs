//! What every view in one window shares.

use chrono::{DateTime, Local};
use crownui::prelude::{Color, Cx, Runtime, Signal, Store};

use crate::{
    daemon::{Request, RequestSender},
    icons::IconLoader,
    model::{Notification, NotificationId},
    profile::table::Profiles,
    ui::theme::Theme,
};

/// The handles a window's views are built with. `Copy`, so closures take it by
/// value.
#[derive(Clone, Copy)]
pub struct Env {
    pub theme: Theme,
    pub icons: Store<IconLoader>,
    pub profiles: Store<Profiles>,
    requests: Store<RequestSender>,
    /// The wall clock the relative timestamps are rendered against, ticked by
    /// the window.
    pub now: Signal<DateTime<Local>>,
    /// The card whose inline reply is open. One at a time: it is what holds
    /// the window's keyboard focus.
    pub reply: Signal<Option<NotificationId>>,
}

impl Env {
    pub fn new(cx: &mut Cx, theme: Theme, profiles: Profiles, requests: RequestSender) -> Self {
        Self {
            theme,
            icons: cx.store(IconLoader::new()),
            profiles: cx.store(profiles),
            requests: cx.store(requests),
            now: cx.signal(Local::now()),
            reply: cx.signal(None),
        }
    }

    /// Hands `request` to the daemon. A daemon that has gone away has taken
    /// the windows' reason to exist with it, so there is nothing to recover.
    pub fn send(self, runtime: &Runtime, request: Request) {
        self.requests.with(runtime, |requests| {
            if requests.send(request).is_err() {
                log::warn!("the notification core is gone; a request was dropped");
            }
        });
    }

    /// The accent `notification`'s app is drawn with: its profile's, when it
    /// has one, otherwise the theme's.
    pub fn accent(self, runtime: &mut Runtime, notification: Signal<Notification>) -> Color {
        let key = notification.with(runtime, |notification| notification.app.key().into_owned());
        self.profiles.with(runtime, |profiles| {
            profiles.get(&key).accent_or(self.theme.palette.accent)
        })
    }
}
