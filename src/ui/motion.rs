//! Per-card animation state.
//!
//! Nothing here keeps a duration: each value is a [`Spring`] sampled against
//! the window's clock, which is what lets a card be retargeted mid-flight —
//! re-flowed while still entering, dismissed while still sliding — without the
//! discontinuity a tween would have at the moment its end point changed.

use std::time::Duration;

use crate::util::spring::Spring;

/// Reveal-space velocity a dismissal starts with, in card widths per second.
///
/// A critically-damped spring covers only a few percent of its distance in the
/// first frame. For an enter that reads as an unhurried ease-in; for a
/// dismissal it reads as the card being stuck, so the leave spring is kicked.
const DISMISS_VELOCITY: f32 = 3.0;

/// The lifecycle of one card, as the stack sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Entering,
    Resting,
    Leaving,
}

/// The animated state of a single card.
#[derive(Debug, Clone, Copy)]
pub struct CardMotion {
    pub phase: Phase,
    /// 0 = fully off-screen right, 1 = resting. Drives x-offset and alpha.
    reveal: Spring,
    /// Y of the card's top edge in the stack, sprung so re-flow slides.
    offset: Spring,
    /// Live drag displacement while a swipe-to-dismiss is in progress, in
    /// logical pixels. Positive is towards the right edge of the output.
    pub drag: f32,
}

impl CardMotion {
    /// A card that has just been posted, entering at `offset` from the top of
    /// the window.
    pub fn entering(now: Duration, offset: f32) -> Self {
        let mut reveal = Spring::new(0.0, motion::Spring::smooth());
        reveal.set_target(now, 1.0);
        Self {
            phase: Phase::Entering,
            reveal,
            // Starts *at* its offset rather than springing to it: a new card
            // has no previous position to slide from, and animating one in
            // would make the whole stack lurch every time a toast arrives.
            offset: Spring::new(offset, motion::Spring::smooth()),
            drag: 0.0,
        }
    }

    /// Slides the card to a new place in the stack.
    pub fn set_offset(&mut self, now: Duration, offset: f32) {
        self.offset.set_target(now, offset);
    }

    /// Puts the card at `offset` with no slide, for the frames where something
    /// else is already doing the smoothing — a deck driven by its own unfurl
    /// spring, where a second lag would read as rubber.
    pub fn snap_offset(&mut self, offset: f32) {
        self.offset.snap(offset);
    }

    /// The card's current top edge, in logical pixels.
    pub fn top(&self, now: Duration) -> f32 {
        self.offset.value(now)
    }

    /// Begins the leave animation. Idempotent, so a card that is already going
    /// away is not restarted by a second click on it.
    pub fn leave(&mut self, now: Duration) {
        self.leave_with_velocity(now, -DISMISS_VELOCITY);
    }

    /// Leaves carrying the velocity a swipe ended with.
    ///
    /// `reveal_velocity` is in reveal units per second — a card width per
    /// second is `1.0` — and negative means "towards off-screen", so a caller
    /// with a pointer velocity in pixels per second passes
    /// [`dismiss_velocity`]. It is clamped so a dismissal is never *slower*
    /// than the default one; a lazy flick still snaps away.
    pub fn leave_with_velocity(&mut self, now: Duration, reveal_velocity: f32) {
        if self.phase == Phase::Leaving {
            return;
        }
        self.phase = Phase::Leaving;
        self.reveal.launch(
            now,
            0.0,
            reveal_velocity.min(-DISMISS_VELOCITY),
            motion::Spring::snappy(),
        );
    }

    /// Moves the phase on; returns false once the card has finished leaving
    /// and can be dropped.
    pub fn settle(&mut self, now: Duration) -> bool {
        match self.phase {
            Phase::Entering => {
                // The offset spring is deliberately not consulted: a card is
                // done entering when it is fully revealed, and the stack may
                // still be sliding it around long after that.
                if self.reveal.at_rest(now) {
                    self.phase = Phase::Resting;
                }
                true
            }
            Phase::Resting => true,
            Phase::Leaving => !self.reveal.at_rest(now),
        }
    }

    /// Whether nothing is moving, so the window can stop asking for frames.
    pub fn at_rest(&self, now: Duration) -> bool {
        self.reveal.at_rest(now) && self.offset.at_rest(now)
    }

    /// Opacity to draw the card at, always within `0.0..=1.0`.
    pub fn alpha(&self, now: Duration) -> f32 {
        self.reveal.value(now).clamp(0.0, 1.0)
    }

    /// How far right of its resting place the card should be drawn, for a card
    /// `width` logical pixels wide. Includes any live [`drag`](Self::drag).
    pub fn translate_x(&self, now: Duration, width: f32) -> f32 {
        (1.0 - self.alpha(now)) * width + self.drag
    }
}

