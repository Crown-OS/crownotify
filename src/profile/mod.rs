//! Per-application presentation overrides.
//!
//! A profile is how the daemon knows that a notification from Spotify is a
//! music card even though Spotify never says so, and that it should wear the
//! brand green rather than the system accent. It carries presentation only —
//! nothing here can change whether a notification is *shown*, which is policy
//! and belongs to [`Store`](crate::store::Store).
//!
//! The table is static today and will be merged with the user's
//! [`Config`](crate::config::Config) later. Every lookup therefore goes through
//! [`profile_for`], and nothing else may index the table: when the config
//! becomes a second source, that one function is the only thing that changes,
//! and every card keeps working because it never knew where its profile came
//! from in the first place.

pub mod spotify;
pub mod table;

use crownshell::peniko::Color;

use crate::model::Icon;

/// Which card a profile forces a notification onto.
///
/// Deliberately not [`Content`](crate::model::Content): a profile can say
/// "draw this as music" without being able to invent a track title, so the
/// payload still has to come from the notification itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentHint {
    General,
    Call,
    Chat,
    Music,
}

/// Presentation overrides for one application.
///
/// Every field except `grouped` is optional, and `None` means "use the theme's
/// own value" rather than "no accent" — a profile is a patch on the defaults,
/// not a replacement for them.
#[derive(Debug, Clone, PartialEq)]
pub struct AppProfile {
    /// Replaces [`Palette::accent`](crate::ui::theme::Palette::accent) for this
    /// app's cards.
    pub accent: Option<Color>,
    /// Forces a card kind regardless of what the sender asked for.
    pub content: Option<ContentHint>,
    /// Drawn instead of whatever icon the notification carries. For apps that
    /// send a generic or missing icon and are recognisable without one.
    pub icon_override: Option<Icon>,
    /// Whether the notification center buckets this app's entries together.
    pub grouped: bool,
}

impl AppProfile {
    /// `accent` if the profile sets one, otherwise the theme's.
    ///
    /// Cards call this rather than matching on the option themselves, so that
    /// "an app accent wins over the theme accent" is stated once.
    pub fn accent_or(&self, fallback: Color) -> Color {
        self.accent.unwrap_or(fallback)
    }
}

/// The profile every app gets until one is registered for it: no overrides,
/// grouped in the center like everything else.
static DEFAULT: AppProfile = AppProfile {
    accent: None,
    content: None,
    icon_override: None,
    grouped: true,
};

/// One row of the registry: the keys it answers to, and what they resolve to.
struct Registration {
    keys: &'static [&'static str],
    profile: &'static AppProfile,
}

/// Every app the daemon ships a profile for.
///
/// A linear scan is the right structure at this size — a handful of rows, each
/// compared against one short string — and it keeps the table readable, which
/// matters far more than a lookup that is already far cheaper than the frame it
/// happens on.
static REGISTRY: &[Registration] = &[Registration {
    keys: spotify::KEYS,
    profile: &spotify::PROFILE,
}];

/// The profile for an app key (see
/// [`AppIdentity::key`](crate::model::AppIdentity::key)), falling back to
/// [`default_profile`].
///
/// Keys are compared case-insensitively even though `AppIdentity::key` already
/// lower-cases: this is also called with keys from the config and from tests,
/// and a profile that silently stops applying because of a capital letter is a
/// bug nobody would think to look for.
pub fn profile_for(key: &str) -> &'static AppProfile {
    REGISTRY
        .iter()
        .find(|registration| {
            registration
                .keys
                .iter()
                .any(|candidate| candidate.eq_ignore_ascii_case(key))
        })
        .map_or(&DEFAULT, |registration| registration.profile)
}

/// The profile an app with no registration gets.
pub fn default_profile() -> &'static AppProfile {
    &DEFAULT
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::AppIdentity;

    #[test]
    fn spotify_resolves_to_its_own_profile_under_every_packaging() {
        for key in ["spotify", "com.spotify.client", "spotify_spotify"] {
            let profile = profile_for(key);
            assert_eq!(
                profile.content,
                Some(ContentHint::Music),
                "{key} did not resolve to the Spotify profile"
            );
            assert_eq!(profile.accent, Some(spotify::ACCENT));
            assert!(!profile.grouped);
        }
    }

    #[test]
    fn an_unknown_app_falls_back_to_the_default_profile() {
        let profile = profile_for("org.gnome.evolution");
        assert!(std::ptr::eq(profile, default_profile()));
        assert_eq!(profile.accent, None);
        assert_eq!(profile.content, None);
        assert!(
            profile.grouped,
            "the default has to group, or the center would never bucket anything"
        );
    }

    #[test]
    fn an_empty_key_is_not_an_error_and_gets_the_default() {
        assert!(std::ptr::eq(profile_for(""), default_profile()));
    }

    #[test]
    fn lookup_ignores_case_so_a_capital_never_loses_a_profile() {
        assert_eq!(profile_for("Spotify").accent, Some(spotify::ACCENT));
        assert_eq!(
            profile_for("COM.SPOTIFY.CLIENT").accent,
            Some(spotify::ACCENT)
        );
    }

    /// The lookup is fed by `AppIdentity::key`, so the two have to agree about
    /// what a key looks like.
    #[test]
    fn an_identity_from_the_wire_resolves_through_its_own_key() {
        let identity = AppIdentity {
            name: "Spotify Premium".to_string(),
            desktop_entry: Some("com.spotify.Client".to_string()),
            icon: None,
        };
        assert_eq!(
            profile_for(&identity.key()).content,
            Some(ContentHint::Music)
        );

        let plain = AppIdentity {
            name: "Spotify".to_string(),
            desktop_entry: None,
            icon: None,
        };
        assert_eq!(profile_for(&plain.key()).content, Some(ContentHint::Music));
    }

    #[test]
    fn an_app_accent_wins_over_the_theme_accent_and_nothing_else_does() {
        let theme_accent = Color::from_rgba8(0, 122, 255, 255);
        assert_eq!(
            profile_for("spotify").accent_or(theme_accent),
            spotify::ACCENT
        );
        assert_eq!(default_profile().accent_or(theme_accent), theme_accent);
    }

    /// Two rows claiming the same key would make the winner depend on table
    /// order, which is exactly the kind of thing that survives a merge and then
    /// confuses everyone.
    #[test]
    fn no_two_registrations_claim_the_same_key() {
        let mut seen = Vec::new();
        for registration in REGISTRY {
            for key in registration.keys {
                assert!(!seen.contains(key), "{key} is registered more than once");
                seen.push(key);
            }
        }
    }
}
