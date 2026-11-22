//! One window's stack of cards: which notifications it shows, where each one
//! sits, and the springs that move them there.
//!
//! The geometry is the old painter's, kept exactly: [`Stack`] places whole
//! decks from measured heights, [`CardMotion`] slides each card in, re-flows it
//! and flings it out, and a [`Deck`] spring pulls an app's cards apart. What
//! changed is the output: every card owns a [`Frame`] signal its view binds to,
//! and each animation frame rewrites those, so crownui lays the cards out where
//! the springs put them — and the compositor's blur and the window's input
//! region, both taken from layout, follow them frame by frame.
//!
//! Every change that starts a spring is queued as an [`Intent`] and applied at
//! the next frame, against that frame's own timestamp: a card posted after an
//! hour of quiet then slides in from the moment it is first drawn, and a
//! headless test's simulated clock drives the springs exactly as the display
//! does.

use std::{collections::HashMap, mem, ops::Range, time::Duration};

use crownui::prelude::{Cx, Rect, Runtime, Signal, Size, Store};

use crate::{
    model::{Notification, NotificationId},
    ui::{
        deck::{self, Deck, OPEN},
        motion::{CardMotion, Phase, dismiss_velocity},
        stack::{DeckInput, Stack, card_left, card_width},
        theme::Metrics,
    },
    util::spring::Spring,
};

/// How far right a card has to be dragged before it is dismissed.
pub const SWIPE_THRESHOLD: f32 = 40.0;
/// How far one wheel notch scrolls the notification center.
pub const NOTCH: f32 = 60.0;

/// Which store a board shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    /// The toast strip: live notifications, oldest deck at the top, entering
    /// below the stack and swiped away to the right.
    Toasts,
    /// The notification center: history, newest deck at the top, scrolled.
    History,
}

/// Where a card's view is drawn this frame, in the board's own coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Frame {
    pub rect: Rect,
    /// The card as a whole: deck depth times its own reveal.
    pub alpha: f32,
    /// The card's text and controls, which a sliver behind a collapsed deck
    /// does not show.
    pub content: f32,
    /// Placed at all; a deck that did not fit the strip is not.
    pub shown: bool,
    /// Takes clicks. A leaving card is drawn but must not swallow a click meant
    /// for the card sliding up underneath it.
    pub interactive: bool,
}

/// One deck's header strip, shown while it is pulled apart.
#[derive(Debug, Clone, PartialEq)]
pub struct DeckHead {
    pub key: String,
    pub name: String,
}

/// A change waiting for the next frame's timestamp.
enum Intent {
    Show(Vec<Notification>),
    Toggle(String),
    Drag(NotificationId, f32),
    Fling(NotificationId, f32),
    Scroll(f32),
}

struct Tile {
    notification: Notification,
    motion: CardMotion,
    /// The card's height when laid out at full width, as last measured.
    height: f32,
    frame: Option<Signal<Frame>>,
}

/// The board itself, in a reactive [`Store`]: plain data, mutated in place.
pub struct Board {
    side: Side,
    metrics: Metrics,
    /// Live tiles first, in deck order, then the ones on their way out.
    tiles: Vec<Tile>,
    live: usize,
    /// Live decks, in the order they are stacked, and their tiles.
    decks: Vec<(Deck, Range<usize>)>,
    stack: Stack,
    size: Size,
    scroll: Spring,
    heads: HashMap<String, Signal<Frame>>,
    intents: Vec<Intent>,
    scheduled: bool,
}

/// The handles a window's views reach the board through. `Copy`.
#[derive(Clone, Copy)]
pub struct BoardHandle {
    board: Store<Board>,
    /// Every card on screen, leaving ones included; what the views are built from.
    pub cards: Signal<Vec<Notification>>,
    /// The decks of more than one card, which may show a header.
    pub heads: Signal<Vec<DeckHead>>,
}

