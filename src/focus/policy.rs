//! The decision itself: level in, disposition out.

use crate::{
    focus::mode::{Blocked, FocusMode},
    model::InterruptionLevel,
};

/// What the daemon does with a notification the moment it arrives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Onto the toast stack.
    Bypass,
    /// Held back, and delivered when the mode ends.
    Queue,
    /// Straight into the center, with no toast.
    File,
}

/// Decides a notification's fate from its level and the app that sent it.
///
/// A trait so the table can be exercised on its own and a store can be driven
/// by a stub, without either having to build the other.
pub trait FocusPolicy {
    fn outcome_for(&self, level: InterruptionLevel, key: &str) -> Outcome;
}

/// No mode in force: everything gets through.
impl FocusPolicy for () {
    fn outcome_for(&self, _level: InterruptionLevel, _key: &str) -> Outcome {
        Outcome::Bypass
    }
}

impl FocusPolicy for FocusMode {
    fn outcome_for(&self, level: InterruptionLevel, key: &str) -> Outcome {
        if level >= self.minimum_level || self.allows(key) {
            return Outcome::Bypass;
        }
        match self.blocked {
            Blocked::Queue => Outcome::Queue,
            Blocked::File => Outcome::File,
        }
    }
}

/// What happens to `level` from `key` under `mode`, with the two rules no mode
/// may override.
///
/// `Critical` always breaks through, or a focus mode would be a way to silence
/// an alarm. `Passive` never toasts, with or without a mode, which is what
/// makes the level worth having and what a muted app now means.
pub fn resolve(mode: Option<&FocusMode>, level: InterruptionLevel, key: &str) -> Outcome {
    match level {
        InterruptionLevel::Critical => Outcome::Bypass,
        InterruptionLevel::Passive => Outcome::File,
        _ => mode.map_or(Outcome::Bypass, |mode| mode.outcome_for(level, key)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::focus::mode::FocusModes;

    fn work() -> FocusMode {
        FocusMode {
            allowed_apps: vec!["thunderbird".to_string()],
            blocked: Blocked::Queue,
            ..FocusMode::new("work", InterruptionLevel::TimeSensitive)
        }
    }

    #[test]
    fn a_mode_admits_anything_at_or_above_its_minimum_level() {
        assert_eq!(
            work().outcome_for(InterruptionLevel::TimeSensitive, "spotify"),
            Outcome::Bypass
        );
        assert_eq!(
            work().outcome_for(InterruptionLevel::Active, "spotify"),
            Outcome::Queue
        );
    }

    #[test]
    fn an_allowed_app_breaks_through_below_the_minimum() {
        assert_eq!(
            work().outcome_for(InterruptionLevel::Active, "thunderbird"),
            Outcome::Bypass
        );
    }

    #[test]
    fn a_blocked_notification_is_queued_or_filed_as_the_mode_says() {
        let filing = FocusMode {
            blocked: Blocked::File,
            ..work()
        };
        assert_eq!(
            filing.outcome_for(InterruptionLevel::Active, "spotify"),
            Outcome::File
        );
    }

    #[test]
    fn nothing_can_suppress_a_critical_notification() {
        let sealed = FocusMode {
            blocked: Blocked::File,
            ..FocusMode::new("sleep", InterruptionLevel::Critical)
        };
        assert_eq!(
            resolve(Some(&sealed), InterruptionLevel::Critical, "anything"),
            Outcome::Bypass
        );
    }

    #[test]
    fn a_passive_notification_never_toasts_with_or_without_a_mode() {
        assert_eq!(
            resolve(None, InterruptionLevel::Passive, "spotify"),
            Outcome::File
        );
        assert_eq!(
            resolve(Some(&work()), InterruptionLevel::Passive, "thunderbird"),
            Outcome::File
        );
    }

    #[test]
    fn with_no_mode_in_force_everything_else_gets_through() {
        assert_eq!(
            resolve(None, InterruptionLevel::Active, "spotify"),
            Outcome::Bypass
        );
    }

    #[test]
    fn the_built_in_preset_files_everything_below_critical() {
        let modes = FocusModes::default();
        let dnd = modes
            .get(modes.do_not_disturb())
            .expect("the preset exists");
        assert_eq!(
            resolve(Some(dnd), InterruptionLevel::TimeSensitive, "spotify"),
            Outcome::File
        );
        assert_eq!(
            resolve(Some(dnd), InterruptionLevel::Critical, "spotify"),
            Outcome::Bypass
        );
    }
}
