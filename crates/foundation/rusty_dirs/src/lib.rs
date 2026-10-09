//! The per-user config directory, with no dependencies.
//!
//! [`config_root`] is the platform's config base (`%APPDATA%` on Windows,
//! `$XDG_CONFIG_HOME` if set and non-empty, else `~/.config`, elsewhere);
//! [`config_dir`] is an app's folder under it. Both are `None` when the
//! environment names no home, so callers choose their own fallback.
//!
//! State, cache and data directories are not here: apps that need them differ
//! on layout and override variables, so there is no shared rule to hold yet.

use std::ffi::OsString;
use std::path::PathBuf;

/// The platform config base directory.
pub fn config_root() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        root_from(std::env::var_os("APPDATA"), None, None)
    }
    #[cfg(not(windows))]
    {
        root_from(
            None,
            std::env::var_os("XDG_CONFIG_HOME"),
            std::env::var_os("HOME"),
        )
    }
}

/// `name`'s config directory under [`config_root`].
pub fn config_dir(name: &str) -> Option<PathBuf> {
    config_root().map(|root| root.join(name))
}

/// The lookup rule over explicit values, so it can be tested without touching
/// the process environment.
fn root_from(
    appdata: Option<OsString>,
    xdg_config_home: Option<OsString>,
    home: Option<OsString>,
) -> Option<PathBuf> {
    if let Some(appdata) = appdata {
        return Some(PathBuf::from(appdata));
    }
    match xdg_config_home.filter(|x| !x.is_empty()) {
        Some(xdg) => Some(PathBuf::from(xdg)),
        None => home.map(|h| PathBuf::from(h).join(".config")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn os(s: &str) -> Option<OsString> {
        Some(OsString::from(s))
    }

    #[test]
    fn appdata_wins_when_given() {
        assert_eq!(
            root_from(os("C:\\A"), os("/x"), os("/h")),
            Some(PathBuf::from("C:\\A"))
        );
    }

    #[test]
    fn xdg_config_home_beats_home() {
        assert_eq!(
            root_from(None, os("/x"), os("/h")),
            Some(PathBuf::from("/x"))
        );
    }

    #[test]
    fn an_empty_xdg_config_home_is_ignored() {
        assert_eq!(
            root_from(None, os(""), os("/h")),
            Some(PathBuf::from("/h/.config"))
        );
    }

    #[test]
    fn home_gives_dot_config() {
        assert_eq!(
            root_from(None, None, os("/h")),
            Some(PathBuf::from("/h/.config"))
        );
    }

    #[test]
    fn no_environment_gives_none() {
        assert_eq!(root_from(None, None, None), None);
        assert_eq!(root_from(None, os(""), None), None);
    }

    #[test]
    fn config_dir_appends_the_app_name() {
        if let Some(root) = config_root() {
            assert_eq!(config_dir("app"), Some(root.join("app")));
        }
    }
}
