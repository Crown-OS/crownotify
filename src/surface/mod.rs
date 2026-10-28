//! The state both surfaces stand on, and the seam where UI intent becomes
//! transport traffic.
//!
//! crownshell is single-threaded, so `Rc<RefCell<_>>` is all the sharing the
//! toast strip and the notification center need. What the two surfaces cannot
//! do is repaint each other: every callback in [`SurfaceHandler`] repaints only
//! the surface the event landed on. That is what [`Shared::version`] is for —
//! each surface remembers the version it last painted and reports
//! [`SurfaceHandler::needs_redraw`] when the two differ, so a click on a toast
//! can repaint the center.
//!
//! # Why nothing here publishes inline
//!
//! Every `Outgoing` this module produces is appended to a queue and published
//! later, by [`drain_outgoing`](Shared::drain_outgoing) from the event loop.
//! `CalloopService` forbids emitting through the service while inside its own
//! `dispatch`, and an IPC request reaches the store through exactly the same
//! [`NotificationSink`] methods a pointer click does — so there is no way to
//! tell "safe to emit now" from "about to abort the process" at the point the
//! event is produced. Deferring all of them leaves one code path, no
//! re-entrancy and no ordering surprises.
//!
//! [`SurfaceHandler`]: crownshell::SurfaceHandler
//! [`SurfaceHandler::needs_redraw`]: crownshell::SurfaceHandler::needs_redraw

pub mod cards;
pub mod center;
pub mod popup;

use std::{
    cell::{Ref, RefCell, RefMut},
    rc::Rc,
};

use calloop::ping::Ping;

use crate::{
    icons::IconLoader,
    ipc::{Incoming, NotificationSink, Outgoing, OutgoingSender, PhoneCommand, PhoneLink},
    launch::{ExecLauncher, Launcher},
    model::{CloseReason, Content, Notification, NotificationId},
    profile::table::Profiles,
    store::Store,
    ui::{CardAction, MediaAction, Theme},
};

pub use center::Center;
pub use popup::Popup;

/// What the surface still has to do to its own card once [`Shared::perform`]
/// has dealt with the store and the transports.
///
/// The split exists because a reply's text lives in the [`Card`] — the widget
/// the user typed into — and never reaches the store, so the surface has to be
/// the one to drain it.
///
/// [`Card`]: crate::ui::Card
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CardOutcome {
    /// Nothing further: the action was entirely a matter of state.
    Handled,
    /// Open and focus this card's inline reply field.
    FocusReply,
    /// Drain the field with `Card::take_reply` and hand the text to
    /// [`Shared::submit_reply`].
    SubmitReply,
    /// Open or close the card's overflow menu.
    ToggleMenu,
}

/// Everything the two surfaces read, and the only mutable state in the daemon.
///
/// Built once in `main` and never moved afterwards; the surfaces, the IPC
/// service and the D-Bus channel callback all hold a [`SharedState`] handle to
/// it.
pub struct Shared {
    pub store: Store,
    pub theme: Theme,
    /// The static registry merged with the user's per-app config, built once.
    pub profiles: Profiles,
    /// How a card reaches the app it came from. Boxed so the tests can record
    /// activations instead of spawning processes.
    launcher: Box<dyn Launcher>,
    pub icons: IconLoader,
    pub phone: PhoneLink,
    /// The D-Bus thread's end of the outgoing stream. Written by
    /// [`drain_outgoing`](Self::drain_outgoing)'s caller, never from here.
    pub outgoing: OutgoingSender,
    pub center_open: bool,
    /// The last Do Not Disturb state published, so a switch between two modes
    /// does not re-announce a bool that did not change.
    announced_quiet: bool,
    /// Bumped by every change a surface could need to redraw for.
    pub version: u64,
    /// Events waiting to be published. Never drained from inside the call that
    /// produced them — see the module documentation.
    queue: Vec<Outgoing>,
    /// Wakes the event loop so the queue is drained promptly. `None` until
    /// `main` has a loop to wake, and in the tests, where nothing publishes.
    wake: Option<Ping>,
}

impl Shared {
    /// Replaces the launcher, which the tests use to record activations rather
    /// than start anything.
    pub fn set_launcher(&mut self, launcher: Box<dyn Launcher>) {
        self.launcher = launcher;
    }

    /// Replaces the profile table, which `main` builds from the same config the
    /// store came from.
    pub fn set_profiles(&mut self, profiles: Profiles) {
        self.profiles = profiles;
    }

    pub fn new(store: Store, theme: Theme, outgoing: OutgoingSender) -> Self {
        Self {
            store,
            theme,
            profiles: Profiles::default(),
            launcher: Box::new(ExecLauncher),
            icons: IconLoader::new(),
            phone: PhoneLink::new(),
            outgoing,
            center_open: false,
            announced_quiet: false,
            version: 0,
            queue: Vec::new(),
            wake: None,
        }
    }

    /// Points the queue at the event loop that will drain it.
    ///
    /// Separate from [`new`](Self::new) because the loop does not exist yet
    /// when the store is built: `main` needs the store's id counter before it
    /// can start the D-Bus thread, which happens before `crownshell::run`.
    pub fn set_wake(&mut self, wake: Ping) {
        self.wake = Some(wake);
    }

    /// Marks the state dirty so every surface repaints on the next iteration.
    pub fn bump(&mut self) {
        self.version = self.version.wrapping_add(1);
    }

    /// Queues `event` for publication and wakes the loop to publish it.
    pub fn publish(&mut self, event: Outgoing) {
        log::trace!("queued {event:?}");
        self.queue.push(event);
        if let Some(wake) = &self.wake {
            wake.ping();
        }
    }

