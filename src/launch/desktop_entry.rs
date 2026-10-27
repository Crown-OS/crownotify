//! Finding and reading a `.desktop` file.
//!
//! Only the two keys the launcher needs are read. A desktop entry is an INI
//! file with a great deal in it that does not concern starting the program, and
//! parsing the rest would be work in service of nothing.

use std::{fs, path::PathBuf};

use crate::util::xdg;

/// The parts of a desktop entry that say how to start the application.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesktopEntry {
    /// Whether the application can be started by activating its bus name,
    /// which is the way that does not lose the launch to a race with the shell.
    pub dbus_activatable: bool,
    /// The command line, with its field codes already stripped.
    pub exec: Option<String>,
}

impl DesktopEntry {
    /// Reads `<id>.desktop` from the XDG application directories.
    ///
    /// `None` when no such file exists or it cannot be read — an app that has
    /// been uninstalled since it notified is not an error, it is a no-op.
    pub fn find(id: &str) -> Option<Self> {
        let file = format!("{}.desktop", id.trim_end_matches(".desktop"));
        let path = xdg::data_dirs("applications")
            .into_iter()
            .map(|dir| dir.join(&file))
            .find(|path| path.is_file())?;
        Self::parse(&fs::read_to_string(path).ok()?)
    }

    /// Where `id` would be found, for diagnostics and tests.
    pub fn search_paths(id: &str) -> Vec<PathBuf> {
        let file = format!("{}.desktop", id.trim_end_matches(".desktop"));
        xdg::data_dirs("applications")
            .into_iter()
            .map(|dir| dir.join(&file))
            .collect()
    }

    /// Reads the `[Desktop Entry]` group, ignoring every other one.
    ///
    /// The groups that follow it are per-action overrides, and taking an `Exec`
    /// from one of those would start the wrong thing.
    pub fn parse(contents: &str) -> Option<Self> {
        let mut entry = Self {
            dbus_activatable: false,
            exec: None,
        };
        let mut inside = false;
        for line in contents.lines().map(str::trim) {
            if let Some(group) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
                if inside {
                    break;
                }
                inside = group == "Desktop Entry";
                continue;
            }
            if !inside {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            match key.trim() {
                "DBusActivatable" => entry.dbus_activatable = value.trim() == "true",
                "Exec" => entry.exec = Some(strip_field_codes(value.trim())),
                _ => {}
            }
        }
        (entry.dbus_activatable || entry.exec.is_some()).then_some(entry)
    }

    /// The object path a D-Bus activatable entry is reached at: its id with the
    /// dots turned into slashes.
    pub fn object_path(id: &str) -> String {
        let id = id.trim_end_matches(".desktop");
        format!("/{}", id.replace('.', "/").replace('-', "_"))
    }
}

/// Removes the `%`-codes the spec defines, which expand to files and URLs the
/// daemon has none of.
///
/// `%%` is a literal percent and survives. An unknown code is dropped rather
/// than kept, because passing `%w` to a program as an argument is worse than
/// passing nothing.
fn strip_field_codes(exec: &str) -> String {
    let mut out = String::with_capacity(exec.len());
    let mut chars = exec.chars();
    while let Some(character) = chars.next() {
        if character != '%' {
            out.push(character);
            continue;
        }
        match chars.next() {
            Some('%') => out.push('%'),
            Some(_) | None => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_desktop_entry_group_is_read_and_later_groups_are_not() {
        let entry = DesktopEntry::parse(
            "[Desktop Entry]\nName=Mail\nExec=thunderbird %u\nDBusActivatable=true\n\
             \n[Desktop Action Compose]\nExec=thunderbird -compose\n",
        )
        .expect("a readable entry");
        assert!(entry.dbus_activatable);
        assert_eq!(
            entry.exec.as_deref(),
            Some("thunderbird"),
            "the action group's Exec must not win"
        );
    }

    #[test]
    fn field_codes_are_stripped_and_a_literal_percent_survives() {
        assert_eq!(strip_field_codes("app %f %u %i %c %k"), "app");
        assert_eq!(strip_field_codes("app --flag %U file"), "app --flag file");
        assert_eq!(strip_field_codes("app 100%%"), "app 100%");
        assert_eq!(strip_field_codes("app"), "app");
    }

    #[test]
    fn an_entry_with_neither_key_is_not_worth_having() {
        assert_eq!(DesktopEntry::parse("[Desktop Entry]\nName=Mail\n"), None);
        assert_eq!(DesktopEntry::parse(""), None);
    }

    #[test]
    fn a_key_outside_the_desktop_entry_group_is_ignored() {
        assert_eq!(
            DesktopEntry::parse("[Other]\nExec=wrong\n"),
            None,
            "only the Desktop Entry group says how to start the app"
        );
    }

    #[test]
    fn the_object_path_is_the_id_with_its_dots_turned_into_slashes() {
        assert_eq!(
            DesktopEntry::object_path("org.gnome.Nautilus"),
            "/org/gnome/Nautilus"
        );
        assert_eq!(
            DesktopEntry::object_path("org.gnome.Nautilus.desktop"),
            "/org/gnome/Nautilus"
        );
        // A hyphen is not valid in a D-Bus path element.
        assert_eq!(
            DesktopEntry::object_path("com.my-app.Thing"),
            "/com/my_app/Thing"
        );
    }

    #[test]
    fn an_id_is_looked_for_with_and_without_its_suffix() {
        let bare = DesktopEntry::search_paths("org.gnome.Nautilus");
        let suffixed = DesktopEntry::search_paths("org.gnome.Nautilus.desktop");
        assert_eq!(bare, suffixed);
        assert!(
            bare.iter()
                .all(|path| path.ends_with("org.gnome.Nautilus.desktop")),
            "{bare:?}"
        );
    }
}
