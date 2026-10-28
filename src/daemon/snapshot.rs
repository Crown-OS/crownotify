//! What the windows see of the daemon.
//!
//! The store lives on the core thread and the windows on the UI thread, so the
//! windows are handed a copy of everything they draw, republished whenever the
//! daemon's version moves. A notification is a handful of strings and an
//! `Arc`ed image, and a burst of posts is coalesced into one copy per loop
//! iteration, so the copy is cheaper than any way of sharing the store would be.

use crate::{daemon::Daemon, model::Notification};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Snapshot {
    /// Live toasts, oldest first: the order the strip draws its decks in.
    pub toasts: Vec<Notification>,
    /// What the notification center lists, newest first.
    pub history: Vec<Notification>,
    pub center_open: bool,
    pub do_not_disturb: bool,
}

impl Daemon {
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            toasts: self
                .store
                .toasts()
                .map(|entry| entry.notification.clone())
                .collect(),
            history: self
                .store
                .history()
                .map(|entry| entry.notification.clone())
                .collect(),
            center_open: self.center_open,
            do_not_disturb: self.store.do_not_disturb(),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        daemon::Daemon,
        model::{CloseReason, Notification},
        store::{Store, StoreConfig},
    };

    #[test]
    fn a_dismissed_toast_moves_from_the_strip_to_the_center() {
        let (outgoing, _receiver) = async_channel::unbounded();
        let mut daemon = Daemon::new(Store::new(StoreConfig::default()), outgoing);
        let id = daemon.post(Notification::builder("Mail").summary("Ada").build());
        assert_eq!(daemon.snapshot().toasts.len(), 1);

        daemon.close(id, CloseReason::Dismissed);
        let snapshot = daemon.snapshot();
        assert!(snapshot.toasts.is_empty());
        assert_eq!(snapshot.history[0].id, id);
    }

    #[test]
    fn the_center_and_quiet_states_travel_with_the_notifications() {
        let (outgoing, _receiver) = async_channel::unbounded();
        let mut daemon = Daemon::new(Store::new(StoreConfig::default()), outgoing);
        daemon.set_center_open(Some(true));
        daemon.set_do_not_disturb(true);

        let snapshot = daemon.snapshot();
        assert!(snapshot.center_open);
        assert!(snapshot.do_not_disturb);
    }
}
