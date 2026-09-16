//! Named pipes (`CreateNamedPipeW`/`ConnectNamedPipeW`/`WaitNamedPipeW`) —
//! the Windows analog of a Unix named FIFO, or the `/dev/fd`-style
//! externally openable pipe a `coproc`/process-substitution
//! implementation needs. [`crate::handle::create_pipe`]'s anonymous pipes
//! only solve the "handed directly to a child this process itself spawns"
//! case: neither end has a name an arbitrary already-running program can
//! open. rush's own `docs/WINDOWS_JOB_CONTROL.md` ("Deliberately out of
//! scope") and `docs/CAPABILITY_GAPS.md` (C31 process substitution, C66
//! `coproc`) both name a named pipe as the missing primitive for `<(cmd)`/
//! `coproc` on Windows.
//!
//! A caller reads/writes a connected pipe handle exactly like any other
//! handle — wrap it in `std::fs::File`/`OwnedHandle` under the `std`
//! feature, or use [`crate::console::read`] (a plain `ReadFile` wrapper
//! despite its module) under `no_std`. This module only covers creating
//! the server end and connecting/opening the client end, not I/O — the
//! same split [`crate::handle::create_pipe`] already draws for anonymous
//! pipes.
//!
//! No `OVERLAPPED` support: [`connect_server`] blocks synchronously,
//! matching this crate's existing no-overlapped-I/O convention
//! (`handle`'s anonymous pipes, `console::read`).

use crate::error::Win32Error;
use crate::handle::RawHandle;

extern crate alloc;
use alloc::vec::Vec;

/// `CreateNamedPipeW`'s `dwOpenMode`: the pipe can be both read and written
/// through this server handle.
pub const PIPE_ACCESS_DUPLEX: u32 = 0x1 | 0x2;
/// `CreateNamedPipeW`'s `dwOpenMode`: data flows client-to-server only.
pub const PIPE_ACCESS_INBOUND: u32 = 0x1;
/// `CreateNamedPipeW`'s `dwOpenMode`: data flows server-to-client only.
pub const PIPE_ACCESS_OUTBOUND: u32 = 0x2;

/// `CreateNamedPipeW`'s `dwPipeMode`: a plain byte stream, no message
/// framing — the ordinary choice, matching how an anonymous pipe already
/// behaves.
pub const PIPE_TYPE_BYTE: u32 = 0x0;
/// `CreateNamedPipeW`'s `dwPipeMode`: Windows preserves message boundaries
/// (each `WriteFile` call is one discrete message on read-back) — no Unix
/// pipe equivalent.
pub const PIPE_TYPE_MESSAGE: u32 = 0x4;
/// `CreateNamedPipeW`'s `dwPipeMode`: reads return raw bytes regardless of
/// `PIPE_TYPE_MESSAGE`/`PIPE_TYPE_BYTE`.
pub const PIPE_READMODE_BYTE: u32 = 0x0;
/// `CreateNamedPipeW`'s `dwPipeMode`: reads respect message boundaries —
/// only meaningful paired with [`PIPE_TYPE_MESSAGE`].
pub const PIPE_READMODE_MESSAGE: u32 = 0x2;
/// `CreateNamedPipeW`'s `dwPipeMode`: blocking I/O (the ordinary choice,
/// matching every other synchronous call in this crate).
pub const PIPE_WAIT: u32 = 0x0;
/// `CreateNamedPipeW`'s `dwPipeMode`: non-blocking I/O — Microsoft
/// discourages this in favor of overlapped I/O, which this crate doesn't
/// support yet; included for completeness, not recommended.
pub const PIPE_NOWAIT: u32 = 0x1;
/// `CreateNamedPipeW`'s documented sentinel for `nMaxInstances`: no cap on
/// how many server instances of this pipe name may exist at once.
pub const PIPE_UNLIMITED_INSTANCES: u32 = 255;

/// `CreateFileW`'s `dwDesiredAccess` bit for [`open_client`] — read access.
pub const GENERIC_READ: u32 = 0x8000_0000;
/// `CreateFileW`'s `dwDesiredAccess` bit for [`open_client`] — write access.
pub const GENERIC_WRITE: u32 = 0x4000_0000;

const OPEN_EXISTING: u32 = 3;

