//! What a window asks of the daemon.

use crate::{
    daemon::Daemon,
    model::{CloseReason, NotificationId},
    ui::CardAction,
};

/// One thing the user did in a window that reaches past it: everything a
/// window can decide alone — which reply field is open, which deck is pulled
/// apart — never becomes a request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    Perform(NotificationId, CardAction),
    Reply(NotificationId, String),
    Close(NotificationId, CloseReason),
    /// A toast deck's own Clear All.
    DismissGroup(String),
    /// A history deck's own Clear All.
    ClearGroup(String),
    ClearHistory,
    SetCenterOpen(bool),
    SetDoNotDisturb(bool),
}

/// The windows' end of the request channel; `calloop::channel` wakes the core
/// loop with each one.
pub type RequestSender = calloop::channel::Sender<Request>;

impl Daemon {
    pub fn handle(&mut self, request: Request) {
        match request {
            Request::Perform(id, action) => self.perform(id, &action),
            Request::Reply(id, text) => self.submit_reply(id, text),
            Request::Close(id, reason) => self.close(id, reason),
            Request::DismissGroup(key) => self.dismiss_group(&key),
            Request::ClearGroup(key) => self.clear_group(&key),
            Request::ClearHistory => self.clear_history(),
            Request::SetCenterOpen(open) => self.set_center_open(Some(open)),
            Request::SetDoNotDisturb(enabled) => self.set_do_not_disturb(enabled),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ipc::Outgoing,
        model::Notification,
        store::{Store, StoreConfig},
    };

    fn daemon() -> Daemon {
        let (outgoing, _receiver) = async_channel::unbounded();
        Daemon::new(Store::new(StoreConfig::default()), outgoing)
    }

    fn published(daemon: &mut Daemon) -> Vec<String> {
        let mut out = Vec::new();
        daemon.drain_outgoing(&mut out);
        out.iter().map(|event| format!("{event:?}")).collect()
    }

    #[test]
    fn a_swipe_closes_the_toast_as_dismissed() {
        let mut daemon = daemon();
        let id = daemon.post(Notification::builder("Mail").build());

        daemon.handle(Request::Close(id, CloseReason::Dismissed));

        assert_eq!(daemon.store.toasts().len(), 0);
        assert_eq!(
            published(&mut daemon),
            vec![format!(
                "{:?}",
                Outgoing::Closed(id, CloseReason::Dismissed)
            )]
        );
    }

    #[test]
    fn clicking_outside_the_center_closes_it_and_says_so() {
        let mut daemon = daemon();
        daemon.handle(Request::SetCenterOpen(true));
        daemon.handle(Request::SetCenterOpen(false));

        assert!(!daemon.center_open);
        assert_eq!(
            published(&mut daemon),
            vec![
                format!("{:?}", Outgoing::CenterVisibility(true)),
                format!("{:?}", Outgoing::CenterVisibility(false)),
            ]
        );
    }

    #[test]
    fn a_deck_cleared_in_the_center_leaves_the_toasts_alone() {
        let mut daemon = daemon();
        let toast = daemon.post(Notification::builder("Mail").build());
        let old = daemon.post(Notification::builder("Mail").build());
        daemon.close(old, CloseReason::Dismissed);

        daemon.handle(Request::ClearGroup("mail".into()));

        assert_eq!(daemon.store.history().len(), 0);
        assert_eq!(
            daemon
                .store
                .toasts()
                .next()
                .map(|entry| entry.notification.id),
            Some(toast)
        );
    }
}
