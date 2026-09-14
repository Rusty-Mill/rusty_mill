//! Reference tool 2: spawns a child process through a contract
//! `ProcessRunner`, captures stdout/stderr, and reports the exit status.
//! Exercises process spawn + stdio capture only — no fs, no PTY.

use std::ffi::OsString;

use anyhow::bail;
use compat::NativeProcessRunner;
use contract::{ProcessRunner, ProcessSpec};

/// Build a `ProcessSpec` from argv (program name at index 0, `std::env`
/// convention), lossily converting each `OsString` to `String` so a
/// non-Unicode argument degrades to U+FFFD replacement characters instead
/// of panicking the way `std::env::args()` does. `ProcessSpec` is typed
/// `String`/`Vec<String>`, so a lossy conversion at this boundary is
/// unavoidable; mirrors `rustils/crates/coreutils::args::collect_lossy`,
/// duplicated locally rather than pulled in as a dependency — `rusty_test`
/// does not depend on `rustils` (see ARCHITECTURE.md: treat it as a donor
/// to raid for patterns, not a base to converge into).
fn build_spec<I: IntoIterator<Item = OsString>>(argv: I) -> anyhow::Result<ProcessSpec> {
    let mut args = argv
        .into_iter()
        .skip(1)
        .map(|a| a.to_string_lossy().into_owned());
    let Some(program) = args.next() else {
        bail!("usage: proc-runner <program> [args...]");
    };

    let mut spec = ProcessSpec::new(program);
    for arg in args {
        spec = spec.arg(arg);
    }
    Ok(spec)
}

fn main() -> anyhow::Result<()> {
    let spec = build_spec(std::env::args_os())?;

    let runner = NativeProcessRunner;
    let output = runner.run(&spec)?;

    print!("{}", String::from_utf8_lossy(&output.stdout));
    eprint!("{}", String::from_utf8_lossy(&output.stderr));
    std::process::exit(output.status);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_spec_passes_valid_unicode_through_unchanged() {
        let argv = vec![
            OsString::from("proc-runner"),
            OsString::from("echo"),
            OsString::from("café"),
        ];
        let spec = build_spec(argv).expect("valid argv builds a spec");
        assert_eq!(spec.program, "echo");
        assert_eq!(spec.args, vec!["café".to_string()]);
    }

    // The regression this guards: `std::env::args()` panics with "invalid
    // utf-8 sequence" the instant one argv entry isn't valid Unicode.
    // `build_spec` is built on `args_os()` + `to_string_lossy`, which by
    // contract never panics, so this same input must survive.
    #[test]
    #[cfg(unix)]
    fn build_spec_does_not_panic_on_invalid_utf8_argv() {
        use std::os::unix::ffi::OsStringExt;

        // 0xFF is not a valid UTF-8 lead or continuation byte anywhere.
        let bad = OsString::from_vec(vec![b'f', b'o', 0xFF, b'o']);
        let argv = vec![OsString::from("proc-runner"), bad];
        let spec = build_spec(argv).expect("non-unicode argv still builds a spec");
        assert!(
            spec.program.contains('\u{FFFD}'),
            "expected a lossy replacement character, got {:?}",
            spec.program
        );
    }

    #[test]
    #[cfg(windows)]
    fn build_spec_does_not_panic_on_unpaired_surrogate() {
        use std::os::windows::ffi::OsStringExt;

        // An unpaired high surrogate: a legal argv code unit on Windows
        // (argv is UTF-16), but not representable as UTF-8 — exactly what
        // `std::env::args()` panics on.
        let bad = OsString::from_wide(&[0x0066, 0x0066, 0xD800, 0x0066]);
        let argv = vec![OsString::from("proc-runner"), bad];
        let spec = build_spec(argv).expect("non-unicode argv still builds a spec");
        assert!(
            spec.program.contains('\u{FFFD}'),
            "expected a lossy replacement character, got {:?}",
            spec.program
        );
    }
}
