//! Per-card animation state.
//!
//! Every value a card animates is a [`Spring`] from crownshell, so there is
//! exactly one integrator in the process and it is the one that has already
//! been made stable across a stalled frame. Nothing here keeps a duration or a
//! start time: a spring carries its own position and velocity, which is what
//! lets a card be retargeted mid-flight — re-flowed while still entering,
//! dismissed while still sliding — without the discontinuity a tween would
//! have at the moment its start and end points changed.

use crownshell::{Spring, SpringProfile};

/// Reveal-space velocity a dismissal starts with, in card widths per second.
///
/// A critically-damped spring covers only a few percent of its distance in the
/// first frame. For an enter that reads as an unhurried ease-in; for a
/// dismissal it reads as the card being stuck, so the leave spring is kicked.
const DISMISS_VELOCITY: f32 = 3.0;

/// The lifecycle of one toast, as the stack sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Entering,
    Resting,
    Leaving,
}

/// The animated state of a single card.
///
/// Both springs are public because the stack drives them directly: layout
/// retargets [`offset`](Self::offset) when the cards above a card change
/// height, and a swipe writes [`drag`](Self::drag) every pointer motion.
#[derive(Debug)]
pub struct CardAnimation {
    pub phase: Phase,
    /// 0 = fully off-screen right, 1 = resting. Drives x-offset and alpha.
    pub reveal: Spring,
    /// Y of the card's top edge in the stack, sprung so re-flow slides.
    pub offset: Spring,
    /// Live drag displacement while a swipe-to-dismiss is in progress, in
    /// logical pixels. Positive is towards the right edge of the output.
    pub drag: f32,
}

impl CardAnimation {
    /// A card that has just been posted, entering at `offset` from the top of
    /// the stack.
    pub fn entering(offset: f32) -> Self {
        let mut reveal = Spring::with_profile(0.0, SpringProfile::SMOOTH);
        reveal.set_target(1.0);
        Self {
            phase: Phase::Entering,
            reveal,
            // Starts *at* its offset rather than springing to it: a new card
            // has no previous position to slide from, and animating one in
            // would make the whole stack lurch every time a toast arrives.
            offset: Spring::with_profile(offset, SpringProfile::SMOOTH),
            drag: 0.0,
        }
    }

    /// Slides the card to a new place in the stack.
    pub fn set_offset(&mut self, offset: f32) {
        self.offset.set_target(offset);
    }

    /// Puts the card at `offset` with no slide, for the frames where something
    /// else is already doing the smoothing — a deck driven by its own unfurl
    /// spring, where a second lag would read as rubber.
    pub fn snap_offset(&mut self, offset: f32) {
        self.offset.set_target(offset);
        self.offset.snap_to_target();
    }

    /// The card's current top edge in the stack, in logical pixels.
    pub fn top(&self) -> f64 {
        self.offset.position as f64
    }

    /// Begins the leave animation. Idempotent, so a card that is already going
    /// away is not restarted by a second click on it.
    pub fn leave(&mut self) {
        self.leave_with_velocity(-DISMISS_VELOCITY);
    }

    /// Leaves carrying the velocity a swipe ended with.
    ///
    /// `reveal_velocity` is in reveal units per second — a card width per
    /// second is `1.0` — and negative means "towards off-screen", so a caller
    /// with a pointer velocity in pixels per second passes
    /// `-(pixels_per_second / card_width)`. It is clamped so a dismissal is
    /// never *slower* than the default one; a lazy flick still snaps away.
    pub fn leave_with_velocity(&mut self, reveal_velocity: f32) {
        if self.phase == Phase::Leaving {
            return;
        }
        self.phase = Phase::Leaving;
        self.reveal.profile = SpringProfile::SNAPPY;
        self.reveal
            .set_target_with_velocity(0.0, reveal_velocity.min(-DISMISS_VELOCITY));
    }

    /// Advances by `dt`; returns false once the card has finished leaving and
    /// can be dropped.
    pub fn step(&mut self, dt: f32) -> bool {
        self.reveal.step(dt);
        self.offset.step(dt);
        match self.phase {
            Phase::Entering => {
                // The offset spring is deliberately not consulted: a card is
                // done entering when it is fully revealed, and the stack may
                // still be sliding it around long after that.
                if self.reveal.at_rest() {
                    self.phase = Phase::Resting;
                }
                true
            }
            Phase::Resting => true,
            Phase::Leaving => !self.reveal.at_rest(),
        }
    }

    /// Whether nothing is moving, so the surface can stop asking for frames.
    pub fn at_rest(&self) -> bool {
        self.reveal.at_rest() && self.offset.at_rest()
    }

    /// Opacity to draw the card at, always within `0.0..=1.0`.
    pub fn alpha(&self) -> f32 {
        self.reveal.position.clamp(0.0, 1.0)
    }

