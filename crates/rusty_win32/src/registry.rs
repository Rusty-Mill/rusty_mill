//! Windows Registry access (`winreg.h`) — a new module added in round 2,
//! previously excluded by `ARCHITECTURE.md`'s non-goals (see
//! `docs/archive/gap-analysis.md`'s, now closed out and archived, "Round
//! 2: previously out-of-scope subsystems"
//! sweep), now in scope per explicit direction. No current `rush` feature
//! asks for this yet.
//!
//! Started with just the five predefined root keys every other registry
//! call starts from (`RegOpenKeyExW(HKEY_LOCAL_MACHINE, ...)`, etc.); this
//! piece adds [`open_key`]/[`close_key`] — the query/set/delete surface is
//! still follow-up work.
//!
//! `HKEY` gets its own [`HKey`] type rather than reusing
//! [`crate::handle::RawHandle`]: a registry key handle is closed via
//! `RegCloseKey`, not `CloseHandle`, so treating it as an interchangeable
//! `RawHandle` would invite calling the wrong close function on it.
//! `advapi32.dll` (this crate's second non-`kernel32` link, after
//! `console`'s `user32.dll`) is where every registry function lives —
//! flagged as an expected future addition in this crate's own README
//! before this module existed.

extern crate alloc;
use crate::time::Timespec;
use alloc::vec::Vec;

/// A registry key handle — `HKEY`. Closed via [`close_key`], never
/// `CloseHandle`/[`crate::handle::close`] — kept as its own type for
/// exactly that reason, distinct from [`crate::handle::RawHandle`].
pub type HKey = *mut core::ffi::c_void;

#[link(name = "advapi32")]
unsafe extern "system" {
    fn RegOpenKeyExW(
        key: HKey,
        sub_key: *const u16,
        options: u32,
        sam_desired: u32,
        result: *mut HKey,
    ) -> i32;
    fn RegCloseKey(key: HKey) -> i32;
    fn RegCreateKeyExW(
        key: HKey,
        sub_key: *const u16,
        reserved: u32,
        class: *mut u16,
        options: u32,
        sam_desired: u32,
        security_attributes: *const core::ffi::c_void,
        result: *mut HKey,
        disposition: *mut u32,
    ) -> i32;
    fn RegQueryValueExW(
        key: HKey,
        value_name: *const u16,
        reserved: *mut u32,
        value_type: *mut u32,
        data: *mut u8,
        data_size: *mut u32,
    ) -> i32;
    fn RegSetValueExW(
        key: HKey,
        value_name: *const u16,
        reserved: u32,
        value_type: u32,
        data: *const u8,
        data_size: u32,
    ) -> i32;
    fn RegDeleteValueW(key: HKey, value_name: *const u16) -> i32;
    fn RegDeleteKeyExW(key: HKey, sub_key: *const u16, sam_desired: u32, reserved: u32) -> i32;
    fn RegQueryInfoKeyW(
        key: HKey,
        class: *mut u16,
        class_len: *mut u32,
        reserved: *mut u32,
        sub_keys: *mut u32,
        max_sub_key_len: *mut u32,
        max_class_len: *mut u32,
        values: *mut u32,
        max_value_name_len: *mut u32,
        max_value_len: *mut u32,
        security_descriptor_len: *mut u32,
        last_write_time: *mut FileTime,
    ) -> i32;
    fn RegEnumValueW(
        key: HKey,
        index: u32,
        value_name: *mut u16,
        value_name_len: *mut u32,
        reserved: *mut u32,
        value_type: *mut u32,
        data: *mut u8,
        data_size: *mut u32,
    ) -> i32;
    fn RegEnumKeyExW(
        key: HKey,
        index: u32,
        name: *mut u16,
        name_len: *mut u32,
        reserved: *mut u32,
        class: *mut u16,
        class_len: *mut u32,
        last_write_time: *mut FileTime,
    ) -> i32;
    fn RegFlushKey(key: HKey) -> i32;
    fn RegDeleteTreeW(key: HKey, sub_key: *const u16) -> i32;
}

// FILETIME: `size_of` 8, `align_of` 4 on x86_64 — mirrors `time.rs`'s
// private struct of the same shape; duplicated locally rather than
// shared, matching this crate's existing per-module-locality convention
// for tiny FFI-mirror structs (`fs.rs`/`process.rs`/`job.rs`/`console.rs`
// each have their own copy too).
#[repr(C)]
#[derive(Default, Clone, Copy)]
struct FileTime {
    low: u32,
    high: u32,
}
const _: () = assert!(core::mem::size_of::<FileTime>() == 8);
const _: () = assert!(core::mem::align_of::<FileTime>() == 4);

/// 100ns ticks between the FILETIME epoch (1601-01-01) and the Unix epoch
/// (1970-01-01) — the same standard conversion constant `time.rs`/
/// `fs.rs`/`process.rs` use.
const FILETIME_UNIX_EPOCH_DIFF_100NS: i64 = 116_444_736_000_000_000;
const HUNDRED_NS_PER_SEC: i64 = 10_000_000;
const NANOS_PER_HUNDRED_NS: i64 = 100;

fn filetime_to_timespec(ft: FileTime) -> Timespec {
    let ticks_100ns =
        (i64::from(ft.high) << 32 | i64::from(ft.low)) - FILETIME_UNIX_EPOCH_DIFF_100NS;
    let secs = ticks_100ns.div_euclid(HUNDRED_NS_PER_SEC);
    let remainder_100ns = ticks_100ns.rem_euclid(HUNDRED_NS_PER_SEC);
    Timespec {
        secs,
        nanos: (remainder_100ns * NANOS_PER_HUNDRED_NS) as u32,
    }
}

/// `RegDeleteKeyExW`'s `samDesired` — on WOW64, a 32-bit process's
/// ordinary view of `HKEY_LOCAL_MACHINE\Software` is silently redirected
/// to a separate `WOW6432Node` subtree; these bits force targeting the
/// real 64-bit view or the redirected 32-bit view explicitly instead of
/// leaving it to whichever view the calling process happens to run as.
/// `0` (this crate's 64-bit-only target never actually runs under WOW64,
/// so this is mostly for API completeness) leaves it at the default.
/// Verified against mingw-w64's own `winnt.h` macros.
pub const KEY_WOW64_64KEY: u32 = 0x0100;
pub const KEY_WOW64_32KEY: u32 = 0x0200;

/// `RegCreateKeyExW`'s `dwOptions`: an ordinary, disk-persisted key —
/// this crate's only supported option (`REG_OPTION_VOLATILE`/
/// `REG_OPTION_BACKUP_RESTORE`/etc. are out of scope, same as this
/// crate's other modules only covering the common case).
const REG_OPTION_NON_VOLATILE: u32 = 0x0000_0000;
/// `RegCreateKeyExW`'s `lpdwDisposition` out-values — verified against
/// mingw-w64's own `winnt.h` macros.
const REG_OPENED_EXISTING_KEY: u32 = 0x0000_0002;

/// `REGSAM` access-mask bits for [`open_key`] (and every later registry
/// call taking an `access: u32`) — exposed raw and policy-free, matching
/// this crate's existing convention for other bitmask fields
/// (`fs::FILE_ATTRIBUTE_*`, `console::ENABLE_*`). Verified against
/// mingw-w64's own `winnt.h` macros with a compiled `_Static_assert`
/// probe, the same discipline `HKEY_CLASSES_ROOT` etc. above used.
pub const KEY_QUERY_VALUE: u32 = 0x0001;
pub const KEY_READ: u32 = 0x0002_0019;
pub const KEY_WRITE: u32 = 0x0002_0006;
pub const KEY_ALL_ACCESS: u32 = 0x000F_003F;

// The five predefined roots below are `HKEY`-typed sentinel values, not
// real object handles — Windows defines each as a small integer cast
// through `(LONG)`, i.e. a *signed 32-bit* value, before widening to the
// pointer-sized `HKEY`. On a 64-bit target that widening sign-extends:
// `0x80000000` as `LONG` is negative, so the real bit pattern is
// `0xFFFFFFFF80000000`, not `0x0000000080000000`. Verified against
// mingw-w64's own `winreg.h` macros with a compiled `_Static_assert`
// probe (`x86_64-w64-mingw32-gcc`), the same discipline this crate uses
// for every other struct/constant layout it can't check any other way.

/// Classes and file associations — a merged view of
/// `HKEY_LOCAL_MACHINE\Software\Classes` and
/// `HKEY_CURRENT_USER\Software\Classes`.
pub const HKEY_CLASSES_ROOT: HKey = 0xFFFF_FFFF_8000_0000usize as HKey;
/// The current user's own settings (their profile-scoped hive).
pub const HKEY_CURRENT_USER: HKey = 0xFFFF_FFFF_8000_0001usize as HKey;
/// Machine-wide settings, shared by every user on this computer.
pub const HKEY_LOCAL_MACHINE: HKey = 0xFFFF_FFFF_8000_0002usize as HKey;
/// Every loaded user profile's hive, keyed by SID
/// (`HKEY_USERS\<SID>\...`) — `HKEY_CURRENT_USER` is a per-process alias
/// into one entry here.
pub const HKEY_USERS: HKey = 0xFFFF_FFFF_8000_0003usize as HKey;
/// The active hardware profile, a subset of `HKEY_LOCAL_MACHINE` exposed
/// as its own root for legacy reasons — mostly a historical artifact on
/// modern Windows, which no longer really supports multiple hardware
/// profiles.
pub const HKEY_CURRENT_CONFIG: HKey = 0xFFFF_FFFF_8000_0005usize as HKey;

