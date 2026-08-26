//! ConPTY — Windows' pseudoconsole API (`wincon.h`'s `HPCON` family), a
//! new module added in round 2. Previously excluded for an architectural-
//! boundary reason (`rusty_lines` reads its own inherited stdin rather
//! than hosting a child's terminal), not a "no consumer" one — now in
//! scope per explicit round-2 direction, for a terminal-emulator-hosting
//! shell (or anything wanting to spawn a fully-interactive child, not
//! just redirected stdio).
//!
//! `CreatePseudoConsole`/`ResizePseudoConsole`/`ClosePseudoConsole` are
//! declared in mingw-w64's headers but gated behind `NTDDI_VERSION >=
//! NTDDI_WIN10_RS5` (Windows 10 1809) — a C probe needs
//! `-D_WIN32_WINNT=0x0A00 -DWINVER=0x0A00 -DNTDDI_VERSION=0x0A000006` to
//! see past that guard. This only affects a C header's *declaration*
//! visibility, not this crate's own `unsafe extern "system"` bindings
//! (which name the real symbols directly) or linking: `libkernel32.a`'s
//! import library already carries stub symbols for all three
//! unconditionally, verified with `nm`.
//!
//! This first piece is the pseudoconsole lifecycle itself — create,
//! live-resize, close — plus [`Hpcon`] and
//! [`PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE`] (originally filed as a
//! separate issue, folded in here since [`create`]'s own signature
//! already needs the `Hpcon` type: the same real, signature-level
//! dependency that combined `net::bind`/`SocketAddr` into one PR earlier
//! in round 2).
//!
//! [`AttributeList`] rounds this module out with the generic (pre-
//! ConPTY, Vista-era) extended-process-attribute mechanism — the only
//! way to hand an `HPCON` to `CreateProcessW`. `PROC_THREAD_ATTRIBUTE_LIST`'s
//! true size is knowable only at runtime, from a first, size-query-only
//! call's own size-out — a query-then-allocate opaque-byte-buffer
//! pattern, new territory for this crate, distinct from its existing
//! "retry a UTF-16 string buffer at the size the API reports" idiom
//! (e.g. [`crate::console::title`]).
//! `process::spawn_suspended_with_pseudoconsole` is the last round-2 item.

extern crate alloc;
use alloc::vec::Vec;

use crate::console::Coord;
use crate::error::Win32Error;
use crate::handle::RawHandle;

#[link(name = "kernel32")]
unsafe extern "system" {
    fn CreatePseudoConsole(
        size: Coord,
        input: RawHandle,
        output: RawHandle,
        flags: u32,
        hpc: *mut Hpcon,
    ) -> i32;
    fn ResizePseudoConsole(hpc: Hpcon, size: Coord) -> i32;
    fn ClosePseudoConsole(hpc: Hpcon);
}

/// An opaque pseudoconsole handle — `HPCON`, verified as an 8-byte
/// pointer-shaped value against mingw-w64's own `wincon.h`. A distinct
/// type from [`crate::handle::RawHandle`]: its only valid destructor is
/// [`close`]/`ClosePseudoConsole`, never `CloseHandle`.
pub type Hpcon = *mut core::ffi::c_void;

/// The `PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE` attribute id — the value a
/// later round-2 item (issue #188's `AttributeList::update`) needs to
/// bind an [`Hpcon`] into a process's extended startup info before
/// `CreateProcessW`. Computed by Windows' own `ProcThreadAttributeValue`
/// macro from `ProcThreadAttributePseudoConsole` (`22`) plus the
/// `PROC_THREAD_ATTRIBUTE_INPUT` bit (`0x0002_0000`); verified against
/// mingw-w64's own `winbase.h` with a compiled `_Static_assert` probe.
pub const PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE: usize = 0x0002_0016;

const HRESULT_WIN32_FACILITY_MASK: u32 = 0xFFFF_0000;
const HRESULT_WIN32_FACILITY: u32 = 0x8007_0000;

