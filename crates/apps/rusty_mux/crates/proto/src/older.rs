//! What a client tells its user when the server lacks features the client
//! asked for (C36 slice (a), `docs/projects/mux/C36-UPGRADE.md`): the
//! server's version, if it sent one, and what the missing features do.

use crate::Features;

/// Features a current server grants only when started a certain way, so
/// their absence says nothing about its version: UDP needs `--udp`.
const CONDITIONAL: Features = Features::DATAGRAM;

/// A server older than this client: what to tell the user, once per
/// connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Older {
    /// The server's version, `None` from a server older than the field.
    pub server: Option<String>,
    /// What the features it lacks do, in bit order.
    pub missing: Vec<&'static str>,
}

impl Older {
    /// The notice for a connection that `asked` for features and was
    /// `granted` some, by a server of version `server`; `None` when it
    /// lacks nothing that matters.
    pub fn check(asked: Features, granted: Features, server: Option<&str>) -> Option<Older> {
        let missing = asked.without(granted).without(CONDITIONAL).names();
        if missing.is_empty() {
            return None;
        }
        Some(Older {
            server: server.map(str::to_owned),
            missing,
        })
    }

    /// One line: "{target} runs an older rusty_mux (0.1.0; this is 0.2.0)",
    /// or an unknown version when the server sent none.
    pub fn title(&self, target: &str) -> String {
        let client = crate::VERSION;
        match &self.server {
            Some(server) => {
                format!("{target} runs an older rusty_mux ({server}; this is {client})")
            }
            None => format!("{target} runs an unknown rusty_mux version (this is {client})"),
        }
    }

    /// "It lacks layout editing, shared focus. Some actions are unavailable."
    pub fn detail(&self) -> String {
        format!(
            "It lacks {}. Some actions are unavailable.",
            self.missing.join(", ")
        )
    }

    /// [`Older::title`] and [`Older::detail`] on one line, for a
    /// terminal.
    pub fn line(&self, target: &str) -> String {
        format!("{}. {}", self.title(target), self.detail())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_server_with_everything_asked_for_says_nothing() {
        let asked = Features::LAYOUT.union(Features::FOCUS);
        assert_eq!(Older::check(asked, asked, Some("0.1.0")), None);
    }

    #[test]
    fn udp_missing_says_nothing_about_age() {
        let asked = Features::LAYOUT.union(Features::DATAGRAM);
        assert_eq!(Older::check(asked, Features::LAYOUT, None), None);
    }

    #[test]
    fn missing_features_are_named_with_the_version() {
        let asked = Features::LAYOUT_EDIT
            .union(Features::FOCUS)
            .union(Features::SYNC);
        let older = Older::check(asked, Features::SYNC, Some("0.0.9")).expect("older");
        assert_eq!(older.missing, ["layout editing", "shared focus"]);
        let line = older.line("the server");
        assert!(line.starts_with("the server runs an older rusty_mux (0.0.9; this is "));
        assert!(
            line.ends_with("It lacks layout editing, shared focus. Some actions are unavailable.")
        );
    }

    #[test]
    fn a_server_without_a_version_is_unknown() {
        let older = Older::check(Features::FOCUS, Features::NONE, None).expect("older");
        assert!(
            older
                .title("s")
                .starts_with("s runs an unknown rusty_mux version")
        );
    }

    #[test]
    fn reserved_bits_are_never_named() {
        assert_eq!(
            Older::check(Features::CHECKPOINT_V4, Features::NONE, None),
            None
        );
    }
}