    /// Moves everything queued into `out`, oldest first.
    ///
    /// Takes the destination rather than returning a `Vec` so the queue keeps
    /// its allocation and the caller keeps its buffer; this runs on the event
    /// loop, next to the frame path.
    pub fn drain_outgoing(&mut self, out: &mut Vec<Outgoing>) {
        out.append(&mut self.queue);
    }

    /// How many events are waiting to be published.
    pub fn pending(&self) -> usize {
        self.queue.len()
    }

    /// Applies a request that arrived over a channel rather than through the
    /// sink, so the D-Bus side and the IPC side cannot drift apart.
    pub fn apply(&mut self, incoming: Incoming) {
        match incoming {
            Incoming::Post(notification) => {
                self.post(*notification);
            }
            Incoming::Close(id, reason) => self.close(id, reason),
            Incoming::OpenCenter => self.set_center_open(Some(true)),
            Incoming::CloseCenter => self.set_center_open(Some(false)),
            Incoming::ToggleCenter => self.set_center_open(None),
            Incoming::SetDoNotDisturb(enabled) => self.set_do_not_disturb(enabled),
            Incoming::SetFocusMode(name) => self.set_focus_mode(name.as_deref()),
            Incoming::DismissAll => self.dismiss_all(),
            Incoming::InvokeAction(id, action) => self.invoke_action(id, &action),
        }
    }

    /// Files a notification and returns the id it landed under.
    pub fn post(&mut self, notification: Notification) -> NotificationId {
        let id = self.store.post(notification);
        self.bump();
        id
    }

    /// Closes a notification, telling its sender why.
    ///
    /// An id the store has never heard of is ignored rather than announced: a
    /// `NotificationClosed` for a notification that never existed would make
    /// senders re-post, and the freedesktop spec has each id closed once.
    pub fn close(&mut self, id: NotificationId, reason: CloseReason) {
        if !self.store.close(id, reason) {
            log::debug!("close of unknown notification {id} ignored");
            return;
        }
        self.publish(Outgoing::Closed(id, reason));
        self.bump();
    }

    /// `Some(open)` sets the notification center's state, `None` toggles it.
    pub fn set_center_open(&mut self, open: Option<bool>) {
        let target = open.unwrap_or(!self.center_open);
        if target == self.center_open {
            return;
        }
        self.center_open = target;
        self.publish(Outgoing::CenterVisibility(target));
        self.bump();
    }

    /// Turns the built-in preset on, or ends whichever mode is in force.
    pub fn set_do_not_disturb(&mut self, enabled: bool) {
        if self.store.do_not_disturb() == enabled {
            return;
        }
        let released = self.store.set_do_not_disturb(enabled);
        self.announce_focus(released);
    }

    /// Puts a named focus mode in force, announcing it on both transports.
    ///
    /// An unknown name is logged and ignored — the store's rule, so a typo in a
    /// keybinding is not the same request as "stop being quiet".
    pub fn set_focus_mode(&mut self, name: Option<&str>) {
        if self.store.focus_mode() == name {
            return;
        }
        match self.store.set_focus_mode(name) {
            Ok(released) => self.announce_focus(released),
            Err(error) => log::warn!("{error}"),
        }
    }

    /// Publishes the mode change, and the Do Not Disturb bool alongside it
    /// whenever the *activity* changed.
    ///
    /// A client that knows only the bool — crownbar's bell — stays exactly as
    /// correct as it was before modes existed, and switching between two modes
    /// does not flicker it off and on.
    fn announce_focus(&mut self, released: Vec<NotificationId>) {
        let quiet = self.store.do_not_disturb();
        let mode = self.store.focus_mode().map(str::to_string);
        self.publish(Outgoing::FocusModeChanged(mode));
        if quiet != self.announced_quiet {
            self.announced_quiet = quiet;
            self.publish(Outgoing::DoNotDisturbChanged(quiet));
        }
        if !released.is_empty() {
            log::debug!("ending the focus mode delivered {} held", released.len());
        }
        self.bump();
    }

    /// Dismisses every live toast the deck for `key` holds.
    pub fn dismiss_group(&mut self, key: &str) {
        let dismissed = self.store.dismiss_group(key);
        if dismissed.is_empty() {
            return;
        }
        for id in dismissed {
            self.publish(Outgoing::Closed(id, CloseReason::Dismissed));
        }
        self.bump();
    }

    /// Empties the deck for `key` out of the center.
    pub fn clear_group(&mut self, key: &str) {
        self.store.clear_group(key);
        self.bump();
    }

    /// Clears the toast stack, announcing every notification it took down.
    pub fn dismiss_all(&mut self) {
        let dismissed = self.store.dismiss_all();
        if dismissed.is_empty() {
            return;
        }
        for id in dismissed {
            self.publish(Outgoing::Closed(id, CloseReason::Dismissed));
        }
        self.bump();
    }

    /// Empties the notification center. The toasts on screen are left alone.
    pub fn clear_history(&mut self) {
        self.store.clear_history();
        self.bump();
    }

    /// Invokes a named action on behalf of the user, then retires the card.
    ///
    /// The notification is closed because the user has answered it — leaving it
    /// on screen after its action fired is how a toast ends up being clicked
    /// twice, and the spec's own `ActionInvoked` example closes it too.
    pub fn invoke_action(&mut self, id: NotificationId, action: &str) {
        if self.store.get(id).is_none() {
            log::debug!("action {action:?} on unknown notification {id} ignored");
            return;
        }
        self.publish(Outgoing::ActionInvoked(id, action.to_string()));
        self.close(id, CloseReason::Dismissed);
    }

