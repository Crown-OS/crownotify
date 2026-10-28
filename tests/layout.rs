//! Card measurement and stack layout, end to end through the public API.
//!
//! Both are pure functions of (content, width, theme), so none of this needs a
//! compositor or a GPU — only a [`TextContext`], which builds headlessly from
//! the system font database. That is the property being protected as much as
//! the numbers themselves: the day the layout needs a surface to measure
//! itself is the day none of this can be tested at all.

use std::time::Instant;

use chrono::{Duration, Local, TimeZone};
use crownshell::{TextContext, kurbo::Size};

use crownotify::{
    Action, ActionKind, Content, Music, Notification, Urgency,
    profile::{ContentHint, default_profile, profile_for},
    store::Entry,
    ui::Deck,
    ui::{Card, MeasureCtx, Stack, Theme, card_left, card_width, relative_time},
};

/// A body long enough that no reasonable column fits it in four lines.
const LONG_BODY: &str = "The quick brown fox jumps over the lazy dog while the compositor waits \
     for a frame, and then it does it again, and again, and once more for luck, until there is \
     very obviously more text here than any notification should ever carry in a toast.";

const SURFACE: Size = Size::new(400.0, 900.0);

fn entry(notification: Notification) -> Entry {
    Entry {
        notification,
        received: Instant::now(),
        dismissed: false,
    }
}

fn mail(body: &str) -> Entry {
    entry(
        Notification::builder("Mail")
            .id(1)
            .summary("Ada Lovelace")
            .body(body)
            .build(),
    )
}

fn card(entry: &Entry, theme: &Theme) -> Card {
    Card::new(entry, theme, 0.0)
}

fn height(card: &mut Card, tcx: &mut TextContext, theme: &Theme) -> f64 {
    card.measure(
        theme.metrics.card_width,
        &mut MeasureCtx { text: tcx, theme },
    )
}

/// One deck per card, which is what every notification from a different app
/// produces and what the geometry must reduce to.
fn solo(count: usize) -> Vec<(Deck, std::ops::Range<usize>)> {
    (0..count)
        .map(|index| (Deck::new(format!("app-{index}")), index..index + 1))
        .collect()
}

#[test]
fn card_height_grows_with_the_length_of_the_body() {
    let theme = Theme::light();
    let mut tcx = TextContext::new();

    let bodies = [
        "Hi",
        "See you at five, by the fountain.",
        "See you at five, by the fountain, the one with the bronze fish on it, near the gate.",
    ];
    let heights: Vec<f64> = bodies
        .iter()
        .map(|body| height(&mut card(&mail(body), &theme), &mut tcx, &theme))
        .collect();

    for pair in heights.windows(2) {
        assert!(
            pair[1] >= pair[0],
            "a longer body made a shorter card: {} then {}",
            pair[0],
            pair[1]
        );
    }
    assert!(
        heights[2] > heights[0],
        "three lines of body were no taller than one: {} vs {}",
        heights[2],
        heights[0]
    );
}

#[test]
fn the_four_line_body_clamp_stops_the_card_growing() {
    let theme = Theme::light();
    let mut tcx = TextContext::new();

    let clamped = height(&mut card(&mail(LONG_BODY), &theme), &mut tcx, &theme);
    let doubled = height(
        &mut card(&mail(&format!("{LONG_BODY} {LONG_BODY}")), &theme),
        &mut tcx,
        &theme,
    );
    let tenfold = height(
        &mut card(&mail(&LONG_BODY.repeat(10)), &theme),
        &mut tcx,
        &theme,
    );

    assert_eq!(clamped, doubled, "doubling a clamped body grew the card");
    assert_eq!(
        clamped, tenfold,
        "ten copies of a clamped body grew the card"
    );

    let one_line = height(&mut card(&mail("Hi"), &theme), &mut tcx, &theme);
    assert!(
        clamped > one_line,
        "the clamped card {clamped} is not taller than a one-line one {one_line}"
    );
}

