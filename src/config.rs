//! The user's `config.toml`.
//!
//! Read once at start-up from `$XDG_CONFIG_HOME/crownotify/config.toml`,
//! falling back to `~/.config`. Every field is optional and every failure is
//! survivable: a missing file is the normal case, and a file with a typo in it
//! logs a warning and leaves the defaults in place rather than taking the
//! notification daemon down with it. Losing the user's preferred timeout is an
//! annoyance; losing every notification on the system is not.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    time::Duration,
};

use serde::Deserialize;

use crate::{
    focus::{AppPolicy, Blocked, FocusMode, FocusModes},
    model::InterruptionLevel,
    store::StoreConfig,
};

/// Path of the config file relative to the XDG config directory.
const CONFIG_RELATIVE_PATH: &str = "crownotify/config.toml";

/// Which theme to build the UI with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AppearancePreference {
    /// Follow the system. CrownOS has no appearance daemon to ask yet, so this
    /// resolves to light — see [`AppearancePreference::prefers_dark`].
    #[default]
    Auto,
    Light,
    Dark,
}

impl AppearancePreference {
    /// Whether the dark theme should be built.
    ///
    /// Kept here, rather than as a `From` impl on the UI's `Appearance`, so
    /// that the config module stays free of any dependency on the renderer and
    /// remains testable on its own.
    pub fn prefers_dark(self) -> bool {
        matches!(self, Self::Dark)
    }
}

/// Overrides for one application, keyed by
/// [`AppIdentity::key`](crate::model::AppIdentity::key).
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    /// Record this app's notifications in the center but never toast them.
    /// The old spelling of `level = "passive"`, kept because people have it.
    pub muted: bool,
    /// Replaces whatever level this app's notifications claim.
    pub level: Option<InterruptionLevel>,
    /// Overrides the global default timeout for this app.
    pub timeout_ms: Option<u64>,
    /// Overrides the profile's grouping decision.
    pub grouped: Option<bool>,
    /// Accent colour as `#rrggbb`, parsed by the theme rather than here so a
    /// malformed colour cannot invalidate the whole config.
    pub accent: Option<String>,
}

impl AppConfig {
    /// The half of this the store enforces.
    ///
    /// `muted` maps onto a level rather than staying a rule of its own, so the
    /// store has one per-app knob to consult instead of two that could disagree.
    fn policy(&self) -> AppPolicy {
        AppPolicy {
            level: self
                .level
                .or_else(|| self.muted.then_some(InterruptionLevel::Passive)),
            timeout: self.timeout_ms.map(Duration::from_millis),
        }
    }
}

/// One `[focus.<name>]` table.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct FocusConfig {
    pub minimum_level: InterruptionLevel,
    /// App keys allowed through whatever their level.
    pub allowed_apps: Vec<String>,
    pub blocked: Blocked,
}

impl Default for FocusConfig {
    /// An empty `[focus.x]` is a quiet one: a mode that lets everything through
    /// is not a mode.
    fn default() -> Self {
        Self {
            minimum_level: InterruptionLevel::Critical,
            allowed_apps: Vec::new(),
            blocked: Blocked::default(),
        }
    }
}

impl FocusConfig {
    /// Lower-cases the name and every allowed app, because
    /// [`AppIdentity::key`](crate::model::AppIdentity::key) is lower-cased and
    /// the user will write `Slack`.
    fn to_mode(&self, name: &str) -> FocusMode {
        FocusMode {
            name: name.to_lowercase(),
            minimum_level: self.minimum_level,
            allowed_apps: self.allowed_apps.iter().map(|a| a.to_lowercase()).collect(),
            blocked: self.blocked,
        }
    }
}

