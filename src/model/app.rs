//! Who sent the notification.

use std::borrow::Cow;

use crate::model::icon::Icon;

const DESKTOP_SUFFIX: &str = ".desktop";

/// The sending application, as far as the notification records it.
///
/// `name` is a display string the app chose and may change between releases;
/// `desktop_entry` is the stable identifier, when the app bothered to send the
/// hint. Grouping and profile lookup prefer the latter for exactly that
/// reason.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AppIdentity {
    pub name: String,
    pub desktop_entry: Option<String>,
    pub icon: Option<Icon>,
}

impl AppIdentity {
    /// Lower-cased key used for grouping and profile lookup: the desktop
    /// entry when present, otherwise the app name.
    ///
    /// Almost every app already sends a lower-case identifier, so the common
    /// case borrows and the center can bucket a full history without
    /// allocating a string per entry.
    pub fn key(&self) -> Cow<'_, str> {
        let raw = self.desktop_entry.as_deref().unwrap_or(&self.name);
        if raw.chars().any(char::is_uppercase) {
            Cow::Owned(raw.to_lowercase())
        } else {
            Cow::Borrowed(raw)
        }
    }

    /// The app id the compositor knows the sender by: its desktop entry
    /// without the `.desktop` suffix some senders leave on the hint.
    pub fn app_id(&self) -> Option<&str> {
        let entry = self.desktop_entry.as_deref()?;
        Some(entry.strip_suffix(DESKTOP_SUFFIX).unwrap_or(entry))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(name: &str, desktop_entry: Option<&str>) -> AppIdentity {
        AppIdentity {
            name: name.to_string(),
            desktop_entry: desktop_entry.map(str::to_string),
            icon: None,
        }
    }

    #[test]
    fn the_desktop_entry_wins_over_the_display_name() {
        let identity = app("Spotify Premium", Some("spotify"));
        assert_eq!(identity.key(), "spotify");
    }

    #[test]
    fn the_display_name_is_the_key_when_there_is_no_desktop_entry() {
        assert_eq!(app("dunstify", None).key(), "dunstify");
    }

    #[test]
    fn the_app_id_drops_a_trailing_desktop_suffix() {
        assert_eq!(
            app("Files", Some("org.gnome.Nautilus.desktop")).app_id(),
            Some("org.gnome.Nautilus")
        );
        assert_eq!(
            app("Files", Some("org.gnome.Nautilus")).app_id(),
            Some("org.gnome.Nautilus")
        );
        assert_eq!(app("dunstify", None).app_id(), None);
    }

    #[test]
    fn keys_are_lower_cased() {
        assert_eq!(app("Mail", None).key(), "mail");
        assert_eq!(
            app("Signal", Some("org.signal.Signal")).key(),
            "org.signal.signal"
        );
    }

    #[test]
    fn an_already_lower_case_key_is_borrowed_rather_than_allocated() {
        let identity = app("spotify", None);
        assert!(
            matches!(identity.key(), Cow::Borrowed(_)),
            "expected a borrow, got {:?}",
            identity.key()
        );

        let mixed = app("Spotify", None);
        assert!(
            matches!(mixed.key(), Cow::Owned(_)),
            "lower-casing has to allocate"
        );
    }

    #[test]
    fn an_empty_identity_has_an_empty_key() {
        assert_eq!(AppIdentity::default().key(), "");
    }
}