/// Converts a pointer velocity into the units the leave spring wants.
///
/// One card width per second is `1.0` and off-screen is negative, so a flick
/// to the right at `p` pixels per second is `-(p / card_width)`. Getting the
/// sign wrong makes a flicked card lunge back onto the screen before it leaves.
pub fn dismiss_velocity(pixels_per_second: f32, card_width: f32) -> f32 {
    if card_width <= 0.0 {
        return 0.0;
    }
    -(pixels_per_second / card_width)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CARD_WIDTH: f32 = 376.0;

    fn ms(millis: u64) -> Duration {
        Duration::from_millis(millis)
    }

    #[test]
    fn an_entering_card_settles_and_becomes_resting() {
        let mut card = CardMotion::entering(Duration::ZERO, 12.0);
        assert_eq!(card.phase, Phase::Entering);
        assert!(
            card.alpha(Duration::ZERO) < 0.01,
            "a new card starts hidden"
        );

        assert!(card.settle(ms(5_000)));
        assert_eq!(card.phase, Phase::Resting);
        assert_eq!(card.alpha(ms(5_000)), 1.0);
        assert_eq!(card.translate_x(ms(5_000), CARD_WIDTH), 0.0);
        assert_eq!(card.top(ms(5_000)), 12.0, "a new card starts at its offset");
    }

    #[test]
    fn a_leaving_card_finishes_and_reports_that_it_can_be_dropped() {
        let mut card = CardMotion::entering(Duration::ZERO, 0.0);
        card.settle(ms(5_000));
        card.leave(ms(5_000));
        assert!(card.settle(ms(5_016)), "dropped on its first frame");
        assert!(!card.settle(ms(10_000)), "never finished leaving");
    }

    #[test]
    fn alpha_and_translation_stay_in_range_for_the_whole_life_of_a_card() {
        let mut card = CardMotion::entering(Duration::ZERO, 0.0);
        for frame in 0..120 {
            let now = ms(frame * 16);
            if frame == 60 {
                card.leave(now);
            }
            let alpha = card.alpha(now);
            assert!((0.0..=1.0).contains(&alpha), "alpha {alpha} at {frame}");
            let x = card.translate_x(now, CARD_WIDTH);
            assert!((0.0..=CARD_WIDTH).contains(&x), "x {x} at {frame}");
        }
    }

    #[test]
    fn a_dismissal_moves_visibly_on_its_first_frame() {
        let mut card = CardMotion::entering(Duration::ZERO, 0.0);
        card.settle(ms(5_000));
        card.leave(ms(5_000));
        assert!(card.translate_x(ms(5_016), CARD_WIDTH) > 4.0);
    }

    #[test]
    fn a_slow_flick_is_never_slower_than_a_plain_dismissal() {
        let mut plain = CardMotion::entering(Duration::ZERO, 0.0);
        let mut flicked = plain;
        plain.leave(ms(5_000));
        flicked.leave_with_velocity(ms(5_000), dismiss_velocity(10.0, CARD_WIDTH));
        assert_eq!(
            plain.translate_x(ms(5_050), CARD_WIDTH),
            flicked.translate_x(ms(5_050), CARD_WIDTH)
        );
    }

    #[test]
    fn leaving_twice_does_not_restart_the_animation() {
        let mut card = CardMotion::entering(Duration::ZERO, 0.0);
        card.settle(ms(5_000));
        card.leave(ms(5_000));
        let gone = card.translate_x(ms(5_050), CARD_WIDTH);
        card.leave(ms(5_050));
        assert_eq!(card.translate_x(ms(5_050), CARD_WIDTH), gone);
    }

    #[test]
    fn re_flowing_the_stack_slides_the_card_to_its_new_offset() {
        let mut card = CardMotion::entering(Duration::ZERO, 100.0);
        card.set_offset(Duration::ZERO, 40.0);
        let midway = card.top(ms(16));
        assert!(midway < 100.0 && midway > 40.0, "{midway}");
        assert_eq!(card.top(ms(5_000)), 40.0);
    }

    #[test]
    fn a_drag_carries_the_card_with_the_pointer() {
        let mut card = CardMotion::entering(Duration::ZERO, 0.0);
        card.drag = 25.0;
        assert_eq!(card.translate_x(ms(5_000), CARD_WIDTH), 25.0);
    }

    #[test]
    fn a_pointer_velocity_becomes_a_reveal_velocity_in_card_widths_per_second() {
        assert_eq!(dismiss_velocity(376.0, 376.0), -1.0);
        assert_eq!(dismiss_velocity(-188.0, 376.0), 0.5);
        assert_eq!(dismiss_velocity(100.0, 0.0), 0.0);
    }
}
