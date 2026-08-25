//! Sovereign Window handle and event pump.

use crate::event::Event;
#[cfg(any(windows, target_os = "linux"))]
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

// Raw Xlib FFI bindings for the Linux backend — hand-rolled directly against
// `libX11` (no `x11`/`xcb` crate dependency), the same way the Windows
// backend calls raw Win32 functions via `rusty_win32`. Struct layouts and
// constants below are transcribed from `/usr/include/X11/{Xlib,X}.h`.
#[cfg(target_os = "linux")]
mod x11 {
    use core::ffi::{c_char, c_int, c_long, c_uint, c_ulong, c_void};

    pub type Display = c_void;
    pub type XWindow = c_ulong;
    pub type XTime = c_ulong;
    pub type XAtom = c_ulong;
    pub type KeySym = c_ulong;
    pub type CBool = c_int;

    pub const KEY_PRESS_MASK: c_long = 1 << 0;
    pub const KEY_RELEASE_MASK: c_long = 1 << 1;
    pub const BUTTON_PRESS_MASK: c_long = 1 << 2;
    pub const BUTTON_RELEASE_MASK: c_long = 1 << 3;
    pub const POINTER_MOTION_MASK: c_long = 1 << 6;
    pub const EXPOSURE_MASK: c_long = 1 << 15;
    pub const STRUCTURE_NOTIFY_MASK: c_long = 1 << 17;

    pub const KEY_PRESS: c_int = 2;
    pub const KEY_RELEASE: c_int = 3;
    pub const BUTTON_PRESS: c_int = 4;
    pub const BUTTON_RELEASE: c_int = 5;
    pub const MOTION_NOTIFY: c_int = 6;
    pub const EXPOSE: c_int = 12;
    pub const CONFIGURE_NOTIFY: c_int = 22;
    pub const CLIENT_MESSAGE: c_int = 33;

