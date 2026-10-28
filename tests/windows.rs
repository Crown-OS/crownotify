//! Both windows, driven headlessly: what each shows for a snapshot, what a
//! click asks of the daemon, and what the surfaces let through.

use std::sync::Arc;

use calloop::channel::Channel;
use chrono::Local;
use crownotify::{
    daemon::{Request, Snapshot},
    model::{Action, ActionKind, Call, Chat, CloseReason, Content, Notification},
    profile::table::Profiles,
    ui::{CardAction, Theme, Window, center, popup},
};
use crownui::crowntest::{Query, Role, TestApp, role};
use crownui::prelude::{KeyboardInteractivity, Rect};
use tokio::sync::watch;

const STRIP: (f32, f32) = (400.0, 900.0);
const OUTPUT: (f32, f32) = (1440.0, 900.0);

/// A window over `snapshot`, the requests it sends, and the way to publish more.
struct Harness {
    app: TestApp,
    requests: Channel<Request>,
    _publish: watch::Sender<Arc<Snapshot>>,
}

impl Harness {
    fn popup(snapshot: Snapshot) -> Self {
        Self::build(snapshot, STRIP, popup::popup)
    }

    fn center(snapshot: Snapshot) -> Self {
        Self::build(snapshot, OUTPUT, center::center)
    }

    fn build<V: crownui::prelude::View>(
        snapshot: Snapshot,
        (width, height): (f32, f32),
        view: impl FnOnce(Window) -> V,
    ) -> Self {
        let (requests, received) = calloop::channel::channel();
        let (publish, snapshots) = watch::channel(Arc::new(snapshot));
        let window = Window {
            theme: Theme::light(),
            profiles: Profiles::default(),
            requests,
            snapshots,
        };
        let mut app = TestApp::with_size(view(window), width, height);
        app.settle();
        Self {
            app,
            requests: received,
            _publish: publish,
        }
    }

    fn sent(&self) -> Vec<Request> {
        std::iter::from_fn(|| self.requests.try_recv().ok()).collect()
    }

    fn input_region(&mut self) -> Vec<Rect> {
        self.app
            .inline()
            .last_surface()
            .and_then(|surface| surface.input_region.clone())
            .unwrap_or_default()
    }

    fn keyboard(&mut self) -> Option<KeyboardInteractivity> {
        self.app
            .inline()
            .last_surface()
            .and_then(|surface| surface.keyboard)
    }
}

/// A run of text reading exactly `content`.
fn label(content: &str) -> Query {
    role(Role::Text).name(content)
}

fn mail(id: u32, summary: &str) -> Notification {
    Notification::builder("Mail")
        .id(id)
        .summary(summary)
        .body("The quarterly figures are in.")
        .build()
}

fn toasts(toasts: Vec<Notification>) -> Snapshot {
    Snapshot {
        toasts,
        ..Snapshot::default()
    }
}

#[test]
fn a_toast_shows_what_it_says_and_only_its_card_takes_the_pointer() {
    let mut harness = Harness::popup(toasts(vec![mail(1, "Ada Lovelace")]));
    let app = &mut harness.app;
    assert!(app.exists(label("Ada Lovelace")), "{}", app.snapshot());
    assert!(app.exists(label("The quarterly figures are in.")));
    assert!(app.exists(label("Mail")));

    let card = app
        .find(role(Role::ListItem).name("Mail: Ada Lovelace"))
        .bounds;
    assert!(
        (card.max_x() - (STRIP.0 - 12.0)).abs() < 0.5,
        "the card does not hug the right edge: {card:?}"
    );
    let region = harness.input_region();
    assert!(!region.is_empty(), "the card takes no input");
    assert!(
        region.iter().all(|rect| rect.max_y() <= card.max_y() + 0.5),
        "the strip below the card swallows clicks: {region:?}"
    );
}

#[test]
fn clicking_a_toast_activates_it() {
    let mut harness = Harness::popup(toasts(vec![mail(1, "Ada Lovelace")]));
    harness.app.click(label("Ada Lovelace"));
    assert_eq!(
        harness.sent(),
        vec![Request::Perform(1, CardAction::Activate)]
    );
}

#[test]
fn the_close_button_appears_on_hover_and_dismisses() {
    let mut harness = Harness::popup(toasts(vec![mail(1, "Ada Lovelace")]));
    assert!(!harness.app.exists(role(Role::Button).name("Dismiss")));
    harness.app.hover(label("Ada Lovelace")).settle();
    harness.app.click(role(Role::Button).name("Dismiss"));
    assert_eq!(
        harness.sent(),
        vec![Request::Perform(1, CardAction::Dismiss)],
        "the click reached the card underneath as well"
    );
}

#[test]
fn actions_that_do_not_fit_open_from_the_chevron() {
    let mut builder = Notification::builder("Calendar")
        .id(5)
        .summary("Design review");
    for (key, label) in [
        ("snooze", "Snooze"),
        ("join", "Join"),
        ("decline", "Decline"),
        ("later", "Remind me later"),
    ] {
        builder = builder.action(Action {
            key: key.into(),
            label: label.into(),
            kind: ActionKind::Button,
        });
    }
    let mut harness = Harness::popup(toasts(vec![builder.build()]));
    assert!(
        !harness
            .app
            .exists(role(Role::Button).name("Remind me later"))
    );
    let chevron = harness
        .app
        .find(role(Role::Button).name("More actions"))
        .bounds;
    assert!(chevron.size.width > 0.0, "the chevron was squeezed away");

    harness
        .app
        .click(role(Role::Button).name("More actions"))
        .settle();
    harness
        .app
        .click(role(Role::Button).name("Remind me later"));
    assert_eq!(
        harness.sent(),
        vec![Request::Perform(5, CardAction::Invoke("later".into()))]
    );
}

