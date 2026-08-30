//! ConPTY primitives (RFC v2 R5+, D13, convergence roadmap Phase 7,
//! rustils#83, part 2/2 following #82's Linux backend): pseudo console
//! creation, spawning attached to it, resize, and drain-before-close
//! teardown.
//!
//! **Attach happens only at `CreateProcessW` time.** Unlike Unix, there
//! is no Win32 call to attach an already-created pseudo console to an
//! already-running process — `PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE` only
//! exists as a `STARTUPINFOEXW` attribute passed to `CreateProcessW`
//! itself. See `docs/design-discussion-pty.md`'s "shape question"
//! section for why `platform::pty::Pty::spawn` is one atomic operation
//! rather than separable open/attach steps.
//!
//! **Two separate handles, not one fd.** ConPTY's master side is a pair
//! of anonymous pipes — an input pipe the master writes to, an output
//! pipe the master reads from — not a single bidirectional descriptor
//! the way a Unix pty master fd is. `WindowsPtyMaster` (crate root)
//! holds both; there is no honest single-handle `AsHandle`/`AsRawHandle`
//! impl to offer the way `LinuxPtyMaster`'s `AsFd`/`AsRawFd` does, so it
//! isn't attempted — two named accessors instead (see that type's own
//! doc comment).
//!
//! **`ClosePseudoConsole` can deadlock against an un-drained output
//! pipe.** It blocks until conhost's internal writer thread finishes,
//! which can itself be blocked writing into a full pipe nobody is
//! reading — the EOF-vs-exit teardown lesson D13 already flagged.
//! [`close`] drains the output pipe with a bounded, non-blocking
//! `PeekNamedPipe` loop before calling `ClosePseudoConsole`.
//!
//! **ConPTY does not spontaneously EOF when the child exits** — unlike
//! a Unix pty slave, which the kernel closes automatically once its
//! last holder exits. [`spawn_exit_watcher`] is the one place this
//! module does need a background thread: it watches the child and
//! forces `ClosePseudoConsole` once it exits, so `PtyMaster::read`'s own
//! documented `Ok(0)`-on-child-exit contract holds on Windows too, not
//! just Unix. Unlike [`close`], it does *not* drain the output pipe
//! first — see its own doc comment for why draining there raced (and
//! lost real output to) a caller's own concurrent reads, and the
//! shared-close guard against `WindowsPtyMaster::drop` also reaching
//! `ClosePseudoConsole`.

#![allow(unsafe_code)]

use std::ffi::OsStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use platform::error::{ErrorKind, OsCode, PlatformError, Result};
use platform::process::EnvSpec;
use platform::term::WinSize;

use crate::ffi::win32_surface as w;
use crate::sys::errmap;
use crate::sys::handle::{self, OwnedWinHandle};
use crate::sys::proc;
use crate::util::wide::to_wide_nul;

/// Track W (D-15): `rusty_win32::handle::create_pipe`. Direct match, no
/// `set_inheritable` dance needed the way `sys::proc::make_pipe`'s
/// migration needed one — the donor's `create_pipe` already creates
/// *both* ends non-inheritable by default, exactly what the windows-sys
/// arm below achieves by passing null `SECURITY_ATTRIBUTES` (Windows'
/// own default). ConPTY never needs either end inheritable in the first
/// place, unlike a spawned child's stdio pipes — see this function's own
/// prior doc note on that.
#[cfg(feature = "track-w")]
fn create_pipe_pair() -> Result<(OwnedWinHandle, OwnedWinHandle)> {
    let (read, write) =
        rusty_win32::handle::create_pipe().map_err(|e| errmap::trackw_err("CreatePipe", e))?;
    let read = OwnedWinHandle::from_raw(read)
        .ok_or_else(|| PlatformError::new(ErrorKind::Other, OsCode::None, "CreatePipe"))?;
    let write = OwnedWinHandle::from_raw(write)
        .ok_or_else(|| PlatformError::new(ErrorKind::Other, OsCode::None, "CreatePipe"))?;
    Ok((read, write))
}

