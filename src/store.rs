//! The single source of truth for what is on screen and what is remembered.
//!
//! The store owns every policy decision the daemon makes — id allocation,
//! replacement, expiry, the toast limit, Do Not Disturb and how much history
//! to keep — and it makes them without touching Wayland, D-Bus or the clock
//! beyond the [`Instant`] it is handed. That is what lets the rules below be
//! tested exhaustively in milliseconds.
//!
//! # Where a notification lives
//!
//! An entry is in exactly one of two places at a time:
//!
//! * the **toast stack**, oldest first, which is what the popup surface draws;
//! * **history**, newest first, which is what the notification center draws.
//!
//! An entry moves from the first to the second when it expires, is dismissed,
//! or is pushed off the bottom of the stack — never the other way round. Both
//! are `VecDeque`s in draw order, so [`Store::toasts`] and [`Store::history`]
//! hand out iterators that allocate nothing; they are called on every frame,
//! at up to 144 Hz. [`Store::groups`] does allocate, but only the notification
//! center calls it and only while it is open.
//!
//! # Threading
//!
//! The store lives on the compositor's event-loop thread and is never locked.
//! The one exception is the id counter, which the D-Bus thread shares as an
//! `Arc<AtomicU32>` — see [`Store::id_counter`].

use std::{
    borrow::Cow,
    collections::{HashMap, VecDeque},
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    },
    time::{Duration, Instant},
};

use crate::{
    error::{Error, Result},
    focus::{AppPolicy, FocusModes, Outcome, policy::resolve},
    model::{AppIdentity, CloseReason, InterruptionLevel, Notification, NotificationId},
};

/// A notification plus the bookkeeping the UI needs but the model does not.
#[derive(Debug)]
pub struct Entry {
    pub notification: Notification,
    pub received: Instant,
    /// Set once the toast has left the screen; the entry lives on in history.
    pub dismissed: bool,
}

/// Tuning the store reads on every decision. Comes from the user's config.
#[derive(Debug, Clone)]
pub struct StoreConfig {
    pub default_timeout: Duration,
    /// How long a `Critical` toast stays up. `None` — the default — means a
    /// critical notification never disappears on its own, which is the whole
    /// point of the urgency.
    pub critical_timeout: Option<Duration>,
    pub max_toasts: usize,
    pub max_history: usize,
    pub focus: FocusModes,
    /// Keyed by [`AppIdentity::key`], lower-cased by the config loader.
    pub apps: HashMap<String, AppPolicy>,
}

impl Default for StoreConfig {
    fn default() -> Self {
        Self {
            default_timeout: Duration::from_secs(5),
            critical_timeout: None,
            max_toasts: 5,
            max_history: 100,
            focus: FocusModes::default(),
            apps: HashMap::new(),
        }
    }
}

/// Live notifications and the history behind them.
#[derive(Debug)]
pub struct Store {
    config: StoreConfig,
    /// Oldest first, so the newest toast draws last (bottom of the stack).
    toasts: VecDeque<Entry>,
    /// Newest first, which is the order the center reads it in.
    history: VecDeque<Entry>,
    /// Held by the active mode, oldest first. Not on screen and not in the
    /// center: a held notification has not happened yet as far as the user is
    /// concerned, but its sender may still replace or withdraw it.
    held: VecDeque<Entry>,
    ids: Arc<AtomicU32>,
    /// Index into `config.focus`.
    active_focus: Option<usize>,
}

impl Store {
    pub fn new(config: StoreConfig) -> Self {
        Self {
            config,
            toasts: VecDeque::new(),
            history: VecDeque::new(),
            held: VecDeque::new(),
            ids: Arc::new(AtomicU32::new(0)),
            active_focus: None,
        }
    }

    /// The counter [`Store::next_id`] allocates from, for the D-Bus thread.
    ///
    /// `Notify` has to return an id to its caller synchronously, long before
    /// the UI thread wakes up and posts the notification, so the transport
    /// allocates the id itself with [`allocate_id`] on this counter and hands
    /// it to [`Store::post`], which honours it. Sharing the counter is what
    /// keeps the two allocators from ever colliding.
    pub fn id_counter(&self) -> Arc<AtomicU32> {
        Arc::clone(&self.ids)
    }

    /// Allocates the next free id; never returns 0 (0 means "new" in the spec).
    pub fn next_id(&mut self) -> NotificationId {
        allocate_id(&self.ids)
    }

