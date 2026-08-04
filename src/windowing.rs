//! FFI Bindings for User32 and GDI32 Windowing, Messaging, and Framebuffer blitting.

#![cfg(windows)]

extern crate alloc;
use alloc::vec::Vec;
use core::ffi::c_void;

pub type HWND = *mut c_void;
pub type HDC = *mut c_void;
pub type HBRUSH = *mut c_void;
pub type HICON = *mut c_void;
pub type HCURSOR = *mut c_void;
pub type HINSTANCE = *mut c_void;
pub type WNDPROC = Option<unsafe extern "system" fn(HWND, u32, usize, isize) -> isize>;

#[repr(C)]
pub struct WNDCLASSEXW {
    pub cb_size: u32,
    pub style: u32,
    pub lpfn_wnd_proc: WNDPROC,
    pub cb_cls_extra: i32,
    pub cb_wnd_extra: i32,
    pub h_instance: HINSTANCE,
    pub h_icon: HICON,
    pub h_cursor: HCURSOR,
    pub hbr_background: HBRUSH,
    pub lpsz_menu_name: *const u16,
    pub lpsz_class_name: *const u16,
    pub h_icon_sm: HICON,
}

#[repr(C)]
pub struct POINT {
    pub x: i32,
    pub y: i32,
}

#[repr(C)]
pub struct MSG {
    pub hwnd: HWND,
    pub message: u32,
    pub w_param: usize,
    pub l_param: isize,
    pub time: u32,
    pub pt: POINT,
}

#[repr(C)]
pub struct BITMAPINFOHEADER {
    pub bi_size: u32,
    pub bi_width: i32,
    pub bi_height: i32,
    pub bi_planes: u16,
    pub bi_bit_count: u16,
    pub bi_compression: u32,
    pub bi_size_image: u32,
    pub bi_x_pels_per_meter: i32,
    pub bi_y_pels_per_meter: i32,
    pub bi_clr_used: u32,
    pub bi_clr_important: u32,
}

#[repr(C)]
pub struct RGBQUAD {
    pub blue: u8,
    pub green: u8,
    pub red: u8,
    pub reserved: u8,
}

#[repr(C)]
pub struct BITMAPINFO {
    pub header: BITMAPINFOHEADER,
    pub colors: [RGBQUAD; 1],
}

#[link(name = "user32")]
#[link(name = "gdi32")]
unsafe extern "system" {
    pub fn GetModuleHandleW(lp_module_name: *const u16) -> HINSTANCE;
    pub fn RegisterClassExW(lpwcx: *const WNDCLASSEXW) -> u16;
    pub fn CreateWindowExW(
        dw_ex_style: u32,
        lp_class_name: *const u16,
        lp_window_name: *const u16,
        dw_style: u32,
        x: i32,
        y: i32,
        n_width: i32,
        n_height: i32,
        h_wnd_parent: HWND,
        h_menu: *mut c_void,
        h_instance: HINSTANCE,
        lp_param: *mut c_void,
    ) -> HWND;
    pub fn ShowWindow(h_wnd: HWND, n_cmd_show: i32) -> i32;
    pub fn UpdateWindow(h_wnd: HWND) -> i32;
    pub fn GetDC(h_wnd: HWND) -> HDC;
    pub fn ReleaseDC(h_wnd: HWND, h_dc: HDC) -> i32;
    pub fn PeekMessageW(
        lp_msg: *mut MSG,
        h_wnd: HWND,
        w_msg_filter_min: u32,
        w_msg_filter_max: u32,
        w_remove_msg: u32,
    ) -> i32;
    pub fn TranslateMessage(lp_msg: *const MSG) -> i32;
    pub fn DispatchMessageW(lp_msg: *const MSG) -> isize;
    pub fn DefWindowProcW(h_wnd: HWND, msg: u32, w_param: usize, l_param: isize) -> isize;
    pub fn StretchDIBits(
        hdc: HDC,
        x_dest: i32,
        y_dest: i32,
        dest_width: i32,
        dest_height: i32,
        x_src: i32,
        y_src: i32,
        src_width: i32,
        src_height: i32,
        lp_bits: *const c_void,
        lpbmi: *const BITMAPINFO,
        i_usage: u32,
        rop: u32,
    ) -> i32;
}

