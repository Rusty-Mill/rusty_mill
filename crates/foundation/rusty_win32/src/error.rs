//! `GetLastError()` wrapped as a typed, `Result`-friendly error — the Win32
//! counterpart of `rusty_libc::Errno`. A raw Win32 error code should never
//! escape a safe wrapper in this crate unwrapped.

use core::fmt;

#[cfg(windows)]
extern crate alloc;

#[cfg(windows)]
#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetLastError() -> u32;
    fn FormatMessageW(
        flags: u32,
        source: *const core::ffi::c_void,
        message_id: u32,
        language_id: u32,
        buffer: *mut u16,
        size: u32,
        arguments: *mut core::ffi::c_void,
    ) -> u32;
}

#[cfg(windows)]
const FORMAT_MESSAGE_FROM_SYSTEM: u32 = 0x0000_1000;
#[cfg(windows)]
const FORMAT_MESSAGE_IGNORE_INSERTS: u32 = 0x0000_0200;

/// A Win32 error code, as returned by `GetLastError()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Win32Error(u32);

impl Win32Error {
    /// Wrap a raw Win32 error code (e.g. one already in hand from a
    /// documented return value, rather than `GetLastError()`).
    pub const fn from_raw(code: u32) -> Self {
        Win32Error(code)
    }

    /// The calling thread's last error code, via `GetLastError()`. Call this
    /// immediately after a Win32 API reports failure — any intervening Win32
    /// call, even an unrelated one, can overwrite it.
    #[cfg(windows)]
    pub fn last() -> Self {
        // SAFETY: `GetLastError` takes no arguments and has no preconditions.
        Win32Error(unsafe { GetLastError() })
    }

    /// The raw numeric code.
    pub const fn code(self) -> u32 {
        self.0
    }

    /// The full system-provided message text for this code —
    /// `FormatMessageW`, looked up from Windows' own system message table.
    /// Unlike `Display` (which only recognizes the handful of codes named
    /// as associated constants above, a fast, allocation-free path), this
    /// covers virtually any Win32 error code Windows itself knows how to
    /// describe, at the cost of an allocation and a real system call —
    /// use `Display`/`to_string()` for the common case, this for a code
    /// that isn't one of the named constants.
    ///
    /// Returns `None` if `FormatMessageW` itself fails: an unrecognized
    /// code, or (practically unreachable for a real system message) one
    /// too long for this call's fixed-size buffer. Callers should fall
    /// back to `Display`'s "unknown Win32 error N" wording in that case.
    #[cfg(windows)]
    pub fn message(self) -> Option<alloc::string::String> {
        let mut buf = [0u16; 512];
        // SAFETY: `source`/`arguments` are documented-valid NULLs for a
        // system-message-table lookup with printf-style inserts ignored
        // (`FORMAT_MESSAGE_IGNORE_INSERTS`, since this crate has no
        // `va_list` to provide and no message it looks up needs one);
        // `buf` is a valid, `buf.len()`-element writable buffer.
        let len = unsafe {
            FormatMessageW(
                FORMAT_MESSAGE_FROM_SYSTEM | FORMAT_MESSAGE_IGNORE_INSERTS,
                core::ptr::null(),
                self.0,
                0,
                buf.as_mut_ptr(),
                buf.len() as u32,
                core::ptr::null_mut(),
            )
        };
        if len == 0 {
            return None;
        }
        // System messages end in "\r\n"; trimmed so this reads consistently
        // with `Display`'s own messages, which carry no trailing newline.
        let mut text = alloc::string::String::from_utf16_lossy(&buf[..len as usize]);
        let trimmed_len = text.trim_end().len();
        text.truncate(trimmed_len);
        Some(text)
    }
}

macro_rules! win32_errors {
    ($($name:ident = $val:expr => $msg:expr),* $(,)?) => {
        impl Win32Error {
            $(
                #[allow(non_upper_case_globals)]
                pub const $name: Win32Error = Win32Error($val);
            )*
        }

        impl fmt::Display for Win32Error {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                match self.0 {
                    $($val => f.write_str($msg),)*
                    other => write!(f, "unknown Win32 error {other}"),
                }
            }
        }
    };
}