    /// Inserts (or replaces, when `notification.id` matches a live entry) and
    /// returns the id the notification was filed under.
    ///
    /// A replacement keeps the entry's position in the stack — a chat app
    /// editing its own notification must not make it jump to the front and
    /// shove the others down. Its timer does restart, because the content the
    /// user is being asked to read is new.
    ///
    /// A notification whose id is only in history is re-posted as a fresh
    /// toast under the same id, and the stale history copy is dropped so that
    /// no id is ever filed twice.
    pub fn post(&mut self, mut notification: Notification) -> NotificationId {
        if notification.id == 0 {
            notification.id = self.next_id();
        }
        let id = notification.id;

        if let Some(entry) = self
            .toasts
            .iter_mut()
            .chain(self.held.iter_mut())
            .find(|entry| entry.notification.id == id)
        {
            entry.notification = notification;
            entry.received = Instant::now();
            return id;
        }
        self.history.retain(|entry| entry.notification.id != id);

        let outcome = self.outcome_for(&mut notification);
        let entry = Entry {
            notification,
            received: Instant::now(),
            dismissed: outcome == Outcome::File,
        };

        match outcome {
            Outcome::Bypass => {
                self.toasts.push_back(entry);
                self.enforce_toast_limit();
            }
            Outcome::File => self.retain(entry),
            Outcome::Queue => self.hold(entry),
        }
        id
    }

    /// What the active mode does with `notification`, resolving its level first.
    ///
    /// The level is written back so every later reader — both cards, the
    /// center, a relay — sees the one the daemon actually judged it at.
    fn outcome_for(&self, notification: &mut Notification) -> Outcome {
        let key = notification.app.key();
        if notification.level != InterruptionLevel::Critical
            && let Some(level) = self.config.apps.get(key.as_ref()).and_then(|app| app.level)
        {
            // Guarded on Critical because a typo in a config file must not be
            // able to silence an alarm.
            notification.level = level;
        }
        let mode = self
            .active_focus
            .and_then(|index| self.config.focus.get(index));
        match resolve(mode, notification.level, &key) {
            // A transient notification is a status readout; delivering it ten
            // minutes late would be a lie, and `retain` drops it entirely.
            Outcome::Queue if notification.transient => Outcome::File,
            outcome => outcome,
        }
    }

    /// Holds `entry` for the active mode to release.
    ///
    /// A queue longer than the history it would become cannot all be delivered,
    /// so the oldest is filed into the center rather than dropped: a held
    /// notification is never lost, only demoted.
    fn hold(&mut self, entry: Entry) {
        self.held.push_back(entry);
        while self.held.len() > self.config.max_history {
            let Some(oldest) = self.held.pop_front() else {
                break;
            };
            self.retain(oldest);
        }
    }

    /// Re-judges everything held against the mode now in force and delivers
    /// what it admits, oldest first. Returns the ids that reached the screen.
    fn release_held(&mut self) -> Vec<NotificationId> {
        let mut delivered = Vec::new();
        for mut entry in std::mem::take(&mut self.held) {
            let id = entry.notification.id;
            match self.outcome_for(&mut entry.notification) {
                Outcome::Bypass => {
                    // A notification held for an hour has not been on screen
                    // for an hour, and would otherwise expire the instant it
                    // appeared.
                    entry.received = Instant::now();
                    self.toasts.push_back(entry);
                    self.enforce_toast_limit();
                    delivered.push(id);
                }
                Outcome::File => self.retain(entry),
                Outcome::Queue => self.held.push_back(entry),
            }
        }
        delivered
    }

    pub fn get(&self, id: NotificationId) -> Option<&Entry> {
        self.toasts
            .iter()
            .chain(self.history.iter())
            .chain(self.held.iter())
            .find(|entry| entry.notification.id == id)
    }

    pub fn get_mut(&mut self, id: NotificationId) -> Option<&mut Entry> {
        self.toasts
            .iter_mut()
            .chain(self.history.iter_mut())
            .chain(self.held.iter_mut())
            .find(|entry| entry.notification.id == id)
    }

    /// Removes from the toast stack. Returns false if the id is unknown.
    ///
    /// The entry drops into history unless it was `transient`. An id that is
    /// only in history is removed from it instead, which is what a per-card
    /// close button in the notification center does.
    pub fn close(&mut self, id: NotificationId, reason: CloseReason) -> bool {
        log::debug!("closing notification {id} ({reason:?})");

        if let Some(index) = self
            .toasts
            .iter()
            .position(|entry| entry.notification.id == id)
            && let Some(entry) = self.toasts.remove(index)
        {
            self.retain(entry);
            return true;
        }

        // A sender withdrawing a notification it posted into a focus mode is
        // honoured, rather than the notification being toasted an hour later.
        if let Some(index) = self
            .held
            .iter()
            .position(|entry| entry.notification.id == id)
        {
            self.held.remove(index);
            return true;
        }

        let before = self.history.len();
        self.history.retain(|entry| entry.notification.id != id);
        before != self.history.len()
    }

    /// Every toast whose timeout has elapsed, oldest first. Removes them.
    pub fn expire(&mut self, now: Instant) -> Vec<NotificationId> {
        let mut expired = Vec::new();
        let mut index = 0;
        while index < self.toasts.len() {
            let elapsed = deadline(&self.config, &self.toasts[index]).is_some_and(|limit| {
                now.saturating_duration_since(self.toasts[index].received) >= limit
            });
            if elapsed && let Some(entry) = self.toasts.remove(index) {
                expired.push(entry.notification.id);
                self.retain(entry);
            } else {
                index += 1;
            }
        }
        expired
    }