    #[repr(C)]
    #[derive(Clone, Copy)]
    pub struct XKeyEvent {
        pub type_: c_int,
        pub serial: c_ulong,
        pub send_event: CBool,
        pub display: *mut Display,
        pub window: XWindow,
        pub root: XWindow,
        pub subwindow: XWindow,
        pub time: XTime,
        pub x: c_int,
        pub y: c_int,
        pub x_root: c_int,
        pub y_root: c_int,
        pub state: c_uint,
        pub keycode: c_uint,
        pub same_screen: CBool,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    pub struct XButtonEvent {
        pub type_: c_int,
        pub serial: c_ulong,
        pub send_event: CBool,
        pub display: *mut Display,
        pub window: XWindow,
        pub root: XWindow,
        pub subwindow: XWindow,
        pub time: XTime,
        pub x: c_int,
        pub y: c_int,
        pub x_root: c_int,
        pub y_root: c_int,
        pub state: c_uint,
        pub button: c_uint,
        pub same_screen: CBool,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    pub struct XMotionEvent {
        pub type_: c_int,
        pub serial: c_ulong,
        pub send_event: CBool,
        pub display: *mut Display,
        pub window: XWindow,
        pub root: XWindow,
        pub subwindow: XWindow,
        pub time: XTime,
        pub x: c_int,
        pub y: c_int,
        pub x_root: c_int,
        pub y_root: c_int,
        pub state: c_uint,
        pub is_hint: c_char,
        pub same_screen: CBool,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    pub struct XConfigureEvent {
        pub type_: c_int,
        pub serial: c_ulong,
        pub send_event: CBool,
        pub display: *mut Display,
        pub event: XWindow,
        pub window: XWindow,
        pub x: c_int,
        pub y: c_int,
        pub width: c_int,
        pub height: c_int,
        pub border_width: c_int,
        pub above: XWindow,
        pub override_redirect: CBool,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    pub struct XExposeEvent {
        pub type_: c_int,
        pub serial: c_ulong,
        pub send_event: CBool,
        pub display: *mut Display,
        pub window: XWindow,
        pub x: c_int,
        pub y: c_int,
        pub width: c_int,
        pub height: c_int,
        pub count: c_int,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    pub struct XClientMessageEvent {
        pub type_: c_int,
        pub serial: c_ulong,
        pub send_event: CBool,
        pub display: *mut Display,
        pub window: XWindow,
        pub message_type: XAtom,
        pub format: c_int,
        pub data: [c_long; 5],
    }

    // Mirrors Xlib's `XEvent` union field-for-field (plus the full-size `pad`
    // member) so this has identical size/alignment/layout to the real thing.
    #[repr(C)]
    #[derive(Clone, Copy)]
    pub union XEvent {
        pub type_: c_int,
        pub key: XKeyEvent,
        pub button: XButtonEvent,
        pub motion: XMotionEvent,
        pub configure: XConfigureEvent,
        pub expose: XExposeEvent,
        pub client: XClientMessageEvent,
        pub pad: [c_long; 24],
    }

    #[link(name = "X11")]
    unsafe extern "C" {
        pub fn XOpenDisplay(display_name: *const c_char) -> *mut Display;
        pub fn XDefaultScreen(display: *mut Display) -> c_int;
        pub fn XRootWindow(display: *mut Display, screen_number: c_int) -> XWindow;
        pub fn XBlackPixel(display: *mut Display, screen_number: c_int) -> c_ulong;
        pub fn XWhitePixel(display: *mut Display, screen_number: c_int) -> c_ulong;
        #[allow(clippy::too_many_arguments)]
        pub fn XCreateSimpleWindow(
            display: *mut Display,
            parent: XWindow,
            x: c_int,
            y: c_int,
            width: c_uint,
            height: c_uint,
            border_width: c_uint,
            border: c_ulong,
            background: c_ulong,
        ) -> XWindow;
        pub fn XStoreName(display: *mut Display, w: XWindow, window_name: *const c_char) -> c_int;
        pub fn XSelectInput(display: *mut Display, w: XWindow, event_mask: c_long) -> c_int;
        pub fn XInternAtom(
            display: *mut Display,
            atom_name: *const c_char,
            only_if_exists: CBool,
        ) -> XAtom;
        pub fn XSetWMProtocols(
            display: *mut Display,
            w: XWindow,
            protocols: *mut XAtom,
            count: c_int,
        ) -> c_int;
        pub fn XMapWindow(display: *mut Display, w: XWindow) -> c_int;
        pub fn XFlush(display: *mut Display) -> c_int;
        pub fn XPending(display: *mut Display) -> c_int;
        pub fn XNextEvent(display: *mut Display, event_return: *mut XEvent) -> c_int;
        pub fn XLookupKeysym(key_event: *mut XKeyEvent, index: c_int) -> KeySym;
        #[allow(clippy::too_many_arguments)]
        pub fn XLookupString(
            event_struct: *mut XKeyEvent,
            buffer_return: *mut c_char,
            bytes_buffer: c_int,
            keysym_return: *mut KeySym,
            status_in_out: *mut c_void,
        ) -> c_int;
        #[allow(clippy::too_many_arguments)]
        pub fn XClearArea(
            display: *mut Display,
            w: XWindow,
            x: c_int,
            y: c_int,
            width: c_uint,
            height: c_uint,
            exposures: CBool,
        ) -> c_int;
    }
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
    #[cfg(target_os = "linux")]
    display: *mut x11::Display,
    #[cfg(target_os = "linux")]
    x11_window: x11::XWindow,
    #[cfg(target_os = "linux")]
    wm_delete_window: x11::XAtom,
    #[cfg(target_os = "linux")]
    modifiers: ModifiersState,
}

impl Window {
    /// Creates a new Window.
    pub fn new(title: &str, width: u32, height: u32) -> Result<Self, &'static str> {
        #[cfg(windows)]
        let hwnd = unsafe { rusty_win32::windowing::create_native_window(title, width, height) };

        #[cfg(target_os = "linux")]
        let (display, x11_window, wm_delete_window) = unsafe {
            let display = x11::XOpenDisplay(core::ptr::null());
            if display.is_null() {
                return Err("X11: unable to open display (is a Linux desktop session running?)");
            }

            let screen = x11::XDefaultScreen(display);
            let root = x11::XRootWindow(display, screen);
            let black = x11::XBlackPixel(display, screen);
            let white = x11::XWhitePixel(display, screen);
            let win = x11::XCreateSimpleWindow(display, root, 0, 0, width, height, 1, black, white);

            let mut title_cstr = Vec::with_capacity(title.len() + 1);
            title_cstr.extend_from_slice(title.as_bytes());
            title_cstr.push(0);
            x11::XStoreName(
                display,
                win,
                title_cstr.as_ptr() as *const core::ffi::c_char,
            );

            let event_mask = x11::KEY_PRESS_MASK
                | x11::KEY_RELEASE_MASK
                | x11::BUTTON_PRESS_MASK
                | x11::BUTTON_RELEASE_MASK
                | x11::POINTER_MOTION_MASK
                | x11::EXPOSURE_MASK
                | x11::STRUCTURE_NOTIFY_MASK;
            x11::XSelectInput(display, win, event_mask);

            let atom_name = b"WM_DELETE_WINDOW\0";
            let mut wm_delete_window =
                x11::XInternAtom(display, atom_name.as_ptr() as *const core::ffi::c_char, 0);
            x11::XSetWMProtocols(display, win, &mut wm_delete_window, 1);

            x11::XMapWindow(display, win);
            x11::XFlush(display);

            (display, win, wm_delete_window)
        };

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
            display,
            #[cfg(target_os = "linux")]
            x11_window,
            #[cfg(target_os = "linux")]
            wm_delete_window,
            #[cfg(target_os = "linux")]
            modifiers: ModifiersState::default(),
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
        #[cfg(target_os = "linux")]
        {
            self.x11_window as usize as *mut core::ffi::c_void
        }
        #[cfg(not(any(windows, target_os = "linux")))]
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
        #[cfg(target_os = "linux")]
        unsafe {
            while x11::XPending(self.display) > 0 {
                let mut ev: x11::XEvent = core::mem::zeroed();
                x11::XNextEvent(self.display, &mut ev);
                match ev.type_ {
                    x11::CONFIGURE_NOTIFY => {
                        let c = ev.configure;
                        let (w, h) = (c.width as u32, c.height as u32);
                        if w != self.width || h != self.height {
                            self.width = w;
                            self.height = h;
                            events.push(Event::Resized(w, h));
                        }
                    }
                    x11::CLIENT_MESSAGE => {
                        let c = ev.client;
                        if c.format == 32 && c.data[0] as u64 == self.wm_delete_window {
                            events.push(Event::CloseRequested);
                        }
                    }
                    x11::EXPOSE => {
                        if ev.expose.count == 0 {
                            events.push(Event::RedrawRequested);
                        }
                    }
                    x11::MOTION_NOTIFY => {
                        let m = ev.motion;
                        events.push(Event::CursorMoved(m.x as f64, m.y as f64));
                    }
                    x11::BUTTON_PRESS => {
                        let button = ev.button.button;
                        match button {
                            4 => events.push(Event::MouseWheel(1.0)),
                            5 => events.push(Event::MouseWheel(-1.0)),
                            other => {
                                if let Some(btn) = x11_button_to_mouse(other) {
                                    events.push(Event::MousePressed(btn));
                                }
                            }
                        }
                    }
                    x11::BUTTON_RELEASE => {
                        let button = ev.button.button;
                        if button != 4 && button != 5 {
                            if let Some(btn) = x11_button_to_mouse(button) {
                                events.push(Event::MouseReleased(btn));
                            }
                        }
                    }
                    x11::KEY_PRESS | x11::KEY_RELEASE => {
                        let pressed = ev.type_ == x11::KEY_PRESS;
                        let mut key_ev = ev.key;
                        let keysym = x11::XLookupKeysym(&mut key_ev, 0);
                        let key = keysym_to_keycode(keysym);
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

                        if pressed {
                            let mut buf = [0i8; 8];
                            let mut lookup_keysym: x11::KeySym = 0;
                            let n = x11::XLookupString(
                                &mut key_ev,
                                buf.as_mut_ptr(),
                                buf.len() as core::ffi::c_int,
                                &mut lookup_keysym,
                                core::ptr::null_mut(),
                            );
                            // XLookupString only produces Latin-1 text (no IME/full
                            // Unicode composition) — sufficient for basic typed text,
                            // real IME support is tracked as remaining scope.
                            if n > 0 && (buf[0] as u8) >= 0x20 {
                                events.push(Event::ReceivedCharacter(buf[0] as u8 as char));
                            }
                        }
                    }
                    _ => {}
                }
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
        #[cfg(target_os = "linux")]
        unsafe {
            x11::XClearArea(self.display, self.x11_window, 0, 0, 0, 0, 1);
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

/// Maps an X11 keysym to a [`KeyCode`].
#[cfg(target_os = "linux")]
fn keysym_to_keycode(keysym: x11::KeySym) -> KeyCode {
    match keysym {
        0xff08 => KeyCode::Backspace,        // XK_BackSpace
        0xff09 => KeyCode::Tab,              // XK_Tab
        0xff0d => KeyCode::Return,           // XK_Return
        0xff1b => KeyCode::Escape,           // XK_Escape
        0x0020 => KeyCode::Space,            // XK_space
        0xff51 => KeyCode::Left,             // XK_Left
        0xff52 => KeyCode::Up,               // XK_Up
        0xff53 => KeyCode::Right,            // XK_Right
        0xff54 => KeyCode::Down,             // XK_Down
        0xffe1 | 0xffe2 => KeyCode::Shift,   // XK_Shift_L / XK_Shift_R
        0xffe3 | 0xffe4 => KeyCode::Control, // XK_Control_L / XK_Control_R
        0xffe9 | 0xffea => KeyCode::Alt,     // XK_Alt_L / XK_Alt_R
        // Printable ASCII/Latin-1 keysyms are numerically equal to their code
        // point (0x21..=0x7e; 0x20 space is already handled above).
        0x21..=0x7e => KeyCode::Char(keysym as u8 as char),
        other => KeyCode::Unknown(other as u32),
    }
}

/// Maps an X11 button number to a [`MouseButton`] (buttons 4/5 are the
/// scroll wheel, handled separately as [`Event::MouseWheel`]).
#[cfg(target_os = "linux")]
fn x11_button_to_mouse(button: core::ffi::c_uint) -> Option<MouseButton> {
    match button {
        1 => Some(MouseButton::Left),
        2 => Some(MouseButton::Middle),
        3 => Some(MouseButton::Right),
        _ => None,
    }
}

#[cfg(all(test, target_os = "linux"))]
mod x11_tests {
    use super::*;

    #[test]
    fn maps_named_keysyms() {
        assert_eq!(keysym_to_keycode(0xff0d), KeyCode::Return);
        assert_eq!(keysym_to_keycode(0xff1b), KeyCode::Escape);
        assert_eq!(keysym_to_keycode(0xff51), KeyCode::Left);
        assert_eq!(keysym_to_keycode(0xff08), KeyCode::Backspace);
    }

    #[test]
    fn maps_modifier_keysyms() {
        assert_eq!(keysym_to_keycode(0xffe1), KeyCode::Shift);
        assert_eq!(keysym_to_keycode(0xffe2), KeyCode::Shift);
        assert_eq!(keysym_to_keycode(0xffe3), KeyCode::Control);
        assert_eq!(keysym_to_keycode(0xffe9), KeyCode::Alt);
    }

    #[test]
    fn maps_printable_keysyms_to_char() {
        assert_eq!(keysym_to_keycode(0x61), KeyCode::Char('a')); // XK_a
        assert_eq!(keysym_to_keycode(0x7a), KeyCode::Char('z')); // XK_z
        assert_eq!(keysym_to_keycode(0x30), KeyCode::Char('0')); // XK_0
        assert_eq!(keysym_to_keycode(0x20), KeyCode::Space);
    }

    #[test]
    fn unknown_keysym_falls_back() {
        assert_eq!(keysym_to_keycode(0xdead), KeyCode::Unknown(0xdead));
    }

    #[test]
    fn maps_x11_button_numbers() {
        assert_eq!(x11_button_to_mouse(1), Some(MouseButton::Left));
        assert_eq!(x11_button_to_mouse(2), Some(MouseButton::Middle));
        assert_eq!(x11_button_to_mouse(3), Some(MouseButton::Right));
        assert_eq!(x11_button_to_mouse(4), None);
        assert_eq!(x11_button_to_mouse(5), None);
    }
}