#[test]
fn a_card_with_actions_is_taller_than_the_same_card_without() {
    let theme = Theme::light();
    let mut tcx = TextContext::new();

    let bare = height(&mut card(&mail("Ready."), &theme), &mut tcx, &theme);
    let with_actions = height(
        &mut card(
            &entry(
                Notification::builder("Mail")
                    .id(1)
                    .summary("Ada Lovelace")
                    .body("Ready.")
                    .action(Action {
                        key: "archive".into(),
                        label: "Archive".into(),
                        kind: ActionKind::Button,
                    })
                    .build(),
            ),
            &theme,
        ),
        &mut tcx,
        &theme,
    );

    assert!(
        with_actions >= bare + theme.metrics.button,
        "an action row added only {} px, less than a button is tall",
        with_actions - bare
    );
}

#[test]
fn a_critical_notification_makes_room_for_its_eyebrow() {
    let theme = Theme::light();
    let mut tcx = TextContext::new();

    let normal = height(&mut card(&mail("Ready."), &theme), &mut tcx, &theme);
    let critical = height(
        &mut card(
            &entry(
                Notification::builder("Mail")
                    .id(1)
                    .summary("Ada Lovelace")
                    .body(LONG_BODY)
                    .urgency(Urgency::Critical)
                    .build(),
            ),
            &theme,
        ),
        &mut tcx,
        &theme,
    );
    assert!(critical > normal, "{critical} vs {normal}");
}

#[test]
fn measurement_is_the_same_in_both_appearances() {
    let mut tcx = TextContext::new();
    let (light, dark) = (Theme::light(), Theme::dark());

    let in_light = height(&mut card(&mail(LONG_BODY), &light), &mut tcx, &light);
    let in_dark = height(&mut card(&mail(LONG_BODY), &dark), &mut tcx, &dark);
    assert_eq!(
        in_light, in_dark,
        "switching appearance re-flowed the stack, which it must never do"
    );
}

#[test]
fn stack_offsets_accumulate_with_the_gap() {
    let theme = Theme::light();
    let mut tcx = TextContext::new();
    let mut cards = vec![
        card(&mail("one"), &theme),
        card(&mail(LONG_BODY), &theme),
        card(&mail("three"), &theme),
    ];
    let mut stack = Stack::new();

    let decks = solo(cards.len());

    stack.layout(
        &mut cards,
        &decks,
        SURFACE,
        &mut MeasureCtx {
            text: &mut tcx,
            theme: &theme,
        },
    );

    let placements = stack.placements();
    assert_eq!(placements.len(), 3, "got {placements:?}");
    assert_eq!(stack.hidden(), 0);
    assert_eq!(placements[0].rect.y0, theme.metrics.card_margin);

    for pair in placements.windows(2) {
        assert!(
            (pair[1].rect.y0 - pair[0].rect.y1 - theme.metrics.card_gap).abs() < 1e-9,
            "cards {} and {} are {} apart, not {}",
            pair[0].index,
            pair[1].index,
            pair[1].rect.y0 - pair[0].rect.y1,
            theme.metrics.card_gap
        );
    }

    // And every card was told where it now lives, so a re-flow slides.
    for (index, placement) in placements.iter().enumerate() {
        assert_eq!(placement.index, index);
        for _ in 0..600 {
            cards[index].animation.step(1.0 / 60.0);
        }
        assert!(
            (cards[index].animation.top() - placement.rect.y0).abs() < 0.5,
            "card {index} settled at {} instead of {}",
            cards[index].animation.top(),
            placement.rect.y0
        );
    }
}

#[test]
fn cards_are_right_anchored_at_the_width_the_theme_asks_for() {
    let theme = Theme::light();
    let mut tcx = TextContext::new();
    let mut cards = vec![card(&mail("one"), &theme)];
    let mut stack = Stack::new();

    let decks = solo(cards.len());

    stack.layout(
        &mut cards,
        &decks,
        SURFACE,
        &mut MeasureCtx {
            text: &mut tcx,
            theme: &theme,
        },
    );
    let rect = stack.placements()[0].rect;
    assert_eq!(rect.width(), card_width(SURFACE, &theme));
    assert_eq!(rect.x0, card_left(SURFACE, &theme));
    assert_eq!(rect.x1, SURFACE.width - theme.metrics.card_margin);
}

