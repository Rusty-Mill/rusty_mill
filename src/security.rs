//! Windows file/directory security (owner + DACL) — `aclapi.h`, a new
//! module added in round 2, previously excluded by `ARCHITECTURE.md`'s
//! non-goals (see `docs/archive/gap-analysis.md`'s, now closed out and
//! archived, "Round 2: previously out-of-scope
//! subsystems" sweep), now in scope per explicit direction. No current
//! `rush` feature asks for this yet.
//!
//! Scope: file/directory owner and DACL inspection+modification — an
//! `icacls`/`ls -l`/`chmod`/`chown`-equivalent, not a from-scratch
//! reimplementation of the whole Windows security model. SACLs/auditing,
//! privilege/token manipulation, impersonation, and low-level
//! absolute-SD plumbing are all explicitly out of scope (see
//! `docs/archive/gap-analysis.md`'s own design notes for this module).
//!
//! This first piece is the core round trip: path → owner `PSID`/DACL
//! `PACL`, and back. A `PSID`/`PACL`/security-descriptor blob is
//! Windows' famously variable-length, self-describing structure —
//! treated as an opaque pointer manipulated only through the OS's own
//! accessor functions, never a locally-defined fixed-layout struct the
//! way this crate mirrors most other Win32 structs.

extern crate alloc;
use alloc::vec::Vec;

#[link(name = "advapi32")]
unsafe extern "system" {
    fn GetNamedSecurityInfoW(
        object_name: *const u16,
        object_type: u32,
        security_info: u32,
        owner: *mut *mut core::ffi::c_void,
        group: *mut *mut core::ffi::c_void,
        dacl: *mut *mut core::ffi::c_void,
        sacl: *mut *mut core::ffi::c_void,
        security_descriptor: *mut *mut core::ffi::c_void,
    ) -> u32;
    fn SetNamedSecurityInfoW(
        object_name: *mut u16,
        object_type: u32,
        security_info: u32,
        owner: *mut core::ffi::c_void,
        group: *mut core::ffi::c_void,
        dacl: *mut core::ffi::c_void,
        sacl: *mut core::ffi::c_void,
    ) -> u32;
    fn GetAclInformation(
        acl: *const Acl,
        acl_information: *mut core::ffi::c_void,
        acl_information_length: u32,
        acl_information_class: u32,
    ) -> i32;
    fn GetAce(acl: *const Acl, ace_index: u32, ace: *mut *mut core::ffi::c_void) -> i32;
    fn SetEntriesInAclW(
        count: u32,
        entries: *const ExplicitAccess,
        old_acl: *const Acl,
        new_acl: *mut *mut Acl,
    ) -> u32;
    fn BuildTrusteeWithSidW(trustee: *mut Trustee, sid: *mut core::ffi::c_void);
    fn BuildTrusteeWithNameW(trustee: *mut Trustee, name: *mut u16);
    fn LookupAccountSidW(
        system_name: *const u16,
        sid: *mut core::ffi::c_void,
        name: *mut u16,
        name_len: *mut u32,
        referenced_domain_name: *mut u16,
        domain_len: *mut u32,
        sid_name_use: *mut i32,
    ) -> i32;
    fn LookupAccountNameW(
        system_name: *const u16,
        account_name: *const u16,
        sid: *mut u8,
        sid_len: *mut u32,
        referenced_domain_name: *mut u16,
        domain_len: *mut u32,
        sid_name_use: *mut i32,
    ) -> i32;
    fn ConvertSidToStringSidW(sid: *mut core::ffi::c_void, string_sid: *mut *mut u16) -> i32;
    fn ConvertStringSidToSidW(string_sid: *const u16, sid: *mut *mut core::ffi::c_void) -> i32;
    fn InitializeAcl(acl: *mut Acl, acl_length: u32, acl_revision: u32) -> i32;
    fn AddAccessAllowedAce(
        acl: *mut Acl,
        ace_revision: u32,
        access_mask: u32,
        sid: *mut core::ffi::c_void,
    ) -> i32;
    fn AddAccessDeniedAce(
        acl: *mut Acl,
        ace_revision: u32,
        access_mask: u32,
        sid: *mut core::ffi::c_void,
    ) -> i32;
    fn GetLengthSid(sid: *mut core::ffi::c_void) -> u32;
    fn IsValidSid(sid: *mut core::ffi::c_void) -> i32;
    fn CopySid(
        dest_length: u32,
        dest_sid: *mut core::ffi::c_void,
        source_sid: *mut core::ffi::c_void,
    ) -> i32;
    fn EqualSid(a: *mut core::ffi::c_void, b: *mut core::ffi::c_void) -> i32;
    fn CreateWellKnownSid(
        well_known_sid_type: i32,
        domain_sid: *mut core::ffi::c_void,
        sid: *mut core::ffi::c_void,
        sid_size: *mut u32,
    ) -> i32;
    fn ConvertSecurityDescriptorToStringSecurityDescriptorW(
        sd: *mut core::ffi::c_void,
        requested_revision: u32,
        security_information: u32,
        string_sd: *mut *mut u16,
        string_sd_len: *mut u32,
    ) -> i32;
    fn ConvertStringSecurityDescriptorToSecurityDescriptorW(
        string_sd: *const u16,
        string_sd_revision: u32,
        sd: *mut *mut core::ffi::c_void,
        sd_size: *mut u32,
    ) -> i32;
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn LocalFree(mem: *mut core::ffi::c_void) -> *mut core::ffi::c_void;
}

// ACL: `size_of` 8, `align_of` 2 on x86_64 — verified against
// mingw-w64's own `winnt.h` with a compiled `_Static_assert` probe, the
// same discipline this crate uses for every other struct layout it can't
// check any other way. A real, variable-length ACL has its ACEs packed
// immediately after this fixed header — not represented as a Rust field,
// the same "fixed-header-only" treatment `fs.rs`'s
// `ReparseDataBufferSymlinkHeader` already uses for a different
// variable-length Win32 structure. This crate never reads `Acl`'s own
// fields directly (not even `ace_count`) — always through
// `GetAclInformation`/`GetAce`, matching `docs/archive/gap-analysis.md`'s
// design notes for this module: an ACL is manipulated only through its own
// accessor functions, the same as a `PSID`.
#[repr(C)]
pub struct Acl {
    acl_revision: u8,
    sbz1: u8,
    acl_size: u16,
    ace_count: u16,
    sbz2: u16,
}
const _: () = assert!(core::mem::size_of::<Acl>() == 8);
const _: () = assert!(core::mem::align_of::<Acl>() == 2);

// ACE_HEADER: `size_of` 4 — every ACE (allow, deny, audit, …) starts
// with this fixed prefix regardless of its own specific shape.
#[repr(C)]
#[derive(Clone, Copy)]
struct AceHeader {
    ace_type: u8,
    ace_flags: u8,
    ace_size: u16,
}
const _: () = assert!(core::mem::size_of::<AceHeader>() == 4);

/// `ACE_HEADER.AceType`'s two ordinary kinds — the only ones this module
/// decodes further (audit, object-specific, callback, and other rarer
/// ACE types are out of scope; [`acl_entries`] still reports them, as
/// `Other`, rather than silently dropping them from the list).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AceKind {
    /// `ACCESS_ALLOWED_ACE_TYPE`.
    Allow,
    /// `ACCESS_DENIED_ACE_TYPE`.
    Deny,
    /// Any other `AceType` this module doesn't decode further.
    Other(u8),
}
const ACCESS_ALLOWED_ACE_TYPE: u8 = 0;
const ACCESS_DENIED_ACE_TYPE: u8 = 1;

/// `GetAclInformation`'s `ACL_INFORMATION_CLASS` value this module
/// needs — the ACE count (plus byte-usage figures this crate doesn't
/// otherwise expose).
const ACL_SIZE_INFORMATION_CLASS: u32 = 2;

// ACL_SIZE_INFORMATION: `size_of` 12 — three plain `DWORD`s, no padding.
#[repr(C)]
#[derive(Default, Clone, Copy)]
struct AclSizeInformation {
    ace_count: u32,
    acl_bytes_in_use: u32,
    acl_bytes_free: u32,
}
const _: () = assert!(core::mem::size_of::<AclSizeInformation>() == 12);

/// One decoded ACE from [`acl_entries`] — allow/deny ACEs share the same
/// `ACCESS_ALLOWED_ACE`-shaped layout (`Header`/`Mask`/a trailing `SID`),
/// so both are represented the same way here; `kind` distinguishes
/// which.
#[derive(Debug, Clone, Copy)]
pub struct AclEntry {
    pub kind: AceKind,
    /// `ACE_HEADER.AceFlags` — inheritance/inherited/audit bits, exposed
    /// raw and policy-free, matching this crate's existing convention
    /// for other bitmask fields.
    pub flags: u8,
    /// `ACCESS_MASK` — this ACE's permission bits. Only meaningful for
    /// [`AceKind::Allow`]/[`AceKind::Deny`]; `0` for any other kind
    /// (which has no `Mask` field at this same offset to begin with).
    pub mask: u32,
    /// A `PSID` pointing into the ACL's own memory (not separately
    /// owned/freed) — opaque, never decoded structurally by this crate.
    /// `None` for any ACE kind other than
    /// [`AceKind::Allow`]/[`AceKind::Deny`].
    pub sid: Option<*const core::ffi::c_void>,
}

/// `SE_OBJECT_TYPE`'s file-object variant — this module's only supported
/// object type (out of the many `SetNamedSecurityInfoW` itself supports:
/// services, printers, registry keys, …), matching its file/directory-
/// only scope. Verified against mingw-w64's own `accctrl.h`.
const SE_FILE_OBJECT: u32 = 1;

/// `SECURITY_INFORMATION` bits for [`path_security_info`] — which parts
/// of an object's security descriptor to fetch. Exposed raw and
/// policy-free, matching this crate's existing convention for other
/// bitmask fields (`registry::KEY_*`, `fs::FILE_ATTRIBUTE_*`).
/// `SecurityInfoFlags` is a plain alias, not a distinct type. Verified
/// against mingw-w64's own `winnt.h` macros with a compiled
/// `_Static_assert` probe.
pub type SecurityInfoFlags = u32;
pub const OWNER_SECURITY_INFORMATION: SecurityInfoFlags = 0x0000_0001;
pub const GROUP_SECURITY_INFORMATION: SecurityInfoFlags = 0x0000_0002;
pub const DACL_SECURITY_INFORMATION: SecurityInfoFlags = 0x0000_0004;

