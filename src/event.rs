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
    /// Unknown / unmapped key.
    Unknown(u32),
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
    /// Redraw event requested for window.
    RedrawRequested,
}