#[test]
fn an_action_button_invokes_its_key() {
    let notification = Notification::builder("Mail")
        .id(4)
        .summary("Invite")
        .action(Action {
            key: "accept".into(),
            label: "Accept".into(),
            kind: ActionKind::Button,
        })
        .build();
    let mut harness = Harness::popup(toasts(vec![notification]));
    harness.app.click(role(Role::Button).name("Accept"));
    assert_eq!(
        harness.sent(),
        vec![Request::Perform(4, CardAction::Invoke("accept".into()))]
    );
}

#[test]
fn a_call_is_answered_and_declined_from_its_own_buttons() {
    let call = Notification::builder("Phone")
        .id(7)
        .content(Content::Call(Call {
            call_id: "call-1".into(),
            contact: "Grace Hopper".into(),
            number: "+1 555 0100".into(),
            avatar: None,
        }))
        .build();
    let mut harness = Harness::popup(toasts(vec![call]));
    assert!(harness.app.exists(label("Grace Hopper")));
    harness.app.click(role(Role::Button).name("Answer"));
    harness.app.click(role(Role::Button).name("Decline"));
    assert_eq!(
        harness.sent(),
        vec![
            Request::Perform(7, CardAction::Answer),
            Request::Perform(7, CardAction::Decline),
        ]
    );
}

#[test]
fn an_inline_reply_takes_the_keyboard_only_while_it_is_open() {
    let chat = Notification::builder("Signal")
        .id(9)
        .summary("Ada")
        .body("Are we still on for five?")
        .content(Content::Chat(Chat {
            conversation_id: "thread-7".into(),
            sender: "Ada".into(),
            avatar: None,
            sent_at: Local::now(),
        }))
        .action(Action {
            key: "inline-reply".into(),
            label: "Reply".into(),
            kind: ActionKind::InlineReply {
                placeholder: "Reply to Ada".into(),
            },
        })
        .build();
    let mut harness = Harness::popup(toasts(vec![chat]));
    assert_ne!(harness.keyboard(), Some(KeyboardInteractivity::OnDemand));

    harness.app.click(role(Role::Button).name("Reply")).settle();
    assert_eq!(harness.keyboard(), Some(KeyboardInteractivity::OnDemand));

    harness
        .app
        .type_text("on my way")
        .press_key("Enter")
        .settle();
    assert_eq!(harness.sent(), vec![Request::Reply(9, "on my way".into())]);
    assert_eq!(harness.keyboard(), Some(KeyboardInteractivity::None));
}

#[test]
fn a_collapsed_deck_opens_on_the_first_click_and_clears_from_its_header() {
    let mut harness = Harness::popup(toasts(vec![mail(1, "First"), mail(2, "Second")]));
    assert!(!harness.app.exists(role(Role::Button).name("Clear All")));

    harness.app.click(label("Second")).settle();
    assert!(
        harness.sent().is_empty(),
        "the click activated instead of opening"
    );
    harness.app.click(role(Role::Button).name("Clear All"));
    assert_eq!(harness.sent(), vec![Request::DismissGroup("mail".into())]);
}

#[test]
fn a_swipe_to_the_right_dismisses_the_toast() {
    let mut harness = Harness::popup(toasts(vec![mail(1, "Ada Lovelace")]));
    let card = harness
        .app
        .find(role(Role::ListItem).name("Mail: Ada Lovelace"))
        .bounds;
    let (x, y) = (card.min_x() + 40.0, card.min_y() + 20.0);
    harness
        .app
        .pointer_down(x, y)
        .pointer_move(x + 20.0, y)
        .pointer_move(x + 60.0, y)
        .pointer_move(x + 120.0, y)
        .pointer_up(x + 120.0, y)
        .settle();
    assert_eq!(
        harness.sent(),
        vec![Request::Close(1, CloseReason::Dismissed)]
    );
}

#[test]
fn a_closed_center_draws_nothing_and_takes_no_input() {
    let mut harness = Harness::center(Snapshot {
        history: vec![mail(3, "Old news")],
        ..Snapshot::default()
    });
    assert!(!harness.app.exists(label("Notifications")));
    assert!(
        harness
            .input_region()
            .iter()
            .all(|rect| rect.size.width <= 0.0),
        "a closed center swallows clicks: {:?}",
        harness.input_region()
    );
}

#[test]
fn an_open_center_lists_history_and_closes_on_escape_or_outside_click() {
    let mut harness = Harness::center(Snapshot {
        history: vec![mail(3, "Old news")],
        center_open: true,
        ..Snapshot::default()
    });
    let app = &mut harness.app;
    assert!(app.exists(label("Notifications")), "{}", app.snapshot());
    assert!(app.exists(label("Old news")));
    assert_eq!(harness.keyboard(), Some(KeyboardInteractivity::OnDemand));

    harness
        .app
        .focus(role(Role::Button).name("Do Not Disturb"))
        .press_key("Escape");
    harness.app.pointer_down(20.0, 20.0).pointer_up(20.0, 20.0);
    assert_eq!(
        harness.sent(),
        vec![Request::SetCenterOpen(false), Request::SetCenterOpen(false)]
    );
}

#[test]
fn the_center_header_clears_history_and_toggles_do_not_disturb() {
    let mut harness = Harness::center(Snapshot {
        history: vec![mail(3, "Old news")],
        center_open: true,
        ..Snapshot::default()
    });
    harness.app.click(role(Role::Button).name("Do Not Disturb"));
    harness.app.click(role(Role::Button).name("Clear All"));
    assert_eq!(
        harness.sent(),
        vec![Request::SetDoNotDisturb(true), Request::ClearHistory]
    );
}
