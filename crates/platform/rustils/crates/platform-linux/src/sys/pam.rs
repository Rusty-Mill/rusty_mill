//! Account checks through the site's PAM policy (`pam_acct_mgmt` only).
//!
//! libpam is loaded with `dlopen` (mux D267): no link, no `-dev` package,
//! and a host without it still runs the binary. Only absolute paths from
//! a short list of system directories are tried, with `RTLD_LOCAL`, so a
//! root process never searches `LD_LIBRARY_PATH`.
//!
//! Scope (mux D266, D273): the account, not the person. Never
//! `pam_authenticate`, never a password, never `pam_open_session` (nothing
//! could close it). The conversation function answers every prompt with
//! `PAM_CONV_ERR`, so a module that asks for input fails the check.

#![allow(unsafe_code)]

use std::ffi::{CStr, CString};
use std::path::Path;

use crate::ffi::libc_surface as c;

const PAM_SUCCESS: c::c_int = 0;
const PAM_CONV_ERR: c::c_int = 19;
const PAM_RUSER: c::c_int = 8;
const PAM_SILENT: c::c_int = 0x8000;

/// Where `libpam.so.0` is looked for, in order. Absolute on purpose.
const LIBRARIES: [&str; 8] = [
    "/lib/x86_64-linux-gnu/libpam.so.0",
    "/usr/lib/x86_64-linux-gnu/libpam.so.0",
    "/lib/aarch64-linux-gnu/libpam.so.0",
    "/usr/lib/aarch64-linux-gnu/libpam.so.0",
    "/lib64/libpam.so.0",
    "/usr/lib64/libpam.so.0",
    "/lib/libpam.so.0",
    "/usr/lib/libpam.so.0",
];

/// The one directory a service file is accepted from: a vendor file or
/// `/etc/pam.conf` would make Linux-PAM allow or fall back to `other`.
const SERVICE_DIR: &str = "/etc/pam.d";

#[repr(C)]
struct Message {
    style: c::c_int,
    text: *const c::c_char,
}

#[repr(C)]
struct Response {
    resp: *mut c::c_char,
    code: c::c_int,
}

type Converse =
    extern "C" fn(c::c_int, *mut *const Message, *mut *mut Response, *mut c::c_void) -> c::c_int;

#[repr(C)]
struct Conv {
    conv: Converse,
    data: *mut c::c_void,
}

type Start = unsafe extern "C" fn(
    *const c::c_char,
    *const c::c_char,
    *const Conv,
    *mut *mut c::c_void,
) -> c::c_int;
type SetItem = unsafe extern "C" fn(*mut c::c_void, c::c_int, *const c::c_void) -> c::c_int;
type AcctMgmt = unsafe extern "C" fn(*mut c::c_void, c::c_int) -> c::c_int;
type End = unsafe extern "C" fn(*mut c::c_void, c::c_int) -> c::c_int;
type StrError = unsafe extern "C" fn(*mut c::c_void, c::c_int) -> *const c::c_char;

/// The five libpam functions this uses, and the library they came from.
pub struct Pam {
    lib: *mut c::c_void,
    start: Start,
    set_item: SetItem,
    acct_mgmt: AcctMgmt,
    end: End,
    strerror: StrError,
}

// SAFETY: the pointers are to code, not data; libpam handles are made and
// ended inside one `check`, so no state is shared between threads.
unsafe impl Send for Pam {}
// SAFETY: as for `Send` above.
unsafe impl Sync for Pam {}

impl std::fmt::Debug for Pam {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Pam")
    }
}

/// Answers every prompt with an error: nobody is there to type.
extern "C" fn refuse_prompts(
    _n: c::c_int,
    _msgs: *mut *const Message,
    _resp: *mut *mut Response,
    _data: *mut c::c_void,
) -> c::c_int {
    PAM_CONV_ERR
}

impl Pam {
    /// Loads libpam and checks that `service` is `/etc/pam.d/<service>`
    /// and starts. Fails closed: every `Err` means do not serve.
    pub fn open(service: &str) -> Result<Pam, String> {
        service_file(service)?;
        let pam = Pam::load()?;
        // The probe: a library that loads but cannot start is no use.
        pam.start_ok(service)
            .map_err(|e| format!("the PAM probe for service {service} failed: {e}"))?;
        Ok(pam)
    }

    /// `pam_start` then `pam_end` for `service`: a probe of the library and
    /// the service, not of any account.
    fn start_ok(&self, service: &str) -> Result<(), String> {
        self.session(service, "root", |_| (PAM_SUCCESS, Ok(())))
    }