#[link(name = "kernel32")]
unsafe extern "system" {
    fn CreateNamedPipeW(
        name: *const u16,
        open_mode: u32,
        pipe_mode: u32,
        max_instances: u32,
        out_buffer_size: u32,
        in_buffer_size: u32,
        default_timeout: u32,
        security_attributes: *const core::ffi::c_void,
    ) -> RawHandle;
    fn ConnectNamedPipe(named_pipe: RawHandle, overlapped: *mut core::ffi::c_void) -> i32;
    fn DisconnectNamedPipe(named_pipe: RawHandle) -> i32;
    fn SetNamedPipeHandleState(
        named_pipe: RawHandle,
        mode: *const u32,
        max_collection_count: *const u32,
        collect_data_timeout: *const u32,
    ) -> i32;
    fn WaitNamedPipeW(name: *const u16, timeout_ms: u32) -> i32;
    fn GetNamedPipeInfo(
        named_pipe: RawHandle,
        flags: *mut u32,
        out_buffer_size: *mut u32,
        in_buffer_size: *mut u32,
        max_instances: *mut u32,
    ) -> i32;
    fn CreateFileW(
        file_name: *const u16,
        desired_access: u32,
        share_mode: u32,
        security_attributes: *const core::ffi::c_void,
        creation_disposition: u32,
        flags_and_attributes: u32,
        template_file: RawHandle,
    ) -> RawHandle;
    fn TransactNamedPipe(
        named_pipe: RawHandle,
        in_buffer: *const u8,
        in_buffer_size: u32,
        out_buffer: *mut u8,
        out_buffer_size: u32,
        bytes_read: *mut u32,
        overlapped: *mut core::ffi::c_void,
    ) -> i32;
    fn CallNamedPipeW(
        name: *const u16,
        in_buffer: *const u8,
        in_buffer_size: u32,
        out_buffer: *mut u8,
        out_buffer_size: u32,
        bytes_read: *mut u32,
        timeout_ms: u32,
    ) -> i32;
}

/// Build the full `\\.\pipe\<name>` UTF-16, NUL-terminated pipe path every
/// named-pipe Win32 call requires — `name` is just the pipe's own name
/// (e.g. `"rush-coproc-1234-5"`), not the namespace prefix, which is a
/// fixed Windows convention this function bakes in rather than policy a
/// caller decides (the same way `console::CP_UTF8` bakes in a fixed
/// codepage value rather than every raw codepage id).
fn full_pipe_name(name: &str) -> Vec<u16> {
    r"\\.\pipe\"
        .encode_utf16()
        .chain(name.encode_utf16())
        .chain(core::iter::once(0))
        .collect()
}

/// Create one instance of a named pipe server — `CreateNamedPipeW`.
/// `open_mode`/`pipe_mode` are the raw `PIPE_*` bits above (e.g.
/// `PIPE_ACCESS_DUPLEX`, `PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT`)
/// — this function is a thin, policy-free wrapper, the same as this
/// crate's `console::ENABLE_*` mode bits. `max_instances` should usually be
/// `1` for a point-to-point pipe (a `coproc`/process-substitution use
/// case); pass [`PIPE_UNLIMITED_INSTANCES`] for a fan-out server. `0` for
/// `out_buffer_size`/`in_buffer_size` lets the system choose a default.
pub fn create_server(
    name: &str,
    open_mode: u32,
    pipe_mode: u32,
    max_instances: u32,
    out_buffer_size: u32,
    in_buffer_size: u32,
) -> Result<RawHandle, Win32Error> {
    let full_name = full_pipe_name(name);
    // SAFETY: `full_name` is a valid, NUL-terminated UTF-16 string;
    // `default_timeout = 0` (system default) and
    // `security_attributes = NULL` (default security, non-inheritable) are
    // documented-valid inputs; every other argument is a plain value.
    let handle = unsafe {
        CreateNamedPipeW(
            full_name.as_ptr(),
            open_mode,
            pipe_mode,
            max_instances,
            out_buffer_size,
            in_buffer_size,
            0,
            core::ptr::null(),
        )
    };
    if handle.is_null() || handle as isize == -1 {
        Err(Win32Error::last())
    } else {
        Ok(handle)
    }
}