/// A path's owner/DACL, as returned by [`path_security_info`] (or built
/// by hand via [`PathSecurityInfo::from_raw_parts`] to pass to
/// [`set_path_security_info`]). `owner`/`dacl` are opaque `PSID`/`PACL`
/// pointers this crate never decodes structurally — pass them to
/// another Win32 API (`LookupAccountSidW`, a future `security` module
/// item) to make sense of them, or straight back into
/// [`set_path_security_info`] to write them onto a (possibly different)
/// path.
#[derive(Debug)]
pub struct PathSecurityInfo {
    // The whole `PSECURITY_DESCRIPTOR` block `GetNamedSecurityInfoW`
    // allocated — `owner`/`dacl` below point *into* this same block (a
    // self-relative security descriptor), so freeing it on `Drop` frees
    // them too; they are never separately `LocalFree`d. `None` (nothing
    // to free) for a value built by hand via `from_raw_parts` rather
    // than returned from `path_security_info`.
    security_descriptor: Option<*mut core::ffi::c_void>,
    owner: Option<*mut core::ffi::c_void>,
    dacl: Option<*mut core::ffi::c_void>,
}

impl PathSecurityInfo {
    /// Build a `PathSecurityInfo` from raw, already-valid `PSID`/`PACL`
    /// pointers obtained elsewhere — for a caller constructing a
    /// security descriptor to apply via [`set_path_security_info`]
    /// rather than one read back via [`path_security_info`]. Neither
    /// pointer is freed on `Drop`: this crate has no allocator-tracking
    /// to know whether it safely can, so ownership/lifetime of whatever
    /// `owner`/`dacl` point at remains entirely the caller's
    /// responsibility.
    ///
    /// # Safety
    ///
    /// `owner`, if `Some`, must be a valid `PSID` for as long as this
    /// value exists and for every operation it's passed to. Same for
    /// `dacl` as a valid `PACL`.
    pub unsafe fn from_raw_parts(
        owner: Option<*mut core::ffi::c_void>,
        dacl: Option<*mut core::ffi::c_void>,
    ) -> Self {
        PathSecurityInfo {
            security_descriptor: None,
            owner,
            dacl,
        }
    }

    /// The object's owner `PSID`, if [`path_security_info`] was asked
    /// for it (`OWNER_SECURITY_INFORMATION` in its `info` flags) —
    /// opaque, never decoded structurally by this crate.
    pub fn owner(&self) -> Option<*mut core::ffi::c_void> {
        self.owner
    }

    /// The object's DACL `PACL`, if [`path_security_info`] was asked for
    /// it (`DACL_SECURITY_INFORMATION` in its `info` flags) — opaque,
    /// never decoded structurally by this crate.
    pub fn dacl(&self) -> Option<*mut core::ffi::c_void> {
        self.dacl
    }

    /// The whole self-relative `PSECURITY_DESCRIPTOR` block
    /// `path_security_info` fetched (`owner()`/`dacl()` point into this
    /// same block) — `None` for a value built by hand via
    /// [`PathSecurityInfo::from_raw_parts`] rather than one
    /// `path_security_info` returned. Useful as-is for [`sd_to_string`],
    /// which treats a security descriptor as an opaque blob.
    pub fn raw_security_descriptor(&self) -> Option<*mut core::ffi::c_void> {
        self.security_descriptor
    }
}

impl Drop for PathSecurityInfo {
    fn drop(&mut self) {
        if let Some(sd) = self.security_descriptor {
            // SAFETY: `sd` is the exact `PSECURITY_DESCRIPTOR` pointer
            // `GetNamedSecurityInfoW` allocated for this value and
            // hasn't been freed yet — freed here exactly once, on this
            // value's only path to being dropped.
            let _ = unsafe { LocalFree(sd) };
        }
    }
}

/// Read `path`'s owner and/or DACL — `GetNamedSecurityInfoW`. `info`
/// selects which parts to fetch (e.g. [`OWNER_SECURITY_INFORMATION`] `|`
/// [`DACL_SECURITY_INFORMATION`]); a part not requested comes back as
/// `None` from the result's own accessor, not an error.
///
/// Reports failure via its own return value directly — never
/// `GetLastError` — so a nonzero return is passed straight to
/// [`crate::error::Win32Error::from_raw`] rather than `Win32Error::last`.
pub fn path_security_info(
    path: &str,
    info: SecurityInfoFlags,
) -> Result<PathSecurityInfo, crate::error::Win32Error> {
    let wide: Vec<u16> = path.encode_utf16().chain(core::iter::once(0)).collect();
    let mut owner: *mut core::ffi::c_void = core::ptr::null_mut();
    let mut group: *mut core::ffi::c_void = core::ptr::null_mut();
    let mut dacl: *mut core::ffi::c_void = core::ptr::null_mut();
    let mut security_descriptor: *mut core::ffi::c_void = core::ptr::null_mut();
    // SAFETY: `wide` is a valid, NUL-terminated UTF-16 string live for
    // the whole call; every out-pointer is a valid pointer to one of
    // this function's own locals; `sacl` is null since this module's
    // scope explicitly excludes SACLs.
    let status = unsafe {
        GetNamedSecurityInfoW(
            wide.as_ptr(),
            SE_FILE_OBJECT,
            info,
            &mut owner,
            &mut group,
            &mut dacl,
            core::ptr::null_mut(),
            &mut security_descriptor,
        )
    };
    if status != 0 {
        return Err(crate::error::Win32Error::from_raw(status));
    }
    Ok(PathSecurityInfo {
        security_descriptor: if security_descriptor.is_null() {
            None
        } else {
            Some(security_descriptor)
        },
        owner: if owner.is_null() { None } else { Some(owner) },
        dacl: if dacl.is_null() { None } else { Some(dacl) },
    })
}

/// Write `path`'s owner and/or DACL — `SetNamedSecurityInfoW`. Which
/// parts actually get written is inferred from `info` itself: `owner()`
/// present sets the owner, `dacl()` present sets the DACL; a part that's
/// `None` is left untouched on `path` (`SetNamedSecurityInfoW`'s own
/// null-pointer-means-"don't touch this part" convention, not a
/// separate flags parameter this crate would otherwise have to keep in
/// sync with `info`'s own contents).
///
/// # Safety
///
/// Every non-`None` pointer `info` carries (`owner()`/`dacl()`) must
/// still be a valid `PSID`/`PACL` at the time of this call.
pub unsafe fn set_path_security_info(
    path: &str,
    info: &PathSecurityInfo,
) -> Result<(), crate::error::Win32Error> {
    let mut wide: Vec<u16> = path.encode_utf16().chain(core::iter::once(0)).collect();
    let mut security_info: u32 = 0;
    if info.owner.is_some() {
        security_info |= OWNER_SECURITY_INFORMATION;
    }
    if info.dacl.is_some() {
        security_info |= DACL_SECURITY_INFORMATION;
    }
    // SAFETY: `wide` is a valid, mutable, NUL-terminated UTF-16 buffer
    // (`SetNamedSecurityInfoW` takes `LPWSTR`, not `LPCWSTR`, though it
    // doesn't document actually mutating it); `info.owner`/`info.dacl`
    // are valid per this function's own safety contract, or null
    // (documented as "leave this part unchanged"); `group`/`sacl` are
    // always null, since this module doesn't track a group SID and
    // explicitly excludes SACLs.
    let status = unsafe {
        SetNamedSecurityInfoW(
            wide.as_mut_ptr(),
            SE_FILE_OBJECT,
            security_info,
            info.owner.unwrap_or(core::ptr::null_mut()),
            core::ptr::null_mut(),
            info.dacl.unwrap_or(core::ptr::null_mut()),
            core::ptr::null_mut(),
        )
    };
    if status == 0 {
        Ok(())
    } else {
        Err(crate::error::Win32Error::from_raw(status))
    }
}

/// Enumerate `acl`'s ACEs — `GetAclInformation` (for the ACE count) plus
/// one `GetAce` call per entry — turning an opaque DACL/SACL into the
/// human-readable permission list `icacls`/`ls -l` displays. Each `GetAce`
/// call hands back a pointer into `acl`'s own existing memory (not a
/// fresh allocation), so the returned entries' [`AclEntry::sid`] pointers
/// stay valid only as long as `acl` itself does.
///
/// # Safety
///
/// `acl` must be a valid, currently-live `PACL` — e.g. one
/// [`PathSecurityInfo::dacl`] returned, kept alive (the `PathSecurityInfo`
/// it came from not yet dropped) for as long as this call and the
/// returned entries are in use.
pub unsafe fn acl_entries(
    acl: *const Acl,
) -> Result<alloc::vec::Vec<AclEntry>, crate::error::Win32Error> {
    let mut size_info = AclSizeInformation::default();
    // SAFETY: `acl` is caller-supplied per this function's own safety
    // contract; `size_info` is a valid out-pointer of exactly the size
    // named by `acl_information_length`.
    let ok = unsafe {
        GetAclInformation(
            acl,
            (&mut size_info as *mut AclSizeInformation).cast(),
            core::mem::size_of::<AclSizeInformation>() as u32,
            ACL_SIZE_INFORMATION_CLASS,
        )
    };
    if ok == 0 {
        return Err(crate::error::Win32Error::last());
    }

    let mut entries = alloc::vec::Vec::with_capacity(size_info.ace_count as usize);
    for index in 0..size_info.ace_count {
        let mut ace_ptr: *mut core::ffi::c_void = core::ptr::null_mut();
        // SAFETY: `acl` is the same caller-supplied, valid `PACL`;
        // `index` is within `0..ace_count`, the range `GetAclInformation`
        // just reported; `ace_ptr` is a valid out-pointer.
        let ok = unsafe { GetAce(acl, index, &mut ace_ptr) };
        if ok == 0 {
            return Err(crate::error::Win32Error::last());
        }
        // SAFETY: a successful `GetAce` guarantees `ace_ptr` points to a
        // real ACE at least `ACE_HEADER`-sized (every ACE, regardless of
        // its specific kind, starts with this fixed prefix).
        let header: AceHeader = unsafe { core::ptr::read_unaligned(ace_ptr.cast()) };
        let kind = match header.ace_type {
            ACCESS_ALLOWED_ACE_TYPE => AceKind::Allow,
            ACCESS_DENIED_ACE_TYPE => AceKind::Deny,
            other => AceKind::Other(other),
        };
        let (mask, sid) = if matches!(kind, AceKind::Allow | AceKind::Deny) {
            // SAFETY: an allow/deny ACE shares `ACCESS_ALLOWED_ACE`'s
            // layout, guaranteed at least 12 bytes (`Header` + `Mask` +
            // the trailing SID's first `DWORD`) by Windows itself for
            // these two ACE types.
            let mask: u32 = unsafe { core::ptr::read_unaligned(ace_ptr.byte_add(4).cast()) };
            // SAFETY: same guarantee — byte offset 8 is
            // `ACCESS_ALLOWED_ACE::SidStart`, the trailing SID's start.
            let sid = unsafe { ace_ptr.byte_add(8) }.cast_const();
            (mask, Some(sid))
        } else {
            (0, None)
        };
        entries.push(AclEntry {
            kind,
            flags: header.ace_flags,
            mask,
            sid,
        });
    }
    Ok(entries)
}

