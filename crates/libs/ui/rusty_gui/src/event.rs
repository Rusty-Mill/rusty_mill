//! Input events and keycodes for rusty_gui.

/// Keyboard key identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KeyCode {
    /// Character key input.
    Char(char),
    /// Escape key.
    Escape,
    /// Return / Enter key.
    Return,
    /// Backspace key.
    Backspace,
    /// Tab key.
    Tab,
    /// Space key.
    Space,
    /// Up arrow.
    Up,
    /// Down arrow.
    Down,
    /// Left arrow.
    Left,
    /// Right arrow.
    Right,
    /// Shift key (either side).
    Shift,
    /// Control key (either side).
    Control,
    /// Alt key (either side).
    Alt,
    /// Unknown / unmapped key.
    Unknown(u32),
}

/// Snapshot of which modifier keys are currently held down.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ModifiersState {
    /// Either Shift key is held.
    pub shift: bool,
    /// Either Control key is held.
    pub ctrl: bool,
    /// Either Alt key is held.
    pub alt: bool,
}

/// Mouse button identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MouseButton {
    /// Left mouse button.
    Left,
    /// Right mouse button.
    Right,
    /// Middle mouse button.
    Middle,
}

/// Sovereign GUI input event.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// Window closed by user.
    CloseRequested,
    /// Window resized to new width and height.
    Resized(u32, u32),
    /// Key pressed down.
    KeyPressed(KeyCode),
    /// Key released.
    KeyReleased(KeyCode),
    /// Mouse moved to new (x, y) coordinates.
    CursorMoved(f64, f64),
    /// Mouse button pressed.
    MousePressed(MouseButton),
    /// Mouse button released.
    MouseReleased(MouseButton),
    /// Mouse wheel scrolled; positive is up/away from the user.
    MouseWheel(f64),
    /// A Unicode character was typed, reflecting the active keyboard layout
    /// and modifier state (as opposed to the raw, layout-independent
    /// [`KeyCode`] carried by [`Event::KeyPressed`]).
    ReceivedCharacter(char),
    /// The held-modifier-keys state changed.
    ModifiersChanged(ModifiersState),
    /// Redraw event requested for window.
    RedrawRequested,
}
