//! The XDG base directories, resolved once.
//!
//! Three things in the daemon need them and each would otherwise re-derive the
//! `$XDG_*` / `$HOME` fallback chain for itself: the icon theme walks
//! `icons/`, the config loader reads `crownotify/config.toml`, and the launcher
//! looks for `applications/*.desktop`. The spec's fallbacks are easy to get
//! subtly wrong — an empty variable is not an unset one — so they are written
//! down here and nowhere else.

use std::{env, ffi::OsString, path::PathBuf};

/// What the spec defines when `$XDG_DATA_DIRS` is unset or empty.
const DEFAULT_DATA_DIRS: &str = "/usr/local/share:/usr/share";

/// `$XDG_CONFIG_HOME`, else `$HOME/.config`.
pub fn config_home() -> Option<PathBuf> {
    non_empty("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| home().map(|home| home.join(".config")))
}

/// Every XDG data directory with `suffix` appended, in the order the spec says
/// to search them: the user's own first, then the system's.
///
/// `$HOME/.icons` is included for `icons` only, because it is a legacy location
/// the icon theme spec still requires and nothing else uses.
pub fn data_dirs(suffix: &str) -> Vec<PathBuf> {
    let mut dirs = Vec::new();

    let data_home = non_empty("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| home().map(|home| home.join(".local/share")));
    if let Some(dir) = data_home {
        dirs.push(dir.join(suffix));
    }

    if suffix == "icons"
        && let Some(home) = home()
    {
        dirs.push(home.join(".icons"));
    }

    let configured = env::var("XDG_DATA_DIRS")
        .ok()
        .filter(|dirs| !dirs.is_empty());
    for dir in configured
        .as_deref()
        .unwrap_or(DEFAULT_DATA_DIRS)
        .split(':')
    {
        if !dir.is_empty() {
            dirs.push(PathBuf::from(dir).join(suffix));
        }
    }

    dirs
}

fn home() -> Option<PathBuf> {
    env::var_os("HOME").map(PathBuf::from)
}

/// A variable set to the empty string is not a value; the spec says to treat it
/// as unset and fall back.
fn non_empty(key: &str) -> Option<OsString> {
    env::var_os(key).filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The environment is process-wide, so the cases that set it share one lock
    /// rather than racing each other under the test harness's threads.
    static ENV: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn with_env(vars: &[(&str, Option<&str>)], body: impl FnOnce()) {
        let _guard = ENV.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let saved: Vec<_> = vars
            .iter()
            .map(|(key, _)| (*key, env::var_os(key)))
            .collect();
        for (key, value) in vars {
            match value {
                Some(value) => unsafe { env::set_var(key, value) },
                None => unsafe { env::remove_var(key) },
            }
        }
        body();
        for (key, value) in saved {
            match value {
                Some(value) => unsafe { env::set_var(key, value) },
                None => unsafe { env::remove_var(key) },
            }
        }
    }

    #[test]
    fn the_user_directory_is_searched_before_the_system_ones() {
        with_env(
            &[
                ("XDG_DATA_HOME", Some("/home/ada/.local/share")),
                ("XDG_DATA_DIRS", Some("/usr/share")),
                ("HOME", Some("/home/ada")),
            ],
            || {
                assert_eq!(
                    data_dirs("applications"),
                    vec![
                        PathBuf::from("/home/ada/.local/share/applications"),
                        PathBuf::from("/usr/share/applications"),
                    ]
                );
            },
        );
    }

    #[test]
    fn an_empty_variable_falls_back_as_though_it_were_unset() {
        with_env(
            &[("XDG_CONFIG_HOME", Some("")), ("HOME", Some("/home/ada"))],
            || assert_eq!(config_home(), Some(PathBuf::from("/home/ada/.config"))),
        );
    }

    #[test]
    fn the_system_directories_have_a_default_the_spec_defines() {
        with_env(
            &[
                ("XDG_DATA_HOME", None),
                ("XDG_DATA_DIRS", None),
                ("HOME", None),
            ],
            || {
                assert_eq!(
                    data_dirs("applications"),
                    vec![
                        PathBuf::from("/usr/local/share/applications"),
                        PathBuf::from("/usr/share/applications"),
                    ]
                );
            },
        );
    }

    #[test]
    fn only_the_icon_search_path_carries_the_legacy_home_directory() {
        with_env(
            &[
                ("XDG_DATA_HOME", Some("/data")),
                ("XDG_DATA_DIRS", Some("/usr/share")),
                ("HOME", Some("/home/ada")),
            ],
            || {
                assert!(data_dirs("icons").contains(&PathBuf::from("/home/ada/.icons")));
                assert!(!data_dirs("applications").contains(&PathBuf::from("/home/ada/.icons")));
            },
        );
    }
}