/// Block until a client connects to `pipe` (from [`create_server`]) —
/// `ConnectNamedPipeW` with no `OVERLAPPED` (a synchronous, blocking wait),
/// matching this crate's no-overlapped-I/O convention.
///
/// Windows documents a real quirk here: if a client already connected
/// between `create_server` and this call, `ConnectNamedPipeW` fails with
/// [`Win32Error::ERROR_PIPE_CONNECTED`] — not a real error, just "already
/// connected, nothing to wait for." This wrapper treats that one specific
/// code as success rather than surfacing it as a failure, the same way
/// [`crate::process::list_processes`] treats
/// [`Win32Error::ERROR_NO_MORE_FILES`] as ordinary end-of-enumeration
/// rather than a real error.
///
/// # Safety
///
/// `pipe` must be a currently-open, valid named-pipe server handle from
/// [`create_server`], not already connected to a client it's waiting on
/// again.
pub unsafe fn connect_server(pipe: RawHandle) -> Result<(), Win32Error> {
    // SAFETY: `pipe` is caller-supplied per this function's own safety
    // contract; `overlapped = NULL` requests the synchronous, blocking
    // behavior this function's own contract documents.
    let ok = unsafe { ConnectNamedPipe(pipe, core::ptr::null_mut()) };
    if ok == 0 {
        let err = Win32Error::last();
        if err == Win32Error::ERROR_PIPE_CONNECTED {
            Ok(())
        } else {
            Err(err)
        }
    } else {
        Ok(())
    }
}

/// Disconnect the client currently connected to `pipe` (from
/// [`create_server`]/[`connect_server`]) and reset the server instance so a
/// subsequent [`connect_server`] call can serve a new client —
/// `DisconnectNamedPipe`. Without this, a served pipe instance is single-use
/// only: the whole server has to be recreated (a fresh [`create_server`]
/// call) to talk to a second client.
///
/// # Safety
///
/// `pipe` must be a currently-open, valid named-pipe server handle from
/// [`create_server`].
pub unsafe fn disconnect_server(pipe: RawHandle) -> Result<(), Win32Error> {
    // SAFETY: `pipe` is caller-supplied per this function's own safety
    // contract.
    let ok = unsafe { DisconnectNamedPipe(pipe) };
    if ok == 0 {
        Err(Win32Error::last())
    } else {
        Ok(())
    }
}

/// Change `pipe`'s mode — `SetNamedPipeHandleState`. `mode` is the raw
/// `PIPE_READMODE_*`/`PIPE_WAIT`/`PIPE_NOWAIT` bitmask already defined
/// above — this function is a thin, policy-free wrapper, the same as
/// [`create_server`]'s raw `PIPE_*` mode parameters. Covers two things at
/// once: switching between byte/message read mode after creation, and
/// [`PIPE_NOWAIT`], the named-pipe equivalent of the non-blocking check
/// [`crate::handle::pipe_bytes_available`] (`PeekNamedPipe`) already gives
/// anonymous pipes.
///
/// # Safety
///
/// `pipe` must be a currently-open, valid named-pipe handle.
pub unsafe fn set_pipe_mode(pipe: RawHandle, mode: u32) -> Result<(), Win32Error> {
    // SAFETY: `pipe` is caller-supplied per this function's own safety
    // contract; `mode` is a valid pointer to a plain `u32` local;
    // `max_collection_count`/`collect_data_timeout = NULL` are
    // documented-valid inputs meaning "leave unchanged" (they only apply to
    // message-mode pipes over a network, out of this function's own scope).
    let ok = unsafe { SetNamedPipeHandleState(pipe, &mode, core::ptr::null(), core::ptr::null()) };
    if ok == 0 {
        Err(Win32Error::last())
    } else {
        Ok(())
    }
}

/// Wait for an instance of the named pipe `name` to become available for
/// connecting — `WaitNamedPipeW`. Needed only when the client might race
/// the server (no instance created yet, or every existing instance already
/// busy with another client); a client that already knows a free server
/// instance is listening can call [`open_client`] directly and skip this.
///
/// Windows reports [`Win32Error::ERROR_FILE_NOT_FOUND`] immediately,
/// regardless of `timeout_ms`, if no instance of `name` exists at all —
/// distinct from a real timeout, which means instances exist but are all
/// currently busy.
pub fn wait_for_server(name: &str, timeout_ms: u32) -> Result<(), Win32Error> {
    let full_name = full_pipe_name(name);
    // SAFETY: `full_name` is a valid, NUL-terminated UTF-16 string.
    let ok = unsafe { WaitNamedPipeW(full_name.as_ptr(), timeout_ms) };
    if ok == 0 {
        Err(Win32Error::last())
    } else {
        Ok(())
    }
}

