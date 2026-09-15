//! winargv oracle (RFC v2 §9.5, first cut): every command line
//! [`winargv::build_command_line_wide`] produces for a normal executable
//! must round-trip through the real Windows splitter —
//! `CommandLineToArgvW` — back to exactly the argv it was built from.
//! Runs only on the Windows CI leg; the pure unit tests in the module
//! cover both legs. (The §9.5 argv-echo *fuzz* oracle is the follow-up;
//! this is the deterministic seed, including an exhaustive sweep over the
//! adversarial alphabet.)
//!
//! Batch (`cmd.exe`) lines have no equivalent parsing API to oracle
//! against; their guarantees are refusal-based and pinned by unit tests.

#![cfg(windows)]
#![allow(unsafe_code)] // oracle FFI below, each call with its invariant

use platform_windows::ffi::win32_surface as w;
use platform_windows::winargv;

/// Parse `line` with the real `CommandLineToArgvW`.
fn os_split(line: &[u16]) -> Vec<Vec<u16>> {
    let mut with_nul = line.to_vec();
    with_nul.push(0);
    let mut argc: i32 = 0;
    // SAFETY: `with_nul` is a valid NUL-terminated UTF-16 buffer and
    // `argc` a valid out-pointer, both outliving the call.
    let argv = unsafe { w::CommandLineToArgvW(with_nul.as_ptr(), &mut argc) };
    assert!(!argv.is_null(), "CommandLineToArgvW failed");
    let mut out = Vec::with_capacity(argc as usize);
    for i in 0..argc as usize {
        // SAFETY: `argv` is the valid array of `argc` NUL-terminated
        // wide strings the call just returned; `i` is in range; each
        // string is walked only to its NUL.
        unsafe {
            let p = *argv.add(i);
            let mut len = 0usize;
            while *p.add(len) != 0 {
                len += 1;
            }
            out.push(std::slice::from_raw_parts(p, len).to_vec());
        }
    }
    // SAFETY: `argv` is the LocalAlloc'd buffer CommandLineToArgvW
    // returned, freed exactly once here, after the last read above.
    unsafe { w::LocalFree(argv.cast()) };
    out
}

fn assert_round_trip(program: &str, args: &[Vec<u16>]) {
    let program_w: Vec<u16> = program.encode_utf16().collect();
    let arg_slices: Vec<&[u16]> = args.iter().map(Vec::as_slice).collect();
    let line = winargv::build_command_line_wide(&program_w, &arg_slices)
        .expect("executable targets accept arbitrary non-NUL args");
    let parsed = os_split(&line);
    assert_eq!(
        parsed.len(),
        args.len() + 1,
        "arg count mismatch for line {:?}",
        String::from_utf16_lossy(&line)
    );
    for (i, arg) in args.iter().enumerate() {
        assert_eq!(
            &parsed[i + 1],
            arg,
            "arg {} mismatch for line {:?}",
            i,
            String::from_utf16_lossy(&line)
        );
    }
}

#[test]
fn known_hard_cases_round_trip() {
    let cases: &[&str] = &[
        "",
        " ",
        "a",
        "hello world",
        r#"say "hi""#,
        r#"a\\"b"#,
        r"trailing\",
        r"trailing\\",
        r#""#,
        r#""""#,
        r"C:\path\with spaces\x",
        "tab\there",
        "100%",
        r#"\" "\" \\" \\\" ""#,
    ];
    let args: Vec<Vec<u16>> = cases.iter().map(|s| s.encode_utf16().collect()).collect();
    assert_round_trip("prog.exe", &args);
}

#[test]
fn program_token_round_trips() {
    let program = r"C:\Program Files\some tool.exe";
    let program_w: Vec<u16> = program.encode_utf16().collect();
    let line = winargv::build_command_line_wide(&program_w, &[]).expect("build");
    let parsed = os_split(&line);
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0], program_w);
}

/// Exhaustive sweep: every string up to length 4 over the adversarial
/// alphabet {`a`, `"`, `\`, space, `%`} — 781 arguments — must round-trip.
/// This is the class of inputs where MSVCRT quoting goes wrong if it ever
/// does; exhaustive beats sampled at this size.
#[test]
fn exhaustive_adversarial_alphabet_round_trips() {
    const ALPHABET: [u16; 5] = [
        b'a' as u16,
        b'"' as u16,
        b'\\' as u16,
        b' ' as u16,
        b'%' as u16,
    ];
    let mut all: Vec<Vec<u16>> = vec![Vec::new()];
    let mut last: Vec<Vec<u16>> = vec![Vec::new()];
    for _ in 0..4 {
        let mut next = Vec::new();
        for prefix in &last {
            for &u in &ALPHABET {
                let mut s = prefix.clone();
                s.push(u);
                next.push(s);
            }
        }
        all.extend(next.iter().cloned());
        last = next;
    }
    // Batches keep each command line small enough to read on failure.
    for chunk in all.chunks(50) {
        assert_round_trip("prog.exe", chunk);
    }
}
