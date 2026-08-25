//! Sovereign Window handle and event pump.

use crate::event::Event;
#[cfg(windows)]
use crate::event::{KeyCode, ModifiersState, MouseButton};
use alloc::string::String;
use alloc::vec::Vec;

// Extra Win32 `user32` bindings not exposed by `rusty_win32::windowing`.
#[cfg(windows)]
#[link(name = "user32")]
unsafe extern "system" {
    fn InvalidateRect(
        hwnd: *mut core::ffi::c_void,
        lprect: *const core::ffi::c_void,
        berase: i32,
    ) -> i32;
}

/// Builder for configuring OS Window properties.
pub struct WindowBuilder {
    title: String,
    width: u32,
    height: u32,
}

impl WindowBuilder {
    /// Creates a new WindowBuilder with defaults.
    pub fn new() -> Self {
        Self {
            title: String::from("Rusty Mill Window"),
            width: 800,
            height: 600,
        }
    }

    /// Sets window title.
    pub fn with_title(mut self, title: &str) -> Self {
        self.title = String::from(title);
        self
    }

    /// Sets inner dimensions.
    pub fn with_inner_size(mut self, width: u32, height: u32) -> Self {
        self.width = width;
        self.height = height;
        self
    }

    /// Builds and creates the OS window.
    pub fn build(self) -> Result<Window, &'static str> {
        Window::new(&self.title, self.width, self.height)
    }
}

impl Default for WindowBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// A handle to a sovereign OS Window.
pub struct Window {
    title: String,
    width: u32,
    height: u32,
    #[cfg(windows)]
    hwnd: *mut core::ffi::c_void,
    #[cfg(windows)]
    modifiers: ModifiersState,
    #[cfg(windows)]
    pending_surrogate: Option<u16>,
    // Placeholder for the real X11/Wayland backend (issue #4) — not yet
    // wired into `Window::new`/`poll_events`.
    #[cfg(target_os = "linux")]
    #[allow(dead_code)]
    x11_window: u64,
}

impl Window {
    /// Creates a new Window.
    pub fn new(title: &str, width: u32, height: u32) -> Result<Self, &'static str> {
        #[cfg(windows)]
        let hwnd = unsafe { rusty_win32::windowing::create_native_window(title, width, height) };