    /// Deadline of the soonest expiry, for arming the timer.
    ///
    /// `Some(Duration::ZERO)` means something is already due. `None` means
    /// nothing on screen expires by itself and no timer is needed at all.
    pub fn next_expiry(&self, now: Instant) -> Option<Duration> {
        self.toasts
            .iter()
            .filter_map(|entry| {
                let limit = deadline(&self.config, entry)?;
                Some(limit.saturating_sub(now.saturating_duration_since(entry.received)))
            })
            .min()
    }

    /// Toasts, newest last (draw order top to bottom).
    pub fn toasts(&self) -> impl ExactSizeIterator<Item = &Entry> {
        self.toasts.iter()
    }

    /// Everything retained for the center, newest first.
    pub fn history(&self) -> impl ExactSizeIterator<Item = &Entry> {
        self.history.iter()
    }

    /// History bucketed by [`AppIdentity::key`], newest group first.
    ///
    /// Groups keep the order their newest member appears in, so an app that
    /// has just notified rises to the top of the center without the whole
    /// history being re-sorted.
    pub fn groups(&self) -> Vec<Group<'_>> {
        Self::bucket(self.history.iter())
    }

    /// Live toasts bucketed by app, newest group first and newest card first
    /// inside each group — deck order.
    pub fn toast_groups(&self) -> Vec<Group<'_>> {
        Self::bucket(self.toasts.iter().rev())
    }

    /// Dismisses every live toast from `key`, as a deck's own Clear All does.
    pub fn dismiss_group(&mut self, key: &str) -> Vec<NotificationId> {
        let mut dismissed = Vec::new();
        let mut kept = VecDeque::with_capacity(self.toasts.len());
        while let Some(entry) = self.toasts.pop_front() {
            if entry.notification.app.key() == key {
                dismissed.push(entry.notification.id);
                self.retain(entry);
            } else {
                kept.push_back(entry);
            }
        }
        self.toasts = kept;
        dismissed
    }

    /// Drops every history entry from `key`.
    pub fn clear_group(&mut self, key: &str) {
        self.history
            .retain(|entry| entry.notification.app.key() != key);
    }

    fn bucket<'a>(entries: impl Iterator<Item = &'a Entry>) -> Vec<Group<'a>> {
        let mut groups: Vec<Group<'a>> = Vec::new();
        for entry in entries {
            let key = entry.notification.app.key();
            match groups.iter_mut().find(|group| group.key == key) {
                Some(group) => group.entries.push(entry),
                None => groups.push(Group {
                    key,
                    app: &entry.notification.app,
                    entries: vec![entry],
                }),
            }
        }
        groups
    }

    /// The mode in force, by the name the config gave it.
    pub fn focus_mode(&self) -> Option<&str> {
        let mode = self.config.focus.get(self.active_focus?)?;
        Some(&mode.name)
    }

    /// Activates `name`, or ends the active mode when `name` is `None`,
    /// returning the ids the change delivered to the screen.
    ///
    /// An unknown name leaves the mode alone rather than clearing it: a typo in
    /// a keybinding must not be the same request as "stop being quiet".
    pub fn set_focus_mode(&mut self, name: Option<&str>) -> Result<Vec<NotificationId>> {
        let index = match name {
            Some(name) => Some(
                self.config
                    .focus
                    .position(name)
                    .ok_or_else(|| Error::UnknownFocusMode(name.to_string()))?,
            ),
            None => None,
        };
        Ok(self.activate(index))
    }

    /// Whether any mode is in force.
    ///
    /// Do Not Disturb, to a client that knows only the bool — crownbar's bell —
    /// means "notifications are being held back", so every mode reads as on and
    /// turning it off ends whichever one was.
    pub fn do_not_disturb(&self) -> bool {
        self.active_focus.is_some()
    }

    /// Turns the built-in preset on, or ends whatever mode is on.
    ///
    /// Toasts already on screen are left alone: the user asked for quiet from
    /// now on, not for whatever they were reading to vanish mid-sentence.
    pub fn set_do_not_disturb(&mut self, enabled: bool) -> Vec<NotificationId> {
        let index = enabled.then(|| self.config.focus.do_not_disturb());
        self.activate(index)
    }

    /// The one place `active_focus` moves, so every entry point releases the
    /// queue and no caller has to remember to.
    fn activate(&mut self, index: Option<usize>) -> Vec<NotificationId> {
        if self.active_focus == index {
            return Vec::new();
        }
        self.active_focus = index;
        self.release_held()
    }

    /// Notifications a focus mode is holding back, oldest first.
    pub fn held(&self) -> impl ExactSizeIterator<Item = &Entry> {
        self.held.iter()
    }

    /// Clears the toast stack, returning the ids that were showing so the
    /// transports can emit `NotificationClosed` for each.
    pub fn dismiss_all(&mut self) -> Vec<NotificationId> {
        let mut dismissed = Vec::with_capacity(self.toasts.len());
        while let Some(entry) = self.toasts.pop_front() {
            dismissed.push(entry.notification.id);
            self.retain(entry);
        }
        dismissed
    }

    pub fn clear_history(&mut self) {
        self.history.clear();
    }

    pub fn config(&self) -> &StoreConfig {
        &self.config
    }

    /// Files a retired entry in history, unless it was `transient` — a
    /// transient notification is by definition one the user is not meant to
    /// find again.
    fn retain(&mut self, mut entry: Entry) {
        entry.dismissed = true;
        if entry.notification.transient {
            return;
        }
        self.history.push_front(entry);
        while self.history.len() > self.config.max_history {
            self.history.pop_back();
        }
    }

    /// Pushes the oldest toasts off the bottom of the stack. They are closed
    /// as `Expired` rather than `Dismissed`, because the user never saw them
    /// go.
    fn enforce_toast_limit(&mut self) {
        while self.toasts.len() > self.config.max_toasts {
            let Some(entry) = self.toasts.pop_front() else {
                break;
            };
            log::debug!(
                "toast {} pushed off a stack of {}",
                entry.notification.id,
                self.config.max_toasts
            );
            self.retain(entry);
        }
    }
}

