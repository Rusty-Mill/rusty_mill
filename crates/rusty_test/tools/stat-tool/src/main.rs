//! Reference tool 1: lists a directory and stats each entry through a
//! contract-scoped `FsRoot`. Exercises the filesystem primitive only —
//! no process spawn, no PTY.

use std::ffi::OsString;
use std::path::Path;

use compat::{NativeCapabilities, Workspace};
use contract::FsRoot;

/// Resolve the target path from an `Option<OsString>` argv[1] (as returned
/// by `std::env::args_os().nth(1)`), defaulting to `.`. Stays in `OsString`
/// the whole way — the argument is only ever fed to `Path`, never printed
/// or compared as text, so there is no reason to route it through `String`
/// and risk the panic `std::env::args()` raises on non-Unicode input.
fn resolve_target(arg: Option<OsString>) -> OsString {
    arg.unwrap_or_else(|| OsString::from("."))
}

fn main() -> anyhow::Result<()> {
    let target = resolve_target(std::env::args_os().nth(1));
    let target_path = Path::new(&target);
    let root = target_path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let name = target_path
        .file_name()
        .map(Path::new)
        .unwrap_or(Path::new("."));

    let ws = Workspace::open_ambient(root)?;
    let caps = NativeCapabilities::detect();
    println!("capabilities: {caps:?}");

    let meta = ws.stat(name)?;
    if meta.is_dir {
        println!("{:<32} {:>10} {:>6} {:>6}", "name", "bytes", "dir", "link");
        for entry in ws.read_dir(name)? {
            println!(
                "{:<32} {:>10} {:>6} {:>6}",
                entry.name, entry.metadata.len, entry.metadata.is_dir, entry.metadata.is_symlink
            );
        }
    } else {
        println!(
            "{}: {} bytes, readonly={}",
            target_path.display(),
            meta.len,
            meta.readonly
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_target_defaults_to_current_dir() {
        assert_eq!(resolve_target(None), OsString::from("."));
    }

    #[test]
    fn resolve_target_passes_valid_unicode_through_unchanged() {
        assert_eq!(
            resolve_target(Some(OsString::from("café"))),
            OsString::from("café")
        );
    }

    // The regression this guards: `std::env::args().nth(1)` panics with
    // "invalid utf-8 sequence" the instant argv[1] isn't valid Unicode.
    // `resolve_target` takes an `OsString` straight from `args_os()` and
    // never converts it to `String`, so this same input must survive and
    // still reach `Path::new` unchanged.
    #[test]
    #[cfg(unix)]
    fn resolve_target_does_not_panic_on_invalid_utf8_arg() {
        use std::os::unix::ffi::OsStringExt;

        // 0xFF is not a valid UTF-8 lead or continuation byte anywhere.
        let bad = OsString::from_vec(vec![b'f', b'o', 0xFF, b'o']);
        let resolved = resolve_target(Some(bad.clone()));
        assert_eq!(resolved, bad);
        // Must also survive being wrapped in a `Path` without panicking.
        let _ = Path::new(&resolved);
    }

    #[test]
    #[cfg(windows)]
    fn resolve_target_does_not_panic_on_unpaired_surrogate() {
        use std::os::windows::ffi::OsStringExt;

        // An unpaired high surrogate: a legal argv code unit on Windows
        // (argv is UTF-16), but not representable as UTF-8 — exactly what
        // `std::env::args()` panics on.
        let bad = OsString::from_wide(&[0x0066, 0x0066, 0xD800, 0x0066]);
        let resolved = resolve_target(Some(bad.clone()));
        assert_eq!(resolved, bad);
        // Must also survive being wrapped in a `Path` without panicking.
        let _ = Path::new(&resolved);
    }
}