/// `TRUSTEE_FORM` — whether a [`Trustee`]'s name field holds a `PSID` or
/// a wide string name. Only these two ordinary forms are supported
/// (`TRUSTEE_BAD_FORM` and the object-specific `_AND_SID`/`_AND_NAME`
/// variants are out of scope). Verified against mingw-w64's own
/// `aclapi.h` with a compiled `_Static_assert` probe.
#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrusteeForm {
    Sid = 0,
    Name = 1,
}

/// `TRUSTEE_TYPE` — what kind of principal a [`Trustee`] names. Only
/// the commonly-used variants are exposed; the rest (`TRUSTEE_IS_DOMAIN`/
/// `_ALIAS`/`_DELETED`/`_INVALID`/`_COMPUTER`) aren't needed for an
/// ordinary user/group ACL entry.
#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrusteeType {
    Unknown = 0,
    User = 1,
    Group = 2,
    WellKnownGroup = 5,
}

// TRUSTEE_W: `size_of` 32, `align_of` 8 — verified against mingw-w64's
// own `aclapi.h` with a compiled `_Static_assert` probe. Genuinely
// fixed-size, unlike `Acl`/`PSID` — an ordinary FFI-mirror struct with
// full field access, per `docs/archive/gap-analysis.md`'s design notes
// for this module.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Trustee {
    // "Multiple trustee" chaining is an obscure, rarely-used mechanism
    // this crate doesn't support — `BuildTrusteeWithSidW`/
    // `BuildTrusteeWithNameW` always set these two fields to "none"
    // themselves, so they stay private rather than exposing a knob with
    // only one ever-correct value.
    multiple_trustee: *mut Trustee,
    multiple_trustee_operation: i32,
    pub trustee_form: TrusteeForm,
    pub trustee_type: TrusteeType,
    /// The real `TRUSTEE_W::ptstrName` field — either a `PSID` (when
    /// `trustee_form` is [`TrusteeForm::Sid`]) or a NUL-terminated wide
    /// string (when [`TrusteeForm::Name`]), reinterpreted either way
    /// through this same `LPWCH`-typed slot, matching the real
    /// `TRUSTEE_W`'s own union-by-convention use of this field.
    pub name: *mut u16,
}

/// Build a `Trustee` naming a `PSID` — `BuildTrusteeWithSidW`. The
/// resulting `Trustee`'s own `name` pointer is `sid` itself (matching
/// `TRUSTEE_W`'s `ptstrName` field's `TrusteeForm::Sid`-reinterpreted-
/// as-`PSID` convention) — nothing is copied.
///
/// `TrusteeType::Unknown` matches `BuildTrusteeWithSidW`'s own behavior:
/// it always sets `TrusteeType` to `TRUSTEE_IS_UNKNOWN`, never inspecting
/// `sid` to guess whether it names a user/group/well-known-group. A
/// caller wanting a specific [`TrusteeType`] recorded sets `.trustee_type`
/// on the result afterward (not exposed as a parameter here, matching
/// `BuildTrusteeWithSidW`'s own real signature).
///
/// # Safety
///
/// `sid` must be a valid `PSID` for as long as the returned `Trustee`
/// (and anything it's passed to, e.g. an [`ExplicitAccess`] entry given
/// to [`build_acl`]) is in use.
pub unsafe fn build_trustee_with_sid(sid: *mut core::ffi::c_void) -> Trustee {
    let mut trustee = core::mem::MaybeUninit::<Trustee>::uninit();
    // SAFETY: `sid` is caller-supplied per this function's own safety
    // contract; `trustee` is a valid, correctly-sized out-pointer.
    unsafe { BuildTrusteeWithSidW(trustee.as_mut_ptr(), sid) };
    // SAFETY: `BuildTrusteeWithSidW` is documented to always fully
    // initialize every field of the `TRUSTEE_W` it's given.
    unsafe { trustee.assume_init() }
}

/// Build a `Trustee` naming a wide-string principal (e.g. `"DOMAIN\name"`,
/// or a `S-1-5-...` SID string) — `BuildTrusteeWithNameW`. Unlike
/// [`build_trustee_with_sid`], this doesn't copy `name`'s bytes either —
/// the returned `Trustee`'s own `name` pointer is `name.as_mut_ptr()`
/// itself. Takes an already-built, NUL-terminated wide buffer (not `&str`,
/// diverging from this issue's literal signature) for exactly that
/// reason: an internally-built temporary `Vec<u16>` would be freed the
/// moment this function returned, leaving the result's `name` pointer
/// dangling — the caller must own a buffer that actually outlives the
/// `Trustee`.
///
/// # Safety
///
/// `name` must be a valid, NUL-terminated UTF-16 string, kept alive (not
/// dropped or reallocated) for as long as the returned `Trustee` (and
/// anything it's passed to) is in use.
pub unsafe fn build_trustee_with_name(name: &mut [u16]) -> Trustee {
    let mut trustee = core::mem::MaybeUninit::<Trustee>::uninit();
    // SAFETY: `name` is caller-supplied per this function's own safety
    // contract; `trustee` is a valid, correctly-sized out-pointer.
    unsafe { BuildTrusteeWithNameW(trustee.as_mut_ptr(), name.as_mut_ptr()) };
    // SAFETY: `BuildTrusteeWithNameW` is documented to always fully
    // initialize every field of the `TRUSTEE_W` it's given.
    unsafe { trustee.assume_init() }
}

/// `ACCESS_MODE` — what an [`ExplicitAccess`] entry does to the ACL being
/// built. Only the four ordinary modes are exposed
/// (`SET_AUDIT_SUCCESS`/`SET_AUDIT_FAILURE` are SACL/auditing-only,
/// explicitly out of this module's scope).
#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessMode {
    Grant = 1,
    Set = 2,
    Deny = 3,
    Revoke = 4,
}

// EXPLICIT_ACCESS_W: `size_of` 48, `Trustee` at offset 16 — verified
// against mingw-w64's own `aclapi.h` with a compiled `_Static_assert`
// probe. Genuinely fixed-size, an ordinary FFI-mirror struct with full
// field access, the same as `Trustee` above.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct ExplicitAccess {
    pub access_permissions: u32,
    pub access_mode: AccessMode,
    pub inheritance: u32,
    pub trustee: Trustee,
}
const _: () = assert!(core::mem::size_of::<ExplicitAccess>() == 48);
const _: () = assert!(core::mem::offset_of!(ExplicitAccess, trustee) == 16);

/// [`build_acl`]'s result — the new `PACL` `SetEntriesInAclW` allocated,
/// freed via `LocalFree` on `Drop` the same way [`PathSecurityInfo`]
/// frees its security-descriptor block.
pub struct BuiltAcl {
    acl: *mut Acl,
}

impl BuiltAcl {
    /// The built ACL, ready to pass to [`set_path_security_info`] (via
    /// [`PathSecurityInfo::from_raw_parts`]) or [`acl_entries`].
    pub fn as_ptr(&self) -> *const Acl {
        self.acl
    }
}

impl Drop for BuiltAcl {
    fn drop(&mut self) {
        // SAFETY: `self.acl` is the exact `PACL` pointer
        // `SetEntriesInAclW` allocated for this value and hasn't been
        // freed yet — freed here exactly once, on this value's only path
        // to being dropped.
        let _ = unsafe { LocalFree(self.acl.cast()) };
    }
}

/// Build a new ACL from `existing` (if any) plus `entries` —
/// `SetEntriesInAclW`, the primitive behind `icacls /grant`/`/deny`.
/// `existing = None` builds a fresh ACL from just `entries`, with
/// nothing carried over.
///
/// Reports failure via its own return value directly — never
/// `GetLastError` — so a nonzero return is passed straight to
/// [`crate::error::Win32Error::from_raw`] rather than `Win32Error::last`.
///
/// # Safety
///
/// `existing`, if `Some`, must be a valid `PACL`. Every [`Trustee`] in
/// `entries` must carry a still-valid `PSID`/name pointer for the
/// duration of this call.
pub unsafe fn build_acl(
    existing: Option<*const Acl>,
    entries: &[ExplicitAccess],
) -> Result<BuiltAcl, crate::error::Win32Error> {
    let mut new_acl: *mut Acl = core::ptr::null_mut();
    // SAFETY: `entries` is a valid slice, passed with its own exact
    // length; `existing` is caller-supplied per this function's own
    // safety contract, or null (documented as "build from just
    // `entries`"); `new_acl` is a valid out-pointer.
    let status = unsafe {
        SetEntriesInAclW(
            entries.len() as u32,
            entries.as_ptr(),
            existing.unwrap_or(core::ptr::null()),
            &mut new_acl,
        )
    };
    if status != 0 {
        return Err(crate::error::Win32Error::from_raw(status));
    }
    Ok(BuiltAcl { acl: new_acl })
}