impl BoardHandle {
    pub fn new(cx: &mut Cx, side: Side, metrics: Metrics) -> Self {
        Self {
            board: cx.store(Board {
                side,
                metrics,
                tiles: Vec::new(),
                live: 0,
                decks: Vec::new(),
                stack: Stack::default(),
                size: Size::ZERO,
                scroll: Spring::new(0.0, motion::Spring::smooth()),
                heads: HashMap::new(),
                intents: Vec::new(),
                scheduled: false,
            }),
            cards: cx.signal(Vec::new()),
            heads: cx.signal(Vec::new()),
        }
    }

    /// Takes the notifications the daemon says this board shows.
    pub fn show(self, cx: &mut Cx, notifications: &[Notification]) {
        self.intend(cx, Intent::Show(notifications.to_vec()));
    }

    /// Pulls the deck for `key` apart, or puts it back together.
    pub fn toggle(self, cx: &mut Cx, key: &str) {
        self.intend(cx, Intent::Toggle(key.to_string()));
    }

    /// Follows a swipe on `id` that has travelled `dx` so far; reports whether
    /// it went far enough to dismiss.
    pub fn drag(self, cx: &mut Cx, id: NotificationId, dx: f32) -> bool {
        self.intend(cx, Intent::Drag(id, dx));
        dx >= SWIPE_THRESHOLD
    }

    /// Sends `id` off to the right at the speed the pointer left it, in pixels
    /// per second.
    pub fn fling(self, cx: &mut Cx, id: NotificationId, velocity: f32) {
        self.intend(cx, Intent::Fling(id, velocity));
    }

    /// Lets go of a swipe that did not go far enough.
    pub fn release(self, cx: &mut Cx, id: NotificationId) {
        self.intend(cx, Intent::Drag(id, 0.0));
    }

    /// Scrolls the center by `delta` pixels, within the content.
    pub fn scroll_by(self, cx: &mut Cx, delta: f32) {
        self.intend(cx, Intent::Scroll(delta));
    }

    /// Drops every card and puts every deck back together, for a window that
    /// went off screen.
    pub fn clear(self, cx: &mut Cx) {
        self.board.update(cx, Board::clear);
        self.cards.set(cx, Vec::new());
        self.heads.set(cx, Vec::new());
    }

    pub fn resize(self, cx: &mut Cx, size: Size) {
        if self
            .board
            .update(cx, |board| mem::replace(&mut board.size, size))
            != size
        {
            self.animate(cx);
        }
    }

    /// Hands the board the frame signal of the view showing `id`.
    pub fn attach(self, cx: &mut Cx, id: NotificationId, frame: Signal<Frame>) {
        self.board.update(cx, |board| {
            if let Some(tile) = board.tile_mut(id) {
                tile.frame = Some(frame);
            }
        });
        self.animate(cx);
    }

    /// Hands the board the frame signal of the header for deck `key`.
    pub fn attach_head(self, cx: &mut Cx, key: String, frame: Signal<Frame>) {
        self.board
            .update(cx, |board| board.heads.insert(key, frame));
        self.animate(cx);
    }

    /// The header for deck `key` went away with its view.
    pub fn detach_head(self, runtime: &mut Runtime, key: &str) {
        self.board.update(runtime, |board| board.heads.remove(key));
    }

    /// The view showing `id` measured it at `height`.
    pub fn measured(self, cx: &mut Cx, id: NotificationId, height: f32) {
        let changed = self.board.update(cx, |board| {
            board
                .tile_mut(id)
                .is_some_and(|tile| mem::replace(&mut tile.height, height) != height)
        });
        if changed {
            self.animate(cx);
        }
    }

    /// The deck a click on the body of `id` pulls apart instead of activating
    /// it: the one `id` heads, while that deck is collapsed.
    pub fn collapsed(self, runtime: &Runtime, id: NotificationId) -> Option<String> {
        self.board.with(runtime, |board| board.collapsed_deck(id))
    }