/// Unwrap an `HRESULT` back to the plain Win32 error code it was built
/// from. `CreatePseudoConsole`/`ResizePseudoConsole` are this crate's
/// only `HRESULT`-returning functions — every other wrapper reports
/// failure the ordinary `GetLastError()` way. Windows documents both as
/// reporting failure via `HRESULT_FROM_WIN32` of the underlying Win32
/// error, so this reverses that same, well-defined transform rather than
/// exposing a raw `HRESULT` this crate's `Win32Error`/`FormatMessageW`
/// machinery wouldn't recognize.
fn win32_error_from_hresult(hr: i32) -> Win32Error {
    let hr = hr as u32;
    if hr & HRESULT_WIN32_FACILITY_MASK == HRESULT_WIN32_FACILITY {
        Win32Error::from_raw(hr & 0xFFFF)
    } else {
        Win32Error::from_raw(hr)
    }
}

/// `CreatePseudoConsole`'s only documented `dwFlags` bit —
/// `PSEUDOCONSOLE_INHERIT_CURSOR`. Passed to [`create_with_flags`]: the new
/// pseudoconsole inherits the parent terminal's *current* cursor position
/// instead of starting at `(0,0)`, useful when creating a pseudoconsole
/// mid-session (e.g. after a resize/re-attach) so a hosted application's
/// rendering continues from where the parent left off rather than
/// resetting. Verified against mingw-w64's own `wincon.h`.
pub const PSEUDOCONSOLE_INHERIT_CURSOR: u32 = 0x0000_0001;

/// Create a pseudoconsole bound to a pipe pair — `CreatePseudoConsole`.
/// `input` is the read end of a pipe the caller writes keystrokes/input
/// into (the pseudoconsole reads from it on the hosted application's
/// behalf); `output` is the write end of a pipe the pseudoconsole
/// renders the hosted application's screen output into (the caller
/// reads from that pipe's other end). `size` is the initial
/// character-cell size.
///
/// `CreatePseudoConsole` duplicates `input`/`output` internally — per
/// Windows' own documented pattern, the caller should close its own
/// copies of `input`/`output` right after this call succeeds (via
/// [`crate::handle::close`]), keeping only the pipe pair's *other* ends
/// (the ones it uses to actually talk to the hosted application) open.
///
/// Requests no optional behavior (`dwFlags = 0`) — see
/// [`create_with_flags`] for a caller that wants
/// [`PSEUDOCONSOLE_INHERIT_CURSOR`].
///
/// # Safety
///
/// `input`/`output` must be currently-open, valid pipe handles (e.g.
/// from [`crate::handle::create_pipe`]).
pub unsafe fn create(
    input: RawHandle,
    output: RawHandle,
    size: Coord,
) -> Result<Hpcon, Win32Error> {
    // SAFETY: forwards this function's own safety contract unchanged;
    // `0` is the same hardcoded `dwFlags` value this function always
    // passed before `create_with_flags` existed.
    unsafe { create_with_flags(input, output, size, 0) }
}

/// [`create`]'s more general counterpart — added alongside it, not
/// replacing it — taking `CreatePseudoConsole`'s `dwFlags` explicitly.
/// `flags` is normally `0` (equivalent to [`create`]) or
/// [`PSEUDOCONSOLE_INHERIT_CURSOR`], the only flag Windows documents for
/// this function. See [`create`] for the `input`/`output`/`size`
/// parameters and the caller's responsibility to close its own copies of
/// `input`/`output` afterward.
///
/// # Safety
///
/// Same contract as [`create`]: `input`/`output` must be currently-open,
/// valid pipe handles (e.g. from [`crate::handle::create_pipe`]).
pub unsafe fn create_with_flags(
    input: RawHandle,
    output: RawHandle,
    size: Coord,
    flags: u32,
) -> Result<Hpcon, Win32Error> {
    let mut hpc: Hpcon = core::ptr::null_mut();
    // SAFETY: `input`/`output` are caller-supplied per this function's
    // own safety contract; `hpc` is a valid out-pointer; `flags` is a
    // plain-old-data bitmask, not a pointer.
    let hr = unsafe { CreatePseudoConsole(size, input, output, flags, &mut hpc) };
    if hr < 0 {
        Err(win32_error_from_hresult(hr))
    } else {
        Ok(hpc)
    }
}