/// `SID_NAME_USE` — what kind of account [`lookup_account_sid`]/
/// [`lookup_account_name`] resolved. Verified against mingw-w64's own
/// `winnt.h` with a compiled `_Static_assert` probe.
#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidNameUse {
    User = 1,
    Group = 2,
    Domain = 3,
    Alias = 4,
    WellKnownGroup = 5,
    DeletedAccount = 6,
    Invalid = 7,
    Unknown = 8,
    Computer = 9,
    /// Any `SID_NAME_USE` value beyond the ones this crate names above
    /// (`SidTypeLabel`/`SidTypeLogonSession`, both rare and outside
    /// ordinary owner/ACE-trustee lookups) — reported rather than
    /// silently coerced to [`Unknown`](SidNameUse::Unknown).
    Other(i32),
}

impl SidNameUse {
    fn from_raw(raw: i32) -> Self {
        match raw {
            1 => SidNameUse::User,
            2 => SidNameUse::Group,
            3 => SidNameUse::Domain,
            4 => SidNameUse::Alias,
            5 => SidNameUse::WellKnownGroup,
            6 => SidNameUse::DeletedAccount,
            7 => SidNameUse::Invalid,
            8 => SidNameUse::Unknown,
            9 => SidNameUse::Computer,
            other => SidNameUse::Other(other),
        }
    }
}

/// [`lookup_account_sid`]/[`lookup_account_name`]'s resolved-account
/// result — a `"DOMAIN\name"`-style display split into its two parts,
/// plus what kind of principal it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountName {
    pub name: alloc::string::String,
    pub domain: alloc::string::String,
    pub sid_name_use: SidNameUse,
}

/// A `PSID` byte buffer [`lookup_account_name`] fills in — this crate's
/// only *owned* SID representation (every other `PSID` this module
/// touches is a borrowed pointer into someone else's memory: a
/// security-descriptor block, an ACL, …), since `LookupAccountNameW`
/// has nothing else to point the caller at.
#[derive(Debug, Clone)]
pub struct SidBuf {
    bytes: Vec<u8>,
}

impl SidBuf {
    /// A `PSID` pointer into this buffer's own memory, valid for as long
    /// as this `SidBuf` isn't dropped or moved-and-reallocated (it
    /// isn't — a `Vec<u8>`'s heap allocation stays put across a move of
    /// the `Vec` value itself).
    pub fn as_ptr(&self) -> *const core::ffi::c_void {
        self.bytes.as_ptr().cast()
    }
}

/// Resolve `sid` to an account name — `LookupAccountSidW`. The reverse
/// direction of [`lookup_account_name`]: turns an owner/ACE `PSID` (e.g.
/// one [`PathSecurityInfo::owner`]/[`AclEntry::sid`] returned) into a
/// `"DOMAIN\name"`-style display string, the way `icacls`/`ls -l` show a
/// human-readable owner instead of a raw SID.
///
/// # Safety
///
/// `sid` must be a valid `PSID` for the duration of this call.
pub unsafe fn lookup_account_sid(
    sid: *mut core::ffi::c_void,
) -> Result<AccountName, crate::error::Win32Error> {
    let mut name_len: u32 = 0;
    let mut domain_len: u32 = 0;
    let mut sid_name_use: i32 = 0;
    // SAFETY: `sid` is caller-supplied per this function's own safety
    // contract; a null `system_name` means "look up on the local
    // system"; null `name`/`referenced_domain_name` with zeroed lengths
    // is documented to report the required sizes without needing real
    // buffers yet.
    let ok = unsafe {
        LookupAccountSidW(
            core::ptr::null(),
            sid,
            core::ptr::null_mut(),
            &mut name_len,
            core::ptr::null_mut(),
            &mut domain_len,
            &mut sid_name_use,
        )
    };
    if ok == 0 {
        let err = crate::error::Win32Error::last();
        if err != crate::error::Win32Error::ERROR_INSUFFICIENT_BUFFER {
            return Err(err);
        }
    }

    let mut name_buf: Vec<u16> = alloc::vec![0u16; name_len as usize];
    let mut domain_buf: Vec<u16> = alloc::vec![0u16; domain_len as usize];
    // SAFETY: `sid` as above; `name_buf`/`domain_buf` are valid buffers
    // matched by `name_len`/`domain_len` naming their exact lengths
    // (from the query above).
    let ok = unsafe {
        LookupAccountSidW(
            core::ptr::null(),
            sid,
            name_buf.as_mut_ptr(),
            &mut name_len,
            domain_buf.as_mut_ptr(),
            &mut domain_len,
            &mut sid_name_use,
        )
    };
    if ok == 0 {
        return Err(crate::error::Win32Error::last());
    }
    Ok(AccountName {
        name: alloc::string::String::from_utf16_lossy(&name_buf[..name_len as usize]),
        domain: alloc::string::String::from_utf16_lossy(&domain_buf[..domain_len as usize]),
        sid_name_use: SidNameUse::from_raw(sid_name_use),
    })
}

/// Resolve `name` (e.g. `"DOMAIN\user"` or a bare local account/group
/// name) to a `PSID` — `LookupAccountNameW`. The reverse direction of
/// [`lookup_account_sid`]: turns a human-typed account name into the SID
/// [`build_trustee_with_sid`]/[`build_acl`] need, the way `chown` accepts
/// a username rather than requiring a raw SID.
pub fn lookup_account_name(name: &str) -> Result<(SidBuf, AccountName), crate::error::Win32Error> {
    let wide_name: Vec<u16> = name.encode_utf16().chain(core::iter::once(0)).collect();
    let mut sid_len: u32 = 0;
    let mut domain_len: u32 = 0;
    let mut sid_name_use: i32 = 0;
    // SAFETY: `wide_name` is a valid, NUL-terminated UTF-16 string live
    // for the whole call; a null `system_name` means "look up on the
    // local system"; null `sid`/`referenced_domain_name` with zeroed
    // lengths is documented to report the required sizes without needing
    // real buffers yet.
    let ok = unsafe {
        LookupAccountNameW(
            core::ptr::null(),
            wide_name.as_ptr(),
            core::ptr::null_mut(),
            &mut sid_len,
            core::ptr::null_mut(),
            &mut domain_len,
            &mut sid_name_use,
        )
    };
    if ok == 0 {
        let err = crate::error::Win32Error::last();
        if err != crate::error::Win32Error::ERROR_INSUFFICIENT_BUFFER {
            return Err(err);
        }
    }

    let mut sid_bytes: Vec<u8> = alloc::vec![0u8; sid_len as usize];
    let mut domain_buf: Vec<u16> = alloc::vec![0u16; domain_len as usize];
    // SAFETY: `wide_name` as above; `sid_bytes`/`domain_buf` are valid
    // buffers matched by `sid_len`/`domain_len` naming their exact
    // lengths (from the query above).
    let ok = unsafe {
        LookupAccountNameW(
            core::ptr::null(),
            wide_name.as_ptr(),
            sid_bytes.as_mut_ptr(),
            &mut sid_len,
            domain_buf.as_mut_ptr(),
            &mut domain_len,
            &mut sid_name_use,
        )
    };
    if ok == 0 {
        return Err(crate::error::Win32Error::last());
    }
    let account = AccountName {
        name: name.into(),
        domain: alloc::string::String::from_utf16_lossy(&domain_buf[..domain_len as usize]),
        sid_name_use: SidNameUse::from_raw(sid_name_use),
    };
    Ok((SidBuf { bytes: sid_bytes }, account))
}

/// Convert `sid` to its `S-1-5-...` string form — `ConvertSidToStringSidW`.
/// The fallback `icacls` itself uses to display a SID that can't be
/// resolved to a name (orphaned/foreign/deleted account) — see
/// [`lookup_account_sid`] for the name-resolving path this is a fallback
/// from. Frees `ConvertSidToStringSidW`'s own output buffer via
/// `LocalFree` before returning, so the result is an ordinary owned
/// `String` rather than another `LocalFree`-on-`Drop` wrapper.
///
/// # Safety
///
/// `sid` must be a valid `PSID` for the duration of this call.
pub unsafe fn sid_to_string(
    sid: *mut core::ffi::c_void,
) -> Result<alloc::string::String, crate::error::Win32Error> {
    let mut string_sid: *mut u16 = core::ptr::null_mut();
    // SAFETY: `sid` is caller-supplied per this function's own safety
    // contract; `string_sid` is a valid out-pointer.
    let ok = unsafe { ConvertSidToStringSidW(sid, &mut string_sid) };
    if ok == 0 {
        return Err(crate::error::Win32Error::last());
    }
    // SAFETY: a successful call guarantees `string_sid` points to a
    // NUL-terminated wide string allocated by this same call, not yet
    // freed — walking it to find the NUL is safe.
    let len = unsafe {
        let mut n = 0usize;
        while *string_sid.add(n) != 0 {
            n += 1;
        }
        n
    };
    // SAFETY: `string_sid` points to `len` valid `u16`s per the walk just
    // done, all still part of the same live allocation.
    let text = alloc::string::String::from_utf16_lossy(unsafe {
        core::slice::from_raw_parts(string_sid, len)
    });
    // SAFETY: `string_sid` is the exact pointer `ConvertSidToStringSidW`
    // allocated for this call and hasn't been freed yet.
    let _ = unsafe { LocalFree(string_sid.cast()) };
    Ok(text)
}

/// [`string_to_sid`]'s result — the `PSID` `ConvertStringSidToSidW`
/// allocated, freed via `LocalFree` on `Drop` the same way [`BuiltAcl`]
/// frees the ACL `SetEntriesInAclW` allocates.
#[derive(Debug)]
pub struct ConvertedSid {
    sid: *mut core::ffi::c_void,
}

impl ConvertedSid {
    /// The parsed `PSID`, ready to pass to [`lookup_account_sid`],
    /// [`sid_to_string`], or an [`ExplicitAccess`] entry's trustee.
    pub fn as_ptr(&self) -> *mut core::ffi::c_void {
        self.sid
    }
}