    /// Carries out everything about `action` that is not the card's own
    /// business, and says what is left for the surface to do.
    pub fn perform(&mut self, id: NotificationId, action: &CardAction) -> CardOutcome {
        if let Some(command) = self
            .store
            .get(id)
            .and_then(|entry| phone_command(&entry.notification.content, action))
        {
            self.phone.send(command);
        }

        match action {
            CardAction::Activate => {
                self.activate(id);
                CardOutcome::Handled
            }
            CardAction::Dismiss => {
                self.close(id, CloseReason::Dismissed);
                CardOutcome::Handled
            }
            CardAction::Invoke(key) => {
                self.invoke_action(id, key);
                CardOutcome::Handled
            }
            CardAction::FocusReply => CardOutcome::FocusReply,
            CardAction::SubmitReply => CardOutcome::SubmitReply,
            // Answering or hanging up ends the card's reason to exist; the
            // phone command has already gone out above.
            // Both belong to the surface that drew the deck: only it knows how
            // far the unfurl spring has got and whether the deck holds live
            // toasts or history.
            CardAction::ToggleStack(_) | CardAction::ClearStack(_) => CardOutcome::Handled,
            // The menu's state lives in the card that drew it.
            CardAction::ToggleMenu => CardOutcome::ToggleMenu,
            CardAction::Answer | CardAction::Decline => {
                self.close(id, CloseReason::Dismissed);
                CardOutcome::Handled
            }
            // A transport button leaves the card up — the whole point of a
            // now-playing card is to be pressed again.
            CardAction::Media(_) => {
                self.bump();
                CardOutcome::Handled
            }
        }
    }

    /// Publishes an inline reply and retires the card that carried it.
    ///
    /// An all-whitespace reply is treated as a mis-click: sending it would put
    /// an empty message in someone's conversation, which is worse than losing a
    /// keystroke.
    pub fn submit_reply(&mut self, id: NotificationId, text: String) {
        if text.trim().is_empty() {
            log::debug!("empty reply to notification {id} discarded");
            return;
        }
        let command = self
            .store
            .get(id)
            .and_then(|entry| reply_command(&entry.notification.content, &text));
        if let Some(command) = command {
            self.phone.send(command);
        }
        self.publish(Outgoing::Replied(id, text));
        self.close(id, CloseReason::Dismissed);
    }

    /// A bare click on the card body: the default action if the sender offered
    /// one, otherwise just a dismissal.
    fn activate(&mut self, id: NotificationId) {
        let Some(notification) = self.store.get(id).map(|entry| &entry.notification) else {
            return;
        };
        // An app that declared a default action wants to handle activation
        // itself, so publishing the action is the whole of the job. Only a
        // notification that claimed none gets its app launched for it.
        if let Some(key) = notification
            .default_action()
            .map(|action| action.key.clone())
        {
            self.invoke_action(id, &key);
            return;
        }
        let entry = notification.app.desktop_entry.clone();
        if let Some(entry) = entry
            && let Err(error) = self.launcher.activate(&entry)
        {
            log::debug!("activating {entry:?} for notification {id} failed: {error}");
        }
        self.close(id, CloseReason::Dismissed);
    }
}

/// A handle to the daemon's state, cheap to clone and safe to hold in every
/// surface, callback and service.
#[derive(Clone)]
pub struct SharedState(Rc<RefCell<Shared>>);

impl SharedState {
    pub fn new(shared: Shared) -> Self {
        Self(Rc::new(RefCell::new(shared)))
    }

    pub fn borrow(&self) -> Ref<'_, Shared> {
        self.0.borrow()
    }

    pub fn borrow_mut(&self) -> RefMut<'_, Shared> {
        self.0.borrow_mut()
    }

    /// Marks state dirty so every surface repaints on the next loop iteration.
    pub fn bump(&self) {
        self.borrow_mut().bump();
    }

    pub fn version(&self) -> u64 {
        self.borrow().version
    }
}

/// The sink both transports are written against.
///
/// Every method is the corresponding [`Shared`] one, so an IPC request and a
/// D-Bus request that mean the same thing cannot behave differently.
impl NotificationSink for SharedState {
    fn post(&mut self, notification: Notification) -> NotificationId {
        self.borrow_mut().post(notification)
    }

    fn close(&mut self, id: NotificationId, reason: CloseReason) {
        self.borrow_mut().close(id, reason);
    }

    fn set_center_open(&mut self, open: Option<bool>) {
        self.borrow_mut().set_center_open(open);
    }

    fn set_do_not_disturb(&mut self, enabled: bool) {
        self.borrow_mut().set_do_not_disturb(enabled);
    }

    fn set_focus_mode(&mut self, name: Option<&str>) {
        self.borrow_mut().set_focus_mode(name);
    }

    fn dismiss_all(&mut self) {
        self.borrow_mut().dismiss_all();
    }

    fn invoke_action(&mut self, id: NotificationId, action: &str) {
        self.borrow_mut().invoke_action(id, action);
    }
}

