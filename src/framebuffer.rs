//! Software Framebuffer memory buffer and presentation target.

use crate::color::Color;
use alloc::vec;
use alloc::vec::Vec;

// Raw Xlib FFI bindings for the Linux presentation path — hand-rolled
// directly against `libX11` (no `x11`/`xcb` crate dependency), the same way
// `rusty_gui`'s own Linux window backend and this crate's Windows blit path
// (via `rusty_win32`) call their platform APIs raw. Opens its own `Display`
// connection rather than reusing `rusty_gui::Window`'s private one (which
// isn't exposed) — a second connection to the same X server drawing onto a
// window it doesn't own is a standard, supported X11 pattern (the same
// thing tools like `xdotool`/`import` do).
#[cfg(target_os = "linux")]
mod x11_present {
    use core::ffi::{c_char, c_int, c_uint, c_ulong, c_void};

    type Display = c_void;
    type Visual = c_void;
    type XImage = c_void;
    type Gc = *mut c_void;

    const ZPIXMAP: c_int = 2;

    #[link(name = "X11")]
    unsafe extern "C" {
        fn XOpenDisplay(display_name: *const c_char) -> *mut Display;
        fn XCloseDisplay(display: *mut Display) -> c_int;
        fn XDefaultScreen(display: *mut Display) -> c_int;
        fn XDefaultVisual(display: *mut Display, screen_number: c_int) -> *mut Visual;
        fn XDefaultDepth(display: *mut Display, screen_number: c_int) -> c_int;
        fn XDefaultGC(display: *mut Display, screen_number: c_int) -> Gc;
        #[allow(clippy::too_many_arguments)]
        fn XCreateImage(
            display: *mut Display,
            visual: *mut Visual,
            depth: c_uint,
            format: c_int,
            offset: c_int,
            data: *mut c_char,
            width: c_uint,
            height: c_uint,
            bitmap_pad: c_int,
            bytes_per_line: c_int,
        ) -> *mut XImage;
        #[allow(clippy::too_many_arguments)]
        fn XPutImage(
            display: *mut Display,
            drawable: c_ulong,
            gc: Gc,
            image: *mut XImage,
            src_x: c_int,
            src_y: c_int,
            dest_x: c_int,
            dest_y: c_int,
            width: c_uint,
            height: c_uint,
        ) -> c_int;
        fn XFree(data: *mut c_void) -> c_int;
        fn XFlush(display: *mut Display) -> c_int;
    }

    /// Blits `pixels` (row-major ARGB `u32`s, `width * height` long) onto
    /// the X11 window identified by `raw_handle` (as returned by
    /// [`rusty_gui::Window::raw_handle`]). A no-op if `raw_handle` is null,
    /// either size is zero, or no X display could be opened (headless, no
    /// `DISPLAY`) — same silent-on-failure contract `blit_pixel_buffer`
    /// already has on the Windows side for a null `HWND`/`HDC`.
    ///
    /// Opens and closes its own `Display` connection per call — matching
    /// `rusty_win32::windowing::blit_pixel_buffer`'s own per-call
    /// `GetDC`/`ReleaseDC` pattern, at the cost of a real protocol
    /// handshake each time (unlike `GetDC`, genuinely not free). Kept this
    /// simple deliberately for the first cut; caching the connection is a
    /// documented follow-up if this ever sits on a hot per-frame path.
    ///
    /// Known limitation, stated plainly: assumes the display's default
    /// visual is a standard 24/32-bit-depth TrueColor visual in the host's
    /// native byte order (true for essentially every modern Linux desktop)
    /// — an exotic visual (8-bit paletted, byte-swapped) will blit with
    /// wrong/garbled colors rather than being detected and rejected.
    pub unsafe fn blit(raw_handle: *mut c_void, width: usize, height: usize, pixels: &[u32]) {
        if raw_handle.is_null() || width == 0 || height == 0 {
            return;
        }
        let drawable = raw_handle as usize as c_ulong;

        unsafe {
            let display = XOpenDisplay(core::ptr::null());
            if display.is_null() {
                return;
            }

            let screen = XDefaultScreen(display);
            let visual = XDefaultVisual(display, screen);
            let depth = XDefaultDepth(display, screen);
            let gc = XDefaultGC(display, screen);

            let image = XCreateImage(
                display,
                visual,
                depth as c_uint,
                ZPIXMAP,
                0,
                pixels.as_ptr() as *mut c_char,
                width as c_uint,
                height as c_uint,
                32,
                (width * 4) as c_int,
            );
            if image.is_null() {
                XCloseDisplay(display);
                return;
            }

            XPutImage(
                display,
                drawable,
                gc,
                image,
                0,
                0,
                0,
                0,
                width as c_uint,
                height as c_uint,
            );
            XFlush(display);

            // Free only the `XImage` header Xlib allocated for us — NOT
            // `XDestroyImage`, which would also try to `free()` `data`;
            // `data` points at `pixels`, which Rust owns, not libX11.
            XFree(image);
            XCloseDisplay(display);
        }
    }
}