/// Live-resize a pseudoconsole — `ResizePseudoConsole`, for a terminal-
/// size-change event.
///
/// # Safety
///
/// `hpc` must be a currently-open, valid pseudoconsole handle from
/// [`create`], not yet [`close`]-d.
pub unsafe fn resize(hpc: Hpcon, size: Coord) -> Result<(), Win32Error> {
    // SAFETY: `hpc` is caller-supplied per this function's own safety
    // contract; `size` is a plain-old-data value, not a pointer.
    let hr = unsafe { ResizePseudoConsole(hpc, size) };
    if hr < 0 {
        Err(win32_error_from_hresult(hr))
    } else {
        Ok(())
    }
}

/// Tear down a pseudoconsole — `ClosePseudoConsole`. Windows documents
/// this as blocking until any hosted application has exited and its
/// output has drained; it also closes its own internally-duplicated
/// copies of the `input`/`output` handles passed to [`create`] (not the
/// caller's own copies of those same two handles, already closed per
/// [`create`]'s own documented pattern, and not the pipe pair's other
/// two ends, which remain the caller's responsibility to close).
///
/// # Safety
///
/// `hpc` must be a currently-open, valid pseudoconsole handle from
/// [`create`], not already closed.
pub unsafe fn close(hpc: Hpcon) {
    // SAFETY: `hpc` is caller-supplied per this function's own safety
    // contract.
    unsafe { ClosePseudoConsole(hpc) }
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn InitializeProcThreadAttributeList(
        attribute_list: *mut u8,
        attribute_count: u32,
        flags: u32,
        size: *mut usize,
    ) -> i32;
    fn UpdateProcThreadAttribute(
        attribute_list: *mut u8,
        flags: u32,
        attribute: usize,
        value: *const core::ffi::c_void,
        size: usize,
        previous_value: *mut core::ffi::c_void,
        return_size: *mut usize,
    ) -> i32;
    fn DeleteProcThreadAttributeList(attribute_list: *mut u8);
}

/// A `PROC_THREAD_ATTRIBUTE_LIST` — the generic (pre-ConPTY, Vista-era)
/// extended-process-attribute mechanism `CreateProcessW` reads via
/// `STARTUPINFOEXW.lpAttributeList`, the only way to hand it an
/// [`Hpcon`] (via [`PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE`]). Its true
/// byte size is knowable only at runtime; [`init`](AttributeList::init)
/// discovers it via a size-query-only first call before allocating the
/// real buffer, an opaque-byte-buffer pattern Windows itself documents
/// this two-call way (distinct from a *reported-size-was-wrong* retry
/// loop like [`crate::service::display_name`]'s).
pub struct AttributeList {
    buffer: Vec<u8>,
    deleted: bool,
}

impl AttributeList {
    /// Allocate and initialize a new attribute list sized for
    /// `attribute_count` entries — `InitializeProcThreadAttributeList`,
    /// called once (ignoring its return value: passing a null attribute-
    /// list pointer is documented as this function's own size-query
    /// mode, which always reports failure this way purely to hand back
    /// the real byte count) to discover the required buffer size, then
    /// again on the freshly allocated buffer to do the real
    /// initialization.
    pub fn init(attribute_count: u32) -> Result<Self, Win32Error> {
        let mut size: usize = 0;
        // SAFETY: a null attribute-list pointer paired with a valid
        // `size` out-pointer is `InitializeProcThreadAttributeList`'s
        // documented size-query mode.
        unsafe {
            InitializeProcThreadAttributeList(core::ptr::null_mut(), attribute_count, 0, &mut size)
        };

        let mut buffer = alloc::vec![0u8; size];
        // SAFETY: `buffer` is exactly `size` bytes, matching the size
        // query above; this call does the real initialization.
        let ok = unsafe {
            InitializeProcThreadAttributeList(buffer.as_mut_ptr(), attribute_count, 0, &mut size)
        };
        if ok == 0 {
            return Err(Win32Error::last());
        }
        Ok(AttributeList {
            buffer,
            deleted: false,
        })
    }