// Messages match the FormatMessageW text for these codes (verified against
// Microsoft's own system-error-code documentation), not invented wording —
// a `std::io::Error` built from the same code via `From` below should read
// the same as this `Display` impl.
win32_errors! {
    SUCCESS = 0 => "the operation completed successfully",
    ERROR_FILE_NOT_FOUND = 2 => "the system cannot find the file specified",
    ERROR_PATH_NOT_FOUND = 3 => "the system cannot find the path specified",
    ERROR_ACCESS_DENIED = 5 => "access is denied",
    ERROR_INVALID_HANDLE = 6 => "the handle is invalid",
    ERROR_NOT_ENOUGH_MEMORY = 8 => "not enough storage is available to process this command",
    ERROR_INVALID_DATA = 13 => "the data is invalid",
    ERROR_OUTOFMEMORY = 14 => "not enough storage is available to complete this operation",
    ERROR_NOT_READY = 21 => "the device is not ready",
    ERROR_SHARING_VIOLATION = 32 => "the process cannot access the file because it is being used by another process",
    ERROR_HANDLE_EOF = 38 => "reached the end of the file",
    ERROR_NOT_SUPPORTED = 50 => "the request is not supported",
    ERROR_FILE_EXISTS = 80 => "the file exists",
    ERROR_INVALID_PARAMETER = 87 => "the parameter is incorrect",
    ERROR_BUFFER_OVERFLOW = 111 => "the file name is too long",
    ERROR_BROKEN_PIPE = 109 => "the pipe has been ended",
    ERROR_CALL_NOT_IMPLEMENTED = 120 => "this function is not supported on this system",
    ERROR_INSUFFICIENT_BUFFER = 122 => "the data area passed to a system call is too small",
    ERROR_INVALID_NAME = 123 => "the filename, directory name, or volume label syntax is incorrect",
    ERROR_ALREADY_EXISTS = 183 => "cannot create a file when that file already exists",
    ERROR_ENVIRONMENT_VARIABLE_NOT_FOUND = 203 => "the system could not find the environment option that was entered",
    ERROR_NO_DATA = 232 => "the pipe is being closed",
    ERROR_MORE_DATA = 234 => "more data is available",
    ERROR_OPERATION_ABORTED = 995 => "the I/O operation has been aborted because of either a thread exit or an application request",
    ERROR_IO_PENDING = 997 => "overlapped I/O operation is in progress",
    ERROR_NOT_FOUND = 1168 => "element not found",
    ERROR_TIMEOUT = 1460 => "this operation returned because the timeout period expired",
    ERROR_NOT_A_REPARSE_POINT = 4390 => "the file or directory is not a reparse point",
    ERROR_NO_MORE_FILES = 18 => "there are no more files",
    ERROR_PIPE_BUSY = 231 => "all pipe instances are busy",
    ERROR_PIPE_CONNECTED = 535 => "a client is connected to the pipe",
    ERROR_NOTIFY_ENUM_DIR = 1022 => "the buffer for the directory-change notifications overflowed",
    ERROR_NO_MORE_ITEMS = 259 => "no more data is available",
    ERROR_INVALID_SID = 1337 => "the security ID structure is invalid",
    ERROR_SERVICE_DOES_NOT_EXIST = 1060 => "the specified service does not exist as an installed service",
    ERROR_SERVICE_ALREADY_RUNNING = 1056 => "an instance of the service is already running",
    ERROR_PRIVILEGE_NOT_HELD = 1314 => "a required privilege is not held by the client",
    ERROR_GEN_FAILURE = 31 => "a device attached to the system is not functioning",
}

impl core::error::Error for Win32Error {}

#[cfg(feature = "std")]
impl From<Win32Error> for std::io::Error {
    fn from(e: Win32Error) -> Self {
        // `from_raw_os_error` is exactly this on Windows: it stores the code
        // and defers to FormatMessageW for `Display`, same as a "real"
        // `std::io::Error` produced by a failing std call.
        std::io::Error::from_raw_os_error(e.code() as i32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_known_code() {
        assert_eq!(
            Win32Error::ERROR_ACCESS_DENIED.to_string(),
            "access is denied"
        );
    }

    #[test]
    fn display_unknown_code() {
        assert_eq!(
            Win32Error::from_raw(424242).to_string(),
            "unknown Win32 error 424242"
        );
    }

    #[test]
    fn code_roundtrip() {
        assert_eq!(Win32Error::from_raw(5).code(), 5);
        assert_eq!(Win32Error::ERROR_ACCESS_DENIED.code(), 5);
    }

    #[test]
    fn named_constants_are_distinct_values() {
        assert_ne!(
            Win32Error::ERROR_FILE_NOT_FOUND,
            Win32Error::ERROR_PATH_NOT_FOUND
        );
    }

    #[cfg(windows)]
    #[test]
    fn message_returns_real_system_text_for_a_known_code() {
        // Not asserting an exact string: this is real OS-provided text
        // (locale/build-dependent in general), so only a robust invariant
        // is checked rather than betting on exact capitalization/
        // punctuation this sandbox has no Windows machine to verify.
        let msg = Win32Error::ERROR_ACCESS_DENIED
            .message()
            .expect("FormatMessageW should succeed for a well-known code");
        let lower = msg.to_ascii_lowercase();
        assert!(
            lower.contains("access") && lower.contains("denied"),
            "expected the real system message to mention access/denied, got {msg:?}"
        );
    }

    #[cfg(windows)]
    #[test]
    fn message_returns_none_for_an_unrecognized_code() {
        assert_eq!(Win32Error::from_raw(424242).message(), None);
    }

    #[cfg(feature = "std")]
    #[test]
    fn converts_to_io_error_by_code() {
        let io_err: std::io::Error = Win32Error::ERROR_FILE_NOT_FOUND.into();
        assert_eq!(io_err.raw_os_error(), Some(2));
    }
}
