//! Sovereign Window handle and event pump.

use crate::event::Event;
use alloc::string::String;
use alloc::vec::Vec;

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
    #[cfg(target_os = "linux")]
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
        let mut events = Vec::new();
        #[cfg(windows)]
        unsafe {
            let mut msg: rusty_win32::windowing::MSG = core::mem::zeroed();
            while rusty_win32::windowing::PeekMessageW(&mut msg, core::ptr::null_mut(), 0, 0, 1) != 0 {
                match msg.message {
                    0x0010 | 0x0002 => { // WM_CLOSE or WM_DESTROY
                        events.push(Event::CloseRequested);
                    }
                    0x0200 => { // WM_MOUSEMOVE
                        let x = (msg.l_param & 0xFFFF) as f64;
                        let y = ((msg.l_param >> 16) & 0xFFFF) as f64;
                        events.push(Event::CursorMoved(x, y));
                    }
                    0x0201 => { // WM_LBUTTONDOWN
                        events.push(Event::MousePressed(crate::event::MouseButton::Left));
                    }
                    0x0202 => { // WM_LBUTTONUP
                        events.push(Event::MouseReleased(crate::event::MouseButton::Left));
                    }
                    0x0100 => { // WM_KEYDOWN
                        if msg.w_param == 0x1B { // VK_ESCAPE
                            events.push(Event::KeyPressed(crate::event::KeyCode::Escape));
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
    pub fn request_redraw(&self) {}
}