    /// Set one attribute — `UpdateProcThreadAttribute`. `attribute` is
    /// an attribute id such as [`PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE`];
    /// `value`/`size` describe the attribute's value bytes (e.g. an
    /// [`Hpcon`] and its own `size_of`).
    ///
    /// # Safety
    ///
    /// `value` must point to `size` valid, readable bytes that stay
    /// alive and unmoved for as long as this `AttributeList` is in use
    /// by `CreateProcessW` (Windows stores the pointer, not a copy of
    /// the bytes, until this list is consumed or deleted).
    pub unsafe fn update(
        &mut self,
        attribute: usize,
        value: *const core::ffi::c_void,
        size: usize,
    ) -> Result<(), Win32Error> {
        // SAFETY: `self.buffer` is a live, `init`-ed attribute list;
        // `value`/`size` are caller-supplied per this function's own
        // safety contract; `previous_value`/`return_size` are documented-
        // valid NULLs (this crate never reads an attribute's prior
        // value).
        let ok = unsafe {
            UpdateProcThreadAttribute(
                self.buffer.as_mut_ptr(),
                0,
                attribute,
                value,
                size,
                core::ptr::null_mut(),
                core::ptr::null_mut(),
            )
        };
        if ok == 0 {
            Err(Win32Error::last())
        } else {
            Ok(())
        }
    }

    /// The raw `PROC_THREAD_ATTRIBUTE_LIST` pointer, ready to embed in a
    /// `STARTUPINFOEXW.lpAttributeList` field —
    /// `process::spawn_suspended_with_pseudoconsole`'s own use.
    pub(crate) fn as_mut_ptr(&mut self) -> *mut u8 {
        self.buffer.as_mut_ptr()
    }

    /// Tear down this attribute list — `DeleteProcThreadAttributeList`.
    /// Idempotent: a second call (or the [`Drop`] impl running after an
    /// explicit call here) is a no-op, not a double-free.
    pub fn delete(&mut self) {
        if !self.deleted {
            // SAFETY: `self.buffer` was successfully initialized by
            // `init` and not yet deleted (`self.deleted` just checked
            // above).
            unsafe { DeleteProcThreadAttributeList(self.buffer.as_mut_ptr()) };
            self.deleted = true;
        }
    }
}