    /// How far right of its resting place the card should be drawn, for a card
    /// `width` logical pixels wide. Includes any live [`drag`](Self::drag).
    pub fn translate_x(&self, width: f64) -> f64 {
        (1.0 - self.alpha() as f64) * width + self.drag as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FRAME: f32 = 1.0 / 60.0;
    const CARD_WIDTH: f64 = 376.0;

    /// Steps at 60 Hz until `step` reports the card can be dropped, returning
    /// how many frames that took — or `None` if it never did.
    fn run_until_dropped(animation: &mut CardAnimation, frames: usize) -> Option<usize> {
        (1..=frames).find(|_| !animation.step(FRAME))
    }

    #[test]
    fn an_entering_card_settles_and_becomes_resting() {
        let mut animation = CardAnimation::entering(0.0);
        assert_eq!(animation.phase, Phase::Entering);
        assert_eq!(animation.alpha(), 0.0);

        let mut frames = 0;
        while animation.phase == Phase::Entering && frames < 600 {
            assert!(
                animation.step(FRAME),
                "an entering card asked to be dropped"
            );
            frames += 1;
        }

        assert_eq!(
            animation.phase,
            Phase::Resting,
            "still entering after {frames} frames"
        );
        assert!(
            frames < 120,
            "entering took {frames} frames at 60 Hz, which is not an animation any more"
        );
        assert!(
            (animation.alpha() - 1.0).abs() < 0.01,
            "{}",
            animation.alpha()
        );
        assert_eq!(animation.translate_x(CARD_WIDTH).round(), 0.0);
    }

    #[test]
    fn a_leaving_card_finishes_and_reports_that_it_can_be_dropped() {
        let mut animation = CardAnimation::entering(0.0);
        run_until_settled(&mut animation);

        animation.leave();
        assert_eq!(animation.phase, Phase::Leaving);

        let frames = run_until_dropped(&mut animation, 600).expect("leaving card never finished");
        assert!(
            frames < 60,
            "dismissal took {frames} frames at 60 Hz; it is meant to feel immediate"
        );
        assert!(
            animation.alpha() < 0.01,
            "a dropped card was still visible at alpha {}",
            animation.alpha()
        );
    }

    /// Steps a card until both its springs stop moving.
    fn run_until_settled(animation: &mut CardAnimation) {
        for _ in 0..600 {
            animation.step(FRAME);
            if animation.at_rest() {
                return;
            }
        }
        panic!("card never settled");
    }

    #[test]
    fn alpha_and_translation_stay_in_range_for_the_whole_life_of_a_card() {
        let mut animation = CardAnimation::entering(0.0);
        let mut leaving_after = 40;

        for frame in 0..400 {
            let alive = animation.step(FRAME);
            let alpha = animation.alpha();
            let x = animation.translate_x(CARD_WIDTH);

            assert!(
                (0.0..=1.0).contains(&alpha),
                "alpha left its range at frame {frame}: {alpha}"
            );
            assert!(
                (-0.001..=CARD_WIDTH + 0.001).contains(&x),
                "translation left its range at frame {frame}: {x}"
            );

            leaving_after -= 1;
            if leaving_after == 0 {
                animation.leave();
            }
            if !alive {
                return;
            }
        }
        panic!("card never finished");
    }

    /// The reason [`Spring::set_target_with_velocity`] exists: without the kick
    /// the first frames of a dismissal are visually indistinguishable from
    /// nothing happening.
    #[test]
    fn a_dismissal_moves_visibly_on_its_first_frame() {
        let mut animation = CardAnimation::entering(0.0);
        run_until_settled(&mut animation);
        animation.leave();
        animation.step(FRAME);

        let travelled = animation.translate_x(CARD_WIDTH);
        assert!(
            travelled > CARD_WIDTH * 0.03,
            "the card moved {travelled} px of {CARD_WIDTH} on the first frame of a dismissal"
        );
    }

    #[test]
    fn a_slow_flick_is_never_slower_than_a_plain_dismissal() {
        let mut flicked = CardAnimation::entering(0.0);
        let mut plain = CardAnimation::entering(0.0);
        run_until_settled(&mut flicked);
        run_until_settled(&mut plain);

        flicked.leave_with_velocity(-0.2);
        plain.leave();
        flicked.step(FRAME);
        plain.step(FRAME);

        assert!(
            flicked.alpha() <= plain.alpha() + f32::EPSILON,
            "a limp flick ({}) dismissed slower than a click ({})",
            flicked.alpha(),
            plain.alpha()
        );
    }

    #[test]
    fn leaving_twice_does_not_restart_the_animation() {
        let mut animation = CardAnimation::entering(0.0);
        run_until_settled(&mut animation);
        animation.leave();
        for _ in 0..5 {
            animation.step(FRAME);
        }
        let mid_flight = animation.alpha();

        animation.leave();
        assert_eq!(
            animation.alpha(),
            mid_flight,
            "a second dismissal jumped the card back"
        );
    }

    #[test]
    fn re_flowing_the_stack_slides_the_card_to_its_new_offset() {
        let mut animation = CardAnimation::entering(0.0);
        run_until_settled(&mut animation);

        animation.set_offset(120.0);
        assert!(!animation.at_rest(), "a re-flow did not wake the card");

        animation.step(FRAME);
        let after_one_frame = animation.top();
        assert!(
            after_one_frame > 0.0 && after_one_frame < 120.0,
            "the card teleported instead of sliding: {after_one_frame}"
        );

        run_until_settled(&mut animation);
        assert!(
            (animation.top() - 120.0).abs() < 0.5,
            "the card settled at {} instead of 120",
            animation.top()
        );
        assert_eq!(
            animation.phase,
            Phase::Resting,
            "a re-flow must not change the phase"
        );
    }

    #[test]
    fn a_drag_carries_the_card_with_the_pointer() {
        let mut animation = CardAnimation::entering(0.0);
        run_until_settled(&mut animation);
        let resting = animation.translate_x(CARD_WIDTH);

        animation.drag = 40.0;
        assert!(
            (animation.translate_x(CARD_WIDTH) - resting - 40.0).abs() < 1e-6,
            "the drag was not added to the resting position: {}",
            animation.translate_x(CARD_WIDTH)
        );

        // Releasing into a dismissal keeps the displacement: the card carries
        // on from where the finger left it rather than snapping back first.
        animation.leave();
        animation.step(FRAME);
        assert!(animation.translate_x(CARD_WIDTH) > 40.0);
    }
}