#[cfg(not(feature = "track-w"))]
fn create_pipe_pair() -> Result<(OwnedWinHandle, OwnedWinHandle)> {
    let mut read: w::HANDLE = std::ptr::null_mut();
    let mut write: w::HANDLE = std::ptr::null_mut();
    // SAFETY: `read`/`write` are valid out-pointers; null security
    // attributes and a zero buffer-size hint are both documented-valid
    // (the OS chooses a default buffer size). Unlike `sys::proc`'s own
    // `make_pipe`, neither end needs to be inheritable — ConPTY consumes
    // the ends handed to it directly (`create_pty` closes them right
    // after `CreatePseudoConsole` returns; see Microsoft's own sample,
    // which does the same), not via child-process handle inheritance.
    let ok = unsafe { w::CreatePipe(&mut read, &mut write, std::ptr::null(), 0) };
    if ok == 0 {
        return Err(errmap::last_win32_err("CreatePipe", OsStr::new("")));
    }
    let read = OwnedWinHandle::from_raw(read)
        .ok_or_else(|| PlatformError::new(ErrorKind::Other, OsCode::None, "CreatePipe"))?;
    let write = OwnedWinHandle::from_raw(write)
        .ok_or_else(|| PlatformError::new(ErrorKind::Other, OsCode::None, "CreatePipe"))?;
    Ok((read, write))
}

/// Create a pseudo console of `size`. Returns `(hpc, master_input,
/// master_output)` — `master_input` is the write end the master writes
/// keystrokes/input to (ConPTY reads the other end), `master_output` is
/// the read end the master reads the child's output from (ConPTY writes
/// the other end).
pub fn create_pty(size: WinSize) -> Result<(w::HPCON, OwnedWinHandle, OwnedWinHandle)> {
    let (conpty_input, master_input) = create_pipe_pair()?;
    let (master_output, conpty_output) = create_pipe_pair()?;

    let result = create_pseudo_console(&conpty_input, &conpty_output, size);
    // conhost duplicates what it needs internally — these ends close now
    // regardless of outcome, matching Microsoft's own ConPTY sample.
    // Preserved across the Track W split below: `result` is computed
    // first and only inspected after both drops run.
    drop(conpty_input);
    drop(conpty_output);
    let hpc = result?;

    Ok((hpc, master_input, master_output))
}

/// `CreatePseudoConsole` — Track W (D-15):
/// `rusty_win32::conpty::create_with_flags(.., 0)` (the flags-taking
/// sibling of `create`, requesting no optional behavior — same as this
/// crate's own hardcoded `dwFlags = 0`).
///
/// Two boundary conversions, neither a reinterpretation of anything: the
/// donor's `Coord` (`x`/`y`, lowercase) versus this crate's `w::COORD`
/// (`X`/`Y`) are the same two `i16` fields under different field-name
/// conventions; the donor's `Hpcon` (`*mut c_void`) versus this crate's
/// `w::HPCON` (`isize`) are the OS's single pointer-sized opaque handle
/// value in two different Rust types. `input`/`output` need no
/// conversion at all — the donor's `RawHandle` and this crate's
/// `w::HANDLE` are both bare `*mut c_void`.
#[cfg(feature = "track-w")]
fn create_pseudo_console(
    input: &OwnedWinHandle,
    output: &OwnedWinHandle,
    size: WinSize,
) -> Result<w::HPCON> {
    let coord = rusty_win32::console::Coord {
        x: size.cols as i16,
        y: size.rows as i16,
    };
    // SAFETY: `input`/`output` are valid, open pipe handles for the
    // duration of this call — `create_with_flags`'s whole safety
    // contract.
    let hpc = unsafe {
        rusty_win32::conpty::create_with_flags(input.as_raw(), output.as_raw(), coord, 0)
    }
    .map_err(|e| errmap::trackw_err("CreatePseudoConsole", e))?;
    Ok(hpc as w::HPCON)
}

#[cfg(not(feature = "track-w"))]
fn create_pseudo_console(
    input: &OwnedWinHandle,
    output: &OwnedWinHandle,
    size: WinSize,
) -> Result<w::HPCON> {
    let coord = w::COORD {
        X: size.cols as i16,
        Y: size.rows as i16,
    };
    let mut hpc: w::HPCON = 0;
    // SAFETY: `input`/`output` are valid open pipe handles for the
    // duration of this call; `hpc` is a valid out-pointer.
    let hr = unsafe { w::CreatePseudoConsole(coord, input.as_raw(), output.as_raw(), 0, &mut hpc) };
    if hr < 0 {
        return Err(errmap::hresult_err(hr, "CreatePseudoConsole"));
    }
    Ok(hpc)
}

