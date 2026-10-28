//! Stack layout: where each card sits, and what happens when they stop fitting.
//!
//! Everything here is a pure function of (measured heights, deck state,
//! window size, metrics). There is no clock and no view in it, which is what
//! lets the whole geometry of the toast strip and of the notification center
//! be asserted in a unit test — the two places a layout bug is otherwise only
//! visible as "the third notification is slightly wrong on my laptop".

use std::ops::Range;

use crownui::prelude::Rect;

use crate::ui::{deck, theme::Metrics};

/// One deck as the layout sees it: its cards' measured heights, top card
/// first, and how far it has unfurled.
#[derive(Debug, Clone, Copy)]
pub struct DeckInput<'a> {
    pub heights: &'a [f32],
    pub unfurl: f32,
}

/// Where the stack put one card.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    /// Which deck the card belongs to, as an index into the decks laid out.
    pub deck: usize,
    /// 0 for the card on top of its deck, then one per card layered behind it.
    pub depth: usize,
    /// The card's resting rectangle, before its own animation displaces it.
    pub rect: Rect,
    /// How far the deck this card belongs to has unfurled.
    pub unfurl: f32,
    /// What the deck asks the card be drawn at, before its own reveal.
    pub alpha: f32,
}

/// Where one deck landed as a whole.
#[derive(Debug, Clone, PartialEq)]
pub struct DeckPlacement {
    /// Index into the decks that were laid out.
    pub index: usize,
    /// The strip the deck's name and Clear All sit in. Zero-height while
    /// collapsed.
    pub header: Rect,
    /// Range of [`Stack::placements`] this deck owns, top card first.
    pub cards: Range<usize>,
}

/// The result of laying a list of decks out in one window.
///
/// Reused across frames: [`layout`](Self::layout) refills its vectors in
/// place, because a re-flow can happen on any animation frame.
#[derive(Debug, Default)]
pub struct Stack {
    placements: Vec<Placement>,
    decks: Vec<DeckPlacement>,
    height: f32,
    hidden: usize,
}

impl Stack {
    /// Every card that was placed, top to bottom.
    pub fn placements(&self) -> &[Placement] {
        &self.placements
    }

    /// Every deck that was placed, top to bottom.
    pub fn decks(&self) -> &[DeckPlacement] {
        &self.decks
    }

    /// From the top edge of the first placed card to the bottom edge of the
    /// last, gaps included. Zero when nothing was placed.
    pub fn height(&self) -> f32 {
        self.height
    }

    /// How many decks did not fit and were left unplaced. They are always the
    /// oldest ones — the head of the list — so the count doubles as the index
    /// of the first deck that *is* placed.
    pub fn hidden(&self) -> usize {
        self.hidden
    }

    /// The placement of the card at `depth` in deck `deck`, or `None` when it
    /// is not drawn.
    pub fn placement(&self, deck: usize, depth: usize) -> Option<&Placement> {
        self.placements
            .iter()
            .find(|placement| placement.deck == deck && placement.depth == depth)
    }

