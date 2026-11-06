//! An attached window's prefix keys (gui-design F2, D49; handoff D104):
//! Ctrl-] arms the prefix, and the next key is a command for the layout,
//! as in the rusty_mux TUI. The mode is one-shot: it ends on that key,
//! with no timer.
//!
//! Pure: [`is_prefix`] tells the arming key and [`command`] what the next
//! one asks for; the window carries them out.

use winit::keyboard::{Key, KeyCode, ModifiersState, NamedKey, PhysicalKey};

use super::layout::Dir;

/// Ctrl-], typed into the focused block by pressing the prefix twice.
pub(crate) const PREFIX: u8 = 0x1d;

/// What the key after the prefix asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Command {
    /// Split the focused block: `Dir::Vertical` right, `Dir::Horizontal` down.
    Split(Dir),
    /// Zoom the focused block, or unzoom.
    Zoom,
    /// Close the focused block.
    Close,
    /// Focus the neighbour that way (one axis at a time).
    Focus(isize, isize),
    /// Focus the block with this F2 number, from 1.
    Jump(usize),
    /// Detach the whole window; the session keeps running.
    Detach,
    /// Type a Ctrl-] into the focused block.
    Type,
    /// Leave the mode without a command (Esc).
    Cancel,
    /// A modifier alone: the mode waits for the key it modifies.
    Wait,
    /// Any other key: dropped, with a bell.
    Unknown,
}

/// Whether `key` with `mods` is the prefix, Ctrl-].
pub(crate) fn is_prefix(key: &Key, physical: PhysicalKey, mods: ModifiersState) -> bool {
    let bracket = matches!(key, Key::Character(c) if c == "]")
        || physical == PhysicalKey::Code(KeyCode::BracketRight);
    mods.control_key() && !mods.alt_key() && bracket
}

/// What `key`, pressed while the prefix is armed, asks for.
pub(crate) fn command(key: &Key, physical: PhysicalKey, mods: ModifiersState) -> Command {
    if is_prefix(key, physical, mods) {
        return Command::Type;
    }
    match key {
        Key::Named(named) => named_command(*named),
        Key::Character(c) => char_command(c),
        _ => Command::Unknown,
    }
}

fn named_command(key: NamedKey) -> Command {
    match key {
        NamedKey::Escape => Command::Cancel,
        NamedKey::ArrowLeft => Command::Focus(-1, 0),
        NamedKey::ArrowRight => Command::Focus(1, 0),
        NamedKey::ArrowUp => Command::Focus(0, -1),
        NamedKey::ArrowDown => Command::Focus(0, 1),
        NamedKey::Shift
        | NamedKey::Control
        | NamedKey::Alt
        | NamedKey::Super
        | NamedKey::AltGraph
        | NamedKey::Meta => Command::Wait,
        _ => Command::Unknown,
    }
}

fn char_command(c: &str) -> Command {
    match c {
        "|" => Command::Split(Dir::Vertical),
        "-" => Command::Split(Dir::Horizontal),
        "z" => Command::Zoom,
        "x" => Command::Close,
        "d" => Command::Detach,
        "h" => Command::Focus(-1, 0),
        "l" => Command::Focus(1, 0),
        "k" => Command::Focus(0, -1),
        "j" => Command::Focus(0, 1),
        _ => match c.parse::<usize>() {
            Ok(n @ 1..=9) => Command::Jump(n),
            _ => Command::Unknown,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NONE: ModifiersState = ModifiersState::empty();
    const OTHER: PhysicalKey = PhysicalKey::Code(KeyCode::KeyA);

    fn ch(c: &str) -> Command {
        command(&Key::Character(c.into()), OTHER, NONE)
    }

    #[test]
    fn ctrl_bracket_arms_the_prefix() {
        let key = Key::Character("]".into());
        assert!(is_prefix(&key, OTHER, ModifiersState::CONTROL));
        assert!(!is_prefix(&key, OTHER, NONE), "a plain ] is typed");
        let layout = Key::Character("+".into());
        let bracket = PhysicalKey::Code(KeyCode::BracketRight);
        assert!(
            is_prefix(&layout, bracket, ModifiersState::CONTROL),
            "the key where ] is on a US layout"
        );
    }

    #[test]
    fn the_next_key_is_the_tui_command() {
        assert_eq!(ch("|"), Command::Split(Dir::Vertical));
        assert_eq!(ch("-"), Command::Split(Dir::Horizontal));
        assert_eq!(ch("z"), Command::Zoom);
        assert_eq!(ch("x"), Command::Close);
        assert_eq!(ch("d"), Command::Detach);
        assert_eq!(ch("h"), Command::Focus(-1, 0));
        assert_eq!(ch("j"), Command::Focus(0, 1));
        let arrow = Key::Named(NamedKey::ArrowUp);
        assert_eq!(command(&arrow, OTHER, NONE), Command::Focus(0, -1));
    }

    #[test]
    fn digits_one_to_nine_jump() {
        assert_eq!(ch("1"), Command::Jump(1));
        assert_eq!(ch("9"), Command::Jump(9));
        assert_eq!(ch("0"), Command::Unknown);
    }

    #[test]
    fn the_prefix_twice_types_it() {
        let key = Key::Character("]".into());
        assert_eq!(command(&key, OTHER, ModifiersState::CONTROL), Command::Type);
    }

    #[test]
    fn esc_cancels_a_modifier_waits_and_the_rest_are_unknown() {
        let esc = Key::Named(NamedKey::Escape);
        assert_eq!(command(&esc, OTHER, NONE), Command::Cancel);
        let shift = Key::Named(NamedKey::Shift);
        assert_eq!(command(&shift, OTHER, NONE), Command::Wait);
        assert_eq!(ch("q"), Command::Unknown);
        assert_eq!(ch("Z"), Command::Unknown, "the TUI's keys are lower case");
        let enter = Key::Named(NamedKey::Enter);
        assert_eq!(command(&enter, OTHER, NONE), Command::Unknown);
    }
}