    fn load() -> Result<Pam, String> {
        let lib = LIBRARIES
            .iter()
            .filter(|p| Path::new(p).exists())
            .find_map(|p| {
                let path = CString::new(*p).ok()?;
                // SAFETY: `path` is a valid C string; `dlopen` has no other
                // precondition. A null return is a failed load.
                let lib = unsafe { c::dlopen(path.as_ptr(), c::RTLD_NOW | c::RTLD_LOCAL) };
                (!lib.is_null()).then_some(lib)
            })
            .ok_or("libpam.so.0 could not be loaded from a system directory")?;
        // SAFETY (each `sym`): the symbol is the documented libpam entry
        // point, and its C signature is the one in the type it is cast to.
        let sym = |name: &str| -> Result<*mut c::c_void, String> {
            let n = CString::new(name).map_err(|e| e.to_string())?;
            // SAFETY: `lib` is a live `dlopen` handle and `n` a valid C string.
            let f = unsafe { c::dlsym(lib, n.as_ptr()) };
            if f.is_null() {
                // SAFETY: `lib` came from `dlopen` above and is not used again.
                unsafe { c::dlclose(lib) };
                return Err(format!("libpam has no {name}"));
            }
            Ok(f)
        };
        // SAFETY: see above; a data pointer and a function pointer have the
        // same size on every Linux target.
        unsafe {
            Ok(Pam {
                lib,
                start: std::mem::transmute::<*mut c::c_void, Start>(sym("pam_start")?),
                set_item: std::mem::transmute::<*mut c::c_void, SetItem>(sym("pam_set_item")?),
                acct_mgmt: std::mem::transmute::<*mut c::c_void, AcctMgmt>(sym("pam_acct_mgmt")?),
                end: std::mem::transmute::<*mut c::c_void, End>(sym("pam_end")?),
                strerror: std::mem::transmute::<*mut c::c_void, StrError>(sym("pam_strerror")?),
            })
        }
    }

    /// Whether the site's policy lets `account` log in now. `ruser` is the
    /// peer's local user, or `None` (a tailnet peer has no local name).
    /// Everything but `PAM_SUCCESS` is an error, including
    /// `PAM_NEW_AUTHTOK_REQD`: an expired password cannot be changed here.
    pub fn check(&self, service: &str, account: &str, ruser: Option<&str>) -> Result<(), String> {
        let ruser = ruser
            .map(|r| CString::new(r).map_err(|_| "a user name holds a NUL".to_owned()))
            .transpose()?;
        self.session(service, account, |h| {
            if let Some(r) = &ruser {
                // SAFETY: `h` is a live handle; `r` is a valid C string that
                // `pam_set_item` copies.
                let rc = unsafe { (self.set_item)(h, PAM_RUSER, r.as_ptr().cast()) };
                if rc != PAM_SUCCESS {
                    return (rc, self.ok(h, rc));
                }
            }
            // SAFETY: `h` is a live handle.
            let rc = unsafe { (self.acct_mgmt)(h, PAM_SILENT) };
            (rc, self.ok(h, rc))
        })
    }

    /// `pam_start`, then `body` with the handle, then `pam_end` with the status
    /// of the last PAM call `body` made (which modules see when they clean up).
    fn session(
        &self,
        service: &str,
        account: &str,
        body: impl FnOnce(*mut c::c_void) -> (c::c_int, Result<(), String>),
    ) -> Result<(), String> {
        let service = CString::new(service).map_err(|_| "the service holds a NUL".to_owned())?;
        let account = CString::new(account).map_err(|_| "the account holds a NUL".to_owned())?;
        let conv = Conv {
            conv: refuse_prompts,
            data: std::ptr::null_mut(),
        };
        let mut h: *mut c::c_void = std::ptr::null_mut();
        // SAFETY: both strings and `conv` outlive the handle (it ends below);
        // `h` is a valid out pointer.
        let rc = unsafe { (self.start)(service.as_ptr(), account.as_ptr(), &conv, &mut h) };
        if rc != PAM_SUCCESS || h.is_null() {
            return Err(format!("pam_start failed ({rc})"));
        }
        let (status, result) = body(h);
        // SAFETY: `h` came from a successful `pam_start` and is ended once.
        unsafe { (self.end)(h, status) };
        result
    }

    fn ok(&self, h: *mut c::c_void, rc: c::c_int) -> Result<(), String> {
        if rc == PAM_SUCCESS {
            return Ok(());
        }
        // SAFETY: `h` is live; `pam_strerror` returns a static string or null.
        let text = unsafe {
            let p = (self.strerror)(h, rc);
            if p.is_null() {
                String::new()
            } else {
                CStr::from_ptr(p).to_string_lossy().into_owned()
            }
        };
        Err(format!("{text} ({rc})"))
    }

    /// A `Pam` over the given functions and no library: for tests.
    #[cfg(test)]
    fn with(
        start: Start,
        set_item: SetItem,
        acct_mgmt: AcctMgmt,
        end: End,
        strerror: StrError,
    ) -> Pam {
        Pam {
            lib: std::ptr::null_mut(),
            start,
            set_item,
            acct_mgmt,
            end,
            strerror,
        }
    }
}

impl Drop for Pam {
    fn drop(&mut self) {
        if !self.lib.is_null() {
            // SAFETY: `lib` came from `dlopen`, and no function pointer is
            // used after the `Pam` that owns them is gone.
            unsafe { c::dlclose(self.lib) };
        }
    }
}