/// Spawn `command_line` (winargv-built, not yet NUL-terminated) attached
/// to `hpc` as its console, with working directory `cwd`.
///
/// **No Job Object grouping here.** An earlier version of this function
/// suspected Job Object assignment as the cause of a real, then-open bug
/// (child console I/O never reaching the pseudo console's pipes,
/// leaking to the spawning process's own ambient console instead) and
/// removed it to test that hypothesis; live CI re-verification showed
/// the identical failure with or without a Job Object, disproving it —
/// see the `STARTF_USESTDHANDLES` comment below in this function's body
/// for the actual root cause and fix. The removal itself has not been
/// reverted (Job Object grouping is orthogonal to the actual bug and can
/// come back separately if `kill_tree` support on a pty-hosted `Child`
/// is wanted), so `kill_tree` stays `Unsupported` on Windows for pty
/// children — a real, deliberate scope reduction from
/// `platform::pty::Pty::spawn`'s stated contract, not silently missing.
/// Returns `(process, pid)`.
///
/// **Stays on windows-sys in both Track W configurations (D-15).** The
/// attribute-list trio (`InitializeProcThreadAttributeList`/
/// `UpdateProcThreadAttribute`/`DeleteProcThreadAttributeList`) and the
/// `CreateProcessW` call below are the two pieces `sys::pty` could not
/// migrate when the rest of this module did (rustils#109); both are
/// closed against the donor's *current shape*, on concrete grounds, not
/// deferred:
///
/// - **`AttributeList`'s raw pointer is unreachable.** The donor's
///   `conpty::AttributeList` covers `init`/`update`/`delete`/`Drop`
///   correctly, but the one accessor that would let a caller outside
///   `rusty_win32` embed it into a `STARTUPINFOEXW` —
///   `AttributeList::as_mut_ptr` — is `pub(crate)`, not `pub`. This is a
///   genuine upstream gap, not a design mismatch: were it `pub`, the
///   attribute-list trio above would migrate cleanly, since
///   `spawn_suspended_with_pseudoconsole` already proves the sequence
///   works. Nothing to file against *this* crate; it is rusty_win32's
///   own visibility to widen if a future slice wants it.
/// - **`CreateProcessW` itself has the same three blockers `sys::proc::
///   spawn`'s own doc comment records** (no per-spawn std-handle
///   override, no `lpCurrentDirectory`, `&str` where winargv produces
///   `&[u16]`) **plus a fourth, specific to this call site**: the donor's
///   `spawn_suspended_with_pseudoconsole` hardcodes `CREATE_SUSPENDED`
///   with no accompanying `resume()` call or suspended-thread handle in
///   its return type, where this function deliberately spawns running
///   (see the `CREATE_SUSPENDED` comment further down, in this
///   function's own body) — adopting it would mean either restructuring
///   this function's contract to expose a resume step, or discarding the
///   donor's own thread handle and silently leaking a suspended child if
///   nothing ever resumes it.
/// - **Most load-bearing of all**: this function's `STARTF_USESTDHANDLES`
///   fix (below, in its own extensively-commented body) is a real
///   Windows kernel workaround this crate discovered through live CI
///   failures against `microsoft/terminal` discussion #15814, not
///   documented anywhere Microsoft ships. The donor's
///   `spawn_suspended_with_pseudoconsole` predates that discovery and
///   does not have it. Adopting the donor's `CreateProcessW` sequence
///   as-is would very likely **reintroduce** the exact bug this function
///   exists to avoid — a live console-handle leak from a CI runner's own
///   redirected stdio into every ConPTY-hosted child, silent until a
///   test actually reads output and times out. This is not a preference
///   difference like the other three; it is a correctness regression
///   this repo has already paid to find once.
pub fn spawn_attached(
    hpc: w::HPCON,
    command_line: &[u16],
    cwd: &OsStr,
    env: &EnvSpec,
) -> Result<(OwnedWinHandle, u32)> {
    let mut line: Vec<u16> = command_line.to_vec();
    line.push(0);
    let cwd_w = to_wide_nul(cwd);
    let block = proc::env_block(env);

    let mut attr_list_size: usize = 0;
    // SAFETY: a null attribute-list pointer with count=1 is the
    // documented way to query the required buffer size — this call is
    // *expected* to report failure here; only `attr_list_size` (written
    // regardless of the return value, per the documented query mode) is
    // read afterward.
    unsafe {
        w::InitializeProcThreadAttributeList(std::ptr::null_mut(), 1, 0, &mut attr_list_size);
    }
    if attr_list_size == 0 {
        return Err(errmap::last_win32_err(
            "InitializeProcThreadAttributeList",
            OsStr::new(""),
        ));
    }
    let mut attr_buf: Vec<u8> = vec![0u8; attr_list_size];
    let attr_list: w::LPPROC_THREAD_ATTRIBUTE_LIST = attr_buf.as_mut_ptr().cast();
    // SAFETY: `attr_list` points to a buffer of exactly `attr_list_size`
    // bytes, sized by the query above, outliving every use of
    // `attr_list` in this function (owned by the still-in-scope
    // `attr_buf`).
    let ok = unsafe { w::InitializeProcThreadAttributeList(attr_list, 1, 0, &mut attr_list_size) };
    if ok == 0 {
        return Err(errmap::last_win32_err(
            "InitializeProcThreadAttributeList",
            OsStr::new(""),
        ));
    }

    // `DeleteProcThreadAttributeList` must run on every exit path from
    // here on — computed as a value first, cleaned up once after,
    // rather than duplicating the call at every early return.
    let result = (|| -> Result<(OwnedWinHandle, u32)> {
        // `PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE`'s `lpValue` is the raw
        // `HPCON` value itself, reinterpreted as the pointer-sized
        // argument — not a pointer *to* a variable holding it. This
        // matches Microsoft's own ConPTY sample exactly
        // (`UpdateProcThreadAttribute(..., hPC, sizeof(HPCON), ...)`,
        // `hPC` passed by value); passing `&hpc` here instead (the
        // address of the local variable) was this issue's actual first
        // bug — it compiled, `UpdateProcThreadAttribute` and
        // `CreateProcessW` both reported success, and the child process
        // spawned and exited normally, but with no pseudo console
        // genuinely attached at all: every live test that depends on
        // reading real child output timed out having seen zero bytes,
        // while the tests that never read output (resize, the
        // drop-without-draining teardown check) passed — caught by CI,
        // not by inspection, exactly the kind of thing this untestable
        // (no local Windows execution) code needed live verification for.
        //
        // SAFETY: `attr_list` was just successfully initialized above;
        // `hpc` is a valid, live pseudo console handle for the duration
        // of this call (the caller's — it outlives the spawned process).
        let ok = unsafe {
            w::UpdateProcThreadAttribute(
                attr_list,
                0,
                w::PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE as usize,
                hpc as *const core::ffi::c_void,
                std::mem::size_of::<w::HPCON>(),
                std::ptr::null_mut(),
                std::ptr::null(),
            )
        };
        if ok == 0 {
            return Err(errmap::last_win32_err(
                "UpdateProcThreadAttribute",
                OsStr::new(""),
            ));
        }

        // SAFETY: STARTUPINFOEXW is plain-old-data for which all-zeroes
        // is a valid starting value; `cb`/`lpAttributeList`/`dwFlags` are
        // set before use.
        let mut siex: w::STARTUPINFOEXW = unsafe { std::mem::zeroed() };
        siex.StartupInfo.cb = std::mem::size_of::<w::STARTUPINFOEXW>() as u32;
        siex.lpAttributeList = attr_list;
        // The actual root cause, found via `microsoft/terminal` discussion
        // #15814 (maintainer DHowett) after `CREATE_SUSPENDED` removal,
        // Job Object removal, test serialization, and a shell-vs-no-shell
        // diagnostic all failed to change this bug's failure signature:
        // when the *spawning* process's own standard handles are
        // themselves redirected — exactly `cargo test`'s situation under
        // any CI runner, whose stdout/stderr are piped/captured rather
        // than a real interactive console — the kernel still duplicates
        // those redirected handles into the child by default, even with
        // `bInheritHandles = FALSE` and `PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE`
        // set. That duplication is legacy console-handle-inheritance
        // behavior the pseudo-console attribute alone does not suppress —
        // a real Windows kernel gap, not a mistake in this function's
        // sequence (which already matched Microsoft's own ConPTY sample
        // byte-for-byte; live CI logs confirmed every child's real output
        // was reaching the CI job's own ambient/redirected console
        // instantly, never blocking on our unread pipes, while our own
        // pipe read timed out having seen only conhost's initial VT-mode
        // negotiation — proof the attribute was being silently ignored,
        // not merely racing). Setting `STARTF_USESTDHANDLES` with null
        // std handles is the documented-by-maintainer workaround: it
        // tells `CreateProcessW` to use the (null) handles given here
        // instead of auto-duplicating the parent's own, which is exactly
        // what stops the leak — the child then has no legacy std handles
        // at all, and genuinely falls through to the pseudo console
        // wired via the attribute list instead.
        siex.StartupInfo.dwFlags |= w::STARTF_USESTDHANDLES;

        // Deliberately **not** `CREATE_SUSPENDED`, matching Microsoft's
        // own ConPTY sample (which creates the process running, no
        // suspend step). This crate's own `sys::proc::spawn` suspends
        // specifically to make `NewGroup`'s Job Object membership
        // race-free before the child's first instruction — an earlier
        // version of this function layered that same convention on top
        // here too, hypothesizing it as the cause of a real, still-open
        // bug (child console I/O not reaching the pseudo console's
        // pipes, timing out with only conhost's own initial VT-mode
        // negotiation ever received). Removing `CREATE_SUSPENDED` alone
        // turned out **not** to fix it — live CI re-verified this,
        // byte-for-byte the same failure with or without the flag — so
        // that specific hypothesis was disproven, not confirmed; the
        // flag stays removed only because it still matches the reference
        // sample more closely, not because it was shown to matter. The
        // real cause turned out to be upstream of this flag entirely —
        // see the `STARTF_USESTDHANDLES` comment above.
        let mut flags = w::EXTENDED_STARTUPINFO_PRESENT;
        if block.is_some() {
            flags |= w::CREATE_UNICODE_ENVIRONMENT;
        }
        let env_ptr = block
            .as_ref()
            .map_or(std::ptr::null(), |b| b.as_ptr().cast::<core::ffi::c_void>());

        // SAFETY: PROCESS_INFORMATION is plain-old-data; CreateProcessW
        // overwrites it on success before it's read.
        let mut pi: w::PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
        let si_ptr = (&siex as *const w::STARTUPINFOEXW).cast::<w::STARTUPINFOW>();
        // SAFETY: `line` is an owned, mutable, NUL-terminated UTF-16
        // buffer (CreateProcessW may write into it); `cwd_w` is
        // NUL-terminated and outlives the call; `env_ptr` is null
        // (inherit) or a well-formed double-NUL block per
        // `proc::env_block`; `si_ptr` points at a fully initialized
        // `STARTUPINFOEXW` whose leading `STARTUPINFOW.cb` is set to the
        // *extended* struct's size — the documented way `CreateProcessW`
        // recognizes `EXTENDED_STARTUPINFO_PRESENT` and reads
        // `lpAttributeList`; `pi` is a valid out-pointer;
        // `bInheritHandles = FALSE` — ConPTY wiring goes through the
        // attribute list, not inherited std handles, matching
        // Microsoft's own sample; null security attributes and
        // application name are documented-valid.
        let ok = unsafe {
            w::CreateProcessW(
                std::ptr::null(),
                line.as_mut_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                0,
                flags,
                env_ptr,
                cwd_w.as_ptr(),
                si_ptr,
                &mut pi,
            )
        };
        if ok == 0 {
            return Err(errmap::last_win32_err("CreateProcessW", cwd));
        }

        let process = OwnedWinHandle::from_raw(pi.hProcess)
            .ok_or_else(|| PlatformError::new(ErrorKind::Other, OsCode::None, "CreateProcessW"))?;
        // The main thread runs immediately (not suspended, see above) —
        // its handle is not retained, mirroring `sys::proc::spawn`'s own
        // non-grouped path.
        drop(OwnedWinHandle::from_raw(pi.hThread));

        // No Job Object creation/assignment here — see this function's
        // own doc comment and the `CREATE_SUSPENDED` comment above: this
        // step is the last structural difference between this spawn and
        // Microsoft's own ConPTY sample, and is under direct test as the
        // suspected cause of the still-open console-I/O-doesn't-reach-
        // the-pipes bug (two other hypotheses were tested and disproven
        // first). If removing it fixes the bug, `kill_tree` on a
        // pty-hosted child stays `Unsupported` on Windows as a real,
        // documented scope reduction; if not, this comes back and the
        // investigation continues elsewhere.
        Ok((process, pi.dwProcessId))
    })();

    // SAFETY: `attr_list` was successfully initialized above and is
    // destroyed exactly once here, on every exit path — `attr_buf`
    // (which owns the memory `attr_list` points into) is still in scope.
    unsafe {
        w::DeleteProcThreadAttributeList(attr_list);
    }

    result
}

