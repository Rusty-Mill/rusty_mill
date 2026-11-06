//! Where a user's own server listens unless told otherwise (C69,
//! `docs/projects/mux/C69-LIFECYCLE.md`). No I/O: the caller passes the
//! environment.
//!
//! A socket is the one a command names, else `$RUSTY_MUX_SOCKET`, else
//! [`default_socket`]: `$XDG_RUNTIME_DIR/rusty_mux/default.sock` on Unix,
//! `%LOCALAPPDATA%\rusty_mux\default.sock` on Windows. With the variable
//! unset or not an absolute path there is no default.

use std::ffi::OsString;
use std::path::PathBuf;

/// The variable naming the socket when a command doesn't.
pub const SOCKET_VAR: &str = "RUSTY_MUX_SOCKET";
/// The directory, under the per-user base, that holds the default socket.
pub const DIR: &str = "rusty_mux";
/// The default socket's file name. Its stem names the server's C45 state
/// directory (`rusty_mux/default`).
pub const NAME: &str = "default.sock";

/// The per-user base directory's variable on this platform.
const BASE_VAR: &str = if cfg!(windows) {
    "LOCALAPPDATA"
} else {
    "XDG_RUNTIME_DIR"
};

/// The default socket, or `None` when the base directory's variable is
/// unset, empty or not absolute.
pub fn default_socket(env: impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    let base = PathBuf::from(env(BASE_VAR)?);
    base.is_absolute().then(|| base.join(DIR).join(NAME))
}

/// `given`, else `$RUSTY_MUX_SOCKET` (if not empty), else the default; the
/// error says what to set when there is none.
pub fn resolve(
    given: Option<PathBuf>,
    env: impl Fn(&str) -> Option<OsString>,
) -> Result<PathBuf, String> {
    if let Some(socket) = given {
        return Ok(socket);
    }
    if let Some(socket) = env(SOCKET_VAR).filter(|v| !v.is_empty()) {
        return Ok(socket.into());
    }
    default_socket(env).ok_or_else(|| {
        format!("no socket: name one, set {SOCKET_VAR}, or set {BASE_VAR} for the default")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env<'a>(vars: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<OsString> + 'a {
        move |name| {
            vars.iter()
                .find(|(k, _)| *k == name)
                .map(|(_, v)| OsString::from(v))
        }
    }

    #[cfg(unix)]
    const BASE: &str = "/run/user/1000";
    #[cfg(windows)]
    const BASE: &str = r"C:\Users\me\AppData\Local";

    #[test]
    fn the_default_is_under_the_per_user_base() {
        let socket = default_socket(env(&[(BASE_VAR, BASE)]));
        assert_eq!(
            socket,
            Some(PathBuf::from(BASE).join("rusty_mux").join("default.sock"))
        );
    }

    #[test]
    fn no_base_or_a_relative_one_gives_no_default() {
        assert_eq!(default_socket(env(&[])), None);
        assert_eq!(default_socket(env(&[(BASE_VAR, "")])), None);
        assert_eq!(default_socket(env(&[(BASE_VAR, "relative")])), None);
    }

    #[test]
    fn a_named_socket_beats_the_variable_and_the_variable_beats_the_default() {
        let vars = [(SOCKET_VAR, "/tmp/var.sock"), (BASE_VAR, BASE)];
        let named = resolve(Some("/tmp/named.sock".into()), env(&vars));
        assert_eq!(named, Ok(PathBuf::from("/tmp/named.sock")));
        assert_eq!(
            resolve(None, env(&vars)),
            Ok(PathBuf::from("/tmp/var.sock"))
        );
        let empty = [(SOCKET_VAR, ""), (BASE_VAR, BASE)];
        assert_eq!(
            resolve(None, env(&empty)),
            default_socket(env(&empty)).ok_or(String::new())
        );
    }

    #[test]
    fn nothing_to_go_on_is_an_error_naming_what_to_set() {
        let err = resolve(None, env(&[])).expect_err("no socket");
        assert!(err.contains(SOCKET_VAR) && err.contains(BASE_VAR), "{err}");
    }
}
