//! The keyboard: bytes go to the focused block, except after the prefix,
//! Ctrl-], whose next key is a command.
//!
//! | After Ctrl-] | Does |
//! |---|---|
//! | `d` | detach |
//! | arrow, or `h` `j` `k` `l` | focus the block that way |
//! | `z` | zoom the focused block, or unzoom |
//! | `\|` or `-` | split the focused block right or down |
//! | `x` | close the focused block |
//! | Ctrl-] | type a Ctrl-] |
//!
//! Any other key is dropped.

/// Ctrl-].
pub const PREFIX: u8 = 0x1d;

/// A direction on the screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dir {
    Left,
    Right,
    Up,
    Down,
}

/// What a key asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Bytes for the focused block.
    Input(Vec<u8>),
    Detach,
    Focus(Dir),
    ToggleZoom,
    /// Split the focused block; `true` side by side, `false` stacked.
    Split {
        beside: bool,
    },
    Close,
    /// A key after the prefix that means nothing.
    Unknown,
}

/// Where the parser is in a prefix command, which may span reads.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum State {
    #[default]
    Typing,
    Prefix,
    /// ESC after the prefix: an arrow key may follow.
    Escape,
    /// `ESC [` after the prefix.
    Csi,
}

/// Turns keyboard bytes into actions.
#[derive(Debug, Default)]
pub struct Keys {
    state: State,
}

impl Keys {
    /// The actions in `bytes`, in order; typed runs are merged into one
    /// [`Action::Input`].
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<Action> {
        let mut actions = Vec::new();
        let mut typed = Vec::new();
        for &b in bytes {
            let action = match self.state {
                State::Typing if b == PREFIX => self.go(State::Prefix, None),
                State::Typing => {
                    typed.push(b);
                    None
                }
                State::Prefix => self.command(b),
                State::Escape if b == b'[' => self.go(State::Csi, None),
                State::Escape => self.go(State::Typing, Some(Action::Unknown)),
                State::Csi => self.go(State::Typing, Some(arrow(b))),
            };
            let Some(action) = action else {
                continue;
            };
            let input = std::mem::take(&mut typed);
            match action {
                Action::Input(more) => typed = [input, more].concat(),
                other => {
                    if !input.is_empty() {
                        actions.push(Action::Input(input));
                    }
                    actions.push(other);
                }
            }
        }
        if !typed.is_empty() {
            actions.push(Action::Input(typed));
        }
        actions
    }

    fn go(&mut self, state: State, action: Option<Action>) -> Option<Action> {
        self.state = state;
        action
    }

    /// The key after the prefix.
    fn command(&mut self, b: u8) -> Option<Action> {
        let action = match b {
            0x1b => return self.go(State::Escape, None),
            PREFIX => Action::Input(vec![PREFIX]),
            b'd' => Action::Detach,
            b'h' => Action::Focus(Dir::Left),
            b'j' => Action::Focus(Dir::Down),
            b'k' => Action::Focus(Dir::Up),
            b'l' => Action::Focus(Dir::Right),
            b'z' => Action::ToggleZoom,
            b'|' => Action::Split { beside: true },
            b'-' => Action::Split { beside: false },
            b'x' => Action::Close,
            _ => Action::Unknown,
        };
        self.go(State::Typing, Some(action))
    }
}

fn arrow(b: u8) -> Action {
    match b {
        b'A' => Action::Focus(Dir::Up),
        b'B' => Action::Focus(Dir::Down),
        b'C' => Action::Focus(Dir::Right),
        b'D' => Action::Focus(Dir::Left),
        _ => Action::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typing_passes_through_in_one_run() {
        let mut keys = Keys::default();
        assert_eq!(keys.feed(b"ls\r"), vec![Action::Input(b"ls\r".to_vec())]);
        assert_eq!(keys.feed(b""), vec![]);
    }

    #[test]
    fn prefix_commands_split_the_typed_runs() {
        let mut keys = Keys::default();
        assert_eq!(
            keys.feed(b"a\x1dzb\x1dd"),
            vec![
                Action::Input(b"a".to_vec()),
                Action::ToggleZoom,
                Action::Input(b"b".to_vec()),
                Action::Detach,
            ]
        );
    }

    #[test]
    fn a_doubled_prefix_types_one() {
        let mut keys = Keys::default();
        assert_eq!(
            keys.feed(b"a\x1d\x1db"),
            vec![Action::Input(b"a\x1db".to_vec())]
        );
    }

    #[test]
    fn a_command_may_span_reads() {
        let mut keys = Keys::default();
        assert_eq!(keys.feed(b"\x1d"), vec![]);
        assert_eq!(keys.feed(b"\x1b"), vec![]);
        assert_eq!(keys.feed(b"["), vec![]);
        assert_eq!(keys.feed(b"C"), vec![Action::Focus(Dir::Right)]);
        assert_eq!(keys.feed(b"\x1d-"), vec![Action::Split { beside: false }]);
    }

    #[test]
    fn every_command_key_maps() {
        let cases = [
            (&b"\x1dh"[..], Action::Focus(Dir::Left)),
            (b"\x1dj", Action::Focus(Dir::Down)),
            (b"\x1dk", Action::Focus(Dir::Up)),
            (b"\x1dl", Action::Focus(Dir::Right)),
            (b"\x1d\x1b[A", Action::Focus(Dir::Up)),
            (b"\x1d\x1b[B", Action::Focus(Dir::Down)),
            (b"\x1d\x1b[D", Action::Focus(Dir::Left)),
            (b"\x1d|", Action::Split { beside: true }),
            (b"\x1dx", Action::Close),
            (b"\x1dq", Action::Unknown),
            (b"\x1d\x1bq", Action::Unknown),
            (b"\x1d\x1b[Z", Action::Unknown),
        ];
        for (bytes, action) in cases {
            assert_eq!(Keys::default().feed(bytes), vec![action], "{bytes:?}");
        }
    }
}