    fn intend(self, cx: &mut Cx, intent: Intent) {
        self.board.update(cx, |board| board.intents.push(intent));
        self.animate(cx);
    }

    /// Steps every spring on the next display refresh, and keeps doing so for
    /// as long as anything moves.
    fn animate(self, cx: &mut Cx) {
        if self
            .board
            .update(cx, |board| mem::replace(&mut board.scheduled, true))
        {
            return;
        }
        cx.request_animation_frame(move |cx, now| {
            let step = self.board.update(cx, |board| {
                board.scheduled = false;
                board.step(now)
            });
            for (frame, value) in step.frames {
                frame.set(cx, value);
            }
            if let Some(cards) = step.cards {
                self.cards.set(cx, cards);
            }
            if let Some(heads) = step.heads {
                self.heads.set(cx, heads);
            }
            if step.moving {
                self.animate(cx);
            }
        });
    }
}

/// What one animation frame produced.
struct Step {
    frames: Vec<(Signal<Frame>, Frame)>,
    /// The card list again, when a card arrived or finished leaving.
    cards: Option<Vec<Notification>>,
    heads: Option<Vec<DeckHead>>,
    moving: bool,
}

impl Board {
    fn tile_mut(&mut self, id: NotificationId) -> Option<&mut Tile> {
        self.tiles
            .iter_mut()
            .find(|tile| tile.notification.id == id)
    }

    fn clear(&mut self) {
        self.tiles.clear();
        self.live = 0;
        self.decks.clear();
        self.intents.clear();
        self.scroll.snap(0.0);
    }

    /// The key of the deck `id` heads while that deck is collapsed. A deck of
    /// one has nothing behind its card, so a click on it always activates.
    fn collapsed_deck(&self, id: NotificationId) -> Option<String> {
        let index = self.tiles[..self.live]
            .iter()
            .position(|tile| tile.notification.id == id)?;
        let (deck, range) = self
            .decks
            .iter()
            .find(|(_, range)| range.contains(&index))?;
        (range.len() > 1 && !deck.is_opening()).then(|| deck.key().to_string())
    }

    fn content_height(&self) -> f32 {
        self.stack.height() + 2.0 * self.metrics.card_margin
    }

    fn apply(&mut self, intent: Intent, now: Duration) -> Option<Vec<DeckHead>> {
        match intent {
            Intent::Show(notifications) => return Some(self.reconcile(&notifications, now)),
            Intent::Toggle(key) => {
                if let Some((deck, _)) = self.decks.iter_mut().find(|(deck, _)| deck.key() == key) {
                    deck.toggle(now);
                }
            }
            Intent::Drag(id, dx) => {
                if let Some(tile) = self.tile_mut(id) {
                    tile.motion.drag = dx.max(0.0);
                }
            }
            Intent::Fling(id, velocity) => {
                let width = card_width(self.size.width, &self.metrics);
                if let Some(tile) = self.tile_mut(id) {
                    tile.motion
                        .leave_with_velocity(now, dismiss_velocity(velocity, width));
                }
            }
            Intent::Scroll(delta) => {
                let target = clamp_scroll(
                    self.scroll.target() + delta,
                    self.content_height(),
                    self.size.height,
                );
                self.scroll.set_target(now, target);
            }
        }
        None
    }