/// A 2D pixel memory framebuffer.
pub struct Framebuffer {
    width: usize,
    height: usize,
    buffer: Vec<u32>,
}

impl Framebuffer {
    /// Creates a new Framebuffer with width and height.
    pub fn new(width: usize, height: usize) -> Self {
        Self {
            width,
            height,
            buffer: vec![0u32; width * height],
        }
    }

    /// Returns width in pixels.
    pub fn width(&self) -> usize {
        self.width
    }

    /// Returns height in pixels.
    pub fn height(&self) -> usize {
        self.height
    }

    /// Clears the entire buffer to a solid color.
    pub fn clear(&mut self, color: Color) {
        let pixel = color.to_u32();
        self.buffer.fill(pixel);
    }

    /// Sets a pixel at (x, y) coordinates to color.
    pub fn set_pixel(&mut self, x: usize, y: usize, color: Color) {
        if x < self.width && y < self.height {
            self.buffer[y * self.width + x] = color.to_u32();
        }
    }

    /// Reads the color at (x, y), or `None` if out of bounds.
    pub fn get_pixel(&self, x: usize, y: usize) -> Option<Color> {
        if x < self.width && y < self.height {
            Some(Color::from_u32(self.buffer[y * self.width + x]))
        } else {
            None
        }
    }

    /// Presents/blits this framebuffer to an OS window. No-ops on any
    /// platform without a presentation backend yet (currently: everything
    /// except Windows and Linux — see
    /// [issue #3](https://github.com/baileyrd/rusty_gpu/issues/3) for the
    /// still-open macOS gap, blocked upstream on `rusty_gui` lacking a
    /// Cocoa/AppKit backend).
    pub fn present(&self, window: &rusty_gui::Window) {
        #[cfg(windows)]
        unsafe {
            rusty_win32::windowing::blit_pixel_buffer(
                window.raw_handle(),
                self.width,
                self.height,
                &self.buffer,
            );
        }
        #[cfg(target_os = "linux")]
        unsafe {
            x11_present::blit(window.raw_handle(), self.width, self.height, &self.buffer);
        }
        #[cfg(not(any(windows, target_os = "linux")))]
        {
            let _ = window;
        }
    }

    /// Borrows the raw 32-bit pixel buffer slice.
    pub fn as_slice(&self) -> &[u32] {
        &self.buffer
    }

    /// Bulk-copies `pixels` (row-major, `width * height` long, same packing
    /// as [`Color::to_u32`]) into the framebuffer in one pass, replacing its
    /// contents. For a caller that already composited a full frame into its
    /// own buffer (e.g. a text/UI renderer that doesn't itself draw through
    /// [`crate::Pipeline`]) and just needs it on screen via [`Self::present`].
    ///
    /// # Panics
    /// If `pixels.len() != width() * height()`.
    pub fn load(&mut self, pixels: &[u32]) {
        assert_eq!(
            pixels.len(),
            self.width * self.height,
            "pixel buffer length must equal width * height"
        );
        self.buffer.copy_from_slice(pixels);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_replaces_the_buffer_contents_pixel_for_pixel() {
        let mut fb = Framebuffer::new(2, 2);
        fb.set_pixel(0, 0, Color::rgb(1, 2, 3));
        let pixels = [
            Color::rgb(10, 20, 30).to_u32(),
            Color::rgb(40, 50, 60).to_u32(),
            Color::rgb(70, 80, 90).to_u32(),
            Color::rgb(100, 110, 120).to_u32(),
        ];
        fb.load(&pixels);
        assert_eq!(fb.get_pixel(0, 0), Some(Color::rgb(10, 20, 30)));
        assert_eq!(fb.get_pixel(1, 0), Some(Color::rgb(40, 50, 60)));
        assert_eq!(fb.get_pixel(0, 1), Some(Color::rgb(70, 80, 90)));
        assert_eq!(fb.get_pixel(1, 1), Some(Color::rgb(100, 110, 120)));
    }

    #[test]
    #[should_panic(expected = "pixel buffer length must equal width * height")]
    fn load_panics_on_mismatched_length() {
        let mut fb = Framebuffer::new(2, 2);
        fb.load(&[0u32; 3]);
    }
}
