//! Stack layout: where each card sits, and what happens when they stop fitting.
//!
//! Everything here is a pure function of (cards, surface size, theme). There is
//! no Wayland, no scene and no clock in it, which is what lets the whole
//! geometry of the toast strip and of the notification center be asserted in a
//! unit test — the two places a layout bug is otherwise only visible as "the
//! third notification is slightly wrong on my laptop".
//!
//! Positions are handed to the cards through
//! [`CardAnimation::set_offset`](crate::ui::animation::CardAnimation::set_offset)
//! rather than being written straight into their bounds. That is deliberate: a
//! card above another one growing or leaving re-flows the stack, and a spring
//! makes that a slide instead of a jump.

use crownshell::kurbo::{Rect, Size};

use crate::ui::{
    card::{Card, MeasureCtx},
    deck::{self, Deck},
    theme::Theme,
};

/// Where the stack put one card.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    /// Index into the slice that was laid out. Not every index gets a
    /// placement — see [`Stack::hidden`] — so this cannot be inferred from the
    /// position in [`Stack::placements`].
    pub index: usize,
    /// The card's resting rectangle in surface-local logical pixels, before its
    /// own animation displaces it.
    pub rect: Rect,
    /// 0 for the card on top of its deck, then one per card layered behind it.
    pub depth: usize,
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
    /// Every card of the deck, slivers included: the one rectangle a collapsed
    /// deck claims for input, so its peeking slivers never become click targets
    /// of their own.
    pub bounds: Rect,
    /// The strip the Clear All pill sits in. Zero-height while collapsed.
    pub header: Rect,
    /// Range of [`Stack::placements`] this deck owns, top card first.
    pub cards: std::ops::Range<usize>,
    pub unfurl: f32,
}

/// The result of laying a slice of cards out on one surface.
///
/// Reused across frames: [`layout`](Self::layout) refills the placement vector
/// in place rather than returning a fresh one, because a re-flow can happen on
/// any frame and a per-frame allocation in the layout path is the same jitter
/// the [`HitMap`](crate::ui::hit::HitMap) avoids for the same reason.
#[derive(Debug, Default)]
pub struct Stack {
    placements: Vec<Placement>,
    decks: Vec<DeckPlacement>,
    /// Per-deck measured heights, refilled per deck. Reused so a re-flow on the
    /// frame path allocates nothing, for the same reason the placements are.
    heights: Vec<f64>,
    height: f64,
    hidden: usize,
}

impl Stack {
    pub fn new() -> Self {
        Self::default()
    }

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
    pub fn height(&self) -> f64 {
        self.height
    }

    /// How many cards did not fit and were left unplaced. They are always the
    /// oldest ones — the head of the slice — so the count doubles as the index
    /// of the first card that *is* placed.
    pub fn hidden(&self) -> usize {
        self.hidden
    }

    pub fn is_empty(&self) -> bool {
        self.placements.is_empty()
    }

    /// The placement of the card at `index`, or `None` if it was hidden.
    pub fn placement_for(&self, index: usize) -> Option<&Placement> {
        self.placements
            .iter()
            .find(|placement| placement.index == index)
    }