    /// Rebuilds the tile and deck lists for `notifications`, keeping the motion
    /// of every card that is still there and sending off the ones that are not.
    fn reconcile(&mut self, notifications: &[Notification], now: Duration) -> Vec<DeckHead> {
        let entering_at = match self.side {
            Side::Toasts if self.stack.placements().is_empty() => self.metrics.card_margin,
            Side::Toasts => self.metrics.card_margin + self.stack.height() + self.metrics.card_gap,
            Side::History => 0.0,
        };
        let groups = match self.side {
            Side::Toasts => {
                let mut groups = group(notifications.iter().rev());
                groups.reverse();
                groups
            }
            Side::History => group(notifications.iter()),
        };

        let mut old: Vec<Tile> = self.tiles.drain(..).collect();
        let mut old_decks: Vec<Deck> = self.decks.drain(..).map(|(deck, _)| deck).collect();
        let mut heads = Vec::new();
        for (key, members) in groups {
            let mut deck = match old_decks.iter().position(|deck| deck.key() == key) {
                Some(index) => old_decks.swap_remove(index),
                None => Deck::new(key.clone()),
            };
            deck.sync_len(members.len());
            if members.len() > 1 {
                heads.push(DeckHead {
                    key,
                    name: members[0].app.name.clone(),
                });
            }
            let start = self.tiles.len();
            for notification in members {
                let kept = old
                    .iter()
                    .position(|tile| tile.notification.id == notification.id)
                    .map(|index| old.swap_remove(index))
                    .filter(|tile| tile.motion.phase != Phase::Leaving);
                self.tiles.push(match kept {
                    Some(mut tile) => {
                        tile.notification = notification.clone();
                        tile
                    }
                    None => Tile {
                        notification: notification.clone(),
                        motion: CardMotion::entering(now, entering_at),
                        height: 0.0,
                        frame: None,
                    },
                });
            }
            self.decks.push((deck, start..self.tiles.len()));
        }
        self.live = self.tiles.len();
        for mut tile in old {
            tile.motion.leave(now);
            self.tiles.push(tile);
        }
        heads
    }

    /// Every card, in the order they paint: each deck from its deepest sliver
    /// up to its top card, so the top card covers the ones behind it, and the
    /// leaving cards last, over everything they slide across.
    fn card_list(&self) -> Vec<Notification> {
        self.decks
            .iter()
            .flat_map(|(_, range)| self.tiles[range.clone()].iter().rev())
            .chain(&self.tiles[self.live..])
            .map(|tile| tile.notification.clone())
            .collect()
    }