/// `/etc/pam.d/<service>` must exist as a plain file: Linux-PAM would fall
/// back to `other` for a missing one, which may allow.
fn service_file(service: &str) -> Result<(), String> {
    if service.is_empty() || service.contains('/') || service.contains('\0') {
        return Err(format!("{service:?} is not a PAM service name"));
    }
    let path = Path::new(SERVICE_DIR).join(service);
    match std::fs::metadata(&path) {
        Ok(m) if m.is_file() => Ok(()),
        Ok(_) => Err(format!("{} is not a file", path.display())),
        Err(e) => Err(format!(
            "{}: {e} (only {SERVICE_DIR} counts: a vendor file or pam.conf would be \
             refused, never silently allowed)",
            path.display()
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicI32, AtomicUsize, Ordering};

    /// What the fake `pam_acct_mgmt` returns, and what it saw.
    static VERDICT: AtomicI32 = AtomicI32::new(0);
    static END_STATUS: AtomicI32 = AtomicI32::new(0);
    static RUSERS: AtomicUsize = AtomicUsize::new(0);
    static STARTS: AtomicUsize = AtomicUsize::new(0);
    static ENDS: AtomicUsize = AtomicUsize::new(0);
    /// Tests share the statics above.
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    unsafe extern "C" fn start(
        _s: *const c::c_char,
        _u: *const c::c_char,
        conv: *const Conv,
        h: *mut *mut c::c_void,
    ) -> c::c_int {
        STARTS.fetch_add(1, Ordering::SeqCst);
        // The conversation must refuse, whatever it is asked.
        let rc = ((*conv).conv)(
            1,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        );
        assert_eq!(rc, PAM_CONV_ERR);
        *h = 1usize as *mut c::c_void;
        PAM_SUCCESS
    }
    unsafe extern "C" fn set_item(
        _h: *mut c::c_void,
        t: c::c_int,
        _i: *const c::c_void,
    ) -> c::c_int {
        assert_eq!(t, PAM_RUSER);
        RUSERS.fetch_add(1, Ordering::SeqCst);
        PAM_SUCCESS
    }
    unsafe extern "C" fn acct(_h: *mut c::c_void, f: c::c_int) -> c::c_int {
        assert_eq!(f, PAM_SILENT);
        VERDICT.load(Ordering::SeqCst)
    }
    unsafe extern "C" fn end(_h: *mut c::c_void, s: c::c_int) -> c::c_int {
        ENDS.fetch_add(1, Ordering::SeqCst);
        END_STATUS.store(s, Ordering::SeqCst);
        PAM_SUCCESS
    }
    unsafe extern "C" fn strerror(_h: *mut c::c_void, _e: c::c_int) -> *const c::c_char {
        b"fake failure\0".as_ptr().cast()
    }
    fn fake() -> Pam {
        Pam::with(start, set_item, acct, end, strerror)
    }

    #[test]
    fn success_admits_and_the_handle_is_ended() {
        let _g = LOCK.lock().unwrap();
        VERDICT.store(PAM_SUCCESS, Ordering::SeqCst);
        let (s, e) = (STARTS.load(Ordering::SeqCst), ENDS.load(Ordering::SeqCst));
        assert_eq!(fake().check("rusty-mux", "alice", None), Ok(()));
        assert_eq!(STARTS.load(Ordering::SeqCst), s + 1);
        assert_eq!(ENDS.load(Ordering::SeqCst), e + 1);
    }

    #[test]
    fn every_other_answer_refuses_including_an_expired_password() {
        let _g = LOCK.lock().unwrap();
        for rc in [1, 12, 13, 7, 99] {
            VERDICT.store(rc, Ordering::SeqCst);
            let e = fake().check("rusty-mux", "alice", None).unwrap_err();
            assert_eq!(e, format!("fake failure ({rc})"));
            // `pam_end` gets the status of the last call, not `pam_start`'s.
            assert_eq!(END_STATUS.load(Ordering::SeqCst), rc);
        }
    }

    #[test]
    fn ruser_is_set_for_a_local_peer_and_left_unset_for_a_tailnet_one() {
        let _g = LOCK.lock().unwrap();
        VERDICT.store(PAM_SUCCESS, Ordering::SeqCst);
        let before = RUSERS.load(Ordering::SeqCst);
        fake().check("rusty-mux", "alice", Some("bob")).unwrap();
        assert_eq!(RUSERS.load(Ordering::SeqCst), before + 1);
        fake().check("rusty-mux", "alice", None).unwrap();
        assert_eq!(RUSERS.load(Ordering::SeqCst), before + 1);
    }

    #[test]
    fn a_nul_in_a_name_refuses_before_pam_is_asked() {
        let _g = LOCK.lock().unwrap();
        let s = STARTS.load(Ordering::SeqCst);
        assert!(fake().check("rusty-mux", "al\0ice", None).is_err());
        assert!(fake().check("rusty-mux", "alice", Some("b\0b")).is_err());
        assert_eq!(STARTS.load(Ordering::SeqCst), s);
    }

    #[test]
    fn a_service_must_be_a_plain_name_in_the_pam_directory() {
        for bad in ["", "a/b", "../x", "no-such-service-rusty-mux"] {
            assert!(service_file(bad).is_err(), "{bad:?}");
        }
    }
}