/// The phone command a card action asks for, if any.
///
/// This is the one place `ui` and `ipc` meet: [`CardAction`] deliberately names
/// no transport type, so that a card can be asserted on without a socket, and
/// the translation is stated once here rather than in each surface. The
/// notification's [`Content`] is needed because answering addresses a specific
/// call and replying a specific conversation — the action alone does not say
/// which.
pub fn phone_command(content: &Content, action: &CardAction) -> Option<PhoneCommand> {
    match action {
        CardAction::Answer => match content {
            Content::Call(call) => Some(PhoneCommand::Pickup(call.call_id.clone())),
            _ => None,
        },
        CardAction::Decline => match content {
            Content::Call(call) => Some(PhoneCommand::Decline(call.call_id.clone())),
            _ => None,
        },
        CardAction::Media(media) => Some(match media {
            MediaAction::Play => PhoneCommand::Play,
            MediaAction::Pause => PhoneCommand::Pause,
            MediaAction::Next => PhoneCommand::Next,
            MediaAction::Previous => PhoneCommand::Previous,
            MediaAction::Seek(percent) => PhoneCommand::Seek(*percent),
        }),
        CardAction::Activate
        | CardAction::ToggleStack(_)
        | CardAction::ClearStack(_)
        | CardAction::ToggleMenu
        | CardAction::Dismiss
        | CardAction::Invoke(_)
        | CardAction::FocusReply
        | CardAction::SubmitReply => None,
    }
}

