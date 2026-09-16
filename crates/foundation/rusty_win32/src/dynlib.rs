//! `LoadLibraryW`/`GetProcAddress`/`FreeLibrary` — dynamic module loading,
//! the primitive a caller needs to reach a DLL that isn't linked at build
//! time (e.g. a hardware vendor's driver-supplied loader such as
//! `vulkan-1.dll`, resolved at runtime rather than via a static `#[link]`
//! import). [`handle`] already resolves symbols out of a module the
//! process has *already* loaded (`kernel32.dll`, via `GetModuleHandleW`);
//! this module is the counterpart for a module that first has to be
//! loaded.

use crate::error::Win32Error;
use crate::handle::RawHandle;
use crate::wide::to_wide;

extern crate alloc;
use alloc::vec::Vec;

#[link(name = "kernel32")]
unsafe extern "system" {
    fn LoadLibraryW(file_name: *const u16) -> RawHandle;
    fn GetProcAddress(module: RawHandle, proc_name: *const u8) -> *mut core::ffi::c_void;
    fn FreeLibrary(module: RawHandle) -> i32;
}

/// Loads `name` (e.g. `"vulkan-1.dll"`) via `LoadLibraryW`, searching the
/// standard DLL search order.
pub fn load_library(name: &str) -> Result<RawHandle, Win32Error> {
    let wide = to_wide(name)?;
    // SAFETY: `wide` is a valid null-terminated UTF-16 string for the
    // duration of this call.
    let handle = unsafe { LoadLibraryW(wide.as_ptr()) };
    if handle.is_null() {
        return Err(Win32Error::last());
    }
    Ok(handle)
}

/// Resolves an exported symbol's address from an already-loaded module
/// (as returned by [`load_library`]), or `Ok(None)` if `name` isn't
/// exported. `name` must be ASCII, per `GetProcAddress`'s narrow-string
/// contract.
///
/// Rejects any embedded `'\0'` in `name` with
/// [`Win32Error::ERROR_INVALID_PARAMETER`] rather than silently truncating
/// at it — `GetProcAddress` reads a single NUL-terminated narrow string,
/// so an embedded NUL would otherwise resolve a different, truncated
/// symbol with no signal to the caller. Matches this crate's `wide::to_wide`,
/// hardened against the identical shape of bug.
///
/// # Safety
///
/// `module` must be a currently-loaded module handle (from [`load_library`]
/// or otherwise), not freed before this call.
pub unsafe fn get_proc_address(
    module: RawHandle,
    name: &str,
) -> Result<Option<*mut core::ffi::c_void>, Win32Error> {
    if name.contains('\0') {
        return Err(Win32Error::ERROR_INVALID_PARAMETER);
    }
    let mut c_name: Vec<u8> = name.bytes().collect();
    c_name.push(0);
    // SAFETY: `module` is caller-supplied per this function's own safety
    // contract; `c_name` is a valid null-terminated byte string for the
    // duration of this call.
    let proc = unsafe { GetProcAddress(module, c_name.as_ptr()) };
    Ok(if proc.is_null() { None } else { Some(proc) })
}

/// Unloads a module obtained from [`load_library`], decrementing its
/// reference count.
///
/// # Safety
///
/// `module` must be a currently-loaded, valid module handle owned by the
/// caller, not used again (by this crate or anything else) after this
/// call returns.
pub unsafe fn free_library(module: RawHandle) -> Result<(), Win32Error> {
    // SAFETY: `module` is caller-supplied per this function's own safety
    // contract.
    if unsafe { FreeLibrary(module) } == 0 {
        return Err(Win32Error::last());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loading_kernel32_by_name_succeeds_and_resolves_a_known_export() {
        let module = load_library("kernel32.dll").unwrap();
        assert!(!module.is_null());
        // SAFETY: `module` was just loaded above and isn't freed until
        // after this call.
        let proc = unsafe { get_proc_address(module, "GetCurrentProcessId") };
        assert!(proc.unwrap().is_some());
        unsafe { free_library(module).unwrap() };
    }

    #[test]
    fn loading_a_nonexistent_dll_fails() {
        let err = load_library("this_dll_does_not_exist_12345.dll");
        assert!(err.is_err());
    }

    #[test]
    fn resolving_an_unknown_symbol_returns_none() {
        let module = load_library("kernel32.dll").unwrap();
        // SAFETY: `module` was just loaded above and isn't freed until
        // after this call.
        assert!(
            unsafe { get_proc_address(module, "ThisFunctionDoesNotExist12345") }
                .unwrap()
                .is_none()
        );
        unsafe { free_library(module).unwrap() };
    }

    #[test]
    fn resolving_a_symbol_with_an_embedded_nul_is_rejected_instead_of_truncated() {
        // Before this fix, `get_proc_address` faithfully null-terminated
        // whatever bytes `name` contained (`name.bytes().collect()` then
        // `push(0)`), so an embedded NUL didn't stop the encoding — but
        // `GetProcAddress` itself reads only up to the first NUL, so a
        // caller asking for `"Real\0Fake"` would silently resolve
        // `"Real"` instead, the same class of bug `wide::to_wide` guards
        // against for `*W` calls.
        let module = load_library("kernel32.dll").unwrap();
        // SAFETY: `module` was just loaded above and isn't freed until
        // after this call.
        let err = unsafe { get_proc_address(module, "GetCurrentProcessId\0Fake") }.expect_err(
            "a symbol name with an embedded NUL should be rejected, not silently truncated",
        );
        assert_eq!(err, Win32Error::ERROR_INVALID_PARAMETER);
        unsafe { free_library(module).unwrap() };
    }
}