/// One app's worth of history, as the notification center stacks it.
#[derive(Debug)]
pub struct Group<'a> {
    /// Borrowed when the app's key needs no lower-casing, which is the common
    /// case and the one on the frame path.
    pub key: Cow<'a, str>,
    pub app: &'a AppIdentity,
    pub entries: Vec<&'a Entry>,
}

/// Allocates a notification id from a counter shared between the store and the
/// D-Bus thread.
///
/// Zero is skipped on the way past, because `Notify` reads `replaces_id == 0`
/// as "this is a new notification" — handing out 0 would make a notification
/// that can never be replaced or closed. A daemon that runs long enough to
/// wrap `u32` keeps working; it just starts reusing ids that are long gone.
pub fn allocate_id(counter: &AtomicU32) -> NotificationId {
    let previous = counter.fetch_add(1, Ordering::Relaxed);
    match previous.wrapping_add(1) {
        0 => {
            counter.store(1, Ordering::Relaxed);
            1
        }
        id => id,
    }
}

/// How long `entry` may stay on screen, or `None` if it never expires.
///
/// Critical notifications are resolved against `critical_timeout`, so leaving
/// that unset makes them permanent whatever the sender asked for. That is
/// deliberate: an app should not be able to flash a critical alert for 200 ms.
fn deadline(config: &StoreConfig, entry: &Entry) -> Option<Duration> {
    if entry.notification.level == InterruptionLevel::Critical {
        return config
            .critical_timeout
            .and_then(|default| entry.notification.timeout.resolve(default));
    }
    let default = config
        .apps
        .get(entry.notification.app.key().as_ref())
        .and_then(|app| app.timeout)
        .unwrap_or(config.default_timeout);
    entry.notification.timeout.resolve(default)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        focus::{AppPolicy, Blocked, FocusMode, FocusModes},
        model::{Timeout, Urgency},
    };

    fn config() -> StoreConfig {
        StoreConfig {
            default_timeout: Duration::from_secs(5),
            critical_timeout: None,
            max_toasts: 3,
            max_history: 4,
            ..StoreConfig::default()
        }
    }

    fn store() -> Store {
        Store::new(config())
    }

    fn notification(app: &str, summary: &str) -> Notification {
        Notification::builder(app).summary(summary).build()
    }

    fn ids<'a>(entries: impl Iterator<Item = &'a Entry>) -> Vec<u32> {
        entries.map(|entry| entry.notification.id).collect()
    }

    fn toast_ids(store: &Store) -> Vec<u32> {
        ids(store.toasts())
    }

    fn history_ids(store: &Store) -> Vec<u32> {
        ids(store.history())
    }

    #[test]
    fn posting_allocates_an_id_and_puts_the_notification_on_the_stack() {
        let mut store = store();
        let id = store.post(notification("Mail", "Hello"));

        assert_ne!(id, 0, "0 is reserved for 'new' in the freedesktop spec");
        assert_eq!(toast_ids(&store), vec![id]);
        assert!(
            history_ids(&store).is_empty(),
            "a live toast is not in the center yet, got {:?}",
            history_ids(&store)
        );
        assert_eq!(store.get(id).map(|e| e.dismissed), Some(false));
    }

    #[test]
    fn a_pre_allocated_id_is_honoured_so_the_dbus_reply_stays_truthful() {
        let mut store = store();
        let mut notification = notification("Mail", "Hello");
        notification.id = 77;

        assert_eq!(store.post(notification), 77);
        assert_eq!(toast_ids(&store), vec![77]);
    }

    #[test]
    fn next_id_never_returns_zero() {
        let mut store = store();
        for _ in 0..8 {
            assert_ne!(store.next_id(), 0);
        }
    }

    #[test]
    fn ids_skip_zero_when_the_counter_wraps_past_u32_max() {
        let mut store = store();
        store.id_counter().store(u32::MAX, Ordering::Relaxed);

        let wrapped = store.next_id();
        assert_eq!(wrapped, 1, "wrapping must not hand out 0, got {wrapped}");
        assert_eq!(store.next_id(), 2, "and allocation continues from there");
    }

    #[test]
    fn the_dbus_side_allocates_from_the_same_counter_as_the_store() {
        let mut store = store();
        let counter = store.id_counter();

        let first = store.next_id();
        let second = allocate_id(&counter);
        assert_ne!(first, second, "both allocators share one sequence");
    }

    #[test]
    fn replacing_a_live_toast_keeps_its_position_in_the_stack() {
        let mut store = store();
        let first = store.post(notification("Mail", "one"));
        let second = store.post(notification("Chat", "two"));
        let third = store.post(notification("Music", "three"));

        let mut replacement = notification("Mail", "one, edited");
        replacement.id = first;
        assert_eq!(store.post(replacement), first);

        assert_eq!(
            toast_ids(&store),
            vec![first, second, third],
            "a replacement must not jump to the front of the stack"
        );
        assert_eq!(
            store.get(first).map(|e| e.notification.summary.as_str()),
            Some("one, edited")
        );
        assert_eq!(store.toasts().len(), 3, "and it must not duplicate");
    }

    #[test]
    fn replacing_an_entry_that_has_already_left_the_screen_re_toasts_it_once() {
        let mut store = store();
        let id = store.post(notification("Mail", "one"));
        assert!(store.close(id, CloseReason::Dismissed));

        let mut replacement = notification("Mail", "one again");
        replacement.id = id;
        store.post(replacement);

        assert_eq!(toast_ids(&store), vec![id]);
        assert!(
            history_ids(&store).is_empty(),
            "the stale history copy should have been dropped, got {:?}",
            history_ids(&store)
        );
    }

    #[test]
    fn do_not_disturb_records_normal_notifications_without_showing_them() {
        let mut store = store();
        store.set_do_not_disturb(true);

        let low = store.post(Notification::builder("Mail").urgency(Urgency::Low).build());
        let normal = store.post(notification("Mail", "quiet"));

        assert!(
            toast_ids(&store).is_empty(),
            "nothing should be on screen, got {:?}",
            toast_ids(&store)
        );
        assert_eq!(history_ids(&store), vec![normal, low]);
        assert_eq!(store.get(normal).map(|e| e.dismissed), Some(true));
    }

    #[test]
    fn do_not_disturb_never_suppresses_a_critical_notification() {
        let mut store = store();
        store.set_do_not_disturb(true);

        let id = store.post(
            Notification::builder("Alarm")
                .summary("Wake up")
                .urgency(Urgency::Critical)
                .build(),
        );

        assert_eq!(
            toast_ids(&store),
            vec![id],
            "critical is the urgency DND does not silence"
        );
    }

    #[test]
    fn a_transient_notification_suppressed_by_dnd_is_dropped_entirely() {
        let mut store = store();
        store.set_do_not_disturb(true);
        store.post(Notification::builder("Volume").transient(true).build());

        assert!(toast_ids(&store).is_empty());
        assert!(
            history_ids(&store).is_empty(),
            "transient plus suppressed leaves nothing behind, got {:?}",
            history_ids(&store)
        );
    }

    #[test]
    fn transient_notifications_never_reach_history() {
        let mut store = store();
        let id = store.post(
            Notification::builder("Volume")
                .summary("50%")
                .transient(true)
                .build(),
        );

        assert_eq!(toast_ids(&store), vec![id]);
        store.close(id, CloseReason::Expired);

        assert!(
            history_ids(&store).is_empty(),
            "a transient toast leaves no trace, got {:?}",
            history_ids(&store)
        );
        assert!(store.get(id).is_none());
    }

    #[test]
    fn closing_a_toast_moves_it_into_history_and_marks_it_dismissed() {
        let mut store = store();
        let id = store.post(notification("Mail", "Hello"));

        assert!(store.close(id, CloseReason::Dismissed));
        assert!(toast_ids(&store).is_empty());
        assert_eq!(history_ids(&store), vec![id]);
        assert_eq!(store.get(id).map(|e| e.dismissed), Some(true));
    }

    #[test]
    fn closing_an_unknown_id_reports_failure() {
        let mut store = store();
        store.post(notification("Mail", "Hello"));
        assert!(
            !store.close(4242, CloseReason::Closed),
            "4242 was never posted"
        );
    }

    #[test]
    fn closing_a_history_entry_removes_it_from_the_center() {
        let mut store = store();
        let id = store.post(notification("Mail", "Hello"));
        store.close(id, CloseReason::Dismissed);

        assert!(
            store.close(id, CloseReason::Dismissed),
            "second close hits history"
        );
        assert!(history_ids(&store).is_empty());
        assert!(
            !store.close(id, CloseReason::Dismissed),
            "and now it is gone"
        );
    }

    #[test]
    fn toasts_past_the_limit_leave_the_stack_but_stay_in_history() {
        let mut store = store();
        let posted: Vec<u32> = (0..5)
            .map(|i| store.post(notification("Mail", &format!("{i}"))))
            .collect();

        assert_eq!(
            toast_ids(&store),
            posted[2..].to_vec(),
            "only the newest {} should be on screen",
            config().max_toasts
        );
        assert_eq!(
            history_ids(&store),
            vec![posted[1], posted[0]],
            "the ones pushed off are remembered, newest first"
        );
    }

    #[test]
    fn history_drops_the_oldest_entry_once_it_is_full() {
        let mut store = store();
        let posted: Vec<u32> = (0..6)
            .map(|i| {
                let id = store.post(notification("Mail", &format!("{i}")));
                store.close(id, CloseReason::Dismissed);
                id
            })
            .collect();

        let history = history_ids(&store);
        assert_eq!(history.len(), config().max_history, "got {history:?}");
        assert_eq!(
            history,
            vec![posted[5], posted[4], posted[3], posted[2]],
            "the oldest entries are the ones forgotten"
        );
    }

    #[test]
    fn an_expired_toast_is_reported_once_and_then_retained() {
        let mut store = store();
        let id = store.post(
            Notification::builder("Mail")
                .timeout(Timeout::Millis(100))
                .build(),
        );

        let now = Instant::now();
        assert!(
            store.expire(now).is_empty(),
            "nothing is due the instant it is posted"
        );

        let later = now + Duration::from_millis(150);
        assert_eq!(store.expire(later), vec![id]);
        assert!(store.expire(later).is_empty(), "expiry must not repeat");
        assert_eq!(history_ids(&store), vec![id]);
    }

    #[test]
    fn expired_notifications_are_reported_in_post_order() {
        let mut store = store();
        let first = store.post(
            Notification::builder("Mail")
                .timeout(Timeout::Millis(300))
                .build(),
        );
        let second = store.post(
            Notification::builder("Chat")
                .timeout(Timeout::Millis(100))
                .build(),
        );

        let expired = store.expire(Instant::now() + Duration::from_secs(1));
        assert_eq!(
            expired,
            vec![first, second],
            "oldest first, regardless of which deadline was shorter"
        );
    }

    #[test]
    fn a_notification_asking_never_to_expire_is_left_alone() {
        let mut store = store();
        let id = store.post(
            Notification::builder("Update")
                .timeout(Timeout::Never)
                .build(),
        );

        assert!(
            store
                .expire(Instant::now() + Duration::from_secs(3600))
                .is_empty()
        );
        assert_eq!(toast_ids(&store), vec![id]);
        assert_eq!(store.next_expiry(Instant::now()), None);
    }

    #[test]
    fn critical_toasts_do_not_auto_expire_without_a_critical_timeout() {
        let mut store = store();
        store.post(
            Notification::builder("Alarm")
                .urgency(Urgency::Critical)
                .timeout(Timeout::Millis(50))
                .build(),
        );

        let expired = store.expire(Instant::now() + Duration::from_secs(3600));
        assert!(
            expired.is_empty(),
            "critical stays until the user acts, got {expired:?}"
        );
        assert_eq!(
            store.next_expiry(Instant::now()),
            None,
            "and no timer needs arming for it"
        );
    }

    #[test]
    fn critical_toasts_expire_once_a_critical_timeout_is_configured() {
        let mut store = Store::new(StoreConfig {
            critical_timeout: Some(Duration::from_secs(30)),
            ..config()
        });
        let id = store.post(
            Notification::builder("Alarm")
                .urgency(Urgency::Critical)
                .build(),
        );

        assert!(
            store
                .expire(Instant::now() + Duration::from_secs(10))
                .is_empty()
        );
        assert_eq!(
            store.expire(Instant::now() + Duration::from_secs(31)),
            vec![id]
        );
    }

    #[test]
    fn next_expiry_is_the_soonest_deadline_still_outstanding() {
        let mut store = store();
        store.post(
            Notification::builder("Mail")
                .timeout(Timeout::Millis(4000))
                .build(),
        );
        store.post(
            Notification::builder("Chat")
                .timeout(Timeout::Millis(1000))
                .build(),
        );

        let now = Instant::now();
        let next = store.next_expiry(now).expect("two toasts are ticking");
        assert!(
            next <= Duration::from_millis(1000) && next > Duration::from_millis(900),
            "expected roughly a second, got {next:?}"
        );
    }

    #[test]
    fn next_expiry_is_zero_once_something_is_overdue() {
        let mut store = store();
        store.post(
            Notification::builder("Mail")
                .timeout(Timeout::Millis(10))
                .build(),
        );

        let overdue = store
            .next_expiry(Instant::now() + Duration::from_secs(1))
            .expect("the toast is still on screen");
        assert_eq!(overdue, Duration::ZERO, "got {overdue:?}");
    }

    #[test]
    fn next_expiry_is_none_when_nothing_is_on_screen() {
        assert_eq!(store().next_expiry(Instant::now()), None);
    }

    #[test]
    fn dismiss_all_clears_the_stack_and_reports_every_id() {
        let mut store = store();
        let first = store.post(notification("Mail", "one"));
        let second = store.post(notification("Chat", "two"));

        assert_eq!(store.dismiss_all(), vec![first, second]);
        assert!(toast_ids(&store).is_empty());
        assert_eq!(history_ids(&store), vec![second, first]);
        assert!(store.dismiss_all().is_empty(), "and it is idempotent");
    }

    #[test]
    fn clear_history_empties_the_center_without_touching_the_stack() {
        let mut store = store();
        let kept = store.post(notification("Mail", "on screen"));
        let closed = store.post(notification("Chat", "gone"));
        store.close(closed, CloseReason::Dismissed);

        store.clear_history();
        assert!(history_ids(&store).is_empty());
        assert_eq!(toast_ids(&store), vec![kept]);
    }

    #[test]
    fn history_groups_by_app_key_with_the_most_recent_app_first() {
        let mut store = store();
        for (app, summary) in [
            ("Mail", "one"),
            ("Chat", "two"),
            ("Mail", "three"),
            ("Mail", "four"),
        ] {
            let id = store.post(notification(app, summary));
            store.close(id, CloseReason::Dismissed);
        }

        let groups = store.groups();
        assert_eq!(groups.len(), 2, "two apps notified, got {groups:?}");
        assert_eq!(groups[0].key, "mail", "Mail notified most recently");
        assert_eq!(groups[0].entries.len(), 3);
        assert_eq!(groups[1].key, "chat");
        assert_eq!(groups[1].app.name, "Chat");
    }

    #[test]
    fn grouping_uses_the_desktop_entry_so_a_renamed_app_stays_together() {
        let mut store = store();
        for name in ["Spotify", "Spotify Premium"] {
            let id = store.post(Notification::builder(name).desktop_entry("spotify").build());
            store.close(id, CloseReason::Dismissed);
        }

        let groups = store.groups();
        assert_eq!(groups.len(), 1, "got {groups:?}");
        assert_eq!(groups[0].entries.len(), 2);
    }

    #[test]
    fn get_mut_reaches_both_the_stack_and_history() {
        let mut store = store();
        let live = store.post(notification("Mail", "live"));
        let retired = store.post(notification("Chat", "retired"));
        store.close(retired, CloseReason::Dismissed);

        store
            .get_mut(live)
            .expect("live entry")
            .notification
            .body
            .push_str("edited");
        store
            .get_mut(retired)
            .expect("retired entry")
            .notification
            .body
            .push_str("edited");

        assert_eq!(
            store.get(live).map(|e| e.notification.body.as_str()),
            Some("edited")
        );
        assert_eq!(
            store.get(retired).map(|e| e.notification.body.as_str()),
            Some("edited")
        );
        assert!(store.get_mut(999).is_none());
    }

    #[test]
    fn a_zero_history_budget_remembers_nothing() {
        let mut store = Store::new(StoreConfig {
            max_history: 0,
            ..config()
        });
        let id = store.post(notification("Mail", "Hello"));
        store.close(id, CloseReason::Dismissed);

        assert!(history_ids(&store).is_empty());
    }

    fn focused(mode: FocusMode) -> Store {
        Store::new(StoreConfig {
            focus: FocusModes::new(vec![mode]),
            ..config()
        })
    }

    fn work() -> FocusMode {
        FocusMode {
            allowed_apps: vec!["thunderbird".to_string()],
            blocked: Blocked::Queue,
            ..FocusMode::new("work", InterruptionLevel::TimeSensitive)
        }
    }

    #[test]
    fn a_passive_notification_never_reaches_the_screen() {
        let mut store = store();
        let id = store.post(
            Notification::builder("Spotify")
                .level(InterruptionLevel::Passive)
                .build(),
        );
        assert!(store.toasts().next().is_none());
        assert_eq!(history_ids(&store), vec![id]);
    }

    #[test]
    fn a_muted_app_is_filed_without_a_toast() {
        let mut store = Store::new(StoreConfig {
            apps: HashMap::from([(
                "spotify".to_string(),
                AppPolicy {
                    level: Some(InterruptionLevel::Passive),
                    timeout: None,
                },
            )]),
            ..config()
        });
        store.post(Notification::builder("Spotify").build());
        assert!(store.toasts().next().is_none());
        assert_eq!(store.history().len(), 1);
    }

    #[test]
    fn a_per_app_level_override_cannot_silence_a_critical_notification() {
        let mut store = Store::new(StoreConfig {
            apps: HashMap::from([(
                "alarm".to_string(),
                AppPolicy {
                    level: Some(InterruptionLevel::Passive),
                    timeout: None,
                },
            )]),
            ..config()
        });
        store.post(
            Notification::builder("Alarm")
                .level(InterruptionLevel::Critical)
                .build(),
        );
        assert_eq!(store.toasts().len(), 1, "an alarm is not mutable by config");
    }

    #[test]
    fn an_app_timeout_override_beats_the_global_default() {
        let mut store = Store::new(StoreConfig {
            apps: HashMap::from([(
                "slow".to_string(),
                AppPolicy {
                    level: None,
                    timeout: Some(Duration::from_secs(60)),
                },
            )]),
            ..config()
        });
        store.post(Notification::builder("Slow").build());
        let now = Instant::now();
        assert!(
            store.next_expiry(now).unwrap_or_default() > Duration::from_secs(30),
            "the app's own timeout did not reach the deadline"
        );
    }

    #[test]
    fn a_mode_queues_what_it_blocks_and_ending_it_delivers_the_queue() {
        let mut store = focused(work());
        store.set_focus_mode(Some("work")).expect("work exists");

        let id = store.post(Notification::builder("Spotify").build());
        assert!(store.toasts().next().is_none(), "it should be held");
        assert!(store.history().next().is_none(), "held is not filed");
        assert_eq!(store.held().len(), 1);

        assert_eq!(
            store.set_focus_mode(None).expect("clearing always works"),
            vec![id]
        );
        assert_eq!(toast_ids(&store), vec![id]);
        assert_eq!(store.held().len(), 0);
    }

    #[test]
    fn a_mode_that_files_what_it_blocks_leaves_nothing_to_deliver() {
        let mut store = focused(FocusMode {
            blocked: Blocked::File,
            ..work()
        });
        store.set_focus_mode(Some("work")).expect("work exists");
        store.post(Notification::builder("Spotify").build());
        assert_eq!(store.history().len(), 1);
        assert_eq!(store.held().len(), 0);
        assert!(store.set_focus_mode(None).expect("clearing").is_empty());
    }

    #[test]
    fn an_allowed_app_toasts_while_the_mode_is_on() {
        let mut store = focused(work());
        store.set_focus_mode(Some("work")).expect("work exists");
        store.post(Notification::builder("Thunderbird").build());
        assert_eq!(store.toasts().len(), 1);
    }

    #[test]
    fn the_queue_is_released_oldest_first_with_its_clock_restarted() {
        let mut store = focused(work());
        store.set_focus_mode(Some("work")).expect("work exists");
        let first = store.post(Notification::builder("A").build());
        let second = store.post(Notification::builder("B").build());

        let released = store.set_focus_mode(None).expect("clearing");
        assert_eq!(released, vec![first, second]);
        assert_eq!(toast_ids(&store), vec![first, second]);
        // Restarted: a notification held past its timeout must still be seen.
        assert!(store.expire(Instant::now()).is_empty());
    }

    #[test]
    fn a_held_notification_can_be_found_and_closed_before_it_ever_shows() {
        let mut store = focused(work());
        store.set_focus_mode(Some("work")).expect("work exists");
        let id = store.post(Notification::builder("Spotify").build());

        assert!(store.get(id).is_some(), "a sender can still address it");
        assert!(store.close(id, CloseReason::Closed));
        assert_eq!(store.held().len(), 0);
        assert!(store.set_focus_mode(None).expect("clearing").is_empty());
    }

    #[test]
    fn replacing_a_held_notification_updates_it_in_place() {
        let mut store = focused(work());
        store.set_focus_mode(Some("work")).expect("work exists");
        let id = store.post(Notification::builder("Spotify").summary("One").build());
        store.post(
            Notification::builder("Spotify")
                .id(id)
                .summary("Two")
                .build(),
        );
        assert_eq!(store.held().len(), 1);
        assert_eq!(
            store
                .get(id)
                .map(|entry| entry.notification.summary.as_str()),
            Some("Two")
        );
    }

    #[test]
    fn a_transient_notification_is_filed_rather_than_held() {
        let mut store = focused(work());
        store.set_focus_mode(Some("work")).expect("work exists");
        store.post(Notification::builder("Sync").transient(true).build());
        assert_eq!(store.held().len(), 0, "a stale status readout is a lie");
        assert!(store.history().next().is_none(), "transient leaves nothing");
    }

    #[test]
    fn an_unknown_focus_mode_leaves_the_active_one_alone() {
        let mut store = focused(work());
        store.set_focus_mode(Some("work")).expect("work exists");
        assert!(store.set_focus_mode(Some("nope")).is_err());
        assert_eq!(store.focus_mode(), Some("work"));
    }

    #[test]
    fn do_not_disturb_reads_true_for_any_active_mode() {
        let mut store = focused(work());
        assert!(!store.do_not_disturb());
        store.set_focus_mode(Some("work")).expect("work exists");
        assert!(store.do_not_disturb(), "the bell knows only the bool");
        store.set_do_not_disturb(false);
        assert_eq!(store.focus_mode(), None, "turning it off ends the mode");
    }

    #[test]
    fn an_overflowing_queue_files_its_oldest_rather_than_dropping_it() {
        let mut store = focused(work());
        store.set_focus_mode(Some("work")).expect("work exists");
        for index in 0..config().max_history + 2 {
            store.post(Notification::builder(format!("App {index}")).build());
        }
        assert_eq!(store.held().len(), config().max_history);
        assert_eq!(
            store.history().len(),
            2,
            "the oldest were demoted, not lost"
        );
    }
}