impl Drop for AttributeList {
    fn drop(&mut self) {
        self.delete();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::handle;

    #[test]
    fn create_resize_close_round_trips_a_pseudoconsole() {
        let (input_read, input_write) = handle::create_pipe()
            .expect("create_pipe should succeed for the pseudoconsole's input pipe");
        let (output_read, output_write) = handle::create_pipe()
            .expect("create_pipe should succeed for the pseudoconsole's output pipe");

        let size = Coord { x: 80, y: 25 };
        // SAFETY: `input_read`/`output_write` are freshly created, open
        // pipe handles from the calls above.
        let hpc = unsafe { create(input_read, output_write, size) }
            .expect("CreatePseudoConsole should succeed with a fresh pipe pair");
        assert!(
            !hpc.is_null(),
            "a successful CreatePseudoConsole should report a non-null HPCON"
        );

        // CreatePseudoConsole duplicated input_read/output_write
        // internally -- close this test's own copies now, per the
        // documented pattern noted on `create`.
        // SAFETY: `input_read`/`output_write` are still open and were
        // only ever passed to `create` above, never closed.
        unsafe { handle::close(input_read) }
            .expect("closing this test's own copy of the input pipe's read end should succeed");
        unsafe { handle::close(output_write) }
            .expect("closing this test's own copy of the output pipe's write end should succeed");

        let new_size = Coord { x: 100, y: 40 };
        // SAFETY: `hpc` was just created above and hasn't been closed
        // yet.
        unsafe { resize(hpc, new_size) }.expect("ResizePseudoConsole should succeed");

        // SAFETY: `hpc` is still open from the calls above.
        unsafe { close(hpc) };

        // SAFETY: `input_write`/`output_read` are this test's own
        // remaining pipe ends, never passed to `create`/`close` above.
        unsafe { handle::close(input_write) }
            .expect("closing the input pipe's write end should succeed");
        unsafe { handle::close(output_read) }
            .expect("closing the output pipe's read end should succeed");
    }

    #[test]
    fn create_with_flags_accepts_pseudoconsole_inherit_cursor() {
        let (input_read, input_write) = handle::create_pipe()
            .expect("create_pipe should succeed for the pseudoconsole's input pipe");
        let (output_read, output_write) = handle::create_pipe()
            .expect("create_pipe should succeed for the pseudoconsole's output pipe");

        let size = Coord { x: 80, y: 25 };
        // SAFETY: `input_read`/`output_write` are freshly created, open
        // pipe handles from the calls above.
        let hpc = unsafe {
            create_with_flags(input_read, output_write, size, PSEUDOCONSOLE_INHERIT_CURSOR)
        }
        .expect("CreatePseudoConsole should succeed with PSEUDOCONSOLE_INHERIT_CURSOR set");
        assert!(
            !hpc.is_null(),
            "a successful CreatePseudoConsole should report a non-null HPCON"
        );

        // SAFETY: per `create`'s documented pattern (`create_with_flags`
        // shares it), close this test's own copies now that ConPTY has
        // duplicated them internally.
        unsafe { handle::close(input_read) }
            .expect("closing this test's own copy of the input pipe's read end should succeed");
        unsafe { handle::close(output_write) }
            .expect("closing this test's own copy of the output pipe's write end should succeed");

        // SAFETY: `hpc` is still open from the call above.
        unsafe { close(hpc) };

        // SAFETY: `input_write`/`output_read` are this test's own
        // remaining pipe ends, never passed to `create_with_flags`/`close`
        // above.
        unsafe { handle::close(input_write) }
            .expect("closing the input pipe's write end should succeed");
        unsafe { handle::close(output_read) }
            .expect("closing the output pipe's read end should succeed");
    }

    #[test]
    fn attribute_list_init_then_delete_round_trips() {
        let mut list =
            AttributeList::init(1).expect("InitializeProcThreadAttributeList should succeed");
        list.delete();
        // A second explicit delete (before Drop also runs at the end of
        // this scope) must be a no-op, not a double `DeleteProcThreadAttributeList`.
        list.delete();
    }

    #[test]
    fn attribute_list_update_binds_a_pseudoconsole_attribute() {
        let (input_read, input_write) = handle::create_pipe()
            .expect("create_pipe should succeed for the pseudoconsole's input pipe");
        let (output_read, output_write) = handle::create_pipe()
            .expect("create_pipe should succeed for the pseudoconsole's output pipe");
        let size = Coord { x: 80, y: 25 };
        // SAFETY: `input_read`/`output_write` are freshly created, open
        // pipe handles from the calls above.
        let hpc = unsafe { create(input_read, output_write, size) }
            .expect("CreatePseudoConsole should succeed with a fresh pipe pair");
        // SAFETY: per `create`'s documented pattern, close this test's
        // own copies now that ConPTY has duplicated them internally.
        unsafe { handle::close(input_read) }
            .expect("closing this test's own copy of the input pipe's read end should succeed");
        unsafe { handle::close(output_write) }
            .expect("closing this test's own copy of the output pipe's write end should succeed");

        let mut list =
            AttributeList::init(1).expect("InitializeProcThreadAttributeList should succeed");
        // SAFETY: `hpc` is a live, valid HPCON from `create` above, and
        // stays alive (not moved, not dropped) for the rest of this
        // test, well past this `update` call.
        unsafe {
            list.update(
                PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE,
                (&hpc as *const Hpcon).cast(),
                core::mem::size_of::<Hpcon>(),
            )
        }
        .expect("UpdateProcThreadAttribute should succeed binding a real HPCON");

        list.delete();

        // SAFETY: `hpc` is still open from the calls above.
        unsafe { close(hpc) };
        unsafe { handle::close(input_write) }
            .expect("closing the input pipe's write end should succeed");
        unsafe { handle::close(output_read) }
            .expect("closing the output pipe's read end should succeed");
    }
}