    /// Measures and positions `cards` for a surface of `available` size.
    ///
    /// `cards` must be in draw order, oldest first, which is what
    /// [`Store::toasts`](crate::store::Store::toasts) hands out. Each placed
    /// card is retargeted to its new offset, so a card that moved slides there
    /// on the next frames instead of teleporting.
    pub fn layout(
        &mut self,
        cards: &mut [Card],
        decks: &[(Deck, std::ops::Range<usize>)],
        available: Size,
        ctx: &mut MeasureCtx<'_>,
    ) {
        self.placements.clear();
        self.decks.clear();
        self.height = 0.0;
        self.hidden = 0;
        if cards.is_empty() {
            return;
        }

        let metrics = ctx.theme.metrics;
        let width = card_width(available, ctx.theme);
        let left = card_left(available, ctx.theme);
        let budget = (available.height - 2.0 * metrics.card_margin).max(0.0);

        let first = self.first_visible(cards, decks, width, budget, ctx);
        self.hidden = decks[..first].iter().map(|(_, range)| range.len()).sum();

        let mut y = metrics.card_margin;
        for (index, (deck, range)) in decks.iter().enumerate().skip(first) {
            let unfurl = deck.unfurl();
            self.measure_deck(cards, range.clone(), width, ctx);

            let header = Rect::new(
                left,
                y,
                left + width,
                y + deck::header_height(range.len(), unfurl, ctx.theme),
            );
            let top = header.y1;
            let start = self.placements.len();

            for depth in 0..=deck::deepest(range.len(), unfurl) {
                let pose = deck::pose(&self.heights, depth, unfurl, ctx.theme);
                let rect = Rect::new(
                    left + pose.inset,
                    top + pose.top,
                    left + width - pose.inset,
                    top + pose.top + pose.height,
                );
                let card = &mut cards[range.start + depth];
                // A card whose own deck is moving is driven by the unfurl
                // spring; letting its offset spring chase that as well would
                // stack two lags and make the deck feel like rubber.
                if deck.is_settling() {
                    card.animation.snap_offset(rect.y0 as f32);
                } else {
                    card.animation.set_offset(rect.y0 as f32);
                }
                self.placements.push(Placement {
                    index: range.start + depth,
                    rect,
                    depth,
                    unfurl,
                    alpha: pose.alpha,
                });
            }

            let height = deck::height(&self.heights, unfurl, ctx.theme);
            self.decks.push(DeckPlacement {
                index,
                bounds: Rect::new(left, top, left + width, y + height),
                header,
                cards: start..self.placements.len(),
                unfurl,
            });
            y += height + metrics.card_gap;
        }
        self.height = (y - metrics.card_gap - metrics.card_margin).max(0.0);
    }

    /// Fills `heights` with the measured height of every card a deck draws.
    ///
    /// A collapsed deck measures only the cards it shows, so a stack of thirty
    /// costs three measurements rather than thirty.
    fn measure_deck(
        &mut self,
        cards: &mut [Card],
        range: std::ops::Range<usize>,
        width: f64,
        ctx: &mut MeasureCtx<'_>,
    ) {
        self.heights.clear();
        for card in &mut cards[range] {
            let height = card.measure(width, ctx);
            self.heights.push(height);
        }
    }

    /// The oldest deck that still fits in `budget`, counting back from the
    /// newest.
    ///
    /// Overflow is resolved from the *old* end because the newest notification
    /// is the one the user has not seen yet, and a whole deck at a time because
    /// half a deck on the bottom edge is a stack of plates with nothing on it.
    /// The newest is therefore always placed, even on a surface too short to
    /// hold it — showing a clipped card beats silently dropping the thing that
    /// just happened.
    fn first_visible(
        &mut self,
        cards: &mut [Card],
        decks: &[(Deck, std::ops::Range<usize>)],
        width: f64,
        budget: f64,
        ctx: &mut MeasureCtx<'_>,
    ) -> usize {
        let gap = ctx.theme.metrics.card_gap;
        let mut used = 0.0;
        let mut first = decks.len();
        for index in (0..decks.len()).rev() {
            let (deck, range) = &decks[index];
            self.measure_deck(cards, range.clone(), width, ctx);
            let height = deck::height(&self.heights, deck.unfurl(), ctx.theme)
                + deck::header_height(range.len(), deck.unfurl(), ctx.theme);
            let needed = if first == decks.len() {
                height
            } else {
                used + gap + height
            };
            if needed > budget && first != decks.len() {
                break;
            }
            used = needed;
            first = index;
        }
        first
    }
}

/// How wide a card is drawn on a surface `available` wide.
///
/// The design fixes the width at [`Metrics::card_width`], but a surface
/// narrower than that (a phone-sized output, or a compositor that decided the
/// layer surface is only so wide) would otherwise get cards hanging off it.
///
/// [`Metrics::card_width`]: crate::ui::theme::Metrics::card_width
pub fn card_width(available: Size, theme: &Theme) -> f64 {
    let metrics = &theme.metrics;
    metrics
        .card_width
        .min(available.width - 2.0 * metrics.card_margin)
        .max(0.0)
}

