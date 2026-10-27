//! Spotify's presentation overrides.
//!
//! Spotify posts its now-playing state as an ordinary notification with the
//! track in the summary and the artist in the body, so without a profile it
//! would draw as a plain [`General`](crate::model::Content::General) card with
//! no transport controls at all. Forcing [`ContentHint::Music`] is what turns
//! it into the remote control the user expects, and the brand green is what
//! makes it recognisable at a glance in a stack of otherwise identical cards.

use crownshell::peniko::Color;

use crate::profile::{AppProfile, ContentHint};

/// Spotify green, from the brand guidelines: `#1DB954`.
pub const ACCENT: Color = Color::from_rgba8(0x1D, 0xB9, 0x54, 0xFF);

/// The app keys this profile answers to.
///
/// The desktop entry differs between the official package (`spotify`), the
/// Flatpak (`com.spotify.client`) and the Snap (`spotify_spotify`), and
/// [`AppIdentity::key`] falls back to the display name when an app sends no
/// entry at all — so all four spellings have to be listed or the profile
/// silently stops applying after a repackaging.
///
/// [`AppIdentity::key`]: crate::model::AppIdentity::key
pub const KEYS: &[&str] = &["spotify", "com.spotify.client", "spotify_spotify"];

/// The profile itself.
///
/// `grouped` is false on purpose: every track is a replacement of the same
/// now-playing card, so bucketing them in the notification center would produce
/// a stack of one card with a permanently misleading count.
pub const PROFILE: AppProfile = AppProfile {
    accent: Some(ACCENT),
    content: Some(ContentHint::Music),
    icon_override: None,
    grouped: false,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_accent_is_the_brand_green() {
        assert_eq!(ACCENT, Color::from_rgba8(29, 185, 84, 255));
        assert_eq!(PROFILE.accent, Some(ACCENT));
    }

    #[test]
    fn spotify_is_forced_onto_the_music_card_and_never_grouped() {
        // Read back through the registry rather than off the constant, so this
        // also proves the row is actually wired into the lookup.
        let profile = crate::profile::profile_for("spotify");
        assert_eq!(profile.content, Some(ContentHint::Music));
        assert!(
            !profile.grouped,
            "grouping a card that is replaced in place produces a count of one"
        );
    }

    #[test]
    fn every_registered_key_is_already_lower_case() {
        // `AppIdentity::key` lower-cases what it returns, so an upper-case
        // entry here could never match anything.
        for key in KEYS {
            assert_eq!(
                *key,
                key.to_lowercase(),
                "{key} would never match a lower-cased app key"
            );
        }
    }
}