/// `ResizePseudoConsole` — `TIOCSWINSZ`'s ConPTY counterpart.
///
/// Track W (D-15): `rusty_win32::conpty::resize`. Same `Coord`/`Hpcon`
/// boundary conversions as [`create_pseudo_console`].
#[cfg(feature = "track-w")]
pub fn resize(hpc: w::HPCON, size: WinSize) -> Result<()> {
    let coord = rusty_win32::console::Coord {
        x: size.cols as i16,
        y: size.rows as i16,
    };
    // SAFETY: `hpc` is a valid, live pseudo console handle owned by the
    // caller for the duration of this call — `resize`'s whole safety
    // contract.
    unsafe { rusty_win32::conpty::resize(hpc as rusty_win32::conpty::Hpcon, coord) }
        .map_err(|e| errmap::trackw_err("ResizePseudoConsole", e))
}

#[cfg(not(feature = "track-w"))]
pub fn resize(hpc: w::HPCON, size: WinSize) -> Result<()> {
    let coord = w::COORD {
        X: size.cols as i16,
        Y: size.rows as i16,
    };
    // SAFETY: `hpc` is a valid, live pseudo console handle owned by the
    // caller for the duration of this call.
    let hr = unsafe { w::ResizePseudoConsole(hpc, coord) };
    if hr < 0 {
        return Err(errmap::hresult_err(hr, "ResizePseudoConsole"));
    }
    Ok(())
}