/// The x of a card's left edge: the stack is anchored to the right of the
/// surface, one margin in.
pub fn card_left(available: Size, theme: &Theme) -> f64 {
    (available.width - theme.metrics.card_margin - card_width(available, theme)).max(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::card::tests::{card, general};
    use crownshell::TextContext;

    const SURFACE: Size = Size::new(400.0, 900.0);

    fn stack_of(bodies: &[&str]) -> Vec<Card> {
        bodies.iter().map(|body| card(&general(body))).collect()
    }

    fn ctx<'a>(tcx: &'a mut TextContext, theme: &'a Theme) -> MeasureCtx<'a> {
        MeasureCtx { text: tcx, theme }
    }

    /// One deck per card, which is what every notification from a different app
    /// produces and what the geometry must reduce to.
    fn solo(count: usize) -> Vec<(Deck, std::ops::Range<usize>)> {
        (0..count)
            .map(|index| (Deck::new(format!("app-{index}")), index..index + 1))
            .collect()
    }

    #[test]
    fn an_empty_stack_places_nothing_and_has_no_height() {
        let theme = Theme::light();
        let mut tcx = TextContext::new();
        let mut stack = Stack::new();

        stack.layout(&mut [], &solo(0), SURFACE, &mut ctx(&mut tcx, &theme));
        assert!(stack.is_empty());
        assert_eq!(stack.height(), 0.0);
        assert_eq!(stack.hidden(), 0);
        assert_eq!(stack.placement_for(0), None);
    }

    #[test]
    fn offsets_accumulate_with_the_gap_between_cards() {
        let theme = Theme::light();
        let mut tcx = TextContext::new();
        let mut cards = stack_of(&["one", "two and a rather longer body than one", "three"]);
        let mut stack = Stack::new();

        let decks = solo(cards.len());
        stack.layout(&mut cards, &decks, SURFACE, &mut ctx(&mut tcx, &theme));
        let placements = stack.placements();
        assert_eq!(placements.len(), 3, "got {placements:?}");
        assert_eq!(placements[0].rect.y0, theme.metrics.card_margin);

        for pair in placements.windows(2) {
            let gap = pair[1].rect.y0 - pair[0].rect.y1;
            assert!(
                (gap - theme.metrics.card_gap).abs() < 1e-9,
                "cards {} and {} are {gap} apart, not {}",
                pair[0].index,
                pair[1].index,
                theme.metrics.card_gap
            );
        }

        let last = placements[2].rect;
        assert_eq!(
            stack.height(),
            last.y1 - theme.metrics.card_margin,
            "the reported height does not reach the last card"
        );
    }

    #[test]
    fn every_card_is_placed_at_the_width_and_x_the_theme_asks_for() {
        let theme = Theme::light();
        let mut tcx = TextContext::new();
        let mut cards = stack_of(&["one", "two"]);
        let mut stack = Stack::new();

        let decks = solo(cards.len());
        stack.layout(&mut cards, &decks, SURFACE, &mut ctx(&mut tcx, &theme));
        for placement in stack.placements() {
            assert_eq!(placement.rect.width(), theme.metrics.card_width);
            assert_eq!(placement.rect.x1, SURFACE.width - theme.metrics.card_margin);
        }
    }

    /// A surface narrower than the design width must shrink the cards rather
    /// than hang them off the edge.
    #[test]
    fn a_narrow_surface_shrinks_the_cards_to_fit_inside_its_margins() {
        let theme = Theme::light();
        let narrow = Size::new(200.0, 900.0);

        assert_eq!(
            card_width(narrow, &theme),
            200.0 - 2.0 * theme.metrics.card_margin
        );
        assert_eq!(card_left(narrow, &theme), theme.metrics.card_margin);
        assert_eq!(card_width(SURFACE, &theme), theme.metrics.card_width);
    }

    #[test]
    fn a_re_flow_slides_each_card_to_its_new_offset_rather_than_jumping() {
        let theme = Theme::light();
        let mut tcx = TextContext::new();
        let mut cards = stack_of(&["one", "two"]);
        let mut stack = Stack::new();

        let decks = solo(cards.len());
        stack.layout(&mut cards, &decks, SURFACE, &mut ctx(&mut tcx, &theme));
        let settled = stack.placements()[1].rect.y0;
        for _ in 0..600 {
            cards[1].animation.step(1.0 / 60.0);
        }
        assert!((cards[1].animation.top() - settled).abs() < 0.5);

        // The card above grows, so the one below has to move down.
        cards[0].sync(&general(crate::ui::card::tests::LONG_BODY), &theme);
        let decks = solo(cards.len());
        stack.layout(&mut cards, &decks, SURFACE, &mut ctx(&mut tcx, &theme));
        let moved = stack.placements()[1].rect.y0;
        assert!(moved > settled, "the stack did not re-flow: {moved}");
        assert!(
            (cards[1].animation.top() - settled).abs() < 0.5,
            "the card teleported to its new offset instead of springing to it"
        );

        for _ in 0..600 {
            cards[1].animation.step(1.0 / 60.0);
        }
        assert!(
            (cards[1].animation.top() - moved).abs() < 0.5,
            "the card settled at {} instead of {moved}",
            cards[1].animation.top()
        );
    }

    #[test]
    fn a_stack_taller_than_the_surface_hides_the_oldest_cards() {
        let theme = Theme::light();
        let mut tcx = TextContext::new();
        let mut cards = stack_of(&["one", "two", "three", "four", "five"]);

        let single = {
            let mut probe = stack_of(&["one"]);
            let mut stack = Stack::new();
            let decks = solo(probe.len());
            stack.layout(&mut probe, &decks, SURFACE, &mut ctx(&mut tcx, &theme));
            stack.placements()[0].rect.height()
        };

        // Room for exactly two cards and their gap.
        let short = Size::new(
            SURFACE.width,
            2.0 * theme.metrics.card_margin + 2.0 * single + theme.metrics.card_gap,
        );
        let mut stack = Stack::new();
        let decks = solo(cards.len());
        stack.layout(&mut cards, &decks, short, &mut ctx(&mut tcx, &theme));

        assert_eq!(stack.placements().len(), 2, "got {:?}", stack.placements());
        assert_eq!(stack.hidden(), 3);
        assert_eq!(
            stack.placements()[1].index,
            4,
            "the newest card was not the one kept"
        );
        assert_eq!(stack.placement_for(0), None, "an old card was still placed");
        assert!(stack.placement_for(4).is_some());
        assert!(
            stack.placements()[1].rect.y1 <= short.height - theme.metrics.card_margin + 1e-9,
            "the kept cards ran past the bottom margin"
        );
    }

    /// Dropping everything would hide the notification that just arrived, which
    /// is the one thing a toast must never do.
    #[test]
    fn the_newest_card_is_placed_even_on_a_surface_too_short_to_hold_it() {
        let theme = Theme::light();
        let mut tcx = TextContext::new();
        let mut cards = stack_of(&["one", "two", "three"]);
        let mut stack = Stack::new();

        let decks = solo(cards.len());

        stack.layout(
            &mut cards,
            &decks,
            Size::new(SURFACE.width, 10.0),
            &mut ctx(&mut tcx, &theme),
        );
        assert_eq!(stack.placements().len(), 1);
        assert_eq!(stack.placements()[0].index, 2);
        assert_eq!(stack.hidden(), 2);
    }

    #[test]
    fn laying_the_same_stack_out_twice_reuses_the_placement_buffer() {
        let theme = Theme::light();
        let mut tcx = TextContext::new();
        let mut cards = stack_of(&["one", "two", "three"]);
        let mut stack = Stack::new();

        let decks = solo(cards.len());
        stack.layout(&mut cards, &decks, SURFACE, &mut ctx(&mut tcx, &theme));
        let capacity = stack.placements.capacity();
        let first = stack.placements().to_vec();

        let decks = solo(cards.len());
        stack.layout(&mut cards, &decks, SURFACE, &mut ctx(&mut tcx, &theme));
        assert_eq!(
            stack.placements(),
            first.as_slice(),
            "the layout was not stable"
        );
        assert_eq!(
            stack.placements.capacity(),
            capacity,
            "re-laying out re-allocated the placements"
        );
    }
}
