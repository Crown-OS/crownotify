//! A retargetable spring, sampled at the timestamps the display hands out.
//!
//! The cards are moved on the UI thread rather than by crownui's render-thread
//! transitions, because the compositor's blur and the surface's input region are
//! taken from layout: a card animated anywhere else would leave its glass behind.
//! `motion`'s springs are closed-form, so sampling one at any instant is exact
//! and a retarget keeps both position and velocity — a card re-flowed while
//! still entering, or dismissed while still sliding, never jumps.

use std::time::Duration;

use crownui::motion::{self, Animation, Curve};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Spring {
    animation: Animation<f32>,
    curve: Curve,
}

impl Spring {
    /// At rest at `value`.
    pub fn new(value: f32, spring: motion::Spring) -> Self {
        let curve = Curve::from(spring);
        Self {
            animation: Animation::new(value, value, 0.0, Duration::ZERO, curve),
            curve,
        }
    }

    pub fn value(&self, now: Duration) -> f32 {
        self.animation.sample(now).value
    }

    pub fn target(&self) -> f32 {
        self.animation.target()
    }

    pub fn at_rest(&self, now: Duration) -> bool {
        self.animation.sample(now).finished
    }

    /// Heads for `target` from wherever the spring is now, keeping its speed.
    pub fn set_target(&mut self, now: Duration, target: f32) {
        if self.target() != target {
            self.animation = self.animation.retarget(now, target, self.curve);
        }
    }

    /// Heads for `target` from where the spring is now, moving at `velocity`
    /// per second — a flick handing its speed to the spring.
    pub fn launch(&mut self, now: Duration, target: f32, velocity: f32, spring: motion::Spring) {
        self.curve = Curve::from(spring);
        let from = self.value(now);
        self.animation = Animation::new(from, target, velocity, now, self.curve);
    }

    /// Puts the spring at `value`, at rest.
    pub fn snap(&mut self, value: f32) {
        self.animation = Animation::new(value, value, 0.0, Duration::ZERO, self.curve);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECOND: Duration = Duration::from_secs(1);

    #[test]
    fn a_new_spring_is_at_rest_where_it_was_put() {
        let spring = Spring::new(4.0, motion::Spring::smooth());
        assert_eq!(spring.value(Duration::ZERO), 4.0);
        assert!(spring.at_rest(Duration::ZERO));
    }

    #[test]
    fn a_retargeted_spring_moves_continuously_and_settles_on_the_target() {
        let mut spring = Spring::new(0.0, motion::Spring::smooth());
        spring.set_target(Duration::ZERO, 100.0);
        let early = spring.value(Duration::from_millis(16));
        assert!(early > 0.0 && early < 100.0, "jumped to {early}");

        spring.set_target(Duration::from_millis(16), 50.0);
        assert!((spring.value(Duration::from_millis(16)) - early).abs() < 1e-3);
        assert_eq!(spring.value(5 * SECOND), 50.0);
        assert!(spring.at_rest(5 * SECOND));
    }

    #[test]
    fn setting_the_same_target_does_not_restart_the_motion() {
        let mut spring = Spring::new(0.0, motion::Spring::smooth());
        spring.set_target(Duration::ZERO, 1.0);
        let before = spring;
        spring.set_target(Duration::from_millis(100), 1.0);
        assert_eq!(spring, before);
    }
}
