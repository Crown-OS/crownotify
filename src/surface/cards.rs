//! Keeping a surface's cards and decks in step with the store.
//!
//! Both surfaces do the same three things every frame — adopt a card for every
//! entry, carry the surviving decks' springs across, and start the ones that
//! have gone away on their way out — so they do it through one function rather
//! than two that drift apart.

use std::ops::Range;

use crownshell::kurbo::Rect;

use crate::{
    store::Group,
    ui::{Card, Deck, Theme, layout::Placement},
};

/// The decks a surface draws, and which of its cards each one owns.
#[derive(Debug, Default)]
pub struct Decks {
    decks: Vec<(Deck, Range<usize>)>,
    scratch: Vec<(Deck, Range<usize>)>,
}

impl Decks {
    pub fn as_slice(&self) -> &[(Deck, Range<usize>)] {
        &self.decks
    }

    pub fn get(&self, index: usize) -> Option<&Deck> {
        self.decks.get(index).map(|(deck, _)| deck)
    }

    /// Rebuilds the deck list from `groups`, carrying each surviving deck's
    /// unfurl spring — and so its animation in flight — across.
    pub fn reconcile(&mut self, groups: &[Group<'_>]) {
        self.scratch.clear();
        let mut start = 0;
        for group in groups {
            let range = start..start + group.entries.len();
            start = range.end;
            let mut deck = match self
                .decks
                .iter()
                .position(|(deck, _)| deck.key() == group.key.as_ref())
            {
                Some(index) => self.decks.swap_remove(index).0,
                None => Deck::new(group.key.as_ref()),
            };
            deck.sync_len(range.len());
            self.scratch.push((deck, range));
        }
        std::mem::swap(&mut self.decks, &mut self.scratch);
    }

    pub fn toggle(&mut self, key: &str) {
        if let Some((deck, _)) = self.decks.iter_mut().find(|(deck, _)| deck.key() == key) {
            deck.toggle();
        }
    }

    /// Puts every deck back together with no animation, for a surface that has
    /// gone off screen and has nothing to animate for.
    pub fn collapse_all(&mut self) {
        for (deck, _) in &mut self.decks {
            deck.collapse();
        }
    }

    pub fn step(&mut self, dt: f32) {
        for (deck, _) in &mut self.decks {
            deck.step(dt);
        }
    }

    pub fn at_rest(&self) -> bool {
        self.decks.iter().all(|(deck, _)| deck.at_rest())
    }
}

/// Brings `cards` back in line with `groups`, returning how many are still live.
///
/// `groups` must already be in draw order, each deck's newest card first. Cards
/// that have gone away are moved to the tail and started on their leave
/// animation, which is what keeps them drawn while they slide off.
pub fn reconcile(
    cards: &mut Vec<Card>,
    scratch: &mut Vec<Card>,
    decks: &mut Decks,
    groups: &[Group<'_>],
    theme: &Theme,
    entering_at: f32,
) -> usize {
    scratch.clear();
    for group in groups {
        for entry in &group.entries {
            let id = entry.notification.id;
            match cards.iter().position(|card| card.id == id) {
                Some(index) => {
                    let mut card = cards.remove(index);
                    card.sync(entry, theme);
                    scratch.push(card);
                }
                None => scratch.push(Card::new(entry, theme, entering_at)),
            }
        }
    }
    let live = scratch.len();

    for mut card in cards.drain(..) {
        card.animation.leave();
        scratch.push(card);
    }
    std::mem::swap(cards, scratch);
    decks.reconcile(groups);
    live
}

/// The rectangle a placement is actually drawn in: where the stack put it,
/// moved to wherever the card's own springs currently have it.
pub fn displaced(placement: &Placement, card: &Card, width: f64) -> Rect {
    let rect = placement.rect;
    let dx = card.animation.translate_x(width);
    Rect::new(rect.x0 + dx, rect.y0, rect.x1 + dx, rect.y1)
}