impl Drop for ConvertedSid {
    fn drop(&mut self) {
        // SAFETY: `self.sid` is the exact `PSID` pointer
        // `ConvertStringSidToSidW` allocated for this value and hasn't
        // been freed yet — freed here exactly once, on this value's only
        // path to being dropped.
        let _ = unsafe { LocalFree(self.sid) };
    }
}

/// Parse a `S-1-5-...` string SID back into a `PSID` —
/// `ConvertStringSidToSidW`. The reverse of [`sid_to_string`].
pub fn string_to_sid(s: &str) -> Result<ConvertedSid, crate::error::Win32Error> {
    let wide: Vec<u16> = s.encode_utf16().chain(core::iter::once(0)).collect();
    let mut sid: *mut core::ffi::c_void = core::ptr::null_mut();
    // SAFETY: `wide` is a valid, NUL-terminated UTF-16 string live for the
    // whole call; `sid` is a valid out-pointer.
    let ok = unsafe { ConvertStringSidToSidW(wide.as_ptr(), &mut sid) };
    if ok == 0 {
        return Err(crate::error::Win32Error::last());
    }
    Ok(ConvertedSid { sid })
}

/// `ACL_REVISION` — the only ACL revision [`initialize_acl`]/
/// [`add_access_allowed_ace`]/[`add_access_denied_ace`] produce
/// (`ACL_REVISION_DS`, needed only for object-specific ACEs, is out of
/// this module's scope). Verified against mingw-w64's own `winnt.h` with
/// a compiled `_Static_assert` probe.
const ACL_REVISION: u32 = 2;

/// Initialize `buf` in place as a fresh, empty ACL header —
/// `InitializeAcl`, the per-ACE lower-level alternative to [`build_acl`]'s
/// all-at-once `SetEntriesInAclW`, useful for a brand-new object's initial
/// ACL. On success, `buf.as_mut_ptr()` reinterpreted as `*mut Acl` is a
/// valid (empty) `PACL` — pass it to [`add_access_allowed_ace`]/
/// [`add_access_denied_ace`] to add entries, or to [`acl_entries`]/
/// [`set_path_security_info`] as-is.
///
/// `buf` must be at least `size_of::<Acl>()` (8) bytes, with any
/// remaining space left for ACEs added afterward — an undersized `buf`
/// surfaces as an ordinary `Err` from `InitializeAcl` itself rather than
/// a separate check here. No particular alignment of `buf` is required:
/// this crate never reads an `Acl`'s fields directly, always through
/// `GetAclInformation`/`GetAce`, which take it as an opaque pointer.
pub fn initialize_acl(buf: &mut [u8]) -> Result<(), crate::error::Win32Error> {
    // SAFETY: `buf` is a valid, writable buffer of exactly `buf.len()`
    // bytes; `InitializeAcl` only ever writes within that length.
    let ok = unsafe { InitializeAcl(buf.as_mut_ptr().cast(), buf.len() as u32, ACL_REVISION) };
    if ok == 0 {
        Err(crate::error::Win32Error::last())
    } else {
        Ok(())
    }
}

/// Append an allow-access ACE naming `sid` to `acl` — `AddAccessAllowedAce`,
/// the per-ACE alternative to [`build_acl`]'s all-at-once `SetEntriesInAclW`.
///
/// # Safety
///
/// `acl` must be a valid `PACL` — e.g. one just initialized via
/// [`initialize_acl`] — with enough free space for one more
/// `ACCESS_ALLOWED_ACE` sized for `sid`. `sid` must be a valid `PSID` for
/// the duration of this call (its bytes are copied into `acl`, so it need
/// not outlive the call itself).
pub unsafe fn add_access_allowed_ace(
    acl: *mut Acl,
    sid: *mut core::ffi::c_void,
    mask: u32,
) -> Result<(), crate::error::Win32Error> {
    // SAFETY: `acl`/`sid` are caller-supplied per this function's own
    // safety contract.
    let ok = unsafe { AddAccessAllowedAce(acl, ACL_REVISION, mask, sid) };
    if ok == 0 {
        Err(crate::error::Win32Error::last())
    } else {
        Ok(())
    }
}

/// Append a deny-access ACE naming `sid` to `acl` — `AddAccessDeniedAce`,
/// the deny counterpart to [`add_access_allowed_ace`].
///
/// # Safety
///
/// Same contract as [`add_access_allowed_ace`].
pub unsafe fn add_access_denied_ace(
    acl: *mut Acl,
    sid: *mut core::ffi::c_void,
    mask: u32,
) -> Result<(), crate::error::Win32Error> {
    // SAFETY: `acl`/`sid` are caller-supplied per this function's own
    // safety contract.
    let ok = unsafe { AddAccessDeniedAce(acl, ACL_REVISION, mask, sid) };
    if ok == 0 {
        Err(crate::error::Win32Error::last())
    } else {
        Ok(())
    }
}

/// The length in bytes of `sid` — `GetLengthSid`. Needed anywhere a
/// `PSID` must be sized before copying it out of a short-lived buffer
/// (e.g. [`copy_sid`]) into this crate's own storage.
///
/// # Safety
///
/// `sid` must be a valid `PSID` (as [`is_valid_sid`] would confirm) —
/// `GetLengthSid` itself doesn't validate `sid`, and its behavior for an
/// invalid one is undefined per its own documentation.
pub unsafe fn sid_length(sid: *mut core::ffi::c_void) -> u32 {
    // SAFETY: `sid` is caller-supplied per this function's own safety
    // contract.
    unsafe { GetLengthSid(sid) }
}

/// Whether `sid` is a structurally valid `PSID` — `IsValidSid`. Safe to
/// call on any pointer at all (that's the whole point: it's how a caller
/// checks a `PSID` before trusting it to [`sid_length`]/[`copy_sid`]/any
/// other function in this module that assumes validity), so long as
/// `sid` is itself readable memory of a plausible SID's shape — see the
/// safety note below.
///
/// # Safety
///
/// `sid` must point to at least a `PSID`'s minimal fixed header's worth
/// of readable memory (`IsValidSid` reads that header to judge
/// structural validity) — an arbitrary dangling or unmapped pointer is
/// still unsound to pass here, even though a well-formed-but-wrong SID
/// is exactly the case this function exists to catch.
pub unsafe fn is_valid_sid(sid: *mut core::ffi::c_void) -> bool {
    // SAFETY: `sid` is caller-supplied per this function's own safety
    // contract.
    unsafe { IsValidSid(sid) != 0 }
}

/// Copy `sid` into a freshly allocated, owned [`SidBuf`] — `GetLengthSid`
/// (to size the buffer) plus `CopySid`. The only way this module produces
/// an owned copy of a `PSID` that started out as someone else's borrowed
/// pointer (an owner/ACE SID from [`path_security_info`]/[`acl_entries`],
/// which stay valid only as long as their source does).
///
/// # Safety
///
/// `sid` must be a valid `PSID` (as [`is_valid_sid`] would confirm) for
/// the duration of this call.
pub unsafe fn copy_sid(sid: *mut core::ffi::c_void) -> Result<SidBuf, crate::error::Win32Error> {
    // SAFETY: `sid` is caller-supplied per this function's own safety
    // contract.
    let len = unsafe { sid_length(sid) };
    let mut bytes: Vec<u8> = alloc::vec![0u8; len as usize];
    // SAFETY: `bytes` is a valid, writable buffer of exactly `len` bytes,
    // the length `GetLengthSid` itself just reported for `sid`; `sid` is
    // caller-supplied per this function's own safety contract.
    let ok = unsafe { CopySid(len, bytes.as_mut_ptr().cast(), sid) };
    if ok == 0 {
        Err(crate::error::Win32Error::last())
    } else {
        Ok(SidBuf { bytes })
    }
}

/// Whether `a` and `b` name the same SID — `EqualSid`, the only safe way
/// to compare two `PSID`s: a naive byte-for-byte memory comparison isn't
/// safe here, since a SID's trailing sub-authority count varies its
/// total size.
///
/// # Safety
///
/// `a` and `b` must both be valid `PSID`s (as [`is_valid_sid`] would
/// confirm) for the duration of this call.
pub unsafe fn sid_equal(a: *mut core::ffi::c_void, b: *mut core::ffi::c_void) -> bool {
    // SAFETY: `a`/`b` are caller-supplied per this function's own safety
    // contract.
    unsafe { EqualSid(a, b) != 0 }
}

/// `WELL_KNOWN_SID_TYPE` — the well-known principals [`well_known_sid`]
/// can construct without a name-lookup round trip. Only the three named
/// in this module's own scope (`Everyone`/`LocalSystem`/
/// `BuiltinAdministrators` — see issue #163) are exposed; `CreateWellKnownSid`
/// itself supports many more (`WinAnonymousSid`, `WinBuiltinGuestsSid`,
/// dozens of others), left out until a real need for them shows up.
/// Verified against mingw-w64's own `winnt.h` with a compiled
/// `_Static_assert` probe.
#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WellKnownSidType {
    /// `WinWorldSid` — the "Everyone" group.
    Everyone = 1,
    /// `WinLocalSystemSid` — the SYSTEM account.
    LocalSystem = 22,
    /// `WinBuiltinAdministratorsSid` — the local Administrators group.
    BuiltinAdministrators = 26,
}

/// Construct a well-known SID (Everyone, SYSTEM, Administrators) —
/// `CreateWellKnownSid`, without the name-lookup round trip
/// [`lookup_account_name`] would otherwise require. `DomainSid` is always
/// passed as `NULL`: every [`WellKnownSidType`] variant this module
/// exposes is a machine-local or universal principal, never a
/// domain-relative one that would need it.
pub fn well_known_sid(kind: WellKnownSidType) -> Result<SidBuf, crate::error::Win32Error> {
    let mut len: u32 = 0;
    // SAFETY: a null `sid` with `len` zeroed is documented to report the
    // required buffer size (failing with `ERROR_INSUFFICIENT_BUFFER`)
    // without needing a real buffer yet; `domain_sid` null is valid for
    // every `kind` this module exposes.
    let ok = unsafe {
        CreateWellKnownSid(
            kind as i32,
            core::ptr::null_mut(),
            core::ptr::null_mut(),
            &mut len,
        )
    };
    if ok == 0 {
        let err = crate::error::Win32Error::last();
        if err != crate::error::Win32Error::ERROR_INSUFFICIENT_BUFFER {
            return Err(err);
        }
    }

    let mut bytes: Vec<u8> = alloc::vec![0u8; len as usize];
    // SAFETY: `bytes` is a valid buffer matched by `len` naming its exact
    // size (from the query above); `domain_sid` null as above.
    let ok = unsafe {
        CreateWellKnownSid(
            kind as i32,
            core::ptr::null_mut(),
            bytes.as_mut_ptr().cast(),
            &mut len,
        )
    };
    if ok == 0 {
        return Err(crate::error::Win32Error::last());
    }
    bytes.truncate(len as usize);
    Ok(SidBuf { bytes })
}