/// Open the client end of the named pipe `name` — `CreateFileW` against the
/// same `\\.\pipe\<name>` path [`create_server`] listens on, the standard
/// way any process (not just one this crate spawned) connects to a named
/// pipe. `desired_access` is [`GENERIC_READ`]/[`GENERIC_WRITE`] (or both) —
/// this function is a thin, policy-free wrapper, the same as
/// [`create_server`]'s raw `PIPE_*` mode parameters.
pub fn open_client(name: &str, desired_access: u32) -> Result<RawHandle, Win32Error> {
    let full_name = full_pipe_name(name);
    // SAFETY: `full_name` is a valid, NUL-terminated UTF-16 string;
    // `share_mode = 0` (no sharing) and `security_attributes = NULL`
    // (default, non-inheritable) are documented-valid inputs;
    // `creation_disposition = OPEN_EXISTING` is required for a named
    // pipe client (it never creates one); `template_file = NULL` is
    // ignored by `OPEN_EXISTING`, a documented valid input.
    let handle = unsafe {
        CreateFileW(
            full_name.as_ptr(),
            desired_access,
            0,
            core::ptr::null(),
            OPEN_EXISTING,
            0,
            core::ptr::null_mut(),
        )
    };
    if handle.is_null() || handle as isize == -1 {
        Err(Win32Error::last())
    } else {
        Ok(handle)
    }
}

/// `GetNamedPipeInfo`'s `lpFlags`: this handle is the server end (a client
/// handle, the more common case, doesn't carry this bit).
const PIPE_SERVER_END: u32 = 0x0000_0001;

/// [`pipe_info`]'s result — `GetNamedPipeInfo`'s fields, the read-side
/// counterpart to [`create_server`]'s creation-time parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PipeInfo {
    /// `true` if this handle is the server end, `false` if it's a client
    /// end (e.g. one opened via [`open_client`]).
    pub is_server_end: bool,
    /// `true` for [`PIPE_TYPE_MESSAGE`] mode, `false` for [`PIPE_TYPE_BYTE`].
    pub is_message_type: bool,
    /// The pipe's output buffer size, in bytes (`0` if the system chose a
    /// default at creation, same caveat `create_server`'s own parameter
    /// documents).
    pub out_buffer_size: u32,
    /// The pipe's input buffer size, in bytes.
    pub in_buffer_size: u32,
    /// The maximum number of server instances this pipe name allows —
    /// [`PIPE_UNLIMITED_INSTANCES`] if uncapped.
    pub max_instances: u32,
}

/// Read back `pipe`'s own type/mode/buffer-size configuration —
/// `GetNamedPipeInfo`. Works on either a server handle (from
/// [`create_server`]) or a client handle (from [`open_client`]).
///
/// # Safety
///
/// `pipe` must be a currently-open, valid named-pipe handle.
pub unsafe fn pipe_info(pipe: RawHandle) -> Result<PipeInfo, Win32Error> {
    let mut flags: u32 = 0;
    let mut out_buffer_size: u32 = 0;
    let mut in_buffer_size: u32 = 0;
    let mut max_instances: u32 = 0;
    // SAFETY: `pipe` is caller-supplied per this function's own safety
    // contract; the four out-pointers are valid, distinct local variables.
    let ok = unsafe {
        GetNamedPipeInfo(
            pipe,
            &mut flags,
            &mut out_buffer_size,
            &mut in_buffer_size,
            &mut max_instances,
        )
    };
    if ok == 0 {
        return Err(Win32Error::last());
    }
    Ok(PipeInfo {
        is_server_end: flags & PIPE_SERVER_END != 0,
        is_message_type: flags & PIPE_TYPE_MESSAGE != 0,
        out_buffer_size,
        in_buffer_size,
        max_instances,
    })
}

