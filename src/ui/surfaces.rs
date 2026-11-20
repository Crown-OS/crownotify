//! The toast strip on every output with the notifications role, and the notification center on
//! one output, all drawing the same core's snapshots.

use std::{collections::HashSet, sync::Arc};

use crownui::{
    Surfaces,
    prelude::{AnyView, AppConfig, View},
    surfaces::{Output, OutputId, Waker},
};
use tokio::sync::watch;

use crate::{
    config::Config,
    daemon::{RequestSender, Snapshot},
    profile::table::Profiles,
    ui::{CenterOutput, Theme, Window, center, popup},
};

pub struct NotificationSurfaces {
    theme: Theme,
    config: Config,
    requests: RequestSender,
    snapshots: watch::Receiver<Arc<Snapshot>>,
    center: CenterOutput,
    center_outputs: HashSet<OutputId>,
}

impl NotificationSurfaces {
    pub fn new(
        theme: Theme,
        config: Config,
        requests: RequestSender,
        snapshots: watch::Receiver<Arc<Snapshot>>,
    ) -> Self {
        Self {
            theme,
            config,
            requests,
            snapshots,
            center: CenterOutput::default(),
            center_outputs: HashSet::new(),
        }
    }

    fn window(&self) -> Window {
        Window {
            theme: self.theme,
            profiles: Profiles::new(&self.config),
            requests: self.requests.clone(),
            snapshots: self.snapshots.clone(),
        }
    }
}

fn shows_toasts(output: &Output) -> bool {
    output
        .crown
        .as_ref()
        .is_none_or(|crown| crown.shell.notifications())
}

fn placement_key(toasts: bool, center: bool) -> Option<u64> {
    (toasts || center).then_some(u64::from(toasts) | u64::from(center) << 1)
}

impl Surfaces for NotificationSurfaces {
    fn attach(&mut self, runner_waker: Waker) {
        self.center.attach(runner_waker);
    }

    fn placement(&mut self, output: &Output) -> Option<u64> {
        let opening = self.snapshots.borrow().center_open && !self.center.on_screen();
        let center = self.center.hosts(output, opening);
        if center {
            self.center_outputs.insert(output.id);
        } else {
            self.center_outputs.remove(&output.id);
        }
        placement_key(shows_toasts(output), center)
    }

    fn build(&mut self, output: &Output) -> Vec<(AnyView, AppConfig)> {
        let mut surfaces = Vec::with_capacity(2);
        if shows_toasts(output) {
            surfaces.push((popup::popup(self.window()).boxed(), popup::config()));
        }
        if self.center_outputs.contains(&output.id) {
            let seat = self.center.seat(output.id);
            surfaces.push((
                center::center(self.window(), Some(seat)).boxed(),
                center::config(),
            ));
        }
        surfaces
    }
}

#[cfg(test)]
mod tests {
    use super::placement_key;

    #[test]
    fn each_mix_of_surfaces_has_its_own_key() {
        let keys = [
            placement_key(true, false),
            placement_key(false, true),
            placement_key(true, true),
        ];
        assert!(keys.iter().all(Option::is_some));
        assert_ne!(keys[0], keys[1]);
        assert_ne!(keys[1], keys[2]);
        assert_ne!(keys[0], keys[2]);
        assert_eq!(placement_key(false, false), None);
    }
}
