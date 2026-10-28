//! The geometry of one app's stack of cards.
//!
//! A deck is the newest notification from an app with the rest layered behind
//! it. Collapsed, only the top card is drawn in full and the ones behind it
//! peek out as narrower, dimmer slivers; unfurled, every card sits on its own
//! at full width. Everything between the two is a lerp of a single spring, so
//! the unfurl is continuous and can be reversed mid-flight.
//!
//! Every function here is pure arithmetic over `(heights, unfurl, metrics)`,
//! which is what lets the whole geometry be asserted without a GPU or a
//! compositor — the property `tests/layout.rs` exists to protect.

use std::time::Duration;

use crate::{ui::theme::Metrics, util::spring::Spring};

/// How many cards peek out from behind the top of a collapsed deck.
///
/// Two: one sliver reads as an accident, three is a wedge nobody can count.
pub const MAX_PEEK: usize = 2;

/// How much dimmer each peeking card is than the one in front of it.
const PEEK_FADE: f32 = 0.32;

/// The share of the unfurl a card spends getting back to its own height and
/// full width, before any of its contents are drawn.
///
/// Text sliding out from under the card in front of it reads as a rendering
/// fault rather than as an animation, so the shape finishes first.
const SHAPE: f32 = 0.45;

/// Above this a deck counts as open. A critically damped spring approaches its
/// target asymptotically and never actually arrives.
pub const OPEN: f32 = 0.999;

/// Opacity of a card `depth` cards behind the top of a collapsed deck.
///
/// Zero past the last sliver, which is what stops a deck of thirty stacking
/// thirty translucent plates on the same few pixels.
pub fn peek_alpha(depth: usize) -> f32 {
    if depth > MAX_PEEK {
        return 0.0;
    }
    (1.0 - PEEK_FADE * depth as f32).max(0.0)
}

/// How far the deck's *shape* has opened.
fn shape(unfurl: f32) -> f32 {
    (unfurl / SHAPE).clamp(0.0, 1.0)
}

/// Opacity of the contents of the card at `depth`.
///
/// The top card always shows its own; the ones behind it are held at zero
/// until the shape has finished.
pub fn content_alpha(depth: usize, unfurl: f32) -> f32 {
    if depth == 0 {
        return 1.0;
    }
    ((unfurl - SHAPE) / (1.0 - SHAPE)).clamp(0.0, 1.0)
}

/// The deepest card a deck draws: everything while it is unfurling, the top
/// card and its slivers once it has closed.
pub fn deepest(len: usize, unfurl: f32) -> usize {
    let last = len.saturating_sub(1);
    if unfurl > 0.0 {
        last
    } else {
        last.min(MAX_PEEK)
    }
}

fn lerp(from: f32, to: f32, t: f32) -> f32 {
    from + (to - from) * t
}

/// Where one card of a deck sits, relative to the top of the deck's cards.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pose {
    pub top: f32,
    pub height: f32,
    /// Horizontal inset per side, which is what makes a sliver narrower than
    /// the card in front of it.
    ///
    /// An inset rather than an `Affine` scale: scaling would shrink the corner
    /// radius and the shadow with it, and break the rectangle arithmetic the
    /// input and material regions are built from.
    pub inset: f32,
    pub alpha: f32,
}

/// The pose of the card at `depth`, given every card's measured height.
///
/// Stated once because the overflow scan and the placement pass must agree on
/// it exactly; a deck measured one way and placed another loses a card off the
/// bottom of the strip with nothing on screen to explain it.
pub fn pose(heights: &[f32], depth: usize, unfurl: f32, metrics: &Metrics) -> Pose {
    let peek = depth.min(MAX_PEEK);
    let opened = shape(unfurl);

    let collapsed_top = metrics.deck_peek * peek as f32;
    let expanded_top: f32 = heights[..depth.min(heights.len())]
        .iter()
        .map(|height| height + metrics.card_gap)
        .sum();

    let own = heights.get(depth).copied().unwrap_or_default();
    let front = heights.first().copied().unwrap_or_default();

    Pose {
        top: lerp(collapsed_top, expanded_top, unfurl),
        // A sliver mirrors the top card's height rather than its own, so a deck
        // of mismatched cards still reads as one stack of plates.
        height: lerp(front, own, opened),
        inset: metrics.deck_inset * peek as f32 * (1.0 - opened),
        alpha: peek_alpha(depth) + (1.0 - peek_alpha(depth)) * unfurl,
    }
}