#[test]
fn a_stack_that_does_not_fit_drops_its_oldest_cards_and_keeps_the_newest() {
    let theme = Theme::light();
    let mut tcx = TextContext::new();
    let mut stack = Stack::new();

    let single = {
        let mut probe = vec![card(&mail("one"), &theme)];
        let decks = solo(probe.len());
        stack.layout(
            &mut probe,
            &decks,
            SURFACE,
            &mut MeasureCtx {
                text: &mut tcx,
                theme: &theme,
            },
        );
        stack.placements()[0].rect.height()
    };

    let mut cards: Vec<Card> = ["one", "two", "three", "four"]
        .iter()
        .map(|body| card(&mail(body), &theme))
        .collect();
    let short = Size::new(
        SURFACE.width,
        2.0 * theme.metrics.card_margin + 2.0 * single + theme.metrics.card_gap,
    );

    let decks = solo(cards.len());

    stack.layout(
        &mut cards,
        &decks,
        short,
        &mut MeasureCtx {
            text: &mut tcx,
            theme: &theme,
        },
    );
    assert_eq!(stack.placements().len(), 2, "got {:?}", stack.placements());
    assert_eq!(stack.hidden(), 2);
    assert_eq!(stack.placements()[0].index, 2);
    assert_eq!(stack.placements()[1].index, 3);
    assert!(stack.placement_for(0).is_none());
    assert!(
        stack.height() <= short.height - 2.0 * theme.metrics.card_margin + 1e-9,
        "the placed stack {} overflows its budget",
        stack.height()
    );
}

#[test]
fn relative_timestamps_step_through_every_boundary() {
    let now = Local
        .with_ymd_and_hms(2026, 8, 9, 21, 30, 0)
        .single()
        .expect("2026-08-09 21:30 is not an ambiguous local time");

    assert_eq!(relative_time(now, now), "now");
    assert_eq!(relative_time(now, now - Duration::seconds(59)), "now");
    assert_eq!(relative_time(now, now - Duration::seconds(60)), "1m");
    assert_eq!(relative_time(now, now - Duration::minutes(5)), "5m");
    assert_eq!(relative_time(now, now - Duration::minutes(59)), "59m");
    assert_eq!(relative_time(now, now - Duration::hours(12)), "9:30 AM");
    assert_eq!(relative_time(now, now - Duration::days(1)), "Yesterday");
    assert_eq!(relative_time(now, now - Duration::days(5)), "4 Aug");
}

#[test]
fn app_profiles_resolve_spotify_and_fall_back_for_everyone_else() {
    assert_eq!(profile_for("spotify").content, Some(ContentHint::Music));
    assert_eq!(
        profile_for("com.spotify.client").content,
        Some(ContentHint::Music)
    );
    assert!(profile_for("spotify").accent.is_some());

    let fallback = profile_for("org.gnome.evolution");
    assert!(std::ptr::eq(fallback, default_profile()));
    assert_eq!(fallback.content, None);
    assert!(fallback.grouped);
}

/// Spotify posts a plain notification; the profile is what turns it into the
/// card with the transport controls on it.
#[test]
fn a_spotify_notification_is_drawn_as_a_music_card() {
    let theme = Theme::light();
    let mut tcx = TextContext::new();

    let spotify = entry(
        Notification::builder("Spotify")
            .id(7)
            .summary("Blue Monday")
            .body("New Order")
            .build(),
    );
    let plain = mail("New Order");

    let mut music = card(&spotify, &theme);
    let mut general = card(&plain, &theme);
    assert!(
        height(&mut music, &mut tcx, &theme) > height(&mut general, &mut tcx, &theme),
        "the Spotify card has no transport row, so the profile was not applied"
    );

    // And an explicit music payload lands on the same card.
    let mut explicit = card(
        &entry(
            Notification::builder("Player")
                .id(8)
                .content(Content::Music(Music {
                    title: "Blue Monday".into(),
                    artist: "New Order".into(),
                    album_art: None,
                    playing: true,
                    percent: 40,
                }))
                .build(),
        ),
        &theme,
    );
    assert_eq!(
        height(&mut explicit, &mut tcx, &theme),
        height(&mut music, &mut tcx, &theme)
    );
}

/// One deck holding `count` cards, which is what several notifications from the
/// same app produce.
fn deck(count: usize) -> Vec<(Deck, std::ops::Range<usize>)> {
    vec![(Deck::new("mail"), 0..count)]
}