    /// Positions `decks`, oldest first, in a window `width` × `height`.
    pub fn layout(&mut self, decks: &[DeckInput<'_>], width: f32, height: f32, metrics: &Metrics) {
        self.placements.clear();
        self.decks.clear();
        self.height = 0.0;
        self.hidden = 0;
        if decks.is_empty() {
            return;
        }
        let card_width = card_width(width, metrics);
        let left = card_left(width, metrics);
        let budget = (height - 2.0 * metrics.card_margin).max(0.0);
        let first = first_visible(decks, budget, metrics);
        self.hidden = first;

        let mut y = metrics.card_margin;
        for (index, input) in decks.iter().enumerate().skip(first) {
            let len = input.heights.len();
            let header = Rect::from_xywh(
                left,
                y,
                card_width,
                deck::header_height(len, input.unfurl, metrics),
            );
            let top = header.max_y();
            let start = self.placements.len();
            for depth in 0..=deck::deepest(len, input.unfurl) {
                let pose = deck::pose(input.heights, depth, input.unfurl, metrics);
                self.placements.push(Placement {
                    deck: index,
                    depth,
                    rect: Rect::from_xywh(
                        left + pose.inset,
                        top + pose.top,
                        card_width - 2.0 * pose.inset,
                        pose.height,
                    ),
                    unfurl: input.unfurl,
                    alpha: pose.alpha,
                });
            }
            self.decks.push(DeckPlacement {
                index,
                header,
                cards: start..self.placements.len(),
            });
            y += deck::height(input.heights, input.unfurl, metrics) + metrics.card_gap;
        }
        self.height = (y - metrics.card_gap - metrics.card_margin).max(0.0);
    }
}

/// The oldest deck that still fits in `budget`, counting back from the newest.
///
/// Overflow is resolved from the *old* end because the newest notification is
/// the one the user has not seen yet, and a whole deck at a time because half a
/// deck on the bottom edge is a stack of plates with nothing on it. The newest
/// is therefore always placed, even in a window too short to hold it — showing
/// a clipped card beats silently dropping the thing that just happened.
fn first_visible(decks: &[DeckInput<'_>], budget: f32, metrics: &Metrics) -> usize {
    let mut used = 0.0;
    let mut first = decks.len();
    for (index, input) in decks.iter().enumerate().rev() {
        let height = deck::height(input.heights, input.unfurl, metrics);
        let needed = if first == decks.len() {
            height
        } else {
            used + metrics.card_gap + height
        };
        if needed > budget && first != decks.len() {
            break;
        }
        used = needed;
        first = index;
    }
    first
}

/// How wide a card is drawn in a window `available` wide.
///
/// The design fixes the width at [`Metrics::card_width`], but a window
/// narrower than that would otherwise get cards hanging off it.
pub fn card_width(available: f32, metrics: &Metrics) -> f32 {
    metrics
        .card_width
        .min(available - 2.0 * metrics.card_margin)
        .max(0.0)
}

/// The x of a card's left edge: the stack is anchored to the right of the
/// window, one margin in.
pub fn card_left(available: f32, metrics: &Metrics) -> f32 {
    (available - metrics.card_margin - card_width(available, metrics)).max(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const WIDTH: f32 = 400.0;
    const HEIGHT: f32 = 900.0;

    fn metrics() -> Metrics {
        Metrics::default()
    }

    fn solo(heights: &[f32]) -> Vec<DeckInput<'_>> {
        heights
            .chunks(1)
            .map(|heights| DeckInput {
                heights,
                unfurl: 0.0,
            })
            .collect()
    }

    fn laid_out(decks: &[DeckInput<'_>], height: f32) -> Stack {
        let mut stack = Stack::default();
        stack.layout(decks, WIDTH, height, &metrics());
        stack
    }

    #[test]
    fn an_empty_stack_places_nothing_and_has_no_height() {
        let stack = laid_out(&[], HEIGHT);
        assert!(stack.placements().is_empty());
        assert_eq!(stack.height(), 0.0);
        assert_eq!(stack.hidden(), 0);
    }

    #[test]
    fn cards_are_stacked_top_down_with_the_gap_between_them() {
        let metrics = metrics();
        let stack = laid_out(&solo(&[80.0, 60.0]), HEIGHT);
        let [first, second] = [stack.placements()[0], stack.placements()[1]];
        assert_eq!(first.rect.min_y(), metrics.card_margin);
        assert_eq!(second.rect.min_y(), first.rect.max_y() + metrics.card_gap);
        assert_eq!(stack.height(), 80.0 + metrics.card_gap + 60.0);
    }

    #[test]
    fn cards_hug_the_right_edge_one_margin_in() {
        let metrics = metrics();
        let stack = laid_out(&solo(&[80.0]), HEIGHT);
        let rect = stack.placements()[0].rect;
        assert_eq!(rect.max_x(), WIDTH - metrics.card_margin);
        assert_eq!(rect.size.width, metrics.card_width);
    }

    #[test]
    fn a_narrow_window_shrinks_the_card_instead_of_hanging_it_off_the_edge() {
        let metrics = metrics();
        assert_eq!(
            card_width(200.0, &metrics),
            200.0 - 2.0 * metrics.card_margin
        );
        assert_eq!(card_left(200.0, &metrics), metrics.card_margin);
        assert_eq!(card_width(10.0, &metrics), 0.0);
    }

    #[test]
    fn decks_that_do_not_fit_are_dropped_from_the_old_end() {
        let stack = laid_out(&solo(&[100.0, 100.0, 100.0]), 200.0);
        assert_eq!(stack.hidden(), 2);
        assert_eq!(stack.placements().len(), 1);
        assert_eq!(
            stack.placements()[0].deck,
            2,
            "the newest deck was not kept"
        );
    }

    #[test]
    fn the_newest_deck_is_placed_even_when_it_cannot_fit() {
        let stack = laid_out(&solo(&[100.0, 400.0]), 200.0);
        assert_eq!(stack.hidden(), 1);
        assert_eq!(stack.placements()[0].deck, 1);
    }

    #[test]
    fn a_collapsed_deck_places_its_top_card_and_two_slivers_behind_it() {
        let heights = [80.0, 70.0, 90.0, 60.0];
        let stack = laid_out(
            &[DeckInput {
                heights: &heights,
                unfurl: 0.0,
            }],
            HEIGHT,
        );
        assert_eq!(stack.placements().len(), 1 + deck::MAX_PEEK);
        let top = stack.placement(0, 0).map(|placement| placement.rect);
        let sliver = stack.placement(0, 1).map(|placement| placement.rect);
        let (Some(top), Some(sliver)) = (top, sliver) else {
            panic!("the top card and its first sliver were not placed");
        };
        assert_eq!(
            sliver.size.height, top.size.height,
            "a sliver mirrors the top card"
        );
        assert!(sliver.size.width < top.size.width, "a sliver is not inset");
        assert_eq!(stack.decks()[0].header.size.height, 0.0);
    }

    #[test]
    fn an_unfurled_deck_opens_a_header_and_lists_every_card() {
        let metrics = metrics();
        let heights = [80.0, 70.0, 90.0];
        let stack = laid_out(
            &[DeckInput {
                heights: &heights,
                unfurl: 1.0,
            }],
            HEIGHT,
        );
        assert_eq!(stack.placements().len(), 3);
        let header = stack.decks()[0].header;
        assert_eq!(header.size.height, metrics.button + metrics.card_gap);
        assert_eq!(stack.placements()[0].rect.min_y(), header.max_y());
    }

    #[test]
    fn the_stack_height_covers_exactly_what_it_placed() {
        let metrics = metrics();
        let heights = [80.0, 70.0];
        let decks = [
            DeckInput {
                heights: &heights,
                unfurl: 1.0,
            },
            DeckInput {
                heights: &heights[..1],
                unfurl: 0.0,
            },
        ];
        let stack = laid_out(&decks, HEIGHT);
        let bottom = stack
            .placements()
            .iter()
            .map(|placement| placement.rect.max_y())
            .fold(0.0, f32::max);
        assert_eq!(stack.height(), bottom - metrics.card_margin);
    }
}