/// `ClosePseudoConsole` — Track W (D-15): `rusty_win32::conpty::close`.
/// Shared by [`spawn_exit_watcher`] and [`close`] so the boundary
/// conversion lives once.
///
/// # Safety
///
/// `hpc` must be a currently-open, valid pseudo console handle, not
/// already closed.
#[cfg(feature = "track-w")]
unsafe fn close_pseudo_console(hpc: w::HPCON) {
    // SAFETY: forwards this function's own safety contract; `hpc as
    // Hpcon` is the same boundary conversion [`create_pseudo_console`]
    // documents, applied in the other direction.
    unsafe { rusty_win32::conpty::close(hpc as rusty_win32::conpty::Hpcon) }
}

#[cfg(not(feature = "track-w"))]
unsafe fn close_pseudo_console(hpc: w::HPCON) {
    // SAFETY: forwards this function's own safety contract.
    unsafe { w::ClosePseudoConsole(hpc) }
}

/// Poll (via `PeekNamedPipe`, non-blocking) until `output` has data
/// ready to read without blocking, the pipe reports broken (the
/// subsequent `ReadFile` would return `Ok(0)`), or `budget` elapses.
/// `true` in the first two cases — a following `ReadFile` on `output`
/// is guaranteed to return promptly either way; `false` if `budget` ran
/// out with the pipe still open and empty. Exists so a caller that
/// wants a *bounded* read (rustils#83's own integration tests, which
/// need one — see `platform-windows/tests/pty.rs`'s own doc comment for
/// why) can gate a `ReadFile` call itself without this module or
/// [`PtyMaster::read`](platform::pty::PtyMaster::read) needing to grow
/// a timeout parameter neither's contract otherwise calls for.
///
/// Track W (D-15): `rusty_win32::handle::pipe_bytes_available` reports a
/// `PeekNamedPipe` failure as `Err` rather than the raw call's `ok == 0`;
/// folded back to the identical `true` result either way — a broken pipe
/// is exactly the "a following `ReadFile` returns promptly" case this
/// function's own contract already documents, whichever arm answers.
#[cfg(feature = "track-w")]
pub fn wait_readable(output: &OwnedWinHandle, budget: std::time::Duration) -> bool {
    let deadline = std::time::Instant::now() + budget;
    loop {
        // SAFETY: `output` is a valid open pipe handle for the duration
        // of this call.
        match unsafe { rusty_win32::handle::pipe_bytes_available(output.as_raw()) } {
            Ok(available) if available > 0 => return true,
            Err(_) => return true,
            Ok(_) => {}
        }
        if std::time::Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

#[cfg(not(feature = "track-w"))]
pub fn wait_readable(output: &OwnedWinHandle, budget: std::time::Duration) -> bool {
    let deadline = std::time::Instant::now() + budget;
    loop {
        let mut available: u32 = 0;
        // SAFETY: `output` is a valid open pipe handle; `available` is a
        // valid out-pointer; every other out-pointer is null (not
        // needed here).
        let ok = unsafe {
            w::PeekNamedPipe(
                output.as_raw(),
                std::ptr::null_mut(),
                0,
                std::ptr::null_mut(),
                &mut available,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 || available > 0 {
            return true;
        }
        if std::time::Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

/// Wait for `process` to exit, then call bare `ClosePseudoConsole` — the
/// fix for a real gap between ConPTY and this crate's own portable
/// contract (`platform::pty::PtyMaster::read`'s doc: `Ok(0)` once the
/// child has exited, "Windows's broken-pipe-on-child-exit" collapsing to
/// it just like Unix's `EIO`-on-slave-close). Unlike a Unix pty slave,
/// which the kernel closes automatically once its last holder (the
/// child) exits, ConPTY's output pipe stays open — conhost keeps it
/// alive until `ClosePseudoConsole` is explicitly called, confirmed
/// live: a child that has already exited (`WaitForSingleObject` on its
/// process handle already returned) still leaves reads blocked
/// indefinitely with no spontaneous EOF. Microsoft's own `EchoCon`
/// sample has the identical shape for the identical reason: a dedicated
/// thread watches the child and calls `ClosePseudoConsole` itself once
/// it exits, rather than relying on the pipe to self-report.
///
/// **Deliberately does not drain `output` first**, unlike [`close`] —
/// an earlier version of this function called [`close`] (drain, then
/// `ClosePseudoConsole`) here, and live CI showed exactly the failure
/// mode that invites: this thread's own drain competing with the
/// caller's concurrent `PtyMaster::read` for the *same* bytes on the
/// *same* handle, non-deterministically stealing real output out from
/// under tests that were reading it themselves (three previously-passing
/// live tests started seeing only conhost's VT-negotiation bytes,
/// exactly the "eaten by the wrong reader" shape, not a timing fluke).
/// Calling bare `ClosePseudoConsole` here instead has no such race
/// because this thread never touches `output` at all: a `ReadFile` the
/// caller has in flight (or issues next) unblocks naturally once
/// `ClosePseudoConsole` finishes closing conhost's own write-side
/// duplicate — the same broken-pipe signal `output`'s reader always
/// relies on, just triggered here instead of by `Drop`. The trade-off
/// moves, it doesn't vanish: `ClosePseudoConsole` can itself block
/// (documented — [`close`]'s own doc comment) if conhost's writer is
/// stuck behind a full, unread pipe, so this thread can stall in that
/// case until the caller's own reads relieve the backpressure. That's
/// an acceptable place for a stall: this is a detached, never-joined
/// background thread — Rust doesn't wait for it before the process
/// exits, and it isn't on any path a test or a caller's own `read`
/// blocks behind.
///
/// `closed` is a shared "the real close already ran" guard — this
/// function and [`WindowsPtyMaster::drop`](crate::pty::WindowsPtyMaster)
/// both may reach `ClosePseudoConsole` (whichever of "the child exits"
/// or "the caller drops the master first" happens first), and it must
/// run exactly once: a second call on an already-closed `HPCON` is not
/// something to risk. Whichever side loses the compare-exchange does
/// nothing further.
///
/// `process` is duplicated internally (`sys::handle::duplicate`) rather
/// than requiring the caller to hand over a handle this thread would
/// then own — the duplicate's lifetime is this thread's own, independent
/// of whatever `WindowsChild` does with its own handle to the same
/// process. If duplication itself fails, this is a silent no-op: the
/// existing `Drop`-triggered [`close`] remains the fallback path (EOF
/// then only surfaces once the caller drops the master, the pre-fix
/// behavior), rather than failing the whole spawn over a best-effort
/// convenience.
pub fn spawn_exit_watcher(process: &OwnedWinHandle, hpc: w::HPCON, closed: Arc<AtomicBool>) {
    let Ok(dup) = handle::duplicate(process, false) else {
        return;
    };
    std::thread::spawn(move || {
        // The wait itself reuses `sys::proc::wait` (already Track W
        // migrated, D-15 slice 2) rather than a raw `WaitForSingleObject`
        // of its own — the two-armed dispatch already lives there, and
        // this call site never looks at the `Result`: like the original
        // bare `WaitForSingleObject`, only "the process is no longer
        // running" matters, not why or with what exit code. (This
        // corrects a mapping error in the issue that requested this
        // migration, rustils#109, which had proposed
        // `console::wait_readable` here — that function waits on a
        // console *input* handle, not a process handle; there never was
        // a fit.)
        let _ = proc::wait(&dup);
        if closed
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            // SAFETY: `hpc` is a valid, live pseudo console handle;
            // `closed`'s compare-exchange above guarantees this runs at
            // most once across both this thread and `WindowsPtyMaster`'s
            // own `Drop`.
            unsafe {
                close_pseudo_console(hpc);
            }
        }
    });
}

/// Drain `output` (bounded, non-blocking via `PeekNamedPipe`) then call
/// `ClosePseudoConsole` — see this module's doc comment for why the
/// drain has to happen first. The overall time budget is checked on
/// *every* iteration, not only when the pipe is momentarily empty — a
/// child that keeps producing output fast enough to always have
/// something available must not be able to make this loop (and
/// therefore `Drop`) run unbounded.
///
/// This is `WindowsPtyMaster::drop`'s own close path, not
/// [`spawn_exit_watcher`]'s — the watcher deliberately calls bare
/// `ClosePseudoConsole` instead (see its own doc comment for why
/// draining there raced a caller's own concurrent reads). `Drop`'s case
/// is different: the caller has already decided to stop reading
/// (dropping the master), so there's no concurrent reader left to race,
/// and draining first is what keeps this `ClosePseudoConsole` call from
/// deadlocking against conhost's writer if the caller never emptied the
/// pipe (`dropping_an_undrained_master_does_not_deadlock`).
///
/// Track W (D-15): the drain loop's two calls become
/// `rusty_win32::handle::pipe_bytes_available` and
/// `rusty_win32::console::read` — the same two functions
/// [`wait_readable`] and [`spawn_exit_watcher`]'s sibling code already
/// route through — and the final `ClosePseudoConsole` goes through the
/// shared [`close_pseudo_console`].
#[cfg(feature = "track-w")]
pub fn close(hpc: w::HPCON, output: &OwnedWinHandle) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    let mut buf = [0u8; 4096];
    loop {
        if std::time::Instant::now() >= deadline {
            break;
        }
        // SAFETY: `output` is a valid open pipe handle for the duration
        // of this call.
        let available = match unsafe { rusty_win32::handle::pipe_bytes_available(output.as_raw()) }
        {
            Ok(n) => n,
            // Broken pipe (conhost's write end already closed) or any
            // other failure — nothing left to drain either way.
            Err(_) => break,
        };
        if available == 0 {
            std::thread::sleep(std::time::Duration::from_millis(5));
            continue;
        }
        // SAFETY: `output` is a valid open pipe handle for the duration
        // of this call; `buf` is a valid writable region the donor
        // derives its own length from.
        if unsafe { rusty_win32::console::read(output.as_raw(), &mut buf) }.is_err() {
            break;
        }
    }
    // SAFETY: `hpc` is a valid, live pseudo console handle; closed
    // exactly once here (the sole caller, `WindowsPtyMaster::drop`,
    // never calls this twice).
    unsafe {
        close_pseudo_console(hpc);
    }
}

#[cfg(not(feature = "track-w"))]
pub fn close(hpc: w::HPCON, output: &OwnedWinHandle) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    let mut buf = [0u8; 4096];
    loop {
        if std::time::Instant::now() >= deadline {
            break;
        }
        let mut available: u32 = 0;
        // SAFETY: `output` is a valid open pipe handle; `available` is a
        // valid out-pointer; every other out-pointer is null (not needed
        // here).
        let ok = unsafe {
            w::PeekNamedPipe(
                output.as_raw(),
                std::ptr::null_mut(),
                0,
                std::ptr::null_mut(),
                &mut available,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 {
            // Broken pipe (conhost's write end already closed) or any
            // other failure — nothing left to drain either way.
            break;
        }
        if available == 0 {
            std::thread::sleep(std::time::Duration::from_millis(5));
            continue;
        }
        let mut read_count: u32 = 0;
        // SAFETY: `output` is a valid open pipe handle; `buf` is a valid
        // writable region of exactly `buf.len()` bytes and `read_count` a
        // valid out-pointer, both outliving the call.
        let ok = unsafe {
            w::ReadFile(
                output.as_raw(),
                buf.as_mut_ptr(),
                buf.len() as u32,
                &mut read_count,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 {
            break;
        }
    }
    // SAFETY: `hpc` is a valid, live pseudo console handle; closed
    // exactly once here (the sole caller, `WindowsPtyMaster::drop`,
    // never calls this twice).
    unsafe {
        close_pseudo_console(hpc);
    }
}