/// Everything the user can configure.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct Config {
    pub appearance: AppearancePreference,
    /// Start the daemon with Do Not Disturb already on. Predates named modes;
    /// `focus_mode` outranks it when both are written.
    pub do_not_disturb: bool,
    /// The mode the daemon starts in. Empty or unset means none.
    pub focus_mode: Option<String>,
    /// Named focus modes, each its own `[focus.<name>]` table.
    pub focus: HashMap<String, FocusConfig>,
    pub default_timeout_ms: u64,
    /// How long a critical notification stays up. Unset means it never expires
    /// on its own, which is the recommended setting.
    pub critical_timeout_ms: Option<u64>,
    pub max_toasts: usize,
    pub max_history: usize,
    /// Per-app overrides, keyed by the app's lower-cased desktop entry or name.
    pub apps: HashMap<String, AppConfig>,
}

impl Default for Config {
    fn default() -> Self {
        let store = StoreConfig::default();
        Self {
            appearance: AppearancePreference::default(),
            do_not_disturb: false,
            focus_mode: None,
            focus: HashMap::new(),
            default_timeout_ms: store.default_timeout.as_millis() as u64,
            critical_timeout_ms: store.critical_timeout.map(|d| d.as_millis() as u64),
            max_toasts: store.max_toasts,
            max_history: store.max_history,
            apps: HashMap::new(),
        }
    }
}

impl Config {
    /// Loads the user's config, falling back to the defaults for anything the
    /// file does not say — including the file not existing at all.
    pub fn load() -> Self {
        match Self::path() {
            Some(path) => Self::load_from(&path),
            None => {
                log::debug!("no config directory; using defaults");
                Self::default()
            }
        }
    }