/// One-shot write-then-read transaction on an already-connected,
/// message-mode pipe — `TransactNamedPipe`, an alternative to separate
/// write/read calls for a simple request-response protocol. Only valid on
/// a duplex, message-type pipe (`PIPE_ACCESS_DUPLEX` and
/// `PIPE_TYPE_MESSAGE`); a byte-mode or inbound/outbound-only pipe fails
/// this call, per its own documented contract. Returns the number of
/// bytes actually written into `read_buf`.
///
/// # Safety
///
/// `pipe` must be a currently-open, valid named-pipe handle.
pub unsafe fn transact(
    pipe: RawHandle,
    write_buf: &[u8],
    read_buf: &mut [u8],
) -> Result<usize, Win32Error> {
    let mut bytes_read: u32 = 0;
    // SAFETY: `pipe` is caller-supplied per this function's own safety
    // contract; `write_buf`/`read_buf` are valid slices matched by the
    // `..._size` arguments naming their exact lengths; `overlapped = NULL`
    // requests synchronous operation, matching this crate's existing
    // no-overlapped-I/O convention; `bytes_read` is a valid out-pointer.
    let ok = unsafe {
        TransactNamedPipe(
            pipe,
            write_buf.as_ptr(),
            write_buf.len() as u32,
            read_buf.as_mut_ptr(),
            read_buf.len() as u32,
            &mut bytes_read,
            core::ptr::null_mut(),
        )
    };
    if ok == 0 {
        return Err(Win32Error::last());
    }
    Ok(bytes_read as usize)
}