fn laid_out(
    cards: &mut [Card],
    decks: &[(Deck, std::ops::Range<usize>)],
    tcx: &mut TextContext,
    theme: &Theme,
) -> Stack {
    let mut stack = Stack::new();
    stack.layout(cards, decks, SURFACE, &mut MeasureCtx { text: tcx, theme });
    stack
}

#[test]
fn a_collapsed_deck_draws_the_newest_card_over_two_dimmer_narrower_slivers() {
    let mut tcx = TextContext::new();
    let theme = Theme::light();
    let mut cards: Vec<_> = (0..5).map(|_| card(&mail("Body"), &theme)).collect();

    let stack = laid_out(&mut cards, &deck(5), &mut tcx, &theme);
    let placements = stack.placements();

    assert_eq!(placements.len(), 3, "five cards, a top and two slivers");
    for (depth, placement) in placements.iter().enumerate() {
        assert_eq!(placement.depth, depth);
    }
    assert!(placements[0].rect.y0 < placements[1].rect.y0);
    assert!(placements[1].rect.y0 < placements[2].rect.y0);
    assert!(placements[0].rect.width() > placements[1].rect.width());
    assert!(placements[1].rect.width() > placements[2].rect.width());
    assert!(placements[0].alpha > placements[1].alpha);
    assert!(placements[1].alpha > placements[2].alpha);
}

#[test]
fn a_collapsed_deck_is_as_tall_as_its_top_card_plus_its_peeks() {
    let mut tcx = TextContext::new();
    let theme = Theme::light();
    let mut cards: Vec<_> = (0..3).map(|_| card(&mail("Body"), &theme)).collect();

    let stack = laid_out(&mut cards, &deck(3), &mut tcx, &theme);
    let top = stack.placements()[0].rect;
    assert!(
        (stack.height() - (top.height() + 2.0 * theme.metrics.deck_peek)).abs() < 0.5,
        "{} is not the top card plus two peeks",
        stack.height()
    );
}

#[test]
fn a_deck_of_one_lays_out_exactly_where_a_bare_card_always_did() {
    let mut tcx = TextContext::new();
    let theme = Theme::light();
    let mut solo_card = vec![card(&mail("Body"), &theme)];
    let mut deck_card = vec![card(&mail("Body"), &theme)];

    let bare = laid_out(&mut solo_card, &deck(1), &mut tcx, &theme);
    let decked = laid_out(&mut deck_card, &deck(1), &mut tcx, &theme);
    assert_eq!(bare.placements()[0].rect, decked.placements()[0].rect);
    assert_eq!(bare.height(), decked.height());
}

#[test]
fn an_unfurled_deck_places_every_card_at_full_width_with_the_ordinary_gap() {
    let mut tcx = TextContext::new();
    let theme = Theme::light();
    let mut cards: Vec<_> = (0..4).map(|_| card(&mail("Body"), &theme)).collect();

    let mut decks = deck(4);
    decks[0].0.toggle();
    for _ in 0..400 {
        decks[0].0.step(1.0 / 60.0);
    }
    assert!(decks[0].0.is_open(), "the deck never finished opening");

    let stack = laid_out(&mut cards, &decks, &mut tcx, &theme);
    let placements = stack.placements();
    assert_eq!(placements.len(), 4, "unfurled, every card is placed");

    let width = card_width(SURFACE, &theme);
    for placement in placements {
        assert!((placement.rect.width() - width).abs() < 0.5);
        // A critically damped spring approaches its target asymptotically.
        assert!(placement.alpha > 0.999, "{}", placement.alpha);
    }
    for pair in placements.windows(2) {
        let gap = pair[1].rect.y0 - pair[0].rect.y1;
        assert!((gap - theme.metrics.card_gap).abs() < 0.5, "{gap}");
    }
}

