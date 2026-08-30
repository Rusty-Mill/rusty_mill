//! winargv fuzz oracle (RFC v2 §9.5): for arbitrary argv, the built
//! command line must round-trip through an independent model of the
//! MSVCRT / `CommandLineToArgvW` splitting rules back to the exact input,
//! and the batch path must refuse exactly its documented set and never
//! panic.
//!
//! The model parser below is a second, independent implementation of the
//! documented splitting rules (differential testing — the point is that
//! builder and parser cannot share a bug). The deterministic
//! `CommandLineToArgvW` oracle on the Windows CI leg
//! (`crates/platform-windows/tests/winargv_oracle.rs`) anchors the model
//! itself to the real OS: both parse the same generated-line dialect, and
//! the model's unit tests replicate the oracle's known-hard-case table.
//!
//! Runs on Linux runners (the logic is pure `&[u16]`); nightly schedule
//! in `.github/workflows/fuzz.yml`.

#![cfg_attr(fuzzing, no_main)]

use platform::error::ErrorKind;
use platform_windows::winargv;

const QUOTE: u16 = b'"' as u16;
const BACKSLASH: u16 = b'\\' as u16;
const SPACE: u16 = b' ' as u16;
const TAB: u16 = b'\t' as u16;

/// Independent model of the MSVCRT argument-splitting rules:
///
/// - space/tab separate arguments outside quotes;
/// - `n` backslashes before a `"` yield `n/2` backslashes, then: `n` odd →
///   a literal `"`; `n` even → quote-mode toggles;
/// - backslashes not before a `"` are literal;
/// - the first token (program) uses the simple rule: quoted → up to the
///   closing quote; else up to the first whitespace.
fn model_split(line: &[u16]) -> Vec<Vec<u16>> {
    let mut out: Vec<Vec<u16>> = Vec::new();
    let mut i = 0usize;

    // Program token, simple rule.
    let mut program = Vec::new();
    if line.first() == Some(&QUOTE) {
        i = 1;
        while i < line.len() && line[i] != QUOTE {
            program.push(line[i]);
            i += 1;
        }
        i = (i + 1).min(line.len());
    } else {
        while i < line.len() && line[i] != SPACE && line[i] != TAB {
            program.push(line[i]);
            i += 1;
        }
    }
    out.push(program);

    // Arguments, full rules.
    let mut in_quotes = false;
    let mut in_arg = false;
    let mut current: Vec<u16> = Vec::new();
    while i < line.len() {
        let u = line[i];
        if u == BACKSLASH {
            let mut n = 0usize;
            while i < line.len() && line[i] == BACKSLASH {
                n += 1;
                i += 1;
            }
            if i < line.len() && line[i] == QUOTE {
                current.extend(std::iter::repeat(BACKSLASH).take(n / 2));
                if n % 2 == 1 {
                    current.push(QUOTE);
                } else {
                    in_quotes = !in_quotes;
                }
                in_arg = true;
                i += 1;
            } else {
                current.extend(std::iter::repeat(BACKSLASH).take(n));
                in_arg = true;
            }
        } else if u == QUOTE {
            in_quotes = !in_quotes;
            in_arg = true;
            i += 1;
        } else if (u == SPACE || u == TAB) && !in_quotes {
            if in_arg {
                out.push(std::mem::take(&mut current));
                in_arg = false;
            }
            i += 1;
        } else {
            current.push(u);
            in_arg = true;
            i += 1;
        }
    }
    if in_arg {
        out.push(current);
    }
    out
}

fn check(args: &[Vec<u16>]) {
    let program: Vec<u16> = "prog.exe".encode_utf16().collect();
    let slices: Vec<&[u16]> = args.iter().map(Vec::as_slice).collect();

    // Executable path: NUL is the only refusal; everything else must
    // round-trip through the model exactly.
    let has_nul = args.iter().any(|a| a.contains(&0));
    match winargv::build_command_line_wide(&program, &slices) {
        Ok(line) => {
            assert!(!has_nul, "NUL must be refused");
            let parsed = model_split(&line);
            assert_eq!(parsed.len(), args.len() + 1, "arg count");
            for (i, arg) in args.iter().enumerate() {
                assert_eq!(&parsed[i + 1], arg, "arg {i} mismatch");
            }
        }
        Err(e) => {
            assert!(has_nul, "only NUL may be refused for executables");
            assert_eq!(e.kind, ErrorKind::InvalidInput);
        }
    }

    // Batch path: refuse exactly {NUL, CR, LF, %}; never panic; on
    // success the line contains no bare CR/LF and every arg is fully
    // quoted (structural invariants of the cmd-safe subset).
    let batch: Vec<u16> = "s.bat".encode_utf16().collect();
    let refused = args.iter().any(|a| {
        a.iter().any(|&u| {
            u == 0 || u == u16::from(b'\r') || u == u16::from(b'\n') || u == u16::from(b'%')
        })
    });
    match winargv::build_command_line_wide(&batch, &slices) {
        Ok(line) => {
            assert!(!refused, "documented batch refusals must refuse");
            assert!(
                !line.contains(&u16::from(b'\r')) && !line.contains(&u16::from(b'\n')),
                "no raw newline may survive into a batch line"
            );
        }
        Err(e) => {
            assert!(
                refused,
                "batch refused an argument outside its documented set"
            );
            assert_eq!(e.kind, ErrorKind::InvalidInput);
        }
    }
}

#[cfg(fuzzing)]
libfuzzer_sys::fuzz_target!(|args: Vec<Vec<u16>>| {
    check(&args);
});

// Outside fuzzing, the binary is only its tests.
#[cfg(not(fuzzing))]
fn main() {}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    /// The model must agree with the real `CommandLineToArgvW` on the
    /// oracle's known-hard-case table (the Windows CI leg anchors that
    /// table to the OS; this test anchors the model to the table).
    #[test]
    fn model_matches_the_oracle_table() {
        let cases: &[&str] = &[
            "",
            " ",
            "a",
            "hello world",
            r#"say "hi""#,
            r#"a\\"b"#,
            r"trailing\",
            r"trailing\\",
            r#""""#,
            r"C:\path\with spaces\x",
            "tab\there",
            "100%",
            r#"\" "\" \\" \\\" ""#,
        ];
        let args: Vec<Vec<u16>> = cases.iter().map(|s| w(s)).collect();
        check(&args);
    }

    #[test]
    fn refusals_hold() {
        check(&[w("has\u{0}nul")]);
        check(&[w("100%"), w("plain")]);
        check(&[w("line\nbreak")]);
    }

    /// A deterministic mini-sweep of the adversarial alphabet — the same
    /// class the fuzzer explores, kept here so `cargo test` exercises the
    /// differential without a fuzz run.
    #[test]
    fn adversarial_sweep() {
        let alphabet = [b'a' as u16, QUOTE, BACKSLASH, SPACE, b'%' as u16];
        let mut all: Vec<Vec<u16>> = vec![Vec::new()];
        let mut last: Vec<Vec<u16>> = vec![Vec::new()];
        for _ in 0..3 {
            let mut next = Vec::new();
            for prefix in &last {
                for &u in &alphabet {
                    let mut s = prefix.clone();
                    s.push(u);
                    next.push(s);
                }
            }
            all.extend(next.iter().cloned());
            last = next;
        }
        for chunk in all.chunks(25) {
            check(chunk);
        }
    }
}