/// Height of the strip the deck's Clear All pill sits in.
///
/// Zero for a deck of one, and zero while collapsed, so grouping costs the
/// toast strip nothing until the user asks for it.
pub fn header_height(len: usize, unfurl: f32, metrics: &Metrics) -> f32 {
    if len < 2 {
        return 0.0;
    }
    lerp(0.0, metrics.button + metrics.card_gap, unfurl)
}

/// How much room a deck occupies, its Clear All strip included.
pub fn height(heights: &[f32], unfurl: f32, metrics: &Metrics) -> f32 {
    if heights.is_empty() {
        return 0.0;
    }
    let last = pose(heights, deepest(heights.len(), unfurl), unfurl, metrics);
    header_height(heights.len(), unfurl, metrics) + last.top + last.height
}

/// One app's cards, and how far the user has pulled them apart.
#[derive(Debug)]
pub struct Deck {
    key: String,
    /// 0 = one layered deck, 1 = every card on its own.
    ///
    /// The only spring a deck owns: the peek offsets, the widths, the opacities
    /// and the Clear All strip are all functions of it, so there is nothing for
    /// a second spring to fall out of step with.
    unfurl: Spring,
}

impl Deck {
    pub fn new(key: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            unfurl: Spring::new(0.0, motion::Spring::snappy()),
        }
    }

    pub fn key(&self) -> &str {
        &self.key
    }

    /// Clamped to `0.0..=1.0`: every formula above assumes that range, and a
    /// snappy spring overshoots.
    pub fn unfurl(&self, now: Duration) -> f32 {
        self.unfurl.value(now).clamp(0.0, 1.0)
    }

    pub fn is_open(&self, now: Duration) -> bool {
        self.unfurl(now) >= OPEN
    }

    /// Whether the user asked for the deck to be pulled apart, wherever its
    /// spring has got to.
    pub fn is_opening(&self) -> bool {
        self.unfurl.target() > 0.5
    }

    /// Whether the unfurl is still moving, which is what tells the layout to
    /// drive this deck's cards directly instead of through their own springs.
    pub fn is_settling(&self, now: Duration) -> bool {
        !self.unfurl.at_rest(now)
    }

    pub fn toggle(&mut self, now: Duration) {
        let target = if self.is_opening() { 0.0 } else { 1.0 };
        self.unfurl.set_target(now, target);
    }

    /// Puts the deck back together with no animation, for a window that has
    /// gone off screen and has nothing to animate for.
    pub fn collapse(&mut self) {
        self.unfurl.snap(0.0);
    }

    /// A deck of one is never unfurled — there is nothing behind its top card —
    /// so an ungrouped notification lays out exactly as a bare card does.
    pub fn sync_len(&mut self, len: usize) {
        if len < 2 {
            self.collapse();
        }
    }

    pub fn at_rest(&self, now: Duration) -> bool {
        self.unfurl.at_rest(now)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metrics() -> Metrics {
        Metrics::default()
    }

    #[test]
    fn a_collapsed_deck_steps_its_slivers_down_and_in() {
        let metrics = metrics();
        let heights = [80.0, 80.0, 80.0];
        let poses: Vec<_> = (0..3).map(|d| pose(&heights, d, 0.0, &metrics)).collect();

        assert_eq!(poses[0].top, 0.0);
        assert_eq!(poses[1].top, metrics.deck_peek);
        assert_eq!(poses[2].top, 2.0 * metrics.deck_peek);
        assert!(poses[0].inset < poses[1].inset && poses[1].inset < poses[2].inset);
        assert!(poses[0].alpha > poses[1].alpha && poses[1].alpha > poses[2].alpha);
    }

    #[test]
    fn an_unfurled_deck_stacks_its_cards_with_the_ordinary_gap() {
        let metrics = metrics();
        let heights = [60.0, 90.0];
        let second = pose(&heights, 1, 1.0, &metrics);
        assert_eq!(second.top, heights[0] + metrics.card_gap);
        assert_eq!(second.height, heights[1], "a card regains its own height");
        assert_eq!(second.inset, 0.0);
        assert_eq!(second.alpha, 1.0);
    }

    #[test]
    fn a_deck_of_one_is_exactly_a_bare_card() {
        let metrics = metrics();
        let heights = [74.0];
        for unfurl in [0.0, 0.5, 1.0] {
            let only = pose(&heights, 0, unfurl, &metrics);
            assert_eq!(only.top, 0.0);
            assert_eq!(only.height, 74.0);
            assert_eq!(only.inset, 0.0);
            assert_eq!(only.alpha, 1.0);
            assert_eq!(height(&heights, unfurl, &metrics), 74.0);
        }
    }

    #[test]
    fn the_cards_of_a_deck_exactly_fill_the_height_it_reported() {
        let metrics = metrics();
        let heights = [70.0, 84.0, 62.0, 90.0];
        for unfurl in [0.0, 0.25, 0.5, 0.75, 1.0] {
            let last = deepest(heights.len(), unfurl);
            let pose = pose(&heights, last, unfurl, &metrics);
            let expected = header_height(heights.len(), unfurl, &metrics) + pose.top + pose.height;
            assert!(
                (height(&heights, unfurl, &metrics) - expected).abs() < f32::EPSILON * 8.0,
                "unfurl {unfurl}: the overflow scan and the placement pass disagree"
            );
        }
    }

    #[test]
    fn a_half_open_deck_sits_between_its_collapsed_and_open_geometry() {
        let metrics = metrics();
        let heights = [70.0, 84.0, 62.0];
        for depth in 0..heights.len() {
            let closed = pose(&heights, depth, 0.0, &metrics);
            let open = pose(&heights, depth, 1.0, &metrics);
            let half = pose(&heights, depth, 0.5, &metrics);
            assert!(half.top >= closed.top && half.top <= open.top, "{depth}");
            assert!(
                half.inset <= closed.inset && half.inset >= open.inset,
                "{depth}"
            );
            assert!(
                half.alpha >= closed.alpha && half.alpha <= open.alpha,
                "{depth}"
            );
        }
    }

    #[test]
    fn a_card_is_back_at_full_width_before_its_text_appears() {
        let metrics = metrics();
        let heights = [70.0, 84.0];
        for step in 0..=100 {
            let unfurl = step as f32 / 100.0;
            if content_alpha(1, unfurl) > 0.0 {
                let behind = pose(&heights, 1, unfurl, &metrics);
                assert_eq!(
                    behind.inset, 0.0,
                    "text shown while still inset at {unfurl}"
                );
                assert_eq!(
                    behind.height, heights[1],
                    "text shown in a box that does not fit"
                );
            }
        }
    }

    #[test]
    fn the_top_card_shows_its_contents_however_far_the_deck_is_open() {
        for step in 0..=100 {
            let unfurl = step as f32 / 100.0;
            assert_eq!(content_alpha(0, unfurl), 1.0, "top card hidden at {unfurl}");
        }
        assert_eq!(content_alpha(1, 0.0), 0.0, "a sliver showed its contents");
    }

    #[test]
    fn a_collapsed_deck_draws_its_top_card_and_no_more_than_two_slivers() {
        assert_eq!(deepest(1, 0.0), 0);
        assert_eq!(deepest(3, 0.0), 2);
        assert_eq!(deepest(30, 0.0), MAX_PEEK);
        assert_eq!(deepest(30, 1.0), 29, "unfurled, every card is drawn");
        assert_eq!(peek_alpha(MAX_PEEK + 1), 0.0);
    }

    #[test]
    fn a_clear_all_strip_opens_only_while_a_deck_of_more_than_one_is_unfurled() {
        let metrics = metrics();
        assert_eq!(header_height(1, 1.0, &metrics), 0.0);
        assert_eq!(header_height(3, 0.0, &metrics), 0.0);
        assert!(header_height(3, 1.0, &metrics) > 0.0);
    }

    #[test]
    fn a_deck_of_one_never_unfurls() {
        let mut deck = Deck::new("mail");
        deck.toggle(Duration::ZERO);
        deck.sync_len(1);
        let later = Duration::from_secs(5);
        assert_eq!(deck.unfurl(later), 0.0);
        assert!(!deck.is_open(later));
    }

    #[test]
    fn toggling_a_deck_mid_flight_reverses_it_without_a_jump() {
        let mut deck = Deck::new("mail");
        deck.toggle(Duration::ZERO);
        let now = Duration::from_millis(50);
        let caught = deck.unfurl(now);
        assert!(caught > 0.0 && caught < 1.0, "{caught}");
        deck.toggle(now);
        assert!(
            (deck.unfurl(now) - caught).abs() < f32::EPSILON,
            "the position moved when only the target should have"
        );
    }
}