        Ok(Self {
            title: String::from(title),
            width,
            height,
            #[cfg(windows)]
            hwnd,
            #[cfg(windows)]
            modifiers: ModifiersState::default(),
            #[cfg(windows)]
            pending_surrogate: None,
            #[cfg(target_os = "linux")]
            x11_window: 0,
        })
    }

    /// Returns the window width.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Returns the window height.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// Returns the window title.
    pub fn title(&self) -> &str {
        &self.title
    }

    /// Returns the underlying native OS window handle pointer.
    pub fn raw_handle(&self) -> *mut core::ffi::c_void {
        #[cfg(windows)]
        {
            self.hwnd
        }
        #[cfg(not(windows))]
        {
            core::ptr::null_mut()
        }
    }

    /// Polls pending OS events from the window event queue.
    pub fn poll_events(&mut self) -> Vec<Event> {
        // `mut` is only exercised by the `#[cfg(windows)]` backend below —
        // other platforms don't populate `events` yet (see issues #3/#4).
        #[allow(unused_mut)]
        let mut events = Vec::new();
        #[cfg(windows)]
        unsafe {
            let mut msg: rusty_win32::windowing::MSG = core::mem::zeroed();
            while rusty_win32::windowing::PeekMessageW(&mut msg, core::ptr::null_mut(), 0, 0, 1)
                != 0
            {
                match msg.message {
                    0x0010 | 0x0002 => {
                        // WM_CLOSE or WM_DESTROY
                        events.push(Event::CloseRequested);
                    }
                    0x0005 => {
                        // WM_SIZE: low/high words of l_param are the new client width/height.
                        let width = (msg.l_param as usize & 0xFFFF) as u32;
                        let height = ((msg.l_param as usize >> 16) & 0xFFFF) as u32;
                        self.width = width;
                        self.height = height;
                        events.push(Event::Resized(width, height));
                    }
                    0x000F => {
                        // WM_PAINT: report the request; DispatchMessageW below still lets
                        // DefWindowProcW validate the region via Begin/EndPaint as usual.
                        events.push(Event::RedrawRequested);
                    }
                    0x0200 => {
                        // WM_MOUSEMOVE
                        let x = (msg.l_param & 0xFFFF) as f64;
                        let y = ((msg.l_param >> 16) & 0xFFFF) as f64;
                        events.push(Event::CursorMoved(x, y));
                    }
                    0x0201 => {
                        // WM_LBUTTONDOWN
                        events.push(Event::MousePressed(MouseButton::Left));
                    }
                    0x0202 => {
                        // WM_LBUTTONUP
                        events.push(Event::MouseReleased(MouseButton::Left));
                    }
                    0x0204 => {
                        // WM_RBUTTONDOWN
                        events.push(Event::MousePressed(MouseButton::Right));
                    }
                    0x0205 => {
                        // WM_RBUTTONUP
                        events.push(Event::MouseReleased(MouseButton::Right));
                    }
                    0x0207 => {
                        // WM_MBUTTONDOWN
                        events.push(Event::MousePressed(MouseButton::Middle));
                    }
                    0x0208 => {
                        // WM_MBUTTONUP
                        events.push(Event::MouseReleased(MouseButton::Middle));
                    }
                    0x020A => {
                        // WM_MOUSEWHEEL: signed wheel delta lives in the high word of
                        // w_param, in multiples of WHEEL_DELTA (120).
                        let raw = ((msg.w_param >> 16) & 0xFFFF) as u16 as i16;
                        events.push(Event::MouseWheel(raw as f64 / 120.0));
                    }
                    0x0100 | 0x0101 => {
                        // WM_KEYDOWN / WM_KEYUP
                        let pressed = msg.message == 0x0100;
                        let key = vk_to_keycode(msg.w_param);
                        if pressed {
                            events.push(Event::KeyPressed(key));
                        } else {
                            events.push(Event::KeyReleased(key));
                        }

                        let mut modifiers = self.modifiers;
                        match key {
                            KeyCode::Shift => modifiers.shift = pressed,
                            KeyCode::Control => modifiers.ctrl = pressed,
                            KeyCode::Alt => modifiers.alt = pressed,
                            _ => {}
                        }
                        if modifiers != self.modifiers {
                            self.modifiers = modifiers;
                            events.push(Event::ModifiersChanged(modifiers));
                        }
                    }
                    0x0102 => {
                        // WM_CHAR: a UTF-16 code unit, possibly one half of a surrogate
                        // pair for a character outside the Basic Multilingual Plane.
                        let unit = msg.w_param as u16;
                        if let Some(high) = self.pending_surrogate.take() {
                            if (0xDC00..=0xDFFF).contains(&unit) {
                                let scalar = 0x10000
                                    + (((high as u32) - 0xD800) << 10)
                                    + ((unit as u32) - 0xDC00);
                                if let Some(ch) = char::from_u32(scalar) {
                                    events.push(Event::ReceivedCharacter(ch));
                                }
                            }
                        } else if (0xD800..=0xDBFF).contains(&unit) {
                            self.pending_surrogate = Some(unit);
                        } else if unit >= 0x20 {
                            // Below 0x20 are ASCII control characters (Return, Backspace,
                            // Tab, Escape, ...) already reported via KeyPressed above.
                            if let Some(ch) = char::from_u32(unit as u32) {
                                events.push(Event::ReceivedCharacter(ch));
                            }
                        }
                    }
                    _ => {}
                }
                rusty_win32::windowing::TranslateMessage(&msg);
                rusty_win32::windowing::DispatchMessageW(&msg);
            }
        }
        events
    }

    /// Requests a redraw of the window contents.
    pub fn request_redraw(&self) {
        #[cfg(windows)]
        unsafe {
            InvalidateRect(self.hwnd, core::ptr::null(), 1);
        }
    }
}

/// Maps a Win32 virtual-key code to a [`KeyCode`].
#[cfg(windows)]
fn vk_to_keycode(vk: usize) -> KeyCode {
    match vk {
        0x08 => KeyCode::Backspace,
        0x09 => KeyCode::Tab,
        0x0D => KeyCode::Return,
        0x10 => KeyCode::Shift,
        0x11 => KeyCode::Control,
        0x12 => KeyCode::Alt,
        0x1B => KeyCode::Escape,
        0x20 => KeyCode::Space,
        0x25 => KeyCode::Left,
        0x26 => KeyCode::Up,
        0x27 => KeyCode::Right,
        0x28 => KeyCode::Down,
        0x30..=0x39 => KeyCode::Char((b'0' + (vk - 0x30) as u8) as char),
        0x41..=0x5A => KeyCode::Char((b'a' + (vk - 0x41) as u8) as char),
        other => KeyCode::Unknown(other as u32),
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn maps_named_keys() {
        assert_eq!(vk_to_keycode(0x0D), KeyCode::Return);
        assert_eq!(vk_to_keycode(0x1B), KeyCode::Escape);
        assert_eq!(vk_to_keycode(0x25), KeyCode::Left);
        assert_eq!(vk_to_keycode(0x08), KeyCode::Backspace);
    }

    #[test]
    fn maps_modifiers() {
        assert_eq!(vk_to_keycode(0x10), KeyCode::Shift);
        assert_eq!(vk_to_keycode(0x11), KeyCode::Control);
        assert_eq!(vk_to_keycode(0x12), KeyCode::Alt);
    }

    #[test]
    fn maps_letters_and_digits_to_char() {
        assert_eq!(vk_to_keycode(0x41), KeyCode::Char('a'));
        assert_eq!(vk_to_keycode(0x5A), KeyCode::Char('z'));
        assert_eq!(vk_to_keycode(0x30), KeyCode::Char('0'));
        assert_eq!(vk_to_keycode(0x39), KeyCode::Char('9'));
    }

    #[test]
    fn unknown_vk_falls_back() {
        assert_eq!(vk_to_keycode(0xFF), KeyCode::Unknown(0xFF));
    }
}
