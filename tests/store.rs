//! Store lifecycle: what a notification goes through from `Notify` to the
//! notification center, exercised through the public API only.

use std::time::{Duration, Instant};

use crownotify::{
    CloseReason, Notification, Store, StoreConfig, Timeout, Urgency, allocate_id,
    model::{Action, ActionKind, Content, Music},
};

fn store() -> Store {
    Store::new(StoreConfig {
        default_timeout: Duration::from_secs(5),
        critical_timeout: None,
        max_toasts: 3,
        max_history: 5,
        ..StoreConfig::default()
    })
}

fn toast_ids(store: &Store) -> Vec<u32> {
    store.toasts().map(|entry| entry.notification.id).collect()
}

fn history_ids(store: &Store) -> Vec<u32> {
    store.history().map(|entry| entry.notification.id).collect()
}

#[test]
fn a_notification_travels_from_the_stack_to_the_center_and_out_again() {
    let mut store = store();

    let id = store.post(
        Notification::builder("Mail")
            .summary("Ada Lovelace")
            .body("Re: the Analytical Engine")
            .desktop_entry("org.gnome.Evolution")
            .build(),
    );
    assert_eq!(toast_ids(&store), vec![id], "it starts life as a toast");
    assert!(history_ids(&store).is_empty());

    assert!(store.close(id, CloseReason::Dismissed));
    assert!(toast_ids(&store).is_empty(), "and leaves the screen");
    assert_eq!(history_ids(&store), vec![id], "but is remembered");
    assert_eq!(
        store.get(id).map(|entry| entry.dismissed),
        Some(true),
        "and is marked as having been dismissed"
    );

    let groups = store.groups();
    assert_eq!(groups.len(), 1, "got {groups:?}");
    assert_eq!(groups[0].key, "org.gnome.evolution");

    store.clear_history();
    assert!(store.get(id).is_none(), "clearing the center forgets it");
}

#[test]
fn replacing_a_notification_updates_it_without_moving_it() {
    let mut store = store();
    let first = store.post(Notification::builder("Mail").summary("one").build());
    let second = store.post(Notification::builder("Chat").summary("two").build());

    let replacement = Notification::builder("Chat")
        .id(second)
        .summary("two, edited")
        .action(Action {
            key: "default".into(),
            label: "Open".into(),
            kind: ActionKind::Default,
        })
        .build();
    assert_eq!(store.post(replacement), second, "the id is reused");

    assert_eq!(
        toast_ids(&store),
        vec![first, second],
        "a replacement keeps its place in the stack"
    );
    let entry = store.get(second).expect("the replaced entry");
    assert_eq!(entry.notification.summary, "two, edited");
    assert!(
        entry.notification.default_action().is_some(),
        "the new action list replaces the old one"
    );
}

#[test]
fn a_busy_app_never_pushes_more_than_the_configured_number_of_toasts() {
    let mut store = store();
    let posted: Vec<u32> = (0..7)
        .map(|index| {
            store.post(
                Notification::builder("Mail")
                    .summary(format!("message {index}"))
                    .build(),
            )
        })
        .collect();

    assert_eq!(
        toast_ids(&store),
        posted[4..].to_vec(),
        "only the three newest are on screen"
    );
    assert_eq!(
        history_ids(&store),
        vec![posted[3], posted[2], posted[1], posted[0]],
        "the rest are in the center, newest first"
    );
}

#[test]
fn expiry_walks_the_stack_in_post_order_and_arms_the_next_timer() {
    let mut store = store();
    let quick = store.post(
        Notification::builder("Mail")
            .timeout(Timeout::Millis(200))
            .build(),
    );
    let slow = store.post(
        Notification::builder("Chat")
            .timeout(Timeout::Millis(2000))
            .build(),
    );
    let permanent = store.post(
        Notification::builder("Update")
            .timeout(Timeout::Never)
            .build(),
    );

    let start = Instant::now();
    let next = store.next_expiry(start).expect("two toasts are ticking");
    assert!(
        next <= Duration::from_millis(200),
        "the soonest deadline is 200ms, got {next:?}"
    );

    assert_eq!(
        store.expire(start + Duration::from_millis(500)),
        vec![quick]
    );
    assert_eq!(store.expire(start + Duration::from_secs(3)), vec![slow]);
    assert_eq!(
        toast_ids(&store),
        vec![permanent],
        "a notification asking never to expire stays"
    );
    assert_eq!(
        store.next_expiry(start + Duration::from_secs(3)),
        None,
        "with nothing left ticking, no timer is needed"
    );
    assert_eq!(history_ids(&store), vec![slow, quick]);
}

#[test]
fn do_not_disturb_files_notifications_away_but_lets_critical_through() {
    let mut store = store();
    store.set_do_not_disturb(true);
    assert!(store.do_not_disturb());

    let quiet = store.post(Notification::builder("Mail").summary("quiet").build());
    let alarm = store.post(
        Notification::builder("Clock")
            .summary("Wake up")
            .urgency(Urgency::Critical)
            .build(),
    );

    assert_eq!(toast_ids(&store), vec![alarm], "only the alarm is shown");
    assert_eq!(history_ids(&store), vec![quiet]);

    store.set_do_not_disturb(false);
    let loud = store.post(Notification::builder("Mail").summary("loud").build());
    assert_eq!(toast_ids(&store), vec![alarm, loud]);
}