/// `SDDL_REVISION_1` — the only SDDL string format revision
/// [`sd_to_string`]/[`string_to_sd`] produce/consume. Verified against
/// mingw-w64's own `sddl.h` with a compiled `_Static_assert` probe.
const SDDL_REVISION_1: u32 = 1;

/// [`string_to_sd`]'s result — the `PSECURITY_DESCRIPTOR`
/// `ConvertStringSecurityDescriptorToSecurityDescriptorW` allocated,
/// freed via `LocalFree` on `Drop`, matching [`ConvertedSid`]/
/// [`BuiltAcl`]'s existing pattern. Treated as an opaque blob — pass it
/// to [`sd_to_string`] to render it back to SDDL, matching this module's
/// existing opaque-security-descriptor treatment (see
/// [`PathSecurityInfo::raw_security_descriptor`]).
#[derive(Debug)]
pub struct ConvertedSecurityDescriptor {
    sd: *mut core::ffi::c_void,
}

impl ConvertedSecurityDescriptor {
    /// The parsed security descriptor, ready to pass to [`sd_to_string`].
    pub fn as_ptr(&self) -> *mut core::ffi::c_void {
        self.sd
    }
}

impl Drop for ConvertedSecurityDescriptor {
    fn drop(&mut self) {
        // SAFETY: `self.sd` is the exact `PSECURITY_DESCRIPTOR` pointer
        // `ConvertStringSecurityDescriptorToSecurityDescriptorW`
        // allocated for this value and hasn't been freed yet — freed
        // here exactly once, on this value's only path to being dropped.
        let _ = unsafe { LocalFree(self.sd) };
    }
}

/// Render `sd` as an SDDL string —
/// `ConvertSecurityDescriptorToStringSecurityDescriptorW`, a debug/snapshot
/// (`icacls /save`-style) string representation of a security
/// descriptor's full permission state. `info` selects which components
/// to include (`OWNER_SECURITY_INFORMATION`/`GROUP_SECURITY_INFORMATION`/
/// `DACL_SECURITY_INFORMATION`, matching [`path_security_info`]'s own
/// parameter) — diverges from this issue's literal `sd_to_string(sd) ->
/// Result` signature, since the real Win32 function requires a
/// `SECURITY_INFORMATION` selecting which parts to render; this module
/// never touches SACLs, so `info` only ever carries the same three bits
/// [`path_security_info`] does.
///
/// # Safety
///
/// `sd` must be a valid `PSECURITY_DESCRIPTOR` for the duration of this
/// call.
pub unsafe fn sd_to_string(
    sd: *mut core::ffi::c_void,
    info: SecurityInfoFlags,
) -> Result<alloc::string::String, crate::error::Win32Error> {
    let mut string_sd: *mut u16 = core::ptr::null_mut();
    let mut len: u32 = 0;
    // SAFETY: `sd` is caller-supplied per this function's own safety
    // contract; `string_sd`/`len` are valid out-pointers.
    let ok = unsafe {
        ConvertSecurityDescriptorToStringSecurityDescriptorW(
            sd,
            SDDL_REVISION_1,
            info,
            &mut string_sd,
            &mut len,
        )
    };
    if ok == 0 {
        return Err(crate::error::Win32Error::last());
    }
    // `len` counts the terminating NUL; trim it before decoding.
    let char_len = (len as usize).saturating_sub(1);
    // SAFETY: a successful call guarantees `string_sd` points to at least
    // `len` valid `u16`s (including the trailing NUL this crate trims),
    // all part of the same allocation this call just made.
    let text = alloc::string::String::from_utf16_lossy(unsafe {
        core::slice::from_raw_parts(string_sd, char_len)
    });
    // SAFETY: `string_sd` is the exact pointer
    // `ConvertSecurityDescriptorToStringSecurityDescriptorW` allocated
    // for this call and hasn't been freed yet.
    let _ = unsafe { LocalFree(string_sd.cast()) };
    Ok(text)
}