    /// Loads a specific file. Used by [`Config::load`] and by the tests, which
    /// must not depend on the machine they run on.
    pub fn load_from(path: &Path) -> Self {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                log::debug!("no config at {}; using defaults", path.display());
                return Self::default();
            }
            Err(error) => {
                log::warn!("reading {}: {error}; using defaults", path.display());
                return Self::default();
            }
        };

        match toml::from_str(&text) {
            Ok(config) => config,
            Err(error) => {
                log::warn!("{} is malformed: {error}; using defaults", path.display());
                Self::default()
            }
        }
    }

    /// Where the config lives, honouring `XDG_CONFIG_HOME`.
    pub fn path() -> Option<PathBuf> {
        Some(crate::util::xdg::config_home()?.join(CONFIG_RELATIVE_PATH))
    }

    /// The subset of the config the [`Store`](crate::store::Store) enforces.
    pub fn store(&self) -> StoreConfig {
        StoreConfig {
            default_timeout: Duration::from_millis(self.default_timeout_ms),
            critical_timeout: self.critical_timeout_ms.map(Duration::from_millis),
            max_toasts: self.max_toasts,
            max_history: self.max_history,
            focus: FocusModes::new(
                self.focus
                    .iter()
                    .map(|(name, mode)| mode.to_mode(name))
                    .collect(),
            ),
            apps: self
                .apps
                .iter()
                .map(|(key, app)| (key.to_lowercase(), app.policy()))
                .collect(),
        }
    }

    /// The mode the daemon starts in.
    pub fn initial_focus_mode(&self) -> Option<&str> {
        match self.focus_mode.as_deref().filter(|name| !name.is_empty()) {
            Some(name) => Some(name),
            None => self.do_not_disturb.then_some(FocusModes::DO_NOT_DISTURB),
        }
    }

    /// Overrides for an app key, if the user wrote any.
    ///
    /// Lower-cased on the way in because `AppIdentity::key` is, and the user
    /// will write the table header however the app spells its own name.
    pub fn app(&self, key: &str) -> Option<&AppConfig> {
        self.apps
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(key))
            .map(|(_, app)| app)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(text: &str) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("config.toml");
        std::fs::write(&path, text).expect("write config");
        (dir, path)
    }

    #[test]
    fn the_defaults_match_the_stores_own_defaults() {
        let config = Config::default();
        let store = config.store();
        let expected = StoreConfig::default();

        assert_eq!(store.default_timeout, expected.default_timeout);
        assert_eq!(store.critical_timeout, expected.critical_timeout);
        assert_eq!(store.max_toasts, expected.max_toasts);
        assert_eq!(store.max_history, expected.max_history);
    }

    #[test]
    fn a_missing_file_is_not_an_error() {
        let dir = tempfile::tempdir().expect("temp dir");
        let config = Config::load_from(&dir.path().join("nowhere.toml"));
        assert_eq!(config, Config::default());
    }

    #[test]
    fn a_malformed_file_falls_back_to_defaults_instead_of_failing() {
        let (_dir, path) = write("this is not = = toml [[[");
        assert_eq!(
            Config::load_from(&path),
            Config::default(),
            "a typo in the config must not stop notifications working"
        );
    }

    #[test]
    fn a_field_of_the_wrong_type_falls_back_to_defaults() {
        let (_dir, path) = write("max_toasts = \"lots\"\n");
        assert_eq!(Config::load_from(&path), Config::default());
    }

    #[test]
    fn an_empty_file_yields_the_defaults() {
        let (_dir, path) = write("");
        assert_eq!(Config::load_from(&path), Config::default());
    }

    #[test]
    fn every_field_is_optional() {
        let (_dir, path) = write("max_toasts = 2\n");
        let config = Config::load_from(&path);

        assert_eq!(config.max_toasts, 2);
        assert_eq!(
            config.max_history,
            Config::default().max_history,
            "unmentioned fields keep their default"
        );
    }

    #[test]
    fn a_full_config_is_read_into_the_store_settings() {
        let (_dir, path) = write(
            r#"
appearance = "dark"
do_not_disturb = true
default_timeout_ms = 8000
critical_timeout_ms = 30000
max_toasts = 4
max_history = 250
"#,
        );
        let config = Config::load_from(&path);

        assert_eq!(config.appearance, AppearancePreference::Dark);
        assert!(config.do_not_disturb);

        let store = config.store();
        assert_eq!(store.default_timeout, Duration::from_secs(8));
        assert_eq!(store.critical_timeout, Some(Duration::from_secs(30)));
        assert_eq!(store.max_toasts, 4);
        assert_eq!(store.max_history, 250);
    }

    #[test]
    fn an_unset_critical_timeout_means_critical_notifications_never_expire() {
        let (_dir, path) = write("default_timeout_ms = 1000\n");
        assert_eq!(Config::load_from(&path).store().critical_timeout, None);
    }

    #[test]
    fn per_app_overrides_are_keyed_by_app_key() {
        let (_dir, path) = write(
            r##"
[apps.spotify]
muted = true
grouped = false
accent = "#1DB954"

[apps."org.signal.signal"]
timeout_ms = 20000
"##,
        );
        let config = Config::load_from(&path);

        let spotify = config.app("spotify").expect("spotify override");
        assert!(spotify.muted);
        assert_eq!(spotify.grouped, Some(false));
        assert_eq!(spotify.accent.as_deref(), Some("#1DB954"));

        let signal = config.app("org.signal.signal").expect("signal override");
        assert_eq!(signal.timeout_ms, Some(20_000));
        assert!(!signal.muted, "unmentioned overrides stay off");

        assert!(
            config.app("mail").is_none(),
            "apps without a table have none"
        );
    }

    #[test]
    fn appearance_defaults_to_auto_and_only_dark_asks_for_the_dark_theme() {
        assert_eq!(AppearancePreference::default(), AppearancePreference::Auto);
        assert!(!AppearancePreference::Auto.prefers_dark());
        assert!(!AppearancePreference::Light.prefers_dark());
        assert!(AppearancePreference::Dark.prefers_dark());
    }

    #[test]
    fn the_config_path_sits_under_the_xdg_config_directory() {
        let Some(path) = Config::path() else {
            return; // Neither XDG_CONFIG_HOME nor HOME is set; nothing to check.
        };
        assert!(
            path.ends_with(CONFIG_RELATIVE_PATH),
            "expected a path ending in {CONFIG_RELATIVE_PATH}, got {}",
            path.display()
        );
    }

    #[test]
    fn focus_modes_are_read_from_their_own_tables() {
        let (_dir, path) = write(
            r#"
            focus_mode = "work"

            [focus.work]
            minimum_level = "time-sensitive"
            allowed_apps = ["Thunderbird"]
            blocked = "queue"
            "#,
        );
        let config = Config::load_from(&path);
        assert_eq!(config.initial_focus_mode(), Some("work"));

        let store = config.store();
        let index = store.focus.position("work").expect("work was configured");
        let mode = store.focus.get(index).expect("the mode exists");
        assert_eq!(mode.minimum_level, InterruptionLevel::TimeSensitive);
        assert_eq!(mode.blocked, Blocked::Queue);
        assert!(
            mode.allows("thunderbird"),
            "allowed apps are lower-cased to match AppIdentity::key"
        );
    }

    #[test]
    fn an_empty_focus_table_is_a_quiet_one() {
        let (_dir, path) = write(
            "[focus.sleep]
",
        );
        let store = Config::load_from(&path).store();
        let index = store.focus.position("sleep").expect("sleep was configured");
        let mode = store.focus.get(index).expect("the mode exists");
        assert_eq!(
            mode.minimum_level,
            InterruptionLevel::Critical,
            "a mode that lets everything through is not a mode"
        );
    }

    #[test]
    fn the_built_in_do_not_disturb_exists_without_being_written_down() {
        let store = Config::default().store();
        assert!(store.focus.get(store.focus.do_not_disturb()).is_some());
    }

    #[test]
    fn a_configured_do_not_disturb_replaces_the_preset() {
        let (_dir, path) = write(
            "[focus.do-not-disturb]
blocked = \"queue\"
",
        );
        let store = Config::load_from(&path).store();
        let preset = store
            .focus
            .get(store.focus.do_not_disturb())
            .expect("the preset exists");
        assert_eq!(preset.blocked, Blocked::Queue);
    }

    #[test]
    fn the_do_not_disturb_bool_starts_the_daemon_in_the_preset() {
        let (_dir, path) = write(
            "do_not_disturb = true
",
        );
        assert_eq!(
            Config::load_from(&path).initial_focus_mode(),
            Some(FocusModes::DO_NOT_DISTURB)
        );
    }

    #[test]
    fn focus_mode_outranks_the_do_not_disturb_bool() {
        let (_dir, path) = write(
            "do_not_disturb = true
focus_mode = \"sleep\"
[focus.sleep]
",
        );
        assert_eq!(Config::load_from(&path).initial_focus_mode(), Some("sleep"));
    }

    #[test]
    fn per_app_overrides_reach_the_store_now_that_they_are_read() {
        let (_dir, path) = write(
            r#"
            [apps."Com.Spotify.Client"]
            muted = true
            timeout_ms = 20000
            "#,
        );
        let store = Config::load_from(&path).store();
        let policy = store
            .apps
            .get("com.spotify.client")
            .expect("app keys are lower-cased on the way in");
        assert_eq!(policy.level, Some(InterruptionLevel::Passive));
        assert_eq!(policy.timeout, Some(Duration::from_millis(20_000)));
    }

    #[test]
    fn an_explicit_level_outranks_the_muted_shorthand() {
        let (_dir, path) = write(
            "[apps.mail]
muted = true
level = \"time-sensitive\"
",
        );
        let store = Config::load_from(&path).store();
        assert_eq!(
            store.apps.get("mail").and_then(|app| app.level),
            Some(InterruptionLevel::TimeSensitive)
        );
    }

    #[test]
    fn a_malformed_level_falls_back_to_the_defaults_rather_than_failing_the_load() {
        let (_dir, path) = write(
            "[focus.work]
minimum_level = \"very urgent\"
",
        );
        assert_eq!(
            Config::load_from(&path),
            Config::default(),
            "a typo costs the file, never the daemon"
        );
    }
}