#[test]
fn a_transient_toast_leaves_nothing_behind_however_it_ends() {
    let mut store = store();

    let dismissed = store.post(
        Notification::builder("Volume")
            .summary("50%")
            .transient(true)
            .build(),
    );
    store.close(dismissed, CloseReason::Dismissed);

    let expired = store.post(
        Notification::builder("Volume")
            .summary("60%")
            .transient(true)
            .timeout(Timeout::Millis(100))
            .build(),
    );
    assert_eq!(
        store.expire(Instant::now() + Duration::from_secs(1)),
        vec![expired]
    );

    assert!(
        history_ids(&store).is_empty(),
        "transient notifications never reach the center, got {:?}",
        history_ids(&store)
    );
    assert!(store.groups().is_empty());
}

#[test]
fn the_center_groups_a_mixed_history_by_app() {
    let mut store = store();
    for (app, entry, summary) in [
        ("Spotify", "spotify", "Blue Monday"),
        ("Mail", "org.gnome.evolution", "Invoice"),
        ("Spotify", "spotify", "Temptation"),
    ] {
        let id = store.post(
            Notification::builder(app)
                .desktop_entry(entry)
                .summary(summary)
                .content(Content::Music(Music {
                    title: summary.into(),
                    artist: "New Order".into(),
                    album_art: None,
                    playing: true,
                    percent: 0,
                }))
                .build(),
        );
        store.close(id, CloseReason::Expired);
    }

    let groups = store.groups();
    assert_eq!(groups.len(), 2, "got {groups:?}");
    assert_eq!(groups[0].key, "spotify", "Spotify notified most recently");
    assert_eq!(groups[0].entries.len(), 2);
    assert_eq!(
        groups[0].entries[0].notification.summary, "Temptation",
        "within a group the newest comes first"
    );
    assert_eq!(groups[1].key, "org.gnome.evolution");
}

#[test]
fn dismiss_all_empties_the_screen_without_emptying_the_center() {
    let mut store = store();
    let ids: Vec<u32> = (0..3)
        .map(|index| {
            store.post(
                Notification::builder("Mail")
                    .summary(format!("{index}"))
                    .build(),
            )
        })
        .collect();

    assert_eq!(store.dismiss_all(), ids);
    assert!(toast_ids(&store).is_empty());
    assert_eq!(history_ids(&store).len(), 3);
}

#[test]
fn the_id_counter_is_shared_with_the_transport_that_replies_to_notify() {
    let mut store = store();
    let counter = store.id_counter();

    // What the D-Bus thread does: allocate, reply to the caller, then hand the
    // notification to the UI thread, which must file it under that same id.
    let allocated = allocate_id(&counter);
    let filed = store.post(Notification::builder("Mail").id(allocated).build());
    assert_eq!(filed, allocated);

    let from_store = store.next_id();
    assert_ne!(
        from_store, allocated,
        "the two allocators must not collide, both produced {allocated}"
    );
    assert!(store.get(from_store).is_none(), "and the id is still free");
}

#[test]
fn toast_groups_bucket_live_toasts_newest_first() {
    let mut store = store();
    let first = store.post(Notification::builder("Mail").summary("One").build());
    let second = store.post(Notification::builder("Chat").summary("Two").build());
    let third = store.post(Notification::builder("Mail").summary("Three").build());

    let groups = store.toast_groups();
    assert_eq!(groups.len(), 2, "two apps, two decks");
    assert_eq!(groups[0].key, "mail", "the app that notified last leads");
    assert_eq!(
        groups[0]
            .entries
            .iter()
            .map(|entry| entry.notification.id)
            .collect::<Vec<_>>(),
        vec![third, first],
        "newest card on top of its own deck"
    );
    assert_eq!(groups[1].entries.len(), 1);
    assert_eq!(groups[1].entries[0].notification.id, second);
}

#[test]
fn dismissing_a_deck_takes_down_only_that_apps_toasts() {
    let mut store = store();
    let mail = store.post(Notification::builder("Mail").build());
    let chat = store.post(Notification::builder("Chat").build());
    let also_mail = store.post(Notification::builder("Mail").build());

    let dismissed = store.dismiss_group("mail");
    assert_eq!(dismissed, vec![mail, also_mail]);
    assert_eq!(toast_ids(&store), vec![chat], "the other app is untouched");
    assert_eq!(
        history_ids(&store).len(),
        2,
        "a dismissed deck is still findable in the center"
    );
}

#[test]
fn clearing_a_deck_empties_only_that_apps_history() {
    let mut store = store();
    let mail = store.post(Notification::builder("Mail").build());
    let chat = store.post(Notification::builder("Chat").build());
    store.close(mail, CloseReason::Dismissed);
    store.close(chat, CloseReason::Dismissed);

    store.clear_group("mail");
    assert_eq!(history_ids(&store), vec![chat]);
}