/// Helper function to create a native Windows OS Window.
///
/// # Safety
///
/// Must be called from the thread that will subsequently pump this
/// window's messages (`PeekMessageW`/`TranslateMessage`/`DispatchMessageW`)
/// — `RegisterClassExW`/`CreateWindowExW` tie the window class and the
/// returned `HWND` to the calling thread's message queue, the same
/// thread-affinity Win32 itself documents for all windowing calls.
pub unsafe fn create_native_window(title: &str, width: u32, height: u32) -> HWND {
    let class_name: Vec<u16> = "RustyMillWindowClass\0".encode_utf16().collect();
    let title_utf16: Vec<u16> = title.encode_utf16().chain(core::iter::once(0)).collect();

    unsafe {
        let h_instance = GetModuleHandleW(core::ptr::null());

        let wnd_class = WNDCLASSEXW {
            cb_size: core::mem::size_of::<WNDCLASSEXW>() as u32,
            style: 0x0003, // CS_HREDRAW | CS_VREDRAW
            lpfn_wnd_proc: Some(DefWindowProcW),
            cb_cls_extra: 0,
            cb_wnd_extra: 0,
            h_instance,
            h_icon: core::ptr::null_mut(),
            h_cursor: core::ptr::null_mut(),
            hbr_background: core::ptr::null_mut(),
            lpsz_menu_name: core::ptr::null(),
            lpsz_class_name: class_name.as_ptr(),
            h_icon_sm: core::ptr::null_mut(),
        };

        RegisterClassExW(&wnd_class);

        let hwnd = CreateWindowExW(
            0,
            class_name.as_ptr(),
            title_utf16.as_ptr(),
            0x00C00000 | 0x00080000 | 0x00040000 | 0x00020000 | 0x00010000 | 0x10000000, // WS_OVERLAPPEDWINDOW | WS_VISIBLE
            100,
            100,
            width as i32,
            height as i32,
            core::ptr::null_mut(),
            core::ptr::null_mut(),
            h_instance,
            core::ptr::null_mut(),
        );

        ShowWindow(hwnd, 5); // SW_SHOW
        UpdateWindow(hwnd);

        hwnd
    }
}

/// Helper function to blit a raw 32-bit pixel buffer to a Window HDC via StretchDIBits.
///
/// # Safety
///
/// `hwnd` must be a currently-open, valid window handle from
/// [`create_native_window`] (or null, silently skipped). `pixels` must
/// contain at least `width * height` elements — `StretchDIBits` reads
/// exactly that many pixels starting at `pixels.as_ptr()`, with no bounds
/// check against the slice's own length.
pub unsafe fn blit_pixel_buffer(hwnd: HWND, width: usize, height: usize, pixels: &[u32]) {
    if hwnd.is_null() {
        return;
    }

    unsafe {
        let hdc = GetDC(hwnd);
        if hdc.is_null() {
            return;
        }

        let mut bmi: BITMAPINFO = core::mem::zeroed();
        bmi.header.bi_size = core::mem::size_of::<BITMAPINFOHEADER>() as u32;
        bmi.header.bi_width = width as i32;
        bmi.header.bi_height = -(height as i32); // Top-down DIB
        bmi.header.bi_planes = 1;
        bmi.header.bi_bit_count = 32;
        bmi.header.bi_compression = 0; // BI_RGB

        StretchDIBits(
            hdc,
            0,
            0,
            width as i32,
            height as i32,
            0,
            0,
            width as i32,
            height as i32,
            pixels.as_ptr() as *const c_void,
            &bmi,
            0,          // DIB_RGB_COLORS
            0x00CC0020, // SRCCOPY
        );

        ReleaseDC(hwnd, hdc);
    }
}