    fn step(&mut self, now: Duration) -> Step {
        let mut heads = None;
        let mut cards_changed = false;
        for intent in mem::take(&mut self.intents) {
            if let Some(new_heads) = self.apply(intent, now) {
                heads = Some(new_heads);
                cards_changed = true;
            }
        }

        let metrics = self.metrics;
        let heights: Vec<Vec<f32>> = self
            .decks
            .iter()
            .map(|(_, range)| {
                self.tiles[range.clone()]
                    .iter()
                    .map(|tile| tile.height)
                    .collect()
            })
            .collect();
        let inputs: Vec<DeckInput<'_>> = heights
            .iter()
            .zip(&self.decks)
            .map(|(heights, (deck, _))| DeckInput {
                heights,
                unfurl: deck.unfurl(now),
            })
            .collect();
        let budget = match self.side {
            Side::Toasts => self.size.height,
            Side::History => f32::INFINITY,
        };
        self.stack
            .layout(&inputs, self.size.width, budget, &metrics);

        let origin = match self.side {
            Side::Toasts => 0.0,
            Side::History => {
                // A group that collapsed under the pointer can leave the panel
                // scrolled past the end of what is left.
                let clamped = clamp_scroll(
                    self.scroll.target(),
                    self.content_height(),
                    self.size.height,
                );
                self.scroll.set_target(now, clamped);
                -self.scroll.value(now)
            }
        };
        let width = card_width(self.size.width, &metrics);
        let left = card_left(self.size.width, &metrics);
        let mut moving = !self.scroll.at_rest(now);
        let mut frames = Vec::with_capacity(self.tiles.len() + self.heads.len());

        for (index, (deck, range)) in self.decks.iter().enumerate() {
            let settling = deck.is_settling(now);
            moving |= settling;
            let unfurl = deck.unfurl(now);
            for (depth, tile) in self.tiles[range.clone()].iter_mut().enumerate() {
                let frame = match self.stack.placement(index, depth) {
                    Some(placement) => {
                        let rect = placement.rect;
                        // A card whose own deck is moving is driven by the
                        // unfurl spring; letting its offset spring chase that
                        // as well would stack two lags and feel like rubber.
                        if settling {
                            tile.motion.snap_offset(rect.min_y());
                        } else {
                            tile.motion.set_offset(now, rect.min_y());
                        }
                        Frame {
                            rect: Rect::from_xywh(
                                rect.min_x() + tile.motion.translate_x(now, width),
                                tile.motion.top(now) + origin,
                                rect.size.width,
                                rect.size.height,
                            ),
                            alpha: placement.alpha * tile.motion.alpha(now),
                            content: deck::content_alpha(depth, unfurl),
                            shown: true,
                            interactive: true,
                        }
                    }
                    None => Frame::default(),
                };
                tile.motion.settle(now);
                moving |= !tile.motion.at_rest(now);
                if let Some(signal) = tile.frame {
                    frames.push((signal, frame));
                }
            }
        }

        let leaving = self.tiles.len() - self.live;
        let live = self.live;
        let mut kept = live;
        while kept < self.tiles.len() {
            if self.tiles[kept].motion.settle(now) {
                kept += 1;
            } else {
                self.tiles.remove(kept);
            }
        }
        cards_changed |= self.tiles.len() - live != leaving;
        for tile in &self.tiles[live..] {
            moving = true;
            if let Some(signal) = tile.frame {
                frames.push((
                    signal,
                    Frame {
                        rect: Rect::from_xywh(
                            left + tile.motion.translate_x(now, width),
                            tile.motion.top(now) + origin,
                            width,
                            tile.height,
                        ),
                        alpha: tile.motion.alpha(now),
                        content: 1.0,
                        shown: true,
                        interactive: false,
                    },
                ));
            }
        }

        for (key, signal) in &self.heads {
            let header = self
                .stack
                .decks()
                .iter()
                .find(|placed| self.decks[placed.index].0.key() == key)
                .map(|placed| (placed.header, self.decks[placed.index].0.unfurl(now)))
                .filter(|(header, _)| header.size.height > 0.0);
            let frame = match header {
                Some((header, unfurl)) => Frame {
                    rect: Rect::from_xywh(
                        header.min_x(),
                        header.min_y() + origin,
                        header.size.width,
                        header.size.height,
                    ),
                    alpha: unfurl,
                    content: unfurl,
                    shown: true,
                    interactive: unfurl >= OPEN,
                },
                None => Frame::default(),
            };
            frames.push((*signal, frame));
        }

        Step {
            frames,
            cards: cards_changed.then(|| self.card_list()),
            heads,
            moving,
        }
    }
}

/// Buckets `notifications` by app, keeping the order each app first appears
/// in and the order of its members — the store's own grouping, applied to a
/// snapshot.
pub fn group<'a>(
    notifications: impl Iterator<Item = &'a Notification>,
) -> Vec<(String, Vec<&'a Notification>)> {
    let mut groups: Vec<(String, Vec<&Notification>)> = Vec::new();
    for notification in notifications {
        let key = notification.app.key();
        match groups.iter_mut().find(|(group, _)| *group == key) {
            Some((_, members)) => members.push(notification),
            None => groups.push((key.into_owned(), vec![notification])),
        }
    }
    groups
}