/// Parse an SDDL string back into a security descriptor —
/// `ConvertStringSecurityDescriptorToSecurityDescriptorW`. The reverse of
/// [`sd_to_string`].
pub fn string_to_sd(s: &str) -> Result<ConvertedSecurityDescriptor, crate::error::Win32Error> {
    let wide: Vec<u16> = s.encode_utf16().chain(core::iter::once(0)).collect();
    let mut sd: *mut core::ffi::c_void = core::ptr::null_mut();
    // SAFETY: `wide` is a valid, NUL-terminated UTF-16 string live for the
    // whole call; `sd` is a valid out-pointer.
    let ok = unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            wide.as_ptr(),
            SDDL_REVISION_1,
            &mut sd,
            core::ptr::null_mut(),
        )
    };
    if ok == 0 {
        return Err(crate::error::Win32Error::last());
    }
    Ok(ConvertedSecurityDescriptor { sd })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_security_info_reports_an_owner_and_dacl_for_a_real_file() {
        let path = std::env::temp_dir().join("rusty_win32_security_test_path_info.txt");
        std::fs::write(&path, b"hello").expect("creating the test file should succeed");
        let path_str = path.to_str().expect("temp path should be valid UTF-8");

        let info = path_security_info(
            path_str,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
        )
        .expect("GetNamedSecurityInfoW should succeed for a real file");
        assert!(
            info.owner().is_some(),
            "a real file should have an owner SID"
        );
        assert!(info.dacl().is_some(), "a real file should have a DACL");

        std::fs::remove_file(&path).expect("removing the test file should succeed");
    }

    #[test]
    fn set_path_security_info_round_trips_the_files_own_owner_and_dacl() {
        let path = std::env::temp_dir().join("rusty_win32_security_test_set_info.txt");
        std::fs::write(&path, b"hello").expect("creating the test file should succeed");
        let path_str = path.to_str().expect("temp path should be valid UTF-8");

        let info = path_security_info(
            path_str,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
        )
        .expect("GetNamedSecurityInfoW should succeed for a real file");

        // Re-applying a file's own current owner/DACL back onto itself
        // is always permitted (no elevated privilege needed to set an
        // object's owner to one you already are, or a DACL you already
        // fully control) — a safe, self-contained round trip that
        // doesn't depend on SID-construction primitives this crate
        // doesn't have yet (`well_known_sid`/`ConvertStringSidToSidW`
        // are later round-2 items).
        // SAFETY: `info`'s pointers are still valid — it hasn't been
        // dropped, and the file they describe hasn't changed since.
        unsafe { set_path_security_info(path_str, &info) }
            .expect("SetNamedSecurityInfoW should succeed re-applying the file's own owner/DACL");

        std::fs::remove_file(&path).expect("removing the test file should succeed");
    }

    #[test]
    fn path_security_info_fails_for_a_nonexistent_path() {
        let path =
            std::env::temp_dir().join("rusty_win32_security_test_missing_definitely_not_here.txt");
        let path_str = path.to_str().expect("temp path should be valid UTF-8");

        let err = path_security_info(path_str, OWNER_SECURITY_INFORMATION)
            .expect_err("GetNamedSecurityInfoW should fail for a nonexistent path");
        assert_eq!(err, crate::error::Win32Error::ERROR_FILE_NOT_FOUND);
    }

    #[test]
    fn acl_entries_reports_at_least_one_recognized_ace_for_a_real_files_dacl() {
        let path = std::env::temp_dir().join("rusty_win32_security_test_acl_entries.txt");
        std::fs::write(&path, b"hello").expect("creating the test file should succeed");
        let path_str = path.to_str().expect("temp path should be valid UTF-8");

        let info = path_security_info(path_str, DACL_SECURITY_INFORMATION)
            .expect("GetNamedSecurityInfoW should succeed for a real file");
        let dacl = info
            .dacl()
            .expect("a real file should have a DACL")
            .cast_const()
            .cast::<Acl>();

        // SAFETY: `dacl` is a valid `PACL` from `info`, which stays alive
        // (not yet dropped) for this whole call.
        let entries =
            unsafe { acl_entries(dacl) }.expect("GetAclInformation/GetAce should succeed");
        assert!(
            !entries.is_empty(),
            "a real file's DACL should have at least one ACE"
        );
        assert!(
            entries
                .iter()
                .any(|e| matches!(e.kind, AceKind::Allow | AceKind::Deny)),
            "a real file's DACL should have at least one ordinary allow/deny ACE, got: {entries:?}"
        );
        for entry in &entries {
            if matches!(entry.kind, AceKind::Allow | AceKind::Deny) {
                assert!(
                    entry.sid.is_some(),
                    "an allow/deny ACE should always carry a SID"
                );
            }
        }

        std::fs::remove_file(&path).expect("removing the test file should succeed");
    }

    #[test]
    fn build_acl_produces_an_acl_with_the_requested_grant_entry() {
        let path = std::env::temp_dir().join("rusty_win32_security_test_build_acl.txt");
        std::fs::write(&path, b"hello").expect("creating the test file should succeed");
        let path_str = path.to_str().expect("temp path should be valid UTF-8");

        // No `well_known_sid`/`ConvertStringSidToSidW` yet (later round-2
        // items) to construct a fresh SID from scratch — this file's own
        // owner is a real, already-obtained SID good enough to build a
        // self-contained test around.
        let info = path_security_info(path_str, OWNER_SECURITY_INFORMATION)
            .expect("GetNamedSecurityInfoW should succeed for a real file");
        let owner = info.owner().expect("a real file should have an owner SID");

        const GENERIC_READ: u32 = 0x8000_0000;
        // SAFETY: `owner` is a valid `PSID` from `info`, which stays
        // alive (not yet dropped) for this whole test.
        let mut trustee = unsafe { build_trustee_with_sid(owner) };
        trustee.trustee_type = TrusteeType::User;
        let entry = ExplicitAccess {
            access_permissions: GENERIC_READ,
            access_mode: AccessMode::Grant,
            inheritance: 0,
            trustee,
        };

        // SAFETY: `entries` (just built above) carries a still-valid
        // `PSID` pointer; `existing` is `None`.
        let built = unsafe { build_acl(None, &[entry]) }
            .expect("SetEntriesInAclW should succeed building a fresh ACL");

        // SAFETY: `built` is still alive (not yet dropped), so its ACL
        // pointer is still valid.
        let acl_entries_found = unsafe { acl_entries(built.as_ptr()) }
            .expect("GetAclInformation/GetAce should succeed");
        assert_eq!(
            acl_entries_found.len(),
            1,
            "a fresh ACL built from exactly one entry should have exactly one ACE"
        );
        assert_eq!(acl_entries_found[0].kind, AceKind::Allow);
        assert_eq!(acl_entries_found[0].mask, GENERIC_READ);

        std::fs::remove_file(&path).expect("removing the test file should succeed");
    }

    #[test]
    fn build_trustee_with_sid_names_the_given_sid_as_unknown_type() {
        let path = std::env::temp_dir().join("rusty_win32_security_test_build_trustee_sid.txt");
        std::fs::write(&path, b"hello").expect("creating the test file should succeed");
        let path_str = path.to_str().expect("temp path should be valid UTF-8");

        let info = path_security_info(path_str, OWNER_SECURITY_INFORMATION)
            .expect("GetNamedSecurityInfoW should succeed for a real file");
        let owner = info.owner().expect("a real file should have an owner SID");

        // SAFETY: `owner` is a valid `PSID` from `info`, which stays
        // alive (not yet dropped) for this whole test.
        let trustee = unsafe { build_trustee_with_sid(owner) };
        assert_eq!(trustee.trustee_form, TrusteeForm::Sid);
        // `BuildTrusteeWithSidW` always reports `TRUSTEE_IS_UNKNOWN` —
        // it never inspects the SID to guess a more specific type.
        assert_eq!(trustee.trustee_type, TrusteeType::Unknown);
        assert_eq!(
            trustee.name.cast::<core::ffi::c_void>(),
            owner,
            "the trustee's name pointer should be the SID itself, not a copy"
        );

        std::fs::remove_file(&path).expect("removing the test file should succeed");
    }

    #[test]
    fn build_trustee_with_name_names_the_given_wide_string() {
        let mut wide: alloc::vec::Vec<u16> = "Everyone"
            .encode_utf16()
            .chain(core::iter::once(0))
            .collect();
        let expected_ptr = wide.as_mut_ptr();

        // SAFETY: `wide` is a valid, NUL-terminated UTF-16 buffer that
        // stays alive (not dropped/reallocated) for this whole test.
        let trustee = unsafe { build_trustee_with_name(&mut wide) };
        assert_eq!(trustee.trustee_form, TrusteeForm::Name);
        assert_eq!(
            trustee.name, expected_ptr,
            "the trustee's name pointer should be the wide buffer itself, not a copy"
        );
    }

    #[test]
    fn lookup_account_sid_resolves_a_real_files_owner() {
        let path = std::env::temp_dir().join("rusty_win32_security_test_lookup_sid.txt");
        std::fs::write(&path, b"hello").expect("creating the test file should succeed");
        let path_str = path.to_str().expect("temp path should be valid UTF-8");

        let info = path_security_info(path_str, OWNER_SECURITY_INFORMATION)
            .expect("GetNamedSecurityInfoW should succeed for a real file");
        let owner = info.owner().expect("a real file should have an owner SID");

        // SAFETY: `owner` is a valid `PSID` from `info`, which stays
        // alive (not yet dropped) for this whole call.
        let account =
            unsafe { lookup_account_sid(owner) }.expect("LookupAccountSidW should succeed");
        assert!(
            !account.name.is_empty(),
            "a real file's owner should resolve to a non-empty account name"
        );

        std::fs::remove_file(&path).expect("removing the test file should succeed");
    }

    #[test]
    fn lookup_account_name_then_lookup_account_sid_round_trips() {
        let (sid, account) = lookup_account_name("Everyone")
            .expect("LookupAccountNameW should succeed for the well-known Everyone group");
        assert_eq!(account.name, "Everyone");

        // SAFETY: `sid` is still alive (not dropped) for this whole
        // call; `LookupAccountSidW` never writes through its `Sid`
        // parameter despite the real Win32 signature not being
        // `const`-qualified.
        let resolved = unsafe { lookup_account_sid(sid.as_ptr().cast_mut()) }.expect(
            "LookupAccountSidW should succeed resolving the SID lookup_account_name just returned",
        );
        assert!(
            !resolved.name.is_empty(),
            "the round-tripped SID should resolve back to a non-empty account name"
        );
    }

    #[test]
    fn sid_to_string_then_string_to_sid_round_trips_a_real_files_owner() {
        let path = std::env::temp_dir().join("rusty_win32_security_test_sid_string.txt");
        std::fs::write(&path, b"hello").expect("creating the test file should succeed");
        let path_str = path.to_str().expect("temp path should be valid UTF-8");

        let info = path_security_info(path_str, OWNER_SECURITY_INFORMATION)
            .expect("GetNamedSecurityInfoW should succeed for a real file");
        let owner = info.owner().expect("a real file should have an owner SID");

        // SAFETY: `owner` is a valid `PSID` from `info`, which stays
        // alive (not yet dropped) for this whole test.
        let text = unsafe { sid_to_string(owner) }.expect("ConvertSidToStringSidW should succeed");
        assert!(
            text.starts_with("S-1-"),
            "a Windows SID's string form should start with \"S-1-\", got: {text}"
        );

        let parsed =
            string_to_sid(&text).expect("ConvertStringSidToSidW should succeed round-tripping");
        // SAFETY: `owner` and `parsed`'s SID are both still alive for
        // this whole call.
        let original_account =
            unsafe { lookup_account_sid(owner) }.expect("LookupAccountSidW should succeed");
        let parsed_account = unsafe { lookup_account_sid(parsed.as_ptr()) }
            .expect("LookupAccountSidW should succeed on the round-tripped SID");
        assert_eq!(
            original_account, parsed_account,
            "the round-tripped SID should resolve to the same account as the original"
        );

        std::fs::remove_file(&path).expect("removing the test file should succeed");
    }

    #[test]
    fn string_to_sid_fails_for_a_malformed_string() {
        let err = string_to_sid("not a valid sid string")
            .expect_err("ConvertStringSidToSidW should fail on malformed input");
        assert_eq!(err, crate::error::Win32Error::ERROR_INVALID_SID);
    }

    #[test]
    fn initialize_acl_then_add_access_allowed_ace_produces_a_single_ace() {
        let path = std::env::temp_dir().join("rusty_win32_security_test_init_acl_allow.txt");
        std::fs::write(&path, b"hello").expect("creating the test file should succeed");
        let path_str = path.to_str().expect("temp path should be valid UTF-8");

        let info = path_security_info(path_str, OWNER_SECURITY_INFORMATION)
            .expect("GetNamedSecurityInfoW should succeed for a real file");
        let owner = info.owner().expect("a real file should have an owner SID");

        let mut buf = alloc::vec![0u8; 256];
        initialize_acl(&mut buf).expect("InitializeAcl should succeed for a well-sized buffer");
        let acl: *mut Acl = buf.as_mut_ptr().cast();

        const GENERIC_READ: u32 = 0x8000_0000;
        // SAFETY: `acl` was just initialized above with plenty of free
        // space; `owner` is a valid `PSID` from `info`, which stays alive
        // (not yet dropped) for this whole test.
        unsafe { add_access_allowed_ace(acl, owner, GENERIC_READ) }
            .expect("AddAccessAllowedAce should succeed");

        // SAFETY: `acl` is still alive (`buf` hasn't been dropped/moved).
        let entries = unsafe { acl_entries(acl.cast_const()) }
            .expect("GetAclInformation/GetAce should succeed");
        assert_eq!(
            entries.len(),
            1,
            "an ACL initialized from scratch with exactly one added ACE should have exactly one"
        );
        assert_eq!(entries[0].kind, AceKind::Allow);
        assert_eq!(entries[0].mask, GENERIC_READ);

        std::fs::remove_file(&path).expect("removing the test file should succeed");
    }

    #[test]
    fn add_access_denied_ace_appends_a_second_ace_after_an_allowed_one() {
        let path = std::env::temp_dir().join("rusty_win32_security_test_init_acl_deny.txt");
        std::fs::write(&path, b"hello").expect("creating the test file should succeed");
        let path_str = path.to_str().expect("temp path should be valid UTF-8");

        let info = path_security_info(path_str, OWNER_SECURITY_INFORMATION)
            .expect("GetNamedSecurityInfoW should succeed for a real file");
        let owner = info.owner().expect("a real file should have an owner SID");

        let mut buf = alloc::vec![0u8; 256];
        initialize_acl(&mut buf).expect("InitializeAcl should succeed for a well-sized buffer");
        let acl: *mut Acl = buf.as_mut_ptr().cast();

        const GENERIC_READ: u32 = 0x8000_0000;
        const GENERIC_WRITE: u32 = 0x4000_0000;
        // SAFETY: `acl` was just initialized above with plenty of free
        // space; `owner` is a valid `PSID` from `info`, which stays alive
        // (not yet dropped) for this whole test.
        unsafe { add_access_allowed_ace(acl, owner, GENERIC_READ) }
            .expect("AddAccessAllowedAce should succeed");
        unsafe { add_access_denied_ace(acl, owner, GENERIC_WRITE) }
            .expect("AddAccessDeniedAce should succeed");

        // SAFETY: `acl` is still alive (`buf` hasn't been dropped/moved).
        let entries = unsafe { acl_entries(acl.cast_const()) }
            .expect("GetAclInformation/GetAce should succeed");
        assert_eq!(entries.len(), 2, "both added ACEs should be reported");
        assert_eq!(entries[0].kind, AceKind::Allow);
        assert_eq!(entries[0].mask, GENERIC_READ);
        assert_eq!(entries[1].kind, AceKind::Deny);
        assert_eq!(entries[1].mask, GENERIC_WRITE);

        std::fs::remove_file(&path).expect("removing the test file should succeed");
    }

    #[test]
    fn initialize_acl_fails_for_an_undersized_buffer() {
        let mut buf = [0u8; 1];
        let err = initialize_acl(&mut buf)
            .expect_err("InitializeAcl should fail for a buffer smaller than a minimal ACL");
        // `InitializeAcl` reports a too-small buffer as `ERROR_INSUFFICIENT_BUFFER`
        // (122), not `ERROR_INVALID_PARAMETER` -- confirmed by CI on real
        // Windows (windows-latest), not documented clearly enough to have
        // guessed correctly up front.
        assert_eq!(err, crate::error::Win32Error::ERROR_INSUFFICIENT_BUFFER);
    }

    #[test]
    fn is_valid_sid_reports_true_for_a_real_files_owner() {
        let path = std::env::temp_dir().join("rusty_win32_security_test_is_valid_sid.txt");
        std::fs::write(&path, b"hello").expect("creating the test file should succeed");
        let path_str = path.to_str().expect("temp path should be valid UTF-8");

        let info = path_security_info(path_str, OWNER_SECURITY_INFORMATION)
            .expect("GetNamedSecurityInfoW should succeed for a real file");
        let owner = info.owner().expect("a real file should have an owner SID");

        // SAFETY: `owner` is a valid `PSID` from `info`, which stays
        // alive (not yet dropped) for this whole test.
        assert!(unsafe { is_valid_sid(owner) });

        std::fs::remove_file(&path).expect("removing the test file should succeed");
    }

    #[test]
    fn sid_length_then_copy_sid_produces_a_buffer_of_the_reported_length() {
        let path = std::env::temp_dir().join("rusty_win32_security_test_sid_length.txt");
        std::fs::write(&path, b"hello").expect("creating the test file should succeed");
        let path_str = path.to_str().expect("temp path should be valid UTF-8");

        let info = path_security_info(path_str, OWNER_SECURITY_INFORMATION)
            .expect("GetNamedSecurityInfoW should succeed for a real file");
        let owner = info.owner().expect("a real file should have an owner SID");

        // SAFETY: `owner` is a valid `PSID` from `info`, which stays
        // alive (not yet dropped) for this whole test.
        let len = unsafe { sid_length(owner) };
        assert!(len > 0, "a real SID should have a nonzero length");

        // SAFETY: `owner` as above.
        let copied = unsafe { copy_sid(owner) }.expect("CopySid should succeed");
        assert_eq!(
            copied.bytes.len(),
            len as usize,
            "copy_sid's buffer should be exactly sid_length's reported size"
        );

        // The copy should resolve to the same account as the original --
        // confirming CopySid produced a real, independently usable SID,
        // not just a same-length garbage buffer.
        // SAFETY: `owner` and `copied`'s SID are both still alive for
        // this whole call.
        let original_account =
            unsafe { lookup_account_sid(owner) }.expect("LookupAccountSidW should succeed");
        let copied_account = unsafe { lookup_account_sid(copied.as_ptr().cast_mut()) }
            .expect("LookupAccountSidW should succeed on the copied SID");
        assert_eq!(
            original_account, copied_account,
            "the copied SID should resolve to the same account as the original"
        );

        std::fs::remove_file(&path).expect("removing the test file should succeed");
    }

    #[test]
    fn sid_equal_reports_true_for_a_sid_and_its_own_copy() {
        let path = std::env::temp_dir().join("rusty_win32_security_test_sid_equal_same.txt");
        std::fs::write(&path, b"hello").expect("creating the test file should succeed");
        let path_str = path.to_str().expect("temp path should be valid UTF-8");

        let info = path_security_info(path_str, OWNER_SECURITY_INFORMATION)
            .expect("GetNamedSecurityInfoW should succeed for a real file");
        let owner = info.owner().expect("a real file should have an owner SID");

        // SAFETY: `owner` is a valid `PSID` from `info`, which stays
        // alive (not yet dropped) for this whole test.
        let copied = unsafe { copy_sid(owner) }.expect("CopySid should succeed");
        // SAFETY: `owner` and `copied`'s SID are both still alive for
        // this whole call.
        assert!(unsafe { sid_equal(owner, copied.as_ptr().cast_mut()) });

        std::fs::remove_file(&path).expect("removing the test file should succeed");
    }

    #[test]
    fn sid_equal_reports_false_for_two_different_well_known_sids() {
        let (everyone_sid, _) = lookup_account_name("Everyone")
            .expect("LookupAccountNameW should succeed for the well-known Everyone group");
        let path = std::env::temp_dir().join("rusty_win32_security_test_sid_equal_diff.txt");
        std::fs::write(&path, b"hello").expect("creating the test file should succeed");
        let path_str = path.to_str().expect("temp path should be valid UTF-8");

        let info = path_security_info(path_str, OWNER_SECURITY_INFORMATION)
            .expect("GetNamedSecurityInfoW should succeed for a real file");
        let owner = info.owner().expect("a real file should have an owner SID");

        // A real file's owner is never the well-known "Everyone" group.
        // SAFETY: `owner` is a valid `PSID` from `info`, which stays
        // alive (not yet dropped) for this whole test; `everyone_sid`'s
        // SID is likewise still alive.
        assert!(!unsafe { sid_equal(owner, everyone_sid.as_ptr().cast_mut()) });

        std::fs::remove_file(&path).expect("removing the test file should succeed");
    }

    #[test]
    fn well_known_sid_everyone_matches_lookup_account_names_everyone_sid() {
        let (looked_up_sid, _) = lookup_account_name("Everyone")
            .expect("LookupAccountNameW should succeed for the well-known Everyone group");
        let constructed_sid = well_known_sid(WellKnownSidType::Everyone)
            .expect("CreateWellKnownSid should succeed for WinWorldSid");

        // SAFETY: both SIDs are still alive (not dropped) for this whole
        // call.
        assert!(unsafe {
            sid_equal(
                looked_up_sid.as_ptr().cast_mut(),
                constructed_sid.as_ptr().cast_mut(),
            )
        });
    }

    #[test]
    fn well_known_sid_administrators_and_local_system_are_valid_and_distinct() {
        let administrators = well_known_sid(WellKnownSidType::BuiltinAdministrators)
            .expect("CreateWellKnownSid should succeed for WinBuiltinAdministratorsSid");
        let local_system = well_known_sid(WellKnownSidType::LocalSystem)
            .expect("CreateWellKnownSid should succeed for WinLocalSystemSid");

        // SAFETY: both SIDs were just constructed above and are still
        // alive for this whole call.
        assert!(unsafe { is_valid_sid(administrators.as_ptr().cast_mut()) });
        assert!(unsafe { is_valid_sid(local_system.as_ptr().cast_mut()) });
        assert!(!unsafe {
            sid_equal(
                administrators.as_ptr().cast_mut(),
                local_system.as_ptr().cast_mut(),
            )
        });
    }

    #[test]
    fn sd_to_string_produces_an_sddl_string_starting_with_owner() {
        let path = std::env::temp_dir().join("rusty_win32_security_test_sd_to_string.txt");
        std::fs::write(&path, b"hello").expect("creating the test file should succeed");
        let path_str = path.to_str().expect("temp path should be valid UTF-8");

        let info = path_security_info(
            path_str,
            OWNER_SECURITY_INFORMATION | GROUP_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
        )
        .expect("GetNamedSecurityInfoW should succeed for a real file");
        let sd = info
            .raw_security_descriptor()
            .expect("a real file's path_security_info should carry a security descriptor block");

        // SAFETY: `sd` is valid from `info`, which stays alive (not yet
        // dropped) for this whole test.
        let sddl = unsafe {
            sd_to_string(
                sd,
                OWNER_SECURITY_INFORMATION | GROUP_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            )
        }
        .expect("ConvertSecurityDescriptorToStringSecurityDescriptorW should succeed");
        assert!(
            sddl.starts_with("O:"),
            "an SDDL string including owner info should start with \"O:\", got: {sddl}"
        );

        std::fs::remove_file(&path).expect("removing the test file should succeed");
    }

    #[test]
    fn sd_to_string_then_string_to_sd_round_trips_to_the_same_sddl_string() {
        let path = std::env::temp_dir().join("rusty_win32_security_test_sd_round_trip.txt");
        std::fs::write(&path, b"hello").expect("creating the test file should succeed");
        let path_str = path.to_str().expect("temp path should be valid UTF-8");

        let requested_info =
            OWNER_SECURITY_INFORMATION | GROUP_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION;
        let info = path_security_info(path_str, requested_info)
            .expect("GetNamedSecurityInfoW should succeed for a real file");
        let sd = info
            .raw_security_descriptor()
            .expect("a real file's path_security_info should carry a security descriptor block");

        // SAFETY: `sd` is valid from `info`, which stays alive (not yet
        // dropped) for this whole test.
        let original_sddl = unsafe { sd_to_string(sd, requested_info) }
            .expect("ConvertSecurityDescriptorToStringSecurityDescriptorW should succeed");

        let parsed = string_to_sd(&original_sddl)
            .expect("ConvertStringSecurityDescriptorToSecurityDescriptorW should succeed");
        // SAFETY: `parsed`'s SD is still alive (not yet dropped) for this
        // whole call.
        let round_tripped_sddl = unsafe { sd_to_string(parsed.as_ptr(), requested_info) }
            .expect("ConvertSecurityDescriptorToStringSecurityDescriptorW should succeed on the round-tripped SD");
        assert_eq!(
            original_sddl, round_tripped_sddl,
            "converting to SDDL, back to a security descriptor, and back to SDDL again should be stable"
        );

        std::fs::remove_file(&path).expect("removing the test file should succeed");
    }

    #[test]
    fn string_to_sd_fails_for_a_malformed_string() {
        let result = string_to_sd("not a valid sddl string");
        assert!(
            result.is_err(),
            "ConvertStringSecurityDescriptorToSecurityDescriptorW should fail on malformed input"
        );
    }
}
