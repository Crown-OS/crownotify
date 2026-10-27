//! One focus mode, and the set the daemon chooses between.

use std::time::Duration;

use serde::Deserialize;

use crate::model::InterruptionLevel;

/// What a mode does with a notification it does not admit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Blocked {
    /// Held back, and delivered when the mode ends.
    #[default]
    Queue,
    /// Filed straight into the center, with no toast and no later delivery.
    File,
}

/// One named focus mode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FocusMode {
    /// Lower-cased at construction; this is the name on the wire.
    pub name: String,
    pub minimum_level: InterruptionLevel,
    /// App keys, spelled as [`AppIdentity::key`] spells them.
    ///
    /// [`AppIdentity::key`]: crate::model::AppIdentity::key
    pub allowed_apps: Vec<String>,
    pub blocked: Blocked,
}

impl FocusMode {
    pub fn new(name: impl AsRef<str>, minimum_level: InterruptionLevel) -> Self {
        Self {
            name: name.as_ref().to_lowercase(),
            minimum_level,
            allowed_apps: Vec::new(),
            blocked: Blocked::default(),
        }
    }

    /// Whether this mode lets `key` through whatever its level.
    pub fn allows(&self, key: &str) -> bool {
        self.allowed_apps
            .iter()
            .any(|allowed| allowed.eq_ignore_ascii_case(key))
    }
}

/// Every mode the user defined, plus the built-in Do Not Disturb.
///
/// A `Vec` rather than a map: there are a handful of modes, each compared
/// against one short string, and a stable order is what makes a picker
/// predictable and these tests readable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FocusModes(Vec<FocusMode>);

impl FocusModes {
    /// The mode `set_do_not_disturb` turns on, and the one every client that
    /// knows only the freedesktop-era bool is really asking for.
    pub const DO_NOT_DISTURB: &'static str = "do-not-disturb";

    /// Sorts by name and guarantees the preset is present, so
    /// [`do_not_disturb`](Self::do_not_disturb) cannot fail and no caller has
    /// to handle a missing built-in.
    ///
    /// A mode the user wrote under the preset's name replaces it rather than
    /// joining it, which is how Do Not Disturb is made to queue instead of file.
    pub fn new(mut modes: Vec<FocusMode>) -> Self {
        if !modes.iter().any(|mode| mode.name == Self::DO_NOT_DISTURB) {
            modes.push(Self::preset());
        }
        modes.sort_by(|a, b| a.name.cmp(&b.name));
        Self(modes)
    }

    /// Do Not Disturb as it behaved before modes existed: everything below
    /// `Critical` filed away, nothing held for later.
    fn preset() -> FocusMode {
        FocusMode {
            blocked: Blocked::File,
            ..FocusMode::new(Self::DO_NOT_DISTURB, InterruptionLevel::Critical)
        }
    }

    pub fn position(&self, name: &str) -> Option<usize> {
        self.0
            .iter()
            .position(|mode| mode.name.eq_ignore_ascii_case(name))
    }

    pub fn get(&self, index: usize) -> Option<&FocusMode> {
        self.0.get(index)
    }

    /// Index of the preset. Infallible by construction.
    pub fn do_not_disturb(&self) -> usize {
        self.position(Self::DO_NOT_DISTURB).unwrap_or_default()
    }

    pub fn iter(&self) -> impl ExactSizeIterator<Item = &FocusMode> {
        self.0.iter()
    }
}

impl Default for FocusModes {
    fn default() -> Self {
        Self::new(Vec::new())
    }
}

/// What the user's config says about one application.
///
/// Presentation — an accent, a forced card kind — belongs to
/// [`AppProfile`](crate::profile::AppProfile). This is the half that decides
/// whether a notification is shown at all, which is policy and so reaches the
/// store instead.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AppPolicy {
    /// Replaces whatever level the sender claimed. `muted = true` in config is
    /// spelled here as `Some(Passive)`.
    pub level: Option<InterruptionLevel>,
    /// Beats `StoreConfig::default_timeout` for this app.
    pub timeout: Option<Duration>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_set_has_a_do_not_disturb_whether_or_not_one_was_configured() {
        let modes = FocusModes::default();
        assert!(modes.get(modes.do_not_disturb()).is_some());
        assert_eq!(
            modes.get(modes.do_not_disturb()).map(|mode| mode.blocked),
            Some(Blocked::File)
        );
    }

    #[test]
    fn a_configured_preset_replaces_the_built_in_one_rather_than_joining_it() {
        let modes = FocusModes::new(vec![FocusMode {
            blocked: Blocked::Queue,
            ..FocusMode::new(FocusModes::DO_NOT_DISTURB, InterruptionLevel::TimeSensitive)
        }]);
        assert_eq!(modes.iter().len(), 1);
        let preset = modes
            .get(modes.do_not_disturb())
            .expect("the preset exists");
        assert_eq!(preset.blocked, Blocked::Queue);
        assert_eq!(preset.minimum_level, InterruptionLevel::TimeSensitive);
    }

    #[test]
    fn modes_are_found_by_name_ignoring_case() {
        let modes = FocusModes::new(vec![FocusMode::new("Work", InterruptionLevel::Critical)]);
        assert!(modes.position("work").is_some());
        assert!(modes.position("WORK").is_some());
        assert!(modes.position("sleep").is_none());
    }

    #[test]
    fn allowed_apps_are_matched_by_app_key_ignoring_case() {
        let mode = FocusMode {
            allowed_apps: vec!["org.mozilla.thunderbird".to_string()],
            ..FocusMode::new("work", InterruptionLevel::Critical)
        };
        assert!(mode.allows("org.mozilla.thunderbird"));
        assert!(mode.allows("ORG.Mozilla.Thunderbird"));
        assert!(!mode.allows("spotify"));
    }
}