/// Clamps a scroll offset to the range the content actually occupies.
///
/// Content shorter than the viewport does not scroll at all.
pub fn clamp_scroll(offset: f32, content: f32, viewport: f32) -> f32 {
    offset.clamp(0.0, (content - viewport).max(0.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(app: &str, id: NotificationId) -> Notification {
        Notification::builder(app).id(id).summary("Ada").build()
    }

    fn board(side: Side) -> Board {
        Board {
            side,
            metrics: Metrics::default(),
            tiles: Vec::new(),
            live: 0,
            decks: Vec::new(),
            stack: Stack::default(),
            size: Size::new(400.0, 900.0),
            scroll: Spring::new(0.0, motion::Spring::smooth()),
            heads: HashMap::new(),
            intents: Vec::new(),
            scheduled: false,
        }
    }

    fn ids(board: &Board) -> Vec<NotificationId> {
        board
            .tiles
            .iter()
            .map(|tile| tile.notification.id)
            .collect()
    }

    fn painted(board: &Board) -> Vec<NotificationId> {
        board.card_list().iter().map(|card| card.id).collect()
    }

    #[test]
    fn grouping_keeps_first_appearance_order_and_member_order() {
        let list = [note("Mail", 1), note("Chat", 2), note("Mail", 3)];
        let groups = group(list.iter());
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].0, "mail");
        assert_eq!(
            groups[0].1.iter().map(|n| n.id).collect::<Vec<_>>(),
            vec![1, 3]
        );
    }

    #[test]
    fn the_strip_stacks_the_oldest_deck_first_with_its_newest_card_on_top() {
        let mut board = board(Side::Toasts);
        board.reconcile(
            &[note("Mail", 1), note("Chat", 2), note("Mail", 3)],
            Duration::ZERO,
        );
        assert_eq!(ids(&board), vec![2, 3, 1]);
    }

    #[test]
    fn a_deck_paints_its_slivers_under_its_top_card() {
        let mut board = board(Side::Toasts);
        board.reconcile(
            &[note("Mail", 1), note("Chat", 2), note("Mail", 3)],
            Duration::ZERO,
        );
        assert_eq!(painted(&board), vec![2, 1, 3]);
    }

    #[test]
    fn a_card_that_left_the_store_is_kept_until_it_has_finished_leaving() {
        let mut board = board(Side::Toasts);
        board.reconcile(&[note("Mail", 1), note("Chat", 2)], Duration::ZERO);
        let heads = board.reconcile(&[note("Chat", 2)], Duration::from_secs(1));
        assert!(heads.is_empty());
        assert_eq!(board.live, 1);
        assert_eq!(
            ids(&board),
            vec![2, 1],
            "the leaving card was dropped at once"
        );

        let step = board.step(Duration::from_secs(10));
        assert_eq!(ids(&board), vec![2]);
        assert!(step.cards.is_some(), "the views were not told it went");
    }

    #[test]
    fn a_replacement_keeps_its_card_and_its_motion() {
        let mut board = board(Side::Toasts);
        board.reconcile(&[note("Mail", 1)], Duration::ZERO);
        let replaced = Notification::builder("Mail")
            .id(1)
            .summary("Edited")
            .build();
        board.reconcile(&[replaced], Duration::from_millis(50));
        assert_eq!(board.tiles.len(), 1);
        assert_eq!(board.tiles[0].notification.summary, "Edited");
        assert_eq!(board.tiles[0].motion.phase, Phase::Entering);
    }

    #[test]
    fn only_the_top_card_of_a_collapsed_deck_of_several_pulls_it_apart() {
        let mut board = board(Side::Toasts);
        board.reconcile(
            &[note("Chat", 1), note("Mail", 2), note("Mail", 3)],
            Duration::ZERO,
        );
        assert_eq!(board.collapsed_deck(1), None, "a deck of one activates");
        assert_eq!(board.collapsed_deck(3).as_deref(), Some("mail"));

        board.apply(Intent::Toggle("mail".into()), Duration::ZERO);
        assert_eq!(board.collapsed_deck(3), None, "an opening deck activates");
    }

    #[test]
    fn scrolling_stops_at_the_top_and_at_the_end_of_the_content() {
        assert_eq!(clamp_scroll(-50.0, 1000.0, 400.0), 0.0);
        assert_eq!(clamp_scroll(900.0, 1000.0, 400.0), 600.0);
        assert_eq!(clamp_scroll(200.0, 1000.0, 400.0), 200.0);
    }

    #[test]
    fn content_that_fits_the_panel_does_not_scroll_at_all() {
        assert_eq!(clamp_scroll(120.0, 300.0, 400.0), 0.0);
    }
}