/// A one-shot client-side request-response call against the named pipe
/// `name` — `CallNamedPipeW`. Combines [`wait_for_server`]/[`open_client`]/
/// [`transact`]/close into a single call, for a caller that only needs one
/// round trip and doesn't want to keep the pipe open afterward. Returns
/// the number of bytes actually written into `read_buf`.
pub fn call(
    name: &str,
    write_buf: &[u8],
    read_buf: &mut [u8],
    timeout_ms: u32,
) -> Result<usize, Win32Error> {
    let full_name = full_pipe_name(name);
    let mut bytes_read: u32 = 0;
    // SAFETY: `full_name` is a valid, NUL-terminated UTF-16 string;
    // `write_buf`/`read_buf` are valid slices matched by the `..._size`
    // arguments naming their exact lengths; `bytes_read` is a valid
    // out-pointer.
    let ok = unsafe {
        CallNamedPipeW(
            full_name.as_ptr(),
            write_buf.as_ptr(),
            write_buf.len() as u32,
            read_buf.as_mut_ptr(),
            read_buf.len() as u32,
            &mut bytes_read,
            timeout_ms,
        )
    };
    if ok == 0 {
        return Err(Win32Error::last());
    }
    Ok(bytes_read as usize)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::os::windows::io::{FromRawHandle, OwnedHandle};

    #[test]
    fn server_then_client_round_trips_bytes() {
        let name = "rusty_win32_test_pipe_basic";
        let server = create_server(
            name,
            PIPE_ACCESS_DUPLEX,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
            1,
            512,
            512,
        )
        .expect("CreateNamedPipeW should succeed");

        // The client races the server's own `connect_server` below — wait
        // for the instance to exist and be listening from a second thread,
        // exactly the shape a real coproc client/server pair would take
        // (two independent processes, here two threads for a
        // single-binary test). A raw `HANDLE` isn't `Send`, so the handle
        // crosses the thread boundary as a plain `usize` and is cast back
        // below — the value itself is just an opaque kernel-object id, safe
        // to move as an integer.
        let client_thread = std::thread::spawn(move || {
            wait_for_server(name, 5_000).expect("WaitNamedPipeW should succeed");
            open_client(name, GENERIC_READ | GENERIC_WRITE).expect("CreateFileW should succeed")
                as usize
        });

        // SAFETY: `server` is freshly created via `create_server` above,
        // not yet connected.
        unsafe { connect_server(server) }.expect("ConnectNamedPipeW should succeed");
        let client = client_thread
            .join()
            .expect("client thread should not panic") as RawHandle;

        // SAFETY: both handles are freshly created, valid, and uniquely
        // owned here — nothing else holds or will close them.
        let mut server_file =
            std::fs::File::from(unsafe { OwnedHandle::from_raw_handle(server as _) });
        let mut client_file =
            std::fs::File::from(unsafe { OwnedHandle::from_raw_handle(client as _) });

        client_file.write_all(b"rusty_win32").unwrap();
        let mut buf = [0u8; 32];
        let n = server_file.read(&mut buf).unwrap();
        assert_eq!(&buf[..n], b"rusty_win32");
    }

    /// Both `transact`/`call` tests below run their server and client
    /// halves on independent threads and wait on this bounded timeout
    /// rather than a plain `.join()` — a previous version of these tests
    /// hung a CI job for its full runner timeout (rather than failing
    /// fast) when a pipe-mode mismatch left a fully synchronous,
    /// no-timeout Win32 call blocked forever. This turns any future
    /// regression of that kind into a clear, fast test failure instead.
    const PIPE_TEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

    #[test]
    fn transact_writes_and_reads_a_response_in_one_call() {
        let name = "rusty_win32_test_pipe_transact";
        let server = create_server(
            name,
            PIPE_ACCESS_DUPLEX,
            PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE | PIPE_WAIT,
            1,
            512,
            512,
        )
        .expect("CreateNamedPipeW should succeed");
        let server = server as usize;

        let (client_tx, client_rx) = std::sync::mpsc::channel();
        let client_thread = std::thread::spawn(move || {
            wait_for_server(name, 5_000).expect("WaitNamedPipeW should succeed");
            let client = open_client(name, GENERIC_READ | GENERIC_WRITE)
                .expect("CreateFileW should succeed");
            // A freshly-opened client handle defaults to byte read mode
            // regardless of the server's own creation-time dwPipeMode —
            // TransactNamedPipe requires the calling handle itself to be
            // in message read mode, confirmed by a real ERROR_BAD_PIPE
            // (230) from a CI run without this call.
            // SAFETY: `client` is a freshly opened, valid named-pipe
            // handle.
            unsafe { set_pipe_mode(client, PIPE_READMODE_MESSAGE) }
                .expect("SetNamedPipeHandleState should succeed");
            let mut response = [0u8; 4];
            // SAFETY: `client` is now a message-read-mode, message-type,
            // duplex named-pipe handle; this is the operation under test.
            let bytes_read = unsafe { transact(client, b"ping", &mut response) }
                .expect("TransactNamedPipe should succeed");
            let _ = client_tx.send((client as usize, bytes_read, response));
        });

        let (server_tx, server_rx) = std::sync::mpsc::channel();
        let server_thread = std::thread::spawn(move || {
            let server = server as RawHandle;
            // SAFETY: `server` is freshly created via `create_server`
            // above, not yet connected.
            unsafe { connect_server(server) }.expect("ConnectNamedPipeW should succeed");
            // SAFETY: `server` is freshly created, valid, and uniquely
            // owned here — nothing else holds or will close it.
            let mut server_file =
                std::fs::File::from(unsafe { OwnedHandle::from_raw_handle(server as _) });
            let mut request = [0u8; 4];
            server_file
                .read_exact(&mut request)
                .expect("reading the client's request should succeed");
            server_file
                .write_all(b"pong")
                .expect("writing the response should succeed");
            let _ = server_tx.send(request);
        });

        let request = server_rx
            .recv_timeout(PIPE_TEST_TIMEOUT)
            .expect("server side timed out — possible named-pipe deadlock");
        assert_eq!(&request, b"ping");
        let (client, bytes_read, response) = client_rx
            .recv_timeout(PIPE_TEST_TIMEOUT)
            .expect("client side timed out — possible named-pipe deadlock");
        assert_eq!(bytes_read, 4);
        assert_eq!(&response, b"pong");

        server_thread
            .join()
            .expect("server thread should not panic");
        client_thread
            .join()
            .expect("client thread should not panic");

        // SAFETY: `client` is still a valid, currently-open handle, closed
        // exactly once and not used again after this.
        unsafe { crate::handle::close(client as RawHandle).unwrap() };
    }

    #[test]
    fn call_completes_a_request_response_round_trip_without_a_kept_open_handle() {
        let name = "rusty_win32_test_pipe_call";
        let server = create_server(
            name,
            PIPE_ACCESS_DUPLEX,
            PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE | PIPE_WAIT,
            1,
            512,
            512,
        )
        .expect("CreateNamedPipeW should succeed");
        let server = server as usize;

        let (client_tx, client_rx) = std::sync::mpsc::channel();
        let client_thread = std::thread::spawn(move || {
            let mut response = [0u8; 4];
            let bytes_read =
                call(name, b"ping", &mut response, 5_000).expect("CallNamedPipeW should succeed");
            let _ = client_tx.send((bytes_read, response));
        });

        let (server_tx, server_rx) = std::sync::mpsc::channel();
        let server_thread = std::thread::spawn(move || {
            let server = server as RawHandle;
            // SAFETY: `server` is freshly created via `create_server`
            // above, not yet connected. `CallNamedPipeW` itself calls
            // `WaitNamedPipeW` internally, so no separate wait is needed
            // on the client side.
            unsafe { connect_server(server) }.expect("ConnectNamedPipeW should succeed");
            // SAFETY: `server` is freshly created, valid, and uniquely
            // owned here — nothing else holds or will close it.
            let mut server_file =
                std::fs::File::from(unsafe { OwnedHandle::from_raw_handle(server as _) });
            let mut request = [0u8; 4];
            server_file
                .read_exact(&mut request)
                .expect("reading the client's request should succeed");
            server_file
                .write_all(b"pong")
                .expect("writing the response should succeed");
            let _ = server_tx.send(request);
        });

        let request = server_rx
            .recv_timeout(PIPE_TEST_TIMEOUT)
            .expect("server side timed out — possible named-pipe deadlock");
        assert_eq!(&request, b"ping");
        let (bytes_read, response) = client_rx
            .recv_timeout(PIPE_TEST_TIMEOUT)
            .expect("client side timed out — possible named-pipe deadlock");
        assert_eq!(bytes_read, 4);
        assert_eq!(&response, b"pong");

        server_thread
            .join()
            .expect("server thread should not panic");
        client_thread
            .join()
            .expect("client thread should not panic");
    }

    #[test]
    fn connect_server_succeeds_even_if_the_client_already_connected() {
        let name = "rusty_win32_test_pipe_already_connected";
        let server = create_server(
            name,
            PIPE_ACCESS_DUPLEX,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
            1,
            512,
            512,
        )
        .expect("CreateNamedPipeW should succeed");

        // Connect the client BEFORE calling `connect_server` below — that's
        // exactly the race this test wants, so `ConnectNamedPipe` hits its
        // documented `ERROR_PIPE_CONNECTED` path rather than actually
        // blocking.
        let client = open_client(name, GENERIC_READ | GENERIC_WRITE)
            .expect("CreateFileW should succeed once the server instance exists");

        // SAFETY: `server` is a valid, currently-open server handle; this
        // is the specific documented "already connected" case under test.
        unsafe { connect_server(server) }.expect(
            "connect_server should treat an already-connected client as success, not an error",
        );

        // SAFETY: both handles are freshly created and valid.
        let _server_file =
            std::fs::File::from(unsafe { OwnedHandle::from_raw_handle(server as _) });
        let _client_file =
            std::fs::File::from(unsafe { OwnedHandle::from_raw_handle(client as _) });
    }

    #[test]
    fn disconnect_server_allows_reuse_by_a_second_client() {
        let name = "rusty_win32_test_pipe_disconnect_reuse";
        let server = create_server(
            name,
            PIPE_ACCESS_DUPLEX,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
            1,
            512,
            512,
        )
        .expect("CreateNamedPipeW should succeed");

        let first_client_thread = std::thread::spawn(move || {
            wait_for_server(name, 5_000).expect("WaitNamedPipeW should succeed");
            open_client(name, GENERIC_READ | GENERIC_WRITE).expect("CreateFileW should succeed")
                as usize
        });
        // SAFETY: `server` is freshly created via `create_server` above, not
        // yet connected.
        unsafe { connect_server(server) }.expect("ConnectNamedPipeW should succeed");
        let first_client = first_client_thread
            .join()
            .expect("client thread should not panic") as RawHandle;

        // SAFETY: `first_client` is a freshly created, valid handle, not
        // used again after this.
        unsafe { crate::handle::close(first_client).unwrap() };
        // SAFETY: `server` is a valid, currently-connected server handle;
        // this is the operation under test.
        unsafe { disconnect_server(server) }.expect("DisconnectNamedPipe should succeed");

        // A second client connecting to the *same* server handle is exactly
        // the scenario this primitive exists for — without it, a served
        // instance is single-use only.
        let second_client_thread = std::thread::spawn(move || {
            wait_for_server(name, 5_000).expect("WaitNamedPipeW should succeed");
            open_client(name, GENERIC_READ | GENERIC_WRITE).expect("CreateFileW should succeed")
                as usize
        });
        // SAFETY: `server` is the same handle, now reset by `disconnect_server`.
        unsafe { connect_server(server) }
            .expect("ConnectNamedPipeW should succeed for the reused server instance");
        let second_client = second_client_thread
            .join()
            .expect("client thread should not panic") as RawHandle;

        // SAFETY: both handles are valid and each closed exactly once.
        unsafe {
            crate::handle::close(server).unwrap();
            crate::handle::close(second_client).unwrap();
        }
    }

    #[test]
    fn set_pipe_mode_succeeds_on_a_connected_pipe() {
        let name = "rusty_win32_test_pipe_set_mode";
        let server = create_server(
            name,
            PIPE_ACCESS_DUPLEX,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
            1,
            512,
            512,
        )
        .expect("CreateNamedPipeW should succeed");

        let client_thread = std::thread::spawn(move || {
            wait_for_server(name, 5_000).expect("WaitNamedPipeW should succeed");
            open_client(name, GENERIC_READ | GENERIC_WRITE).expect("CreateFileW should succeed")
                as usize
        });
        // SAFETY: `server` is freshly created via `create_server` above, not
        // yet connected.
        unsafe { connect_server(server) }.expect("ConnectNamedPipeW should succeed");
        let client = client_thread
            .join()
            .expect("client thread should not panic") as RawHandle;

        // SAFETY: `client` is a freshly created, valid named-pipe handle;
        // this is the operation under test — switching the client's read
        // mode to non-blocking.
        unsafe { set_pipe_mode(client, PIPE_READMODE_BYTE | PIPE_NOWAIT) }
            .expect("SetNamedPipeHandleState should succeed");

        // SAFETY: both handles are valid and each closed exactly once.
        unsafe {
            crate::handle::close(server).unwrap();
            crate::handle::close(client).unwrap();
        }
    }

    #[test]
    fn pipe_info_reports_server_and_client_ends_correctly() {
        let name = "rusty_win32_test_pipe_info";
        let server = create_server(
            name,
            PIPE_ACCESS_DUPLEX,
            PIPE_TYPE_MESSAGE | PIPE_READMODE_BYTE | PIPE_WAIT,
            1,
            512,
            256,
        )
        .expect("CreateNamedPipeW should succeed");

        let client_thread = std::thread::spawn(move || {
            wait_for_server(name, 5_000).expect("WaitNamedPipeW should succeed");
            open_client(name, GENERIC_READ | GENERIC_WRITE).expect("CreateFileW should succeed")
                as usize
        });
        // SAFETY: `server` is freshly created via `create_server` above, not
        // yet connected.
        unsafe { connect_server(server) }.expect("ConnectNamedPipeW should succeed");
        let client = client_thread
            .join()
            .expect("client thread should not panic") as RawHandle;

        // SAFETY: `server` is a valid, currently-open named-pipe handle;
        // this is the operation under test.
        let server_info = unsafe { pipe_info(server) }.expect("GetNamedPipeInfo should succeed");
        assert!(server_info.is_server_end);
        assert!(server_info.is_message_type);
        assert_eq!(server_info.out_buffer_size, 512);
        assert_eq!(server_info.in_buffer_size, 256);
        assert_eq!(server_info.max_instances, 1);

        // SAFETY: `client` is a valid, currently-open named-pipe handle.
        let client_info = unsafe { pipe_info(client) }.expect("GetNamedPipeInfo should succeed");
        assert!(!client_info.is_server_end);
        assert!(client_info.is_message_type);

        // SAFETY: both handles are valid and each closed exactly once.
        unsafe {
            crate::handle::close(server).unwrap();
            crate::handle::close(client).unwrap();
        }
    }

    #[test]
    fn wait_for_server_fails_for_a_pipe_with_no_server_instance() {
        // No `create_server` call for this name anywhere — a stable,
        // deterministic "this pipe simply doesn't exist" case, matching
        // `WaitNamedPipeW`'s own documented immediate-failure behavior
        // (distinct from a real timeout, which means instances exist but
        // are all busy).
        let err = wait_for_server("rusty_win32_test_pipe_never_created", 100).unwrap_err();
        assert_eq!(err, Win32Error::ERROR_FILE_NOT_FOUND);
    }
}