/// Open a subkey of `parent` — `RegOpenKeyExW`. The registry analog of
/// [`crate::handle::duplicate`]'s "start from a handle you already have":
/// every deeper registry access starts by opening a subkey of one of the
/// five predefined roots above (or of a key this function already
/// returned), same as opening a nested directory one path component at a
/// time. `access` is a `KEY_*` bitmask (e.g. [`KEY_READ`]).
///
/// Unlike most of this crate's Win32 wrappers, `RegOpenKeyExW` reports
/// failure via its own `LSTATUS` return value directly — never
/// `GetLastError` — so a nonzero return is passed straight to
/// [`crate::error::Win32Error::from_raw`] rather than
/// `Win32Error::last()`.
///
/// # Safety
///
/// `parent` must be a currently-valid `HKey` — one of the predefined
/// roots above, or a key this function previously returned that hasn't
/// been closed yet.
pub unsafe fn open_key(
    parent: HKey,
    subkey: &str,
    access: u32,
) -> Result<HKey, crate::error::Win32Error> {
    let wide: Vec<u16> = crate::wide::to_wide(subkey);
    let mut result: HKey = core::ptr::null_mut();
    // SAFETY: `parent` is caller-supplied per this function's own safety
    // contract; `wide` is a valid, NUL-terminated UTF-16 string live for
    // the whole call; `result` is a valid out-pointer.
    let status = unsafe { RegOpenKeyExW(parent, wide.as_ptr(), 0, access, &mut result) };
    if status == 0 {
        Ok(result)
    } else {
        Err(crate::error::Win32Error::from_raw(status as u32))
    }
}

/// Close a key handle previously returned by [`open_key`] — `RegCloseKey`.
/// Never call this on one of the five predefined roots above: they are
/// sentinel values, not real handles the system reference-counts, and
/// `RegCloseKey` on one is a documented no-op that still returns success,
/// so this crate's own "must close what you open" discipline doesn't
/// actually apply to them the way it does to an [`open_key`] result.
///
/// # Safety
///
/// `key` must be a currently-open handle returned by [`open_key`], not yet
/// closed.
pub unsafe fn close_key(key: HKey) -> Result<(), crate::error::Win32Error> {
    // SAFETY: `key` is caller-supplied per this function's own safety
    // contract.
    let status = unsafe { RegCloseKey(key) };
    if status == 0 {
        Ok(())
    } else {
        Err(crate::error::Win32Error::from_raw(status as u32))
    }
}

/// Which of the two things [`create_key`] actually did —
/// `RegCreateKeyExW`'s `lpdwDisposition` out-parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyDisposition {
    /// The subkey didn't already exist; this call created it.
    CreatedNewKey,
    /// The subkey already existed; this call just opened it, same as
    /// [`open_key`] would have.
    OpenedExistingKey,
}

/// Open `subkey` under `parent`, creating it first if it doesn't already
/// exist — `RegCreateKeyExW`, an idempotent "ensure this key exists" in
/// one call rather than an [`open_key`] a caller would otherwise have to
/// fall back from on `ERROR_FILE_NOT_FOUND`. The returned
/// [`KeyDisposition`] reports which of the two actually happened.
///
/// # Safety
///
/// `parent` must be a currently-valid `HKey` — one of the predefined
/// roots in this module, or a key [`open_key`]/this function previously
/// returned that hasn't been closed yet.
pub unsafe fn create_key(
    parent: HKey,
    subkey: &str,
    access: u32,
) -> Result<(HKey, KeyDisposition), crate::error::Win32Error> {
    let wide: Vec<u16> = crate::wide::to_wide(subkey);
    let mut result: HKey = core::ptr::null_mut();
    let mut disposition: u32 = 0;
    // SAFETY: `parent` is caller-supplied per this function's own safety
    // contract; `wide` is a valid, NUL-terminated UTF-16 string live for
    // the whole call; `result`/`disposition` are valid out-pointers;
    // `class`/`security_attributes` are documented-optional and null here.
    let status = unsafe {
        RegCreateKeyExW(
            parent,
            wide.as_ptr(),
            0,
            core::ptr::null_mut(),
            REG_OPTION_NON_VOLATILE,
            access,
            core::ptr::null(),
            &mut result,
            &mut disposition,
        )
    };
    if status != 0 {
        return Err(crate::error::Win32Error::from_raw(status as u32));
    }
    let disposition = if disposition == REG_OPENED_EXISTING_KEY {
        KeyDisposition::OpenedExistingKey
    } else {
        KeyDisposition::CreatedNewKey
    };
    Ok((result, disposition))
}

/// A registry value's data, decoded per its `dwType` — the seven kinds
/// this crate covers (`REG_LINK`/`REG_RESOURCE_LIST`/etc. are out of
/// scope; a symbolic-key-link and hardware-resource-descriptor format
/// respectively, neither meaningful outside their own narrow subsystems).
#[derive(Debug, Clone, PartialEq)]
pub enum RegistryValue {
    /// `REG_NONE` — no defined type, or a genuinely empty value.
    None,
    /// `REG_SZ` — an ordinary NUL-terminated string.
    Sz(alloc::string::String),
    /// `REG_EXPAND_SZ` — a string containing unexpanded `%ENVVAR%`
    /// references (e.g. `%SystemRoot%\system32`); expanding it is the
    /// caller's job (`ExpandEnvironmentStringsW`, out of this issue's
    /// scope), not this function's — it hands back the raw text exactly
    /// as stored, same as `REG_SZ`.
    ExpandSz(alloc::string::String),
    /// `REG_DWORD` — a 32-bit integer.
    Dword(u32),
    /// `REG_QWORD` — a 64-bit integer.
    Qword(u64),
    /// `REG_BINARY` — an opaque byte blob with no further structure this
    /// crate decodes.
    Binary(Vec<u8>),
    /// `REG_MULTI_SZ` — a list of strings (the on-disk encoding is a
    /// sequence of NUL-terminated UTF-16 strings, itself terminated by an
    /// extra NUL; already split apart here).
    MultiSz(Vec<alloc::string::String>),
}

/// `RegQueryValueExW`'s `dwType` out-values this crate decodes into
/// [`RegistryValue`] variants — verified against mingw-w64's own
/// `winnt.h` macros with a compiled `_Static_assert` probe. Anything else
/// (`REG_LINK`, `REG_RESOURCE_LIST`, …) falls back to
/// [`RegistryValue::None`] rather than failing outright, since a caller
/// asking for an ordinary value's data shouldn't have to know about every
/// obscure type up front.
const REG_NONE: u32 = 0;
const REG_SZ: u32 = 1;
const REG_EXPAND_SZ: u32 = 2;
const REG_BINARY: u32 = 3;
const REG_DWORD: u32 = 4;
const REG_MULTI_SZ: u32 = 7;
const REG_QWORD: u32 = 11;

fn decode_wide_string(buf: &[u8]) -> alloc::string::String {
    let mut units: Vec<u16> = buf
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    // A registry `REG_SZ`/`REG_EXPAND_SZ` value is *usually* stored with a
    // trailing NUL folded into its byte count, but the API docs
    // explicitly warn this isn't guaranteed — strip one if present rather
    // than assuming either way.
    if units.last() == Some(&0) {
        units.pop();
    }
    alloc::string::String::from_utf16_lossy(&units)
}

fn decode_multi_sz(buf: &[u8]) -> Vec<alloc::string::String> {
    let units: Vec<u16> = buf
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    let mut strings = Vec::new();
    let mut start = 0;
    for (i, &unit) in units.iter().enumerate() {
        if unit == 0 {
            // A run of length zero is either two adjacent NULs (an empty
            // string genuinely stored in the list) or the final
            // terminating NUL right after the last real string's own —
            // either way, `REG_MULTI_SZ`'s own double-NUL end marker
            // means this loop should stop adding entries once it sees
            // one, not emit a trailing empty string for it.
            if i > start {
                strings.push(alloc::string::String::from_utf16_lossy(&units[start..i]));
            }
            start = i + 1;
        }
    }
    strings
}

fn decode_u32_le(buf: &[u8]) -> u32 {
    let mut bytes = [0u8; 4];
    let n = buf.len().min(4);
    bytes[..n].copy_from_slice(&buf[..n]);
    u32::from_le_bytes(bytes)
}

fn decode_u64_le(buf: &[u8]) -> u64 {
    let mut bytes = [0u8; 8];
    let n = buf.len().min(8);
    bytes[..n].copy_from_slice(&buf[..n]);
    u64::from_le_bytes(bytes)
}