/// The command that sends `text` back to the conversation `content` came from.
///
/// `None` for anything but a chat card: a reply typed into something that is
/// not a conversation has nowhere to go, and guessing a destination would send
/// a private message to whatever the phone happened to think was current.
pub fn reply_command(content: &Content, text: &str) -> Option<PhoneCommand> {
    match content {
        Content::Chat(chat) => Some(PhoneCommand::Reply {
            conversation_id: chat.conversation_id.clone(),
            text: text.to_string(),
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::focus::FocusModes;
    use crate::{
        model::{Action, ActionKind, Call, Chat, Music},
        store::StoreConfig,
        ui::{Card, Theme},
    };
    use chrono::Local;
    use crownshell::TextContext;

    fn shared() -> Shared {
        let (outgoing, _receiver) = async_channel::unbounded();
        Shared::new(Store::new(StoreConfig::default()), Theme::light(), outgoing)
    }

    fn state() -> SharedState {
        SharedState::new(shared())
    }

    fn notification(app: &str) -> Notification {
        Notification::builder(app).summary("Ada").build()
    }

    fn drained(shared: &mut Shared) -> Vec<Outgoing> {
        let mut out = Vec::new();
        shared.drain_outgoing(&mut out);
        out
    }

    /// The published events as strings, which is enough to assert order and
    /// identity without `Outgoing` having to be `PartialEq`.
    fn published(shared: &mut Shared) -> Vec<String> {
        drained(shared)
            .into_iter()
            .map(|event| format!("{event:?}"))
            .collect()
    }

    fn call(call_id: &str) -> Content {
        Content::Call(Call {
            call_id: call_id.to_string(),
            contact: "Ada".to_string(),
            number: "+44".to_string(),
            avatar: None,
        })
    }

    fn chat(conversation_id: &str) -> Content {
        Content::Chat(Chat {
            conversation_id: conversation_id.to_string(),
            sender: "Ada".to_string(),
            avatar: None,
            sent_at: Local::now(),
        })
    }

    #[test]
    fn posting_through_the_sink_files_the_notification_and_bumps_the_version() {
        let mut state = state();
        let before = state.version();

        let id = state.post(notification("Mail"));

        assert_ne!(id, 0);
        assert_eq!(state.borrow().store.toasts().len(), 1);
        assert!(
            state.version() > before,
            "the surfaces were not told to repaint: {} -> {}",
            before,
            state.version()
        );
        assert_eq!(
            state.borrow().pending(),
            0,
            "posting is not something to announce; the transport already knows"
        );
    }

    #[test]
    fn closing_through_the_sink_publishes_exactly_one_closed_event() {
        let mut state = state();
        let id = state.post(notification("Mail"));

        state.close(id, CloseReason::Closed);
        assert_eq!(
            published(&mut state.borrow_mut()),
            vec![format!("{:?}", Outgoing::Closed(id, CloseReason::Closed))]
        );

        state.close(id, CloseReason::Closed);
        assert_eq!(
            state.borrow().pending(),
            1,
            "the second close hit history, which is still a close"
        );
        state.close(999, CloseReason::Closed);
        assert_eq!(
            state.borrow().pending(),
            1,
            "an id that was never posted must not be announced"
        );
    }

    #[test]
    fn the_center_is_set_by_some_and_toggled_by_none() {
        let mut state = state();

        state.set_center_open(Some(true));
        assert!(state.borrow().center_open);
        state.set_center_open(Some(true));
        state.set_center_open(None);
        assert!(!state.borrow().center_open);

        assert_eq!(
            published(&mut state.borrow_mut()),
            vec![
                format!("{:?}", Outgoing::CenterVisibility(true)),
                format!("{:?}", Outgoing::CenterVisibility(false)),
            ],
            "setting the state it is already in must not announce anything"
        );
    }

    #[test]
    fn do_not_disturb_is_announced_only_when_it_actually_changes() {
        let mut state = state();

        state.set_do_not_disturb(true);
        state.set_do_not_disturb(true);
        assert!(state.borrow().store.do_not_disturb());

        // Do Not Disturb is the built-in mode, so it announces both — once.
        assert_eq!(
            published(&mut state.borrow_mut()),
            vec![
                format!(
                    "{:?}",
                    Outgoing::FocusModeChanged(Some(FocusModes::DO_NOT_DISTURB.to_string()))
                ),
                format!("{:?}", Outgoing::DoNotDisturbChanged(true)),
            ]
        );
    }

    #[test]
    fn switching_between_two_modes_leaves_the_do_not_disturb_bool_alone() {
        let mut state = state();
        state.set_do_not_disturb(true);
        let _ = published(&mut state.borrow_mut());

        state.borrow_mut().set_focus_mode(None);
        let ending = published(&mut state.borrow_mut());
        assert_eq!(
            ending,
            vec![
                format!("{:?}", Outgoing::FocusModeChanged(None)),
                format!("{:?}", Outgoing::DoNotDisturbChanged(false)),
            ],
            "ending a mode has to tell a client that knows only the bool"
        );
    }

    #[test]
    fn an_unknown_focus_mode_leaves_the_active_one_alone() {
        let mut state = state();
        state.set_do_not_disturb(true);
        let _ = published(&mut state.borrow_mut());

        state.borrow_mut().set_focus_mode(Some("does-not-exist"));
        assert!(
            state.borrow().store.do_not_disturb(),
            "a typo is not a request to stop being quiet"
        );
        assert!(published(&mut state.borrow_mut()).is_empty());
    }

    #[test]
    fn dismiss_all_announces_every_toast_it_took_down_and_nothing_more() {
        let mut state = state();
        let first = state.post(notification("Mail"));
        let second = state.post(notification("Chat"));
        let _ = published(&mut state.borrow_mut());

        state.dismiss_all();
        assert_eq!(
            published(&mut state.borrow_mut()),
            vec![
                format!("{:?}", Outgoing::Closed(first, CloseReason::Dismissed)),
                format!("{:?}", Outgoing::Closed(second, CloseReason::Dismissed)),
            ]
        );

        state.dismiss_all();
        assert_eq!(
            state.borrow().pending(),
            0,
            "an empty stack has nothing to announce"
        );
    }

    #[test]
    fn invoking_an_action_announces_it_and_then_retires_the_notification() {
        let mut state = state();
        let id = state.post(notification("Mail"));
        let _ = published(&mut state.borrow_mut());

        state.invoke_action(id, "archive");
        assert_eq!(
            published(&mut state.borrow_mut()),
            vec![
                format!("{:?}", Outgoing::ActionInvoked(id, "archive".to_string())),
                format!("{:?}", Outgoing::Closed(id, CloseReason::Dismissed)),
            ],
            "the action must be announced before the close it causes"
        );

        state.invoke_action(4242, "archive");
        assert_eq!(
            state.borrow().pending(),
            0,
            "an action on a notification nobody has must not be announced"
        );
    }

    #[test]
    fn applying_an_incoming_message_does_exactly_what_the_sink_method_does() {
        let mut through_sink = state();
        let mut through_channel = shared();

        let id = through_sink.post(notification("Mail"));
        let mut posted = notification("Mail");
        posted.id = id;
        through_channel.apply(Incoming::Post(Box::new(posted)));

        through_sink.set_do_not_disturb(true);
        through_channel.apply(Incoming::SetDoNotDisturb(true));

        through_sink.set_center_open(None);
        through_channel.apply(Incoming::ToggleCenter);

        through_sink.invoke_action(id, "archive");
        through_channel.apply(Incoming::InvokeAction(id, "archive".to_string()));

        through_sink.dismiss_all();
        through_channel.apply(Incoming::DismissAll);

        assert_eq!(
            published(&mut through_sink.borrow_mut()),
            published(&mut through_channel),
            "the two transports diverged"
        );
        assert_eq!(
            through_sink.borrow().center_open,
            through_channel.center_open
        );
        assert_eq!(
            through_sink.borrow().store.history().len(),
            through_channel.store.history().len()
        );
    }

    #[test]
    fn every_close_variant_of_apply_matches_its_sink_method() {
        let mut sink = state();
        let mut channel = shared();
        let id = sink.post(notification("Mail"));
        let mut same = notification("Mail");
        same.id = id;
        channel.apply(Incoming::Post(Box::new(same)));

        sink.close(id, CloseReason::Expired);
        channel.apply(Incoming::Close(id, CloseReason::Expired));

        sink.set_center_open(Some(true));
        channel.apply(Incoming::OpenCenter);
        sink.set_center_open(Some(false));
        channel.apply(Incoming::CloseCenter);

        assert_eq!(published(&mut sink.borrow_mut()), published(&mut channel));
    }

    #[test]
    fn publishing_queues_in_order_and_draining_empties_the_queue() {
        let mut shared = shared();

        shared.publish(Outgoing::CenterVisibility(true));
        shared.publish(Outgoing::DoNotDisturbChanged(true));
        shared.publish(Outgoing::Closed(7, CloseReason::Expired));
        assert_eq!(shared.pending(), 3);

        let mut out = vec![Outgoing::Replied(1, "first".to_string())];
        shared.drain_outgoing(&mut out);

        assert_eq!(
            out.iter().map(|e| format!("{e:?}")).collect::<Vec<_>>(),
            vec![
                format!("{:?}", Outgoing::Replied(1, "first".to_string())),
                format!("{:?}", Outgoing::CenterVisibility(true)),
                format!("{:?}", Outgoing::DoNotDisturbChanged(true)),
                format!("{:?}", Outgoing::Closed(7, CloseReason::Expired)),
            ],
            "draining appended in the wrong order"
        );
        assert_eq!(shared.pending(), 0, "draining did not empty the queue");

        let mut again = Vec::new();
        shared.drain_outgoing(&mut again);
        assert!(again.is_empty(), "a second drain produced {again:?}");
    }

    #[test]
    fn every_card_action_translates_to_the_phone_command_it_names() {
        let call = call("call-42");

        assert_eq!(
            phone_command(&call, &CardAction::Answer),
            Some(PhoneCommand::Pickup("call-42".to_string()))
        );
        assert_eq!(
            phone_command(&call, &CardAction::Decline),
            Some(PhoneCommand::Decline("call-42".to_string()))
        );

        for (action, expected) in [
            (MediaAction::Play, PhoneCommand::Play),
            (MediaAction::Pause, PhoneCommand::Pause),
            (MediaAction::Next, PhoneCommand::Next),
            (MediaAction::Previous, PhoneCommand::Previous),
            (MediaAction::Seek(37), PhoneCommand::Seek(37)),
        ] {
            assert_eq!(
                phone_command(&Content::General, &CardAction::Media(action)),
                Some(expected),
                "{action:?} did not reach the phone"
            );
        }

        for action in [
            CardAction::Activate,
            CardAction::Dismiss,
            CardAction::Invoke("archive".to_string()),
            CardAction::FocusReply,
            CardAction::SubmitReply,
        ] {
            assert_eq!(
                phone_command(&call, &action),
                None,
                "{action:?} is not the phone's business"
            );
        }
    }

    /// Answering something that is not a call has no call id to answer, and
    /// inventing one would hang up on whatever the phone was really doing.
    #[test]
    fn answering_a_card_that_is_not_a_call_sends_nothing() {
        assert_eq!(phone_command(&Content::General, &CardAction::Answer), None);
        assert_eq!(phone_command(&chat("thread-1"), &CardAction::Decline), None);
    }

    #[test]
    fn a_reply_is_addressed_to_the_conversation_it_came_from() {
        assert_eq!(
            reply_command(&chat("thread-1"), "on my way"),
            Some(PhoneCommand::Reply {
                conversation_id: "thread-1".to_string(),
                text: "on my way".to_string(),
            })
        );
        assert_eq!(reply_command(&Content::General, "on my way"), None);
        assert_eq!(reply_command(&call("call-42"), "on my way"), None);
    }

    #[test]
    fn performing_a_call_action_sends_the_command_and_takes_the_card_down() {
        let mut shared = shared();
        let id = shared.post(
            Notification::builder("Phone")
                .content(call("call-42"))
                .build(),
        );
        let _ = published(&mut shared);

        assert_eq!(
            shared.perform(id, &CardAction::Answer),
            CardOutcome::Handled
        );
        assert_eq!(
            shared.phone.queued(),
            1,
            "the pickup did not reach the phone link"
        );
        assert_eq!(
            published(&mut shared),
            vec![format!(
                "{:?}",
                Outgoing::Closed(id, CloseReason::Dismissed)
            )]
        );
    }

    #[test]
    fn a_transport_button_leaves_the_card_on_screen() {
        let mut shared = shared();
        let id = shared.post(
            Notification::builder("Spotify")
                .content(Content::Music(Music {
                    title: "Blue Monday".to_string(),
                    artist: "New Order".to_string(),
                    album_art: None,
                    playing: true,
                    percent: 10,
                }))
                .build(),
        );

        shared.perform(id, &CardAction::Media(MediaAction::Pause));
        assert_eq!(shared.store.toasts().len(), 1, "the card was dismissed");
        assert_eq!(shared.phone.queued(), 1);
    }

    #[test]
    fn clicking_the_body_invokes_the_default_action_when_the_sender_offered_one() {
        let mut shared = shared();
        let with_default = shared.post(
            Notification::builder("Mail")
                .action(Action {
                    key: "default".to_string(),
                    label: "Open".to_string(),
                    kind: ActionKind::Default,
                })
                .build(),
        );
        let bare = shared.post(notification("Chat"));
        let _ = published(&mut shared);

        shared.perform(with_default, &CardAction::Activate);
        shared.perform(bare, &CardAction::Activate);

        assert_eq!(
            published(&mut shared),
            vec![
                format!(
                    "{:?}",
                    Outgoing::ActionInvoked(with_default, "default".to_string())
                ),
                format!(
                    "{:?}",
                    Outgoing::Closed(with_default, CloseReason::Dismissed)
                ),
                format!("{:?}", Outgoing::Closed(bare, CloseReason::Dismissed)),
            ]
        );
    }

    #[test]
    fn a_reply_is_published_sent_to_the_phone_and_closes_its_card() {
        let mut shared = shared();
        let id = shared.post(
            Notification::builder("Signal")
                .content(chat("thread-1"))
                .build(),
        );
        let _ = published(&mut shared);

        assert_eq!(
            shared.perform(id, &CardAction::SubmitReply),
            CardOutcome::SubmitReply,
            "the text lives in the card, so the surface has to drain it"
        );
        shared.submit_reply(id, "on my way".to_string());

        assert_eq!(shared.phone.queued(), 1);
        assert_eq!(
            published(&mut shared),
            vec![
                format!("{:?}", Outgoing::Replied(id, "on my way".to_string())),
                format!("{:?}", Outgoing::Closed(id, CloseReason::Dismissed)),
            ]
        );
    }

    #[test]
    fn an_empty_reply_is_a_mis_click_and_goes_nowhere() {
        let mut shared = shared();
        let id = shared.post(
            Notification::builder("Signal")
                .content(chat("thread-1"))
                .build(),
        );
        let _ = published(&mut shared);

        shared.submit_reply(id, "   \n".to_string());
        assert_eq!(shared.phone.queued(), 0);
        assert!(published(&mut shared).is_empty());
        assert_eq!(shared.store.toasts().len(), 1, "the card was taken down");
    }

    #[test]
    fn focusing_a_reply_is_left_entirely_to_the_surface() {
        let mut shared = shared();
        let id = shared.post(
            Notification::builder("Signal")
                .content(chat("thread-1"))
                .build(),
        );
        let _ = published(&mut shared);

        assert_eq!(
            shared.perform(id, &CardAction::FocusReply),
            CardOutcome::FocusReply
        );
        assert!(published(&mut shared).is_empty());
        assert_eq!(shared.store.toasts().len(), 1);
    }

    #[test]
    fn the_version_counter_rises_on_every_change_and_stands_still_otherwise() {
        let mut shared = shared();
        let mut previous = shared.version;
        let mut moved = |shared: &Shared| {
            let changed = shared.version != previous;
            previous = shared.version;
            changed
        };

        let id = shared.post(notification("Mail"));
        assert!(moved(&shared), "posting");
        shared.set_center_open(Some(true));
        assert!(moved(&shared), "opening the center");
        shared.set_center_open(Some(true));
        assert!(!moved(&shared), "opening an already-open center");
        shared.set_do_not_disturb(true);
        assert!(moved(&shared), "turning Do Not Disturb on");
        shared.close(id, CloseReason::Dismissed);
        assert!(moved(&shared), "closing");
        shared.close(id, CloseReason::Dismissed);
        assert!(moved(&shared), "closing the history copy");
        shared.close(id, CloseReason::Dismissed);
        assert!(!moved(&shared), "closing something that is already gone");
        shared.clear_history();
        assert!(moved(&shared), "clearing history");
    }

    /// The counter is what `needs_redraw` compares, and both surfaces read the
    /// same one, so a toast surface's click has to be able to repaint the
    /// center.
    #[test]
    fn a_bump_through_one_handle_is_visible_through_every_other() {
        let state = state();
        let other = state.clone();
        let before = other.version();

        state.bump();
        assert_eq!(other.version(), before + 1);
    }

    // ------------------------------------------------------------------
    // popup: stack reconciliation, swipe threshold, velocity conversion
    // ------------------------------------------------------------------

    fn reconciled(cards: &mut Vec<Card>, scratch: &mut Vec<Card>, store: &Store) -> usize {
        let mut decks = cards::Decks::default();
        let mut groups = store.toast_groups();
        groups.reverse();
        cards::reconcile(cards, scratch, &mut decks, &groups, &Theme::light(), 0.0)
    }

    #[test]
    fn reconciliation_builds_a_card_for_every_toast_in_stack_order() {
        let mut shared = shared();
        let first = shared.post(notification("Mail"));
        let second = shared.post(notification("Chat"));

        let (mut cards, mut scratch) = (Vec::new(), Vec::new());
        let live = reconciled(&mut cards, &mut scratch, &shared.store);

        assert_eq!(live, 2);
        assert_eq!(
            cards.iter().map(|card| card.id).collect::<Vec<_>>(),
            vec![first, second]
        );
    }

    #[test]
    fn reconciliation_keeps_the_card_a_replacement_replaced() {
        let mut shared = shared();
        let id = shared.post(notification("Mail"));
        let (mut cards, mut scratch) = (Vec::new(), Vec::new());
        reconciled(&mut cards, &mut scratch, &shared.store);
        let entering = cards[0].animation.alpha();

        let mut replacement = Notification::builder("Mail").summary("Ada, again").build();
        replacement.id = id;
        shared.store.post(replacement);
        let live = reconciled(&mut cards, &mut scratch, &shared.store);

        assert_eq!(live, 1);
        assert_eq!(cards.len(), 1, "the replacement built a second card");
        assert_eq!(
            cards[0].animation.alpha(),
            entering,
            "the card was rebuilt, so its animation restarted"
        );
    }

    #[test]
    fn a_card_whose_notification_left_the_store_starts_leaving_and_is_kept() {
        let mut shared = shared();
        let staying = shared.post(notification("Mail"));
        let going = shared.post(notification("Chat"));
        let (mut cards, mut scratch) = (Vec::new(), Vec::new());
        reconciled(&mut cards, &mut scratch, &shared.store);

        shared.close(going, CloseReason::Dismissed);
        let live = reconciled(&mut cards, &mut scratch, &shared.store);

        assert_eq!(live, 1, "only one toast is still on the stack");
        assert_eq!(cards.len(), 2, "the departing card was dropped too early");
        assert_eq!(cards[0].id, staying);
        assert_eq!(cards[1].id, going);
        assert_eq!(
            cards[1].animation.phase,
            crate::ui::Phase::Leaving,
            "the departing card was not asked to leave"
        );
    }

    #[test]
    fn a_leaving_card_is_dropped_only_once_its_animation_says_so() {
        let mut shared = shared();
        let id = shared.post(notification("Mail"));
        let (mut cards, mut scratch) = (Vec::new(), Vec::new());
        reconciled(&mut cards, &mut scratch, &shared.store);
        shared.close(id, CloseReason::Dismissed);
        reconciled(&mut cards, &mut scratch, &shared.store);

        let mut frames = 0;
        while !cards.is_empty() && frames < 600 {
            cards.retain_mut(|card| card.animation.step(1.0 / 60.0));
            reconciled(&mut cards, &mut scratch, &shared.store);
            frames += 1;
        }
        assert!(cards.is_empty(), "the card never finished leaving");
        assert!(
            frames > 1,
            "the card was dropped on the first frame, so nothing animated"
        );
    }

    #[test]
    fn only_a_rightward_drag_past_the_threshold_dismisses() {
        assert!(!popup::is_swipe(0.0));
        assert!(!popup::is_swipe(39.9));
        assert!(popup::is_swipe(popup::SWIPE_THRESHOLD));
        assert!(popup::is_swipe(120.0));
        assert!(
            !popup::is_swipe(-80.0),
            "dragging a card back towards the screen is not a dismissal"
        );
    }

    #[test]
    fn a_pointer_velocity_becomes_a_reveal_velocity_in_card_widths_per_second() {
        // One card width per second to the right is one reveal unit per second
        // towards off-screen, which the spring spells as negative.
        assert_eq!(popup::dismiss_velocity(376.0, 376.0), -1.0);
        assert_eq!(popup::dismiss_velocity(1880.0, 376.0), -5.0);
        assert_eq!(popup::dismiss_velocity(0.0, 376.0), 0.0);
        assert_eq!(
            popup::dismiss_velocity(400.0, 0.0),
            0.0,
            "a card with no width must not divide by zero"
        );
    }

    // ------------------------------------------------------------------
    // center: scroll clamping and group collapsing
    // ------------------------------------------------------------------

    #[test]
    fn scrolling_stops_at_the_top_and_at_the_end_of_the_content() {
        assert_eq!(center::clamp_scroll(-40.0, 900.0, 400.0), 0.0);
        assert_eq!(center::clamp_scroll(120.0, 900.0, 400.0), 120.0);
        assert_eq!(center::clamp_scroll(4000.0, 900.0, 400.0), 500.0);
    }

    #[test]
    fn content_that_fits_the_panel_does_not_scroll_at_all() {
        assert_eq!(center::clamp_scroll(200.0, 300.0, 400.0), 0.0);
        assert_eq!(center::clamp_scroll(0.0, 0.0, 400.0), 0.0);
    }

    /// Cards are retained, so the center must not rebuild one it already has —
    /// each rebuild throws away a shaped Parley layout.
    #[test]
    fn the_center_keeps_the_cards_it_already_built() {
        let mut shared = shared();
        let id = shared.post(notification("Mail"));
        shared.close(id, CloseReason::Dismissed);

        let mut tcx = TextContext::new();
        let theme = Theme::light();
        let groups = shared.store.groups();
        let mut cards = Vec::new();
        let mut scratch = Vec::new();
        let mut decks = cards::Decks::default();
        cards::reconcile(&mut cards, &mut scratch, &mut decks, &groups, &theme, 0.0);
        let before = cards[0].measure(
            theme.metrics.card_width,
            &mut crate::ui::MeasureCtx {
                text: &mut tcx,
                theme: &theme,
            },
        );

        cards::reconcile(&mut cards, &mut scratch, &mut decks, &groups, &theme, 0.0);
        assert_eq!(cards.len(), 1, "the card was rebuilt rather than kept");
        assert!(
            !cards[0].needs_measure(),
            "a retained card lost its measurement, and with it its shaped text"
        );
        assert_eq!(
            cards[0].measure(
                theme.metrics.card_width,
                &mut crate::ui::MeasureCtx {
                    text: &mut tcx,
                    theme: &theme
                }
            ),
            before
        );
    }

    #[test]
    fn the_center_builds_a_card_for_an_entry_it_has_never_seen() {
        let mut shared = shared();
        let id = shared.post(notification("Mail"));
        shared.close(id, CloseReason::Dismissed);

        let mut cards = Vec::new();
        let mut scratch = Vec::new();
        let mut decks = cards::Decks::default();
        let live = cards::reconcile(
            &mut cards,
            &mut scratch,
            &mut decks,
            &shared.store.groups(),
            &Theme::light(),
            0.0,
        );
        assert_eq!(live, 1);
        assert_eq!(cards[0].id, id);
        assert!(cards[0].needs_measure(), "a fresh card was never measured");
    }

    /// A launcher that records what it was asked to start.
    #[derive(Clone, Default)]
    struct Recorder(Rc<RefCell<Vec<String>>>);

    impl crate::launch::Launcher for Recorder {
        fn activate(&self, entry: &str) -> crate::error::Result<()> {
            self.0.borrow_mut().push(entry.to_string());
            Ok(())
        }
    }

    fn with_entry(app: &str, entry: &str) -> Notification {
        Notification::builder(app).desktop_entry(entry).build()
    }

    #[test]
    fn activating_a_card_with_no_default_action_launches_its_app() {
        let launched = Recorder::default();
        let mut state = shared();
        state.set_launcher(Box::new(launched.clone()));

        let id = state.post(with_entry("Mail", "org.mozilla.thunderbird"));
        state.perform(id, &CardAction::Activate);

        assert_eq!(*launched.0.borrow(), vec!["org.mozilla.thunderbird"]);
        assert!(
            state.store.toasts().next().is_none(),
            "an activated card still leaves the screen"
        );
    }

    #[test]
    fn an_app_that_declared_a_default_action_is_left_to_handle_it() {
        let launched = Recorder::default();
        let mut state = shared();
        state.set_launcher(Box::new(launched.clone()));

        let id = state.post(
            Notification::builder("Mail")
                .desktop_entry("org.mozilla.thunderbird")
                .action(Action {
                    key: "default".to_string(),
                    label: "Open".to_string(),
                    kind: ActionKind::Default,
                })
                .build(),
        );
        state.perform(id, &CardAction::Activate);

        assert!(
            launched.0.borrow().is_empty(),
            "the app wanted to handle this itself"
        );
        assert!(
            published(&mut state)
                .iter()
                .any(|event| event.contains("ActionInvoked")),
            "the action it declared was not published"
        );
    }

    #[test]
    fn a_notification_with_no_desktop_entry_launches_nothing() {
        let launched = Recorder::default();
        let mut state = shared();
        state.set_launcher(Box::new(launched.clone()));

        let id = state.post(notification("Mail"));
        state.perform(id, &CardAction::Activate);
        assert!(launched.0.borrow().is_empty());
    }
}
