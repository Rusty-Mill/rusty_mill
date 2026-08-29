//! OS-backed random byte source, no external crates.
//!
//! Both backends use the platform's non-blocking CSPRNG: `/dev/urandom`
//! never blocks on modern (5.6+) Linux and on macOS/BSD once the kernel
//! entropy pool is initialized, and `BCryptGenRandom` with
//! `BCRYPT_USE_SYSTEM_PREFERRED_RNG` is likewise non-blocking on Windows.

#[cfg(unix)]
pub(crate) fn fill(buf: &mut [u8]) {
    use std::fs::File;
    use std::io::Read;
    use std::sync::Mutex;
    use std::sync::OnceLock;

    static URANDOM: OnceLock<Mutex<File>> = OnceLock::new();
    let file = URANDOM.get_or_init(|| {
        Mutex::new(File::open("/dev/urandom").expect("rusty_uuid: failed to open /dev/urandom"))
    });
    file.lock()
        .unwrap()
        .read_exact(buf)
        .expect("rusty_uuid: failed to read from /dev/urandom");
}

#[cfg(windows)]
pub(crate) fn fill(buf: &mut [u8]) {
    #[link(name = "bcrypt")]
    extern "system" {
        fn BCryptGenRandom(
            h_algorithm: *mut core::ffi::c_void,
            p_buffer: *mut u8,
            cb_buffer: u32,
            dw_flags: u32,
        ) -> i32;
    }

    const BCRYPT_USE_SYSTEM_PREFERRED_RNG: u32 = 0x0000_0002;

    let status = unsafe {
        BCryptGenRandom(
            core::ptr::null_mut(),
            buf.as_mut_ptr(),
            buf.len() as u32,
            BCRYPT_USE_SYSTEM_PREFERRED_RNG,
        )
    };
    assert_eq!(status, 0, "rusty_uuid: BCryptGenRandom failed");
}

#[cfg(not(any(unix, windows)))]
compile_error!("rusty_uuid: no random source available for this platform");