/// Read `name`'s value data from `key` — `RegQueryValueExW`, decoded into
/// a [`RegistryValue`] rather than a raw byte blob plus a separate type
/// code. Uses the query-size-then-allocate idiom this crate already uses
/// elsewhere (`path::search_path`, `fs::final_path`): a first call with a
/// null data pointer reports the exact required size (and the value's
/// type) without needing to guess a starting buffer size, then a second
/// call actually reads the data into a correctly-sized buffer.
///
/// # Safety
///
/// `key` must be a currently-valid `HKey` — one of the predefined roots
/// in this module, or a key [`open_key`]/[`create_key`] previously
/// returned that hasn't been closed yet.
pub unsafe fn query_value(
    key: HKey,
    name: &str,
) -> Result<RegistryValue, crate::error::Win32Error> {
    let wide_name: Vec<u16> = crate::wide::to_wide(name);

    let mut value_type: u32 = 0;
    let mut size: u32 = 0;
    // SAFETY: `key` is caller-supplied per this function's own safety
    // contract; `wide_name` is a valid, NUL-terminated UTF-16 string live
    // for the whole call; `value_type`/`size` are valid out-pointers. A
    // null `lpData` with a non-null `lpcbData` is documented to report
    // just the type and required size, without touching any data buffer.
    let status = unsafe {
        RegQueryValueExW(
            key,
            wide_name.as_ptr(),
            core::ptr::null_mut(),
            &mut value_type,
            core::ptr::null_mut(),
            &mut size,
        )
    };
    if status != 0 {
        return Err(crate::error::Win32Error::from_raw(status as u32));
    }

    let mut buf: Vec<u8> = alloc::vec![0u8; size as usize];
    let mut actual_size = size;
    // SAFETY: `key`/`wide_name` as above; `buf` is a valid,
    // `size`-byte writable buffer matched by `actual_size` naming its
    // exact length; `value_type`/`actual_size` are valid out-pointers.
    let status = unsafe {
        RegQueryValueExW(
            key,
            wide_name.as_ptr(),
            core::ptr::null_mut(),
            &mut value_type,
            buf.as_mut_ptr(),
            &mut actual_size,
        )
    };
    if status != 0 {
        return Err(crate::error::Win32Error::from_raw(status as u32));
    }
    buf.truncate(actual_size as usize);

    Ok(match value_type {
        REG_SZ => RegistryValue::Sz(decode_wide_string(&buf)),
        REG_EXPAND_SZ => RegistryValue::ExpandSz(decode_wide_string(&buf)),
        REG_DWORD => RegistryValue::Dword(decode_u32_le(&buf)),
        REG_QWORD => RegistryValue::Qword(decode_u64_le(&buf)),
        REG_MULTI_SZ => RegistryValue::MultiSz(decode_multi_sz(&buf)),
        REG_BINARY => RegistryValue::Binary(buf),
        _ => RegistryValue::None,
    })
}

fn encode_wide_string(s: &str) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(s.len() * 2 + 2);
    for unit in s.encode_utf16() {
        bytes.extend_from_slice(&unit.to_le_bytes());
    }
    bytes.extend_from_slice(&0u16.to_le_bytes());
    bytes
}

fn encode_multi_sz(strings: &[alloc::string::String]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for s in strings {
        bytes.extend_from_slice(&encode_wide_string(s));
    }
    // `REG_MULTI_SZ`'s own end marker: an extra NUL after the last
    // string's own NUL terminator, making the whole buffer end in a
    // double NUL. `encode_wide_string` already appended one NUL per
    // string above, so this is the one additional terminator — for an
    // empty list, this alone produces the minimal valid empty
    // `REG_MULTI_SZ` encoding (a single NUL `u16`).
    bytes.extend_from_slice(&0u16.to_le_bytes());
    bytes
}

/// Write `name`'s value data in `key` — `RegSetValueExW`, the write-side
/// counterpart to [`query_value`]. Encodes `value` into the `dwType`/
/// byte-buffer shape each `REG_*` type expects: UTF-16 with a NUL
/// terminator for [`RegistryValue::Sz`]/[`RegistryValue::ExpandSz`], each
/// string NUL-terminated plus a trailing extra NUL for
/// [`RegistryValue::MultiSz`], little-endian bytes for
/// [`RegistryValue::Dword`]/[`RegistryValue::Qword`], and the raw bytes
/// as-is for [`RegistryValue::Binary`]. Creates the value if `name`
/// doesn't already exist under `key`, overwrites it if it does.
///
/// # Safety
///
/// `key` must be a currently-valid `HKey` — one of the predefined roots
/// in this module, or a key [`open_key`]/[`create_key`] previously
/// returned that hasn't been closed yet, opened/created with
/// [`KEY_WRITE`] (or a superset of it, e.g. [`KEY_ALL_ACCESS`]) access.
pub unsafe fn set_value(
    key: HKey,
    name: &str,
    value: &RegistryValue,
) -> Result<(), crate::error::Win32Error> {
    let wide_name: Vec<u16> = crate::wide::to_wide(name);
    let (value_type, data) = match value {
        RegistryValue::None => (REG_NONE, Vec::new()),
        RegistryValue::Sz(s) => (REG_SZ, encode_wide_string(s)),
        RegistryValue::ExpandSz(s) => (REG_EXPAND_SZ, encode_wide_string(s)),
        RegistryValue::Dword(n) => (REG_DWORD, n.to_le_bytes().to_vec()),
        RegistryValue::Qword(n) => (REG_QWORD, n.to_le_bytes().to_vec()),
        RegistryValue::Binary(bytes) => (REG_BINARY, bytes.clone()),
        RegistryValue::MultiSz(strings) => (REG_MULTI_SZ, encode_multi_sz(strings)),
    };
    // SAFETY: `key` is caller-supplied per this function's own safety
    // contract; `wide_name` is a valid, NUL-terminated UTF-16 string live
    // for the whole call; `data` is a valid, `data.len()`-byte buffer
    // (even when empty — `Vec::as_ptr` on an empty `Vec` is a well-defined
    // dangling-but-non-null pointer, never dereferenced when `data_size`
    // is `0`).
    let status = unsafe {
        RegSetValueExW(
            key,
            wide_name.as_ptr(),
            0,
            value_type,
            data.as_ptr(),
            data.len() as u32,
        )
    };
    if status == 0 {
        Ok(())
    } else {
        Err(crate::error::Win32Error::from_raw(status as u32))
    }
}

/// Remove one named value from `key` — `RegDeleteValueW`. Removes only
/// the value itself, never the key it lives under or any of the key's
/// other values/subkeys — the registry analog of deleting a single file
/// out of a directory rather than the directory itself.
///
/// # Safety
///
/// `key` must be a currently-valid `HKey` — one of the predefined roots
/// in this module, or a key [`open_key`]/[`create_key`] previously
/// returned that hasn't been closed yet, opened/created with
/// [`KEY_WRITE`] (or a superset of it, e.g. [`KEY_ALL_ACCESS`]) access.
pub unsafe fn delete_value(key: HKey, name: &str) -> Result<(), crate::error::Win32Error> {
    let wide_name: Vec<u16> = crate::wide::to_wide(name);
    // SAFETY: `key` is caller-supplied per this function's own safety
    // contract; `wide_name` is a valid, NUL-terminated UTF-16 string live
    // for the whole call.
    let status = unsafe { RegDeleteValueW(key, wide_name.as_ptr()) };
    if status == 0 {
        Ok(())
    } else {
        Err(crate::error::Win32Error::from_raw(status as u32))
    }
}

/// Remove a leaf subkey of `parent` — `RegDeleteKeyExW`. The subkey must
/// have no subkeys of its own (its values, if any, are removed along
/// with it) — Windows refuses to delete a key that still has children,
/// the same restriction `rmdir` has for a non-empty directory; delete the
/// deepest subkeys first for a whole subtree. `view` is a `KEY_WOW64_*`
/// bit (or `0` for the default view) — see [`KEY_WOW64_64KEY`]/
/// [`KEY_WOW64_32KEY`].
///
/// # Safety
///
/// `parent` must be a currently-valid `HKey` — one of the predefined
/// roots in this module, or a key [`open_key`]/[`create_key`] previously
/// returned that hasn't been closed yet.
pub unsafe fn delete_key(
    parent: HKey,
    subkey: &str,
    view: u32,
) -> Result<(), crate::error::Win32Error> {
    let wide: Vec<u16> = crate::wide::to_wide(subkey);
    // SAFETY: `parent` is caller-supplied per this function's own safety
    // contract; `wide` is a valid, NUL-terminated UTF-16 string live for
    // the whole call.
    let status = unsafe { RegDeleteKeyExW(parent, wide.as_ptr(), view, 0) };
    if status == 0 {
        Ok(())
    } else {
        Err(crate::error::Win32Error::from_raw(status as u32))
    }
}

/// An in-progress [`enum_values`] enumeration — the value-side analog of
/// [`crate::fs::ReadDir`], but each item's name/data don't share one
/// fixed-size record the way a directory entry does, so this holds its
/// own growable name/data buffers instead of relying on the OS to size
/// them per call.
pub struct RegValueIter {
    key: HKey,
    index: u32,
    name_buf: Vec<u16>,
    data_buf: Vec<u8>,
    done: bool,
}