#[test]
fn unfurling_a_deck_pushes_the_deck_below_it_down() {
    let mut tcx = TextContext::new();
    let theme = Theme::light();
    let mut cards: Vec<_> = (0..4).map(|_| card(&mail("Body"), &theme)).collect();

    let two_decks = |open: bool| {
        let mut first = Deck::new("mail");
        if open {
            first.toggle();
            for _ in 0..400 {
                first.step(1.0 / 60.0);
            }
        }
        vec![(first, 0..3), (Deck::new("chat"), 3..4)]
    };

    // `bounds` starts below the Clear All strip, so the deck's full extent is
    // measured from the top of its header.
    let extent = |stack: &Stack| {
        let deck = &stack.decks()[0];
        deck.bounds.y1 - deck.header.y0
    };

    let closed = laid_out(&mut cards, &two_decks(false), &mut tcx, &theme);
    let closed_second = closed.decks()[1].header.y0;
    let closed_height = extent(&closed);

    let opened = laid_out(&mut cards, &two_decks(true), &mut tcx, &theme);
    let opened_second = opened.decks()[1].header.y0;
    let opened_height = extent(&opened);

    assert!(opened_second > closed_second, "the deck below did not move");
    assert!(
        ((opened_second - closed_second) - (opened_height - closed_height)).abs() < 1.0,
        "it moved by something other than the deck's own growth"
    );
}

#[test]
fn a_deck_that_does_not_fit_is_dropped_whole_rather_than_cut_in_half() {
    let mut tcx = TextContext::new();
    let theme = Theme::light();
    let mut cards: Vec<_> = (0..4).map(|_| card(&mail("Body"), &theme)).collect();
    let decks = vec![(Deck::new("mail"), 0..2), (Deck::new("chat"), 2..4)];

    let mut stack = Stack::new();
    stack.layout(
        &mut cards,
        &decks,
        Size::new(400.0, 140.0),
        &mut MeasureCtx {
            text: &mut tcx,
            theme: &theme,
        },
    );
    assert_eq!(stack.decks().len(), 1, "half a deck is a stack of plates");
    assert_eq!(stack.hidden(), 2, "the whole older deck was dropped");
}

/// A notification carrying more actions than any card is wide.
fn many_actions() -> Entry {
    entry(
        Notification::builder("Calendar")
            .id(9)
            .summary("Standup in 5")
            .actions(
                ["Snooze", "Remind me later", "Open", "Decline", "Delegate"]
                    .iter()
                    .map(|label| Action {
                        key: label.to_lowercase(),
                        label: label.to_string(),
                        kind: ActionKind::Button,
                    })
                    .collect(),
            )
            .build(),
    )
}

#[test]
fn an_action_that_does_not_fit_is_reachable_through_the_menu_rather_than_dropped() {
    let mut tcx = TextContext::new();
    let theme = Theme::light();
    let mut card = card(&many_actions(), &theme);
    let width = card_width(SURFACE, &theme);

    let collapsed = card.measure(
        width,
        &mut MeasureCtx {
            text: &mut tcx,
            theme: &theme,
        },
    );

    // Opening the menu makes the card taller, which is what re-flows the stack.
    card.toggle_menu();
    let expanded = card.measure(
        width,
        &mut MeasureCtx {
            text: &mut tcx,
            theme: &theme,
        },
    );
    assert!(
        expanded > collapsed,
        "an open menu did not grow the card: {collapsed} -> {expanded}"
    );

    card.toggle_menu();
    let closed = card.measure(
        width,
        &mut MeasureCtx {
            text: &mut tcx,
            theme: &theme,
        },
    );
    assert_eq!(
        closed, collapsed,
        "closing the menu did not restore the card"
    );
}

#[test]
fn a_card_whose_actions_all_fit_grows_no_menu() {
    let mut tcx = TextContext::new();
    let theme = Theme::light();
    let one = entry(
        Notification::builder("Calendar")
            .id(10)
            .summary("Standup")
            .action(Action {
                key: "open".to_string(),
                label: "Open".to_string(),
                kind: ActionKind::Button,
            })
            .build(),
    );
    let mut card = card(&one, &theme);
    let width = card_width(SURFACE, &theme);
    let before = card.measure(
        width,
        &mut MeasureCtx {
            text: &mut tcx,
            theme: &theme,
        },
    );

    card.toggle_menu();
    let after = card.measure(
        width,
        &mut MeasureCtx {
            text: &mut tcx,
            theme: &theme,
        },
    );
    assert_eq!(before, after, "there was nothing for a menu to hold");
}
