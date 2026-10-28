//! The profiles the daemon actually consults, once the user's config has had
//! its say.
//!
//! The static registry knows what an app *is* — that Spotify posts now-playing
//! state, and in which green. The config knows what the user wants done about
//! it. Neither alone is the answer, and a `&'static AppProfile` cannot carry a
//! value that came out of a file, so the merge is owned and built once at
//! start-up.

use std::collections::HashMap;

use crownshell::peniko::Color;

use crate::{
    config::Config,
    profile::{AppProfile, default_profile, profile_for},
};

/// Every app profile, merged.
#[derive(Debug, Default)]
pub struct Profiles {
    /// Only apps the user actually wrote a block for. Everything else resolves
    /// straight to the static registry, so the common case allocates nothing
    /// and this map stays as small as the config is.
    overridden: HashMap<String, AppProfile>,
}

impl Profiles {
    /// Merges `config`'s per-app blocks over the static registry.
    pub fn new(config: &Config) -> Self {
        let overridden = config
            .apps
            .iter()
            .map(|(key, app)| {
                let key = key.to_lowercase();
                let mut profile = profile_for(&key).clone();
                if let Some(grouped) = app.grouped {
                    profile.grouped = grouped;
                }
                if let Some(accent) = app.accent.as_deref().and_then(parse_accent) {
                    profile.accent = Some(accent);
                }
                (key, profile)
            })
            .collect();
        Self { overridden }
    }

    /// The profile for an app key, as
    /// [`AppIdentity::key`](crate::model::AppIdentity::key) spells it.
    pub fn get(&self, key: &str) -> &AppProfile {
        self.overridden.get(key).unwrap_or_else(|| profile_for(key))
    }
}

/// Reads `#rrggbb` (or `#rgb`), returning `None` for anything else.
///
/// A malformed colour costs the accent and nothing else — the config module's
/// rule, and the reason this is parsed here rather than by serde.
fn parse_accent(raw: &str) -> Option<Color> {
    let hex = raw.strip_prefix('#')?;
    let channel = |index: usize, width: usize| {
        let slice = hex.get(index..index + width)?;
        let value = u8::from_str_radix(slice, 16).ok()?;
        Some(if width == 1 { value * 0x11 } else { value })
    };
    let width = match hex.len() {
        3 => 1,
        6 => 2,
        _ => return None,
    };
    Some(Color::from_rgba8(
        channel(0, width)?,
        channel(width, width)?,
        channel(2 * width, width)?,
        u8::MAX,
    ))
}

/// The profile an app with no registration and no config block gets.
pub fn fallback() -> &'static AppProfile {
    default_profile()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AppConfig;

    fn config(key: &str, app: AppConfig) -> Config {
        Config {
            apps: HashMap::from([(key.to_string(), app)]),
            ..Config::default()
        }
    }

    #[test]
    fn an_app_with_no_config_block_keeps_its_registered_profile() {
        let profiles = Profiles::default();
        assert_eq!(profiles.get("spotify"), profile_for("spotify"));
        assert!(profiles.get("spotify").content.is_some(), "still music");
    }

    #[test]
    fn the_config_overrides_grouping_without_losing_the_registered_card_kind() {
        let profiles = Profiles::new(&config(
            "Spotify",
            AppConfig {
                grouped: Some(false),
                ..AppConfig::default()
            },
        ));
        let profile = profiles.get("spotify");
        assert!(!profile.grouped, "the user said not to deck this app");
        assert!(
            profile.content.is_some(),
            "the registry still knows it posts now-playing state"
        );
    }

    #[test]
    fn a_configured_accent_replaces_the_registered_one() {
        let profiles = Profiles::new(&config(
            "spotify",
            AppConfig {
                accent: Some("#ff0000".to_string()),
                ..AppConfig::default()
            },
        ));
        assert_eq!(
            profiles.get("spotify").accent,
            Some(Color::from_rgba8(255, 0, 0, 255))
        );
    }

    #[test]
    fn a_malformed_accent_costs_the_accent_and_nothing_else() {
        let profiles = Profiles::new(&config(
            "spotify",
            AppConfig {
                accent: Some("not a colour".to_string()),
                grouped: Some(false),
                ..AppConfig::default()
            },
        ));
        assert_eq!(
            profiles.get("spotify").accent,
            profile_for("spotify").accent
        );
        assert!(!profiles.get("spotify").grouped, "the rest still applied");
    }

    #[test]
    fn both_hex_widths_are_read() {
        assert_eq!(
            parse_accent("#fff"),
            Some(Color::from_rgba8(255, 255, 255, 255))
        );
        assert_eq!(
            parse_accent("#ffffff"),
            Some(Color::from_rgba8(255, 255, 255, 255))
        );
        assert_eq!(parse_accent("ffffff"), None);
        assert_eq!(parse_accent("#ff"), None);
    }
}