impl Iterator for RegValueIter {
    type Item = Result<(alloc::string::String, RegistryValue), crate::error::Win32Error>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        loop {
            let mut name_len = self.name_buf.len() as u32;
            let mut data_len = self.data_buf.len() as u32;
            let mut value_type: u32 = 0;
            // SAFETY: `self.key` is caller-supplied per `enum_values`'s
            // own safety contract, and must still be open (this struct's
            // own invariant, documented there); `name_buf`/`data_buf` are
            // valid, writable buffers matched by `name_len`/`data_len`
            // naming their exact lengths (the name buffer's length
            // already accounts for `RegEnumValueW`'s documented
            // "including the terminating null character" input
            // convention, unlike the data buffer's plain byte count).
            let status = unsafe {
                RegEnumValueW(
                    self.key,
                    self.index,
                    self.name_buf.as_mut_ptr(),
                    &mut name_len,
                    core::ptr::null_mut(),
                    &mut value_type,
                    self.data_buf.as_mut_ptr(),
                    &mut data_len,
                )
            };
            if status == 0 {
                self.index += 1;
                let name =
                    alloc::string::String::from_utf16_lossy(&self.name_buf[..name_len as usize]);
                let data = self.data_buf[..data_len as usize].to_vec();
                let value = match value_type {
                    REG_SZ => RegistryValue::Sz(decode_wide_string(&data)),
                    REG_EXPAND_SZ => RegistryValue::ExpandSz(decode_wide_string(&data)),
                    REG_DWORD => RegistryValue::Dword(decode_u32_le(&data)),
                    REG_QWORD => RegistryValue::Qword(decode_u64_le(&data)),
                    REG_MULTI_SZ => RegistryValue::MultiSz(decode_multi_sz(&data)),
                    REG_BINARY => RegistryValue::Binary(data),
                    _ => RegistryValue::None,
                };
                return Some(Ok((name, value)));
            }
            let err = crate::error::Win32Error::from_raw(status as u32);
            if err == crate::error::Win32Error::ERROR_NO_MORE_ITEMS {
                self.done = true;
                return None;
            }
            if err == crate::error::Win32Error::ERROR_MORE_DATA {
                // A value added after `enum_values`'s own initial sizing
                // query (or, for the name buffer, one `RegEnumValueW`
                // simply doesn't reliably report an exact required size
                // for on this specific failure, unlike
                // `RegQueryValueExW`) can still exceed what was
                // allocated — grow whichever buffer(s) plausibly need it
                // and retry the same index rather than trusting the
                // reported sizes blindly.
                if (name_len as usize) >= self.name_buf.len() {
                    let grown = (name_len as usize + 1).max(self.name_buf.len() * 2);
                    self.name_buf.resize(grown, 0);
                } else {
                    self.name_buf.resize(self.name_buf.len() * 2, 0);
                }
                if (data_len as usize) > self.data_buf.len() {
                    self.data_buf.resize(data_len as usize, 0);
                } else {
                    self.data_buf.resize(self.data_buf.len().max(1) * 2, 0);
                }
                continue;
            }
            self.done = true;
            return Some(Err(err));
        }
    }
}

/// Start enumerating every value under `key` — repeated `RegEnumValueW`
/// calls, one per [`RegValueIter::next`], stopping at
/// `ERROR_NO_MORE_ITEMS`. Queries `key`'s own reported maximum value
/// name/data lengths up front (`RegQueryInfoKeyW`) to size the
/// enumeration's buffers once rather than growing them from nothing on
/// the first item; a value added concurrently that exceeds those
/// maximums is still handled (see [`RegValueIter`]'s `next`), just less
/// commonly.
///
/// # Safety
///
/// `key` must be a currently-valid `HKey` — one of the predefined roots
/// in this module, or a key [`open_key`]/[`create_key`] previously
/// returned — for the entire lifetime of the returned [`RegValueIter`],
/// not merely for this call: the iterator holds `key` and calls
/// `RegEnumValueW` on it again for every item it yields, so closing `key`
/// before the iterator is done with it invalidates every subsequent
/// `next()` call.
pub unsafe fn enum_values(key: HKey) -> RegValueIter {
    let mut max_value_name_len: u32 = 0;
    let mut max_value_len: u32 = 0;
    // SAFETY: `key` is caller-supplied per this function's own safety
    // contract; every other out-pointer is null since this crate only
    // wants the two value-sizing hints — documented-valid when the
    // corresponding piece of information isn't wanted.
    let status = unsafe {
        RegQueryInfoKeyW(
            key,
            core::ptr::null_mut(),
            core::ptr::null_mut(),
            core::ptr::null_mut(),
            core::ptr::null_mut(),
            core::ptr::null_mut(),
            core::ptr::null_mut(),
            core::ptr::null_mut(),
            &mut max_value_name_len,
            &mut max_value_len,
            core::ptr::null_mut(),
            core::ptr::null_mut(),
        )
    };
    // A failed query (unusual, but not a documented impossibility for
    // every kind of key) falls back to a small starting size — `next()`'s
    // own `ERROR_MORE_DATA` growth handles it regardless either way.
    let (name_cap, data_cap) = if status == 0 {
        (max_value_name_len as usize + 1, max_value_len as usize)
    } else {
        (256, 256)
    };
    RegValueIter {
        key,
        index: 0,
        name_buf: alloc::vec![0u16; name_cap.max(1)],
        data_buf: alloc::vec![0u8; data_cap],
        done: false,
    }
}

/// An in-progress [`enum_keys`] enumeration — the subkey-side analog of
/// [`RegValueIter`], but each item carries only a name plus a last-write
/// [`Timespec`] (no data/type to decode), so it only needs one growable
/// buffer.
pub struct RegKeyIter {
    key: HKey,
    index: u32,
    name_buf: Vec<u16>,
    done: bool,
}

impl Iterator for RegKeyIter {
    type Item = Result<(alloc::string::String, Timespec), crate::error::Win32Error>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        loop {
            let mut name_len = self.name_buf.len() as u32;
            let mut last_write = FileTime::default();
            // SAFETY: `self.key` is caller-supplied per `enum_keys`'s own
            // safety contract, and must still be open (this struct's own
            // invariant, documented there); `name_buf` is a valid,
            // writable buffer matched by `name_len` naming its exact
            // length (already accounting for `RegEnumKeyExW`'s documented
            // "including the terminating null character" input
            // convention); `class`/`class_len` are both null together, a
            // documented-valid way to say "this crate doesn't want the
            // class name."
            let status = unsafe {
                RegEnumKeyExW(
                    self.key,
                    self.index,
                    self.name_buf.as_mut_ptr(),
                    &mut name_len,
                    core::ptr::null_mut(),
                    core::ptr::null_mut(),
                    core::ptr::null_mut(),
                    &mut last_write,
                )
            };
            if status == 0 {
                self.index += 1;
                let name =
                    alloc::string::String::from_utf16_lossy(&self.name_buf[..name_len as usize]);
                return Some(Ok((name, filetime_to_timespec(last_write))));
            }
            let err = crate::error::Win32Error::from_raw(status as u32);
            if err == crate::error::Win32Error::ERROR_NO_MORE_ITEMS {
                self.done = true;
                return None;
            }
            if err == crate::error::Win32Error::ERROR_MORE_DATA {
                // Same defensive reasoning as `RegValueIter::next`: a
                // subkey created after `enum_keys`'s own initial sizing
                // query can still exceed what was allocated.
                self.name_buf.resize(self.name_buf.len().max(1) * 2, 0);
                continue;
            }
            self.done = true;
            return Some(Err(err));
        }
    }
}

/// Start enumerating every immediate subkey of `key` — repeated
/// `RegEnumKeyExW` calls, one per [`RegKeyIter::next`], stopping at
/// `ERROR_NO_MORE_ITEMS`. Each item's `Timespec` is the subkey's
/// last-write time, decoded from the raw `FILETIME` `RegEnumKeyExW`
/// itself reports — this crate surfaces it as [`Timespec`] rather than a
/// raw `FILETIME` mirror, the same "decode into a meaningful type, keep
/// the FFI struct private" convention `process::times`/`fs::stat` etc.
/// already follow. Only immediate children, not the whole subtree —
/// recurse by calling this again on each returned name if a deeper walk
/// is needed.
///
/// # Safety
///
/// `key` must be a currently-valid `HKey` — one of the predefined roots
/// in this module, or a key [`open_key`]/[`create_key`] previously
/// returned — for the entire lifetime of the returned [`RegKeyIter`], not
/// merely for this call, the same contract [`enum_values`] documents for
/// its own iterator.
pub unsafe fn enum_keys(key: HKey) -> RegKeyIter {
    let mut max_sub_key_len: u32 = 0;
    // SAFETY: `key` is caller-supplied per this function's own safety
    // contract; every other out-pointer is null since this crate only
    // wants the one sizing hint.
    let status = unsafe {
        RegQueryInfoKeyW(
            key,
            core::ptr::null_mut(),
            core::ptr::null_mut(),
            core::ptr::null_mut(),
            core::ptr::null_mut(),
            &mut max_sub_key_len,
            core::ptr::null_mut(),
            core::ptr::null_mut(),
            core::ptr::null_mut(),
            core::ptr::null_mut(),
            core::ptr::null_mut(),
            core::ptr::null_mut(),
        )
    };
    let name_cap = if status == 0 {
        max_sub_key_len as usize + 1
    } else {
        256
    };
    RegKeyIter {
        key,
        index: 0,
        name_buf: alloc::vec![0u16; name_cap.max(1)],
        done: false,
    }
}

/// [`key_info`]'s result — `RegQueryInfoKeyW`'s subkey/value counts and
/// maximum name/data lengths in one call, the "ask how big first"
/// pattern [`crate::fs::final_path`] already uses elsewhere in this
/// crate, needed to pre-size buffers before enumerating (which
/// [`enum_values`]/[`enum_keys`] already do internally — this exposes
/// the same query directly, for a caller that wants the counts/lengths
/// themselves rather than just an iterator). Doesn't include the class
/// name or security descriptor length — the class name is a legacy
/// concept with no real modern use, and the security descriptor is the
/// `security` module's territory, not this one's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyInfo {
    /// How many immediate subkeys `key` has.
    pub sub_key_count: u32,
    /// The longest subkey name among them, in UTF-16 code units (not
    /// counting a terminating NUL).
    pub max_sub_key_len: u32,
    /// How many values `key` has.
    pub value_count: u32,
    /// The longest value name among them, in UTF-16 code units (not
    /// counting a terminating NUL).
    pub max_value_name_len: u32,
    /// The largest value data size among them, in bytes.
    pub max_value_len: u32,
    /// When `key` (or one of its values) was last modified.
    pub last_write_time: Timespec,
}

/// Query `key`'s subkey/value counts and maximum name/data lengths in one
/// call — `RegQueryInfoKeyW`.
///
/// # Safety
///
/// `key` must be a currently-valid `HKey` — one of the predefined roots
/// in this module, or a key [`open_key`]/[`create_key`] previously
/// returned that hasn't been closed yet.
pub unsafe fn key_info(key: HKey) -> Result<KeyInfo, crate::error::Win32Error> {
    let mut sub_key_count: u32 = 0;
    let mut max_sub_key_len: u32 = 0;
    let mut value_count: u32 = 0;
    let mut max_value_name_len: u32 = 0;
    let mut max_value_len: u32 = 0;
    let mut last_write = FileTime::default();
    // SAFETY: `key` is caller-supplied per this function's own safety
    // contract; every out-pointer is either a valid pointer to one of
    // this function's own locals, or null for the class name/security
    // descriptor length this crate doesn't want — documented-valid when
    // that information isn't needed.
    let status = unsafe {
        RegQueryInfoKeyW(
            key,
            core::ptr::null_mut(),
            core::ptr::null_mut(),
            core::ptr::null_mut(),
            &mut sub_key_count,
            &mut max_sub_key_len,
            core::ptr::null_mut(),
            &mut value_count,
            &mut max_value_name_len,
            &mut max_value_len,
            core::ptr::null_mut(),
            &mut last_write,
        )
    };
    if status != 0 {
        return Err(crate::error::Win32Error::from_raw(status as u32));
    }
    Ok(KeyInfo {
        sub_key_count,
        max_sub_key_len,
        value_count,
        max_value_name_len,
        max_value_len,
        last_write_time: filetime_to_timespec(last_write),
    })
}

/// Force `key`'s changes to disk immediately — `RegFlushKey`, documented
/// as the registry analog of `FlushFileBuffers` (which this crate
/// doesn't itself wrap — no current consumer needs a raw-file-handle
/// flush, unlike this registry-durability gap). Windows normally batches
/// registry writes and flushes them lazily; this closes
/// the durability gap for a caller writing settings right before a risky
/// operation (e.g. right before terminating the process), where waiting
/// for the lazy flush would risk losing the write entirely. Expensive —
/// this crate's own docs match Microsoft's own guidance not to call it
/// except when durability genuinely matters, never as routine practice
/// after every write.
///
/// # Safety
///
/// `key` must be a currently-valid `HKey` — one of the predefined roots
/// in this module, or a key [`open_key`]/[`create_key`] previously
/// returned that hasn't been closed yet.
pub unsafe fn flush_key(key: HKey) -> Result<(), crate::error::Win32Error> {
    // SAFETY: `key` is caller-supplied per this function's own safety
    // contract.
    let status = unsafe { RegFlushKey(key) };
    if status == 0 {
        Ok(())
    } else {
        Err(crate::error::Win32Error::from_raw(status as u32))
    }
}

/// Recursively delete `subkey` (under `parent`) and everything beneath
/// it — values, subkeys, and their own subkeys, all the way down —
/// `RegDeleteTreeW`. [`delete_key`]'s leaf-only restriction (it refuses a
/// subkey that still has children) forces a hand-rolled
/// enumerate-and-recurse loop without this; `RegDeleteTreeW` does that
/// walk internally instead.
///
/// # Safety
///
/// `parent` must be a currently-valid `HKey` — one of the predefined
/// roots in this module, or a key [`open_key`]/[`create_key`] previously
/// returned that hasn't been closed yet.
pub unsafe fn delete_tree(parent: HKey, subkey: &str) -> Result<(), crate::error::Win32Error> {
    let wide: Vec<u16> = crate::wide::to_wide(subkey);
    // SAFETY: `parent` is caller-supplied per this function's own safety
    // contract; `wide` is a valid, NUL-terminated UTF-16 string live for
    // the whole call.
    let status = unsafe { RegDeleteTreeW(parent, wide.as_ptr()) };
    if status == 0 {
        Ok(())
    } else {
        Err(crate::error::Win32Error::from_raw(status as u32))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_keys_are_distinct_and_match_the_documented_sign_extended_sentinels() {
        assert_eq!(HKEY_CLASSES_ROOT as usize, 0xFFFF_FFFF_8000_0000);
        assert_eq!(HKEY_CURRENT_USER as usize, 0xFFFF_FFFF_8000_0001);
        assert_eq!(HKEY_LOCAL_MACHINE as usize, 0xFFFF_FFFF_8000_0002);
        assert_eq!(HKEY_USERS as usize, 0xFFFF_FFFF_8000_0003);
        assert_eq!(HKEY_CURRENT_CONFIG as usize, 0xFFFF_FFFF_8000_0005);

        let roots = [
            HKEY_CLASSES_ROOT,
            HKEY_CURRENT_USER,
            HKEY_LOCAL_MACHINE,
            HKEY_USERS,
            HKEY_CURRENT_CONFIG,
        ];
        for (i, a) in roots.iter().enumerate() {
            for (j, b) in roots.iter().enumerate() {
                assert_eq!(
                    i == j,
                    a == b,
                    "roots at {i} and {j} should differ unless equal indices"
                );
            }
        }
    }

    #[test]
    fn open_key_then_close_key_round_trips_on_a_well_known_subkey() {
        // `SOFTWARE\Microsoft\Windows\CurrentVersion` exists on every
        // Windows install (it's where things like the OS build number
        // live) — a stable, always-present target that doesn't depend on
        // anything this crate itself created.
        // SAFETY: `HKEY_LOCAL_MACHINE` is a predefined, always-valid root.
        let key = unsafe {
            open_key(
                HKEY_LOCAL_MACHINE,
                "SOFTWARE\\Microsoft\\Windows\\CurrentVersion",
                KEY_READ,
            )
        }
        .expect("RegOpenKeyExW should succeed for a well-known, always-present subkey");
        assert!(!key.is_null());
        // SAFETY: `key` was just opened above and not yet closed.
        unsafe { close_key(key) }.expect("RegCloseKey should succeed");
    }

    #[test]
    fn open_key_fails_for_a_nonexistent_subkey() {
        // SAFETY: `HKEY_LOCAL_MACHINE` is a predefined, always-valid root.
        let err = unsafe {
            open_key(
                HKEY_LOCAL_MACHINE,
                "Software\\ThisSubkeyDefinitelyDoesNotExist12345",
                KEY_READ,
            )
        }
        .expect_err("RegOpenKeyExW should fail for a nonexistent subkey");
        assert_eq!(err, crate::error::Win32Error::ERROR_FILE_NOT_FOUND);
    }

    #[test]
    fn create_key_reports_created_then_opened_disposition() {
        // `HKEY_CURRENT_USER\Software` is writable by an ordinary,
        // non-elevated process — unlike `HKEY_LOCAL_MACHINE`, which
        // `open_key`'s tests above only ever read from.
        //
        // Uniquified by this test process's own pid: this crate's CI job
        // runs `cargo test` twice (once per feature set) on the *same*
        // Windows VM, and unlike a temp-file-backed test, there's no
        // `delete_key` yet to clean this up afterward — a fixed subkey
        // name created by the first `cargo test` invocation would still
        // exist when the second invocation's own instance of this test
        // ran, making its "first call creates" assertion fail with
        // `OpenedExistingKey` instead. Confirmed by exactly that failure
        // in this crate's own CI.
        let subkey = format!(
            "Software\\rusty_win32_registry_test_create_key_{}",
            std::process::id()
        );
        let subkey = subkey.as_str();

        // SAFETY: `HKEY_CURRENT_USER` is a predefined, always-valid root.
        let (first, first_disposition) =
            unsafe { create_key(HKEY_CURRENT_USER, subkey, KEY_ALL_ACCESS) }
                .expect("RegCreateKeyExW should succeed creating a new subkey");
        assert!(!first.is_null());
        assert_eq!(first_disposition, KeyDisposition::CreatedNewKey);
        // SAFETY: `first` was just opened above and not yet closed.
        unsafe { close_key(first) }.expect("RegCloseKey should succeed");

        // Calling it again on the exact same subkey — which the call
        // above just made exist — should report the other disposition,
        // deterministically regardless of what order tests in this
        // module happen to run in.
        // SAFETY: same predefined root.
        let (second, second_disposition) =
            unsafe { create_key(HKEY_CURRENT_USER, subkey, KEY_ALL_ACCESS) }
                .expect("RegCreateKeyExW should succeed opening the now-existing subkey");
        assert!(!second.is_null());
        assert_eq!(second_disposition, KeyDisposition::OpenedExistingKey);
        // SAFETY: `second` was just opened above and not yet closed.
        unsafe { close_key(second) }.expect("RegCloseKey should succeed");

        // Clean up now that `delete_key` exists — this test's own subkey
        // is a bare leaf (no subkeys of its own), so it's directly
        // deletable.
        // SAFETY: `HKEY_CURRENT_USER` is the same predefined root.
        unsafe { delete_key(HKEY_CURRENT_USER, subkey, 0) }
            .expect("RegDeleteKeyExW should succeed");
    }

    /// Every value read below lives under this exact, well-documented
    /// key — present on every Windows 10/11/Server install since these
    /// values were introduced (Windows 10, 2015), long before this
    /// crate's own `windows-latest` CI target existed.
    const CURRENT_VERSION_KEY: &str = "SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion";

    #[test]
    fn query_value_reads_a_well_known_reg_sz_value() {
        // SAFETY: `HKEY_LOCAL_MACHINE` is a predefined, always-valid root.
        let key = unsafe { open_key(HKEY_LOCAL_MACHINE, CURRENT_VERSION_KEY, KEY_READ) }
            .expect("RegOpenKeyExW should succeed for a well-known, always-present subkey");
        // SAFETY: `key` was just opened above and stays open for this
        // whole call.
        let value = unsafe { query_value(key, "ProductName") }
            .expect("RegQueryValueExW should succeed reading a well-known REG_SZ value");
        match value {
            RegistryValue::Sz(s) => assert!(!s.is_empty(), "ProductName should be non-empty"),
            other => panic!("expected RegistryValue::Sz, got {other:?}"),
        }
        // SAFETY: `key` is still the same valid, currently-open handle.
        unsafe { close_key(key) }.expect("RegCloseKey should succeed");
    }

    #[test]
    fn query_value_reads_a_well_known_reg_dword_value() {
        // SAFETY: `HKEY_LOCAL_MACHINE` is a predefined, always-valid root.
        let key = unsafe { open_key(HKEY_LOCAL_MACHINE, CURRENT_VERSION_KEY, KEY_READ) }
            .expect("RegOpenKeyExW should succeed for a well-known, always-present subkey");
        // SAFETY: `key` was just opened above and stays open for this
        // whole call.
        let value = unsafe { query_value(key, "CurrentMajorVersionNumber") }
            .expect("RegQueryValueExW should succeed reading a well-known REG_DWORD value");
        match value {
            RegistryValue::Dword(n) => {
                assert!(n > 0, "CurrentMajorVersionNumber should be nonzero")
            }
            other => panic!("expected RegistryValue::Dword, got {other:?}"),
        }
        // SAFETY: `key` is still the same valid, currently-open handle.
        unsafe { close_key(key) }.expect("RegCloseKey should succeed");
    }

    #[test]
    fn query_value_fails_for_a_nonexistent_value_name() {
        // SAFETY: `HKEY_LOCAL_MACHINE` is a predefined, always-valid root.
        let key = unsafe { open_key(HKEY_LOCAL_MACHINE, CURRENT_VERSION_KEY, KEY_READ) }
            .expect("RegOpenKeyExW should succeed for a well-known, always-present subkey");
        // SAFETY: `key` was just opened above and stays open for this
        // whole call.
        let err = unsafe { query_value(key, "ThisValueNameDefinitelyDoesNotExist12345") }
            .expect_err("RegQueryValueExW should fail for a nonexistent value name");
        assert_eq!(err, crate::error::Win32Error::ERROR_FILE_NOT_FOUND);
        // SAFETY: `key` is still the same valid, currently-open handle.
        unsafe { close_key(key) }.expect("RegCloseKey should succeed");
    }

    #[test]
    fn set_value_then_query_value_round_trips_every_variant() {
        // Uniquified by this test process's own pid for the same reason
        // `create_key`'s own test is: this crate's CI job runs `cargo
        // test` twice on the same Windows VM, and while `delete_key`
        // below does clean the key back up, uniquifying is cheap
        // insurance against any run that ends before cleanup runs (e.g. a
        // panic partway through).
        let subkey = format!(
            "Software\\rusty_win32_registry_test_set_value_{}",
            std::process::id()
        );
        // SAFETY: `HKEY_CURRENT_USER` is a predefined, always-valid root.
        let (key, _disposition) =
            unsafe { create_key(HKEY_CURRENT_USER, subkey.as_str(), KEY_ALL_ACCESS) }
                .expect("RegCreateKeyExW should succeed");

        let cases = [
            ("a_sz", RegistryValue::Sz("hello".into())),
            (
                "an_expand_sz",
                RegistryValue::ExpandSz("%SystemRoot%\\system32".into()),
            ),
            ("a_dword", RegistryValue::Dword(0xDEAD_BEEF)),
            ("a_qword", RegistryValue::Qword(0xDEAD_BEEF_0BAD_F00D)),
            (
                "a_binary",
                RegistryValue::Binary(alloc::vec![1, 2, 3, 4, 5]),
            ),
            (
                "a_multi_sz",
                RegistryValue::MultiSz(alloc::vec![
                    "first".into(),
                    "second".into(),
                    "third".into()
                ]),
            ),
            ("an_empty_multi_sz", RegistryValue::MultiSz(Vec::new())),
        ];

        for (name, value) in &cases {
            // SAFETY: `key` was just created above, opened with
            // `KEY_ALL_ACCESS`, and stays open for this whole loop.
            unsafe { set_value(key, name, value) }
                .unwrap_or_else(|e| panic!("RegSetValueExW should succeed for {name}: {e}"));
            // SAFETY: same as above.
            let read_back = unsafe { query_value(key, name) }
                .unwrap_or_else(|e| panic!("RegQueryValueExW should succeed for {name}: {e}"));
            assert_eq!(&read_back, value, "round trip mismatch for {name}");
            // Clean up each value now that `delete_value` exists — this
            // key still can't be removed itself (no `delete_key` yet),
            // but there's no reason to leave its values behind too.
            // SAFETY: same as above.
            unsafe { delete_value(key, name) }
                .unwrap_or_else(|e| panic!("RegDeleteValueW should succeed for {name}: {e}"));
        }

        // SAFETY: `key` is still the same valid, currently-open handle.
        unsafe { close_key(key) }.expect("RegCloseKey should succeed");
        // SAFETY: `HKEY_CURRENT_USER` is the same predefined root; the
        // subkey is now empty of values (the loop above deleted each one)
        // and has no subkeys of its own.
        unsafe { delete_key(HKEY_CURRENT_USER, subkey.as_str(), 0) }
            .expect("RegDeleteKeyExW should succeed");
    }

    #[test]
    fn delete_value_then_query_value_fails_for_the_removed_name() {
        let subkey = format!(
            "Software\\rusty_win32_registry_test_delete_value_{}",
            std::process::id()
        );
        // SAFETY: `HKEY_CURRENT_USER` is a predefined, always-valid root.
        let (key, _disposition) =
            unsafe { create_key(HKEY_CURRENT_USER, subkey.as_str(), KEY_ALL_ACCESS) }
                .expect("RegCreateKeyExW should succeed");

        // SAFETY: `key` was just created above and stays open for this
        // whole test.
        unsafe { set_value(key, "a_value", &RegistryValue::Dword(42)) }
            .expect("RegSetValueExW should succeed");
        // SAFETY: same as above.
        unsafe { delete_value(key, "a_value") }.expect("RegDeleteValueW should succeed");
        // SAFETY: same as above.
        let err = unsafe { query_value(key, "a_value") }
            .expect_err("RegQueryValueExW should fail for a value delete_value just removed");
        assert_eq!(err, crate::error::Win32Error::ERROR_FILE_NOT_FOUND);

        // SAFETY: `key` is still the same valid, currently-open handle.
        unsafe { close_key(key) }.expect("RegCloseKey should succeed");
        // SAFETY: `HKEY_CURRENT_USER` is the same predefined root.
        unsafe { delete_key(HKEY_CURRENT_USER, subkey.as_str(), 0) }
            .expect("RegDeleteKeyExW should succeed");
    }

    #[test]
    fn delete_value_fails_for_a_nonexistent_value_name() {
        // A key opened with `KEY_ALL_ACCESS` (not `KEY_READ`), unlike
        // `query_value`'s/`open_key`'s equivalent tests against
        // `HKEY_LOCAL_MACHINE` — `RegDeleteValueW` needs `KEY_SET_VALUE`
        // access, and using a read-only handle here would leave it
        // ambiguous whether a failure meant "access denied" or "value not
        // found," rather than cleanly isolating the latter.
        let subkey = format!(
            "Software\\rusty_win32_registry_test_delete_value_missing_{}",
            std::process::id()
        );
        // SAFETY: `HKEY_CURRENT_USER` is a predefined, always-valid root.
        let (key, _disposition) =
            unsafe { create_key(HKEY_CURRENT_USER, subkey.as_str(), KEY_ALL_ACCESS) }
                .expect("RegCreateKeyExW should succeed");
        // SAFETY: `key` was just created above and stays open for this
        // whole call.
        let err = unsafe { delete_value(key, "ThisValueNameDefinitelyDoesNotExist12345") }
            .expect_err("RegDeleteValueW should fail for a nonexistent value name");
        assert_eq!(err, crate::error::Win32Error::ERROR_FILE_NOT_FOUND);
        // SAFETY: `key` is still the same valid, currently-open handle.
        unsafe { close_key(key) }.expect("RegCloseKey should succeed");
        // SAFETY: `HKEY_CURRENT_USER` is the same predefined root.
        unsafe { delete_key(HKEY_CURRENT_USER, subkey.as_str(), 0) }
            .expect("RegDeleteKeyExW should succeed");
    }

    #[test]
    fn delete_key_removes_a_leaf_subkey() {
        let subkey = format!(
            "Software\\rusty_win32_registry_test_delete_key_{}",
            std::process::id()
        );
        // SAFETY: `HKEY_CURRENT_USER` is a predefined, always-valid root.
        let (key, _disposition) =
            unsafe { create_key(HKEY_CURRENT_USER, subkey.as_str(), KEY_ALL_ACCESS) }
                .expect("RegCreateKeyExW should succeed");
        // SAFETY: `key` was just created above and not yet closed.
        unsafe { close_key(key) }.expect("RegCloseKey should succeed");

        // SAFETY: `HKEY_CURRENT_USER` is the same predefined root; the
        // subkey created above is a bare leaf with no subkeys of its own.
        unsafe { delete_key(HKEY_CURRENT_USER, subkey.as_str(), 0) }
            .expect("RegDeleteKeyExW should succeed");

        // SAFETY: same predefined root; confirming the deletion actually
        // took effect.
        let err = unsafe { open_key(HKEY_CURRENT_USER, subkey.as_str(), KEY_READ) }
            .expect_err("RegOpenKeyExW should fail: delete_key just removed this subkey");
        assert_eq!(err, crate::error::Win32Error::ERROR_FILE_NOT_FOUND);
    }

    #[test]
    fn delete_key_fails_for_a_nonexistent_subkey() {
        // SAFETY: `HKEY_CURRENT_USER` is a predefined, always-valid root.
        let err = unsafe {
            delete_key(
                HKEY_CURRENT_USER,
                "Software\\ThisSubkeyDefinitelyDoesNotExist12345",
                0,
            )
        }
        .expect_err("RegDeleteKeyExW should fail for a nonexistent subkey");
        assert_eq!(err, crate::error::Win32Error::ERROR_FILE_NOT_FOUND);
    }

    #[test]
    fn enum_values_reports_every_value_that_was_set() {
        let subkey = format!(
            "Software\\rusty_win32_registry_test_enum_values_{}",
            std::process::id()
        );
        // SAFETY: `HKEY_CURRENT_USER` is a predefined, always-valid root.
        let (key, _disposition) =
            unsafe { create_key(HKEY_CURRENT_USER, subkey.as_str(), KEY_ALL_ACCESS) }
                .expect("RegCreateKeyExW should succeed");

        let cases = [
            ("alpha", RegistryValue::Sz("one".into())),
            ("beta", RegistryValue::Dword(7)),
            ("gamma", RegistryValue::Binary(alloc::vec![9, 8, 7])),
        ];
        for (name, value) in &cases {
            // SAFETY: `key` was just created above, opened with
            // `KEY_ALL_ACCESS`, and stays open for this whole loop.
            unsafe { set_value(key, name, value) }
                .unwrap_or_else(|e| panic!("RegSetValueExW should succeed for {name}: {e}"));
        }

        // SAFETY: `key` is still the same valid, currently-open handle,
        // kept open for as long as the iterator below is used.
        let mut found: alloc::vec::Vec<(alloc::string::String, RegistryValue)> =
            unsafe { enum_values(key) }
                .collect::<Result<_, _>>()
                .expect("RegEnumValueW should succeed for every value");
        found.sort_by(|a, b| a.0.cmp(&b.0));

        let mut expected: alloc::vec::Vec<(alloc::string::String, RegistryValue)> = cases
            .iter()
            .map(|(name, value)| ((*name).into(), value.clone()))
            .collect();
        expected.sort_by(|a, b| a.0.cmp(&b.0));

        assert_eq!(found, expected);

        for (name, _) in &cases {
            // SAFETY: `key` is still the same valid, currently-open
            // handle.
            unsafe { delete_value(key, name) }
                .unwrap_or_else(|e| panic!("RegDeleteValueW should succeed for {name}: {e}"));
        }
        // SAFETY: same as above.
        unsafe { close_key(key) }.expect("RegCloseKey should succeed");
        // SAFETY: `HKEY_CURRENT_USER` is the same predefined root.
        unsafe { delete_key(HKEY_CURRENT_USER, subkey.as_str(), 0) }
            .expect("RegDeleteKeyExW should succeed");
    }

    #[test]
    fn enum_values_reports_nothing_for_a_key_with_no_values() {
        let subkey = format!(
            "Software\\rusty_win32_registry_test_enum_values_empty_{}",
            std::process::id()
        );
        // SAFETY: `HKEY_CURRENT_USER` is a predefined, always-valid root.
        let (key, _disposition) =
            unsafe { create_key(HKEY_CURRENT_USER, subkey.as_str(), KEY_ALL_ACCESS) }
                .expect("RegCreateKeyExW should succeed");

        // SAFETY: `key` was just created above, opened with
        // `KEY_ALL_ACCESS`, and stays open for as long as the iterator is
        // used.
        let found: alloc::vec::Vec<_> = unsafe { enum_values(key) }
            .collect::<Result<alloc::vec::Vec<_>, _>>()
            .expect("RegEnumValueW should succeed (by finding nothing) for a key with no values");
        assert!(found.is_empty());

        // SAFETY: `key` is still the same valid, currently-open handle.
        unsafe { close_key(key) }.expect("RegCloseKey should succeed");
        // SAFETY: `HKEY_CURRENT_USER` is the same predefined root.
        unsafe { delete_key(HKEY_CURRENT_USER, subkey.as_str(), 0) }
            .expect("RegDeleteKeyExW should succeed");
    }

    #[test]
    fn enum_keys_reports_every_immediate_subkey() {
        let parent_subkey = format!(
            "Software\\rusty_win32_registry_test_enum_keys_{}",
            std::process::id()
        );
        // SAFETY: `HKEY_CURRENT_USER` is a predefined, always-valid root.
        let (parent, _disposition) =
            unsafe { create_key(HKEY_CURRENT_USER, parent_subkey.as_str(), KEY_ALL_ACCESS) }
                .expect("RegCreateKeyExW should succeed");

        let child_names = ["child_one", "child_two"];
        for name in &child_names {
            // SAFETY: `parent` was just created above, opened with
            // `KEY_ALL_ACCESS`, and stays open for this whole loop.
            let (child, _disposition) = unsafe { create_key(parent, name, KEY_ALL_ACCESS) }
                .unwrap_or_else(|e| panic!("RegCreateKeyExW should succeed for {name}: {e}"));
            // SAFETY: `child` was just created above and not yet closed.
            unsafe { close_key(child) }.expect("RegCloseKey should succeed");
        }

        // SAFETY: `parent` is still the same valid, currently-open
        // handle, kept open for as long as the iterator below is used.
        let found: alloc::vec::Vec<(alloc::string::String, Timespec)> =
            unsafe { enum_keys(parent) }
                .collect::<Result<_, _>>()
                .expect("RegEnumKeyExW should succeed for every subkey");

        let mut found_names: alloc::vec::Vec<alloc::string::String> =
            found.iter().map(|(name, _)| name.clone()).collect();
        found_names.sort();
        let mut expected_names: alloc::vec::Vec<alloc::string::String> =
            child_names.iter().map(|s| (*s).into()).collect();
        expected_names.sort();
        assert_eq!(found_names, expected_names);

        for (_, last_write) in &found {
            assert!(
                last_write.secs > 0,
                "a subkey created moments ago should report a plausible post-1970 last-write time, got {last_write:?}"
            );
        }

        for name in &child_names {
            // SAFETY: `HKEY_CURRENT_USER` is a predefined, always-valid
            // root; `parent` is still the same valid, currently-open
            // handle. Each child is itself a bare leaf (no subkeys of its
            // own).
            unsafe { delete_key(parent, name, 0) }
                .unwrap_or_else(|e| panic!("RegDeleteKeyExW should succeed for {name}: {e}"));
        }
        // SAFETY: `parent` is still the same valid, currently-open
        // handle.
        unsafe { close_key(parent) }.expect("RegCloseKey should succeed");
        // SAFETY: `HKEY_CURRENT_USER` is the same predefined root; the
        // parent subkey is now empty of children (the loop above deleted
        // each one).
        unsafe { delete_key(HKEY_CURRENT_USER, parent_subkey.as_str(), 0) }
            .expect("RegDeleteKeyExW should succeed");
    }

    #[test]
    fn enum_keys_reports_nothing_for_a_key_with_no_subkeys() {
        let subkey = format!(
            "Software\\rusty_win32_registry_test_enum_keys_empty_{}",
            std::process::id()
        );
        // SAFETY: `HKEY_CURRENT_USER` is a predefined, always-valid root.
        let (key, _disposition) =
            unsafe { create_key(HKEY_CURRENT_USER, subkey.as_str(), KEY_ALL_ACCESS) }
                .expect("RegCreateKeyExW should succeed");

        // SAFETY: `key` was just created above, opened with
        // `KEY_ALL_ACCESS`, and stays open for as long as the iterator is
        // used.
        let found: alloc::vec::Vec<_> = unsafe { enum_keys(key) }
            .collect::<Result<alloc::vec::Vec<_>, _>>()
            .expect("RegEnumKeyExW should succeed (by finding nothing) for a key with no subkeys");
        assert!(found.is_empty());

        // SAFETY: `key` is still the same valid, currently-open handle.
        unsafe { close_key(key) }.expect("RegCloseKey should succeed");
        // SAFETY: `HKEY_CURRENT_USER` is the same predefined root.
        unsafe { delete_key(HKEY_CURRENT_USER, subkey.as_str(), 0) }
            .expect("RegDeleteKeyExW should succeed");
    }

    #[test]
    fn key_info_reports_plausible_counts_and_lengths() {
        let subkey = format!(
            "Software\\rusty_win32_registry_test_key_info_{}",
            std::process::id()
        );
        // SAFETY: `HKEY_CURRENT_USER` is a predefined, always-valid root.
        let (key, _disposition) =
            unsafe { create_key(HKEY_CURRENT_USER, subkey.as_str(), KEY_ALL_ACCESS) }
                .expect("RegCreateKeyExW should succeed");

        // SAFETY: `key` was just created above, opened with
        // `KEY_ALL_ACCESS`, and stays open for this whole test.
        unsafe { set_value(key, "a_value_name", &RegistryValue::Dword(1)) }
            .expect("RegSetValueExW should succeed");
        // SAFETY: same as above.
        let (child, _disposition) = unsafe { create_key(key, "a_child", KEY_ALL_ACCESS) }
            .expect("RegCreateKeyExW should succeed");
        // SAFETY: `child` was just created above and not yet closed.
        unsafe { close_key(child) }.expect("RegCloseKey should succeed");

        // SAFETY: `key` is still the same valid, currently-open handle.
        let info = unsafe { key_info(key) }.expect("RegQueryInfoKeyW should succeed");
        assert_eq!(info.sub_key_count, 1);
        assert_eq!(info.value_count, 1);
        assert!(
            info.max_sub_key_len >= "a_child".len() as u32,
            "max_sub_key_len should be at least as long as the one subkey's own name"
        );
        assert!(
            info.max_value_name_len >= "a_value_name".len() as u32,
            "max_value_name_len should be at least as long as the one value's own name"
        );
        assert!(
            info.last_write_time.secs > 0,
            "a key modified moments ago should report a plausible post-1970 last-write time"
        );

        // SAFETY: `key` is still the same valid, currently-open handle.
        unsafe { delete_value(key, "a_value_name") }.expect("RegDeleteValueW should succeed");
        // SAFETY: same as above.
        unsafe { delete_key(key, "a_child", 0) }.expect("RegDeleteKeyExW should succeed");
        // SAFETY: same as above.
        unsafe { close_key(key) }.expect("RegCloseKey should succeed");
        // SAFETY: `HKEY_CURRENT_USER` is the same predefined root; the
        // subkey is now empty of both its value and its child.
        unsafe { delete_key(HKEY_CURRENT_USER, subkey.as_str(), 0) }
            .expect("RegDeleteKeyExW should succeed");
    }

    #[test]
    fn key_info_reports_zero_counts_for_a_fresh_empty_key() {
        let subkey = format!(
            "Software\\rusty_win32_registry_test_key_info_empty_{}",
            std::process::id()
        );
        // SAFETY: `HKEY_CURRENT_USER` is a predefined, always-valid root.
        let (key, _disposition) =
            unsafe { create_key(HKEY_CURRENT_USER, subkey.as_str(), KEY_ALL_ACCESS) }
                .expect("RegCreateKeyExW should succeed");

        // SAFETY: `key` was just created above and stays open for this
        // whole call.
        let info = unsafe { key_info(key) }.expect("RegQueryInfoKeyW should succeed");
        assert_eq!(info.sub_key_count, 0);
        assert_eq!(info.value_count, 0);

        // SAFETY: `key` is still the same valid, currently-open handle.
        unsafe { close_key(key) }.expect("RegCloseKey should succeed");
        // SAFETY: `HKEY_CURRENT_USER` is the same predefined root.
        unsafe { delete_key(HKEY_CURRENT_USER, subkey.as_str(), 0) }
            .expect("RegDeleteKeyExW should succeed");
    }

    #[test]
    fn flush_key_succeeds_after_a_write() {
        let subkey = format!(
            "Software\\rusty_win32_registry_test_flush_key_{}",
            std::process::id()
        );
        // SAFETY: `HKEY_CURRENT_USER` is a predefined, always-valid root.
        let (key, _disposition) =
            unsafe { create_key(HKEY_CURRENT_USER, subkey.as_str(), KEY_ALL_ACCESS) }
                .expect("RegCreateKeyExW should succeed");

        // SAFETY: `key` was just created above, opened with
        // `KEY_ALL_ACCESS`, and stays open for this whole test.
        unsafe { set_value(key, "a_value", &RegistryValue::Dword(1)) }
            .expect("RegSetValueExW should succeed");
        // SAFETY: same as above; this is the operation under test.
        unsafe { flush_key(key) }.expect("RegFlushKey should succeed");

        // SAFETY: `key` is still the same valid, currently-open handle.
        unsafe { delete_value(key, "a_value") }.expect("RegDeleteValueW should succeed");
        // SAFETY: same as above.
        unsafe { close_key(key) }.expect("RegCloseKey should succeed");
        // SAFETY: `HKEY_CURRENT_USER` is the same predefined root.
        unsafe { delete_key(HKEY_CURRENT_USER, subkey.as_str(), 0) }
            .expect("RegDeleteKeyExW should succeed");
    }

    #[test]
    fn delete_tree_removes_a_subkey_and_its_own_child() {
        let parent_subkey = format!(
            "Software\\rusty_win32_registry_test_delete_tree_{}",
            std::process::id()
        );
        // SAFETY: `HKEY_CURRENT_USER` is a predefined, always-valid root.
        let (parent, _disposition) =
            unsafe { create_key(HKEY_CURRENT_USER, parent_subkey.as_str(), KEY_ALL_ACCESS) }
                .expect("RegCreateKeyExW should succeed");
        // SAFETY: `parent` was just created above, opened with
        // `KEY_ALL_ACCESS`, and stays open for this whole test.
        unsafe { set_value(parent, "a_value", &RegistryValue::Dword(1)) }
            .expect("RegSetValueExW should succeed");
        // SAFETY: same as above.
        let (child, _disposition) = unsafe { create_key(parent, "a_child", KEY_ALL_ACCESS) }
            .expect("RegCreateKeyExW should succeed");
        // SAFETY: `child` was just created above and not yet closed —
        // `delete_key` on `parent` alone would refuse to remove it while
        // this child still exists, which is exactly the restriction
        // `delete_tree` doesn't have.
        unsafe { close_key(child) }.expect("RegCloseKey should succeed");
        // SAFETY: `parent` is still the same valid, currently-open
        // handle.
        unsafe { close_key(parent) }.expect("RegCloseKey should succeed");

        // SAFETY: `HKEY_CURRENT_USER` is the same predefined root; this
        // is the operation under test.
        unsafe { delete_tree(HKEY_CURRENT_USER, parent_subkey.as_str()) }
            .expect("RegDeleteTreeW should succeed");

        // SAFETY: same predefined root; confirming the whole subtree —
        // parent included — is actually gone, not just its child.
        let err = unsafe { open_key(HKEY_CURRENT_USER, parent_subkey.as_str(), KEY_READ) }
            .expect_err("RegOpenKeyExW should fail: delete_tree just removed this whole subtree");
        assert_eq!(err, crate::error::Win32Error::ERROR_FILE_NOT_FOUND);
    }

    #[test]
    fn delete_tree_fails_for_a_nonexistent_subkey() {
        // SAFETY: `HKEY_CURRENT_USER` is a predefined, always-valid root.
        let err = unsafe {
            delete_tree(
                HKEY_CURRENT_USER,
                "Software\\ThisSubkeyDefinitelyDoesNotExist12345",
            )
        }
        .expect_err("RegDeleteTreeW should fail for a nonexistent subkey");
        assert_eq!(err, crate::error::Win32Error::ERROR_FILE_NOT_FOUND);
    }
}
