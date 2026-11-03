//! The one command line a Windows spawn hands `CreateProcessW`: the
//! resolved program and `argv`, quoted by [`winargv`] (the security
//! boundary), with [`Command::raw_args`] spliced in verbatim after the
//! program.

use platform::error::{ErrorKind, OsCode, PlatformError, Result};
#[cfg(windows)]
use platform::process::{Command, Spawner};
use winargv::Target;

const SPACE: u16 = b' ' as u16;

/// `cmd`'s command line, wide, with no terminating NUL.
#[cfg(windows)]
pub(crate) fn command_line(cmd: &Command) -> Result<Vec<u16>> {
    use std::ffi::{OsStr, OsString};
    use std::os::windows::ffi::OsStrExt;

    let resolved = crate::WindowsSpawner.resolve(&cmd.program)?;
    let args: Vec<&OsStr> = cmd.argv.iter().map(OsString::as_os_str).collect();
    let line = winargv::build_command_line(&resolved, &args)?;
    let Some(raw) = &cmd.raw_args else {
        return Ok(line);
    };
    let program = winargv::build_command_line(&resolved, &[])?;
    let wide: Vec<u16> = resolved.encode_wide().collect();
    let raw: Vec<u16> = raw.encode_wide().collect();
    splice_raw(line, program.len(), &raw, winargv::classify(&wide))
}

/// `line` with a space and `raw` inserted after its first `program_len`
/// units, the quoted program. Refuses a NUL in `raw`, and any raw text for
/// a batch file, whose arguments must keep `cmd.exe`'s quoting.
fn splice_raw(line: Vec<u16>, program_len: usize, raw: &[u16], target: Target) -> Result<Vec<u16>> {
    if target == Target::Batch {
        return Err(refuse("raw_args: refused for a batch file"));
    }
    if raw.contains(&0) {
        return Err(refuse("raw_args: embedded NUL"));
    }
    let mut out = Vec::with_capacity(line.len() + raw.len() + 1);
    out.extend_from_slice(&line[..program_len]);
    out.push(SPACE);
    out.extend_from_slice(raw);
    out.extend_from_slice(&line[program_len..]);
    Ok(out)
}

fn refuse(what: &'static str) -> PlatformError {
    PlatformError::new(ErrorKind::InvalidInput, OsCode::None, what)
}

#[cfg(test)]
mod tests {
    use super::{splice_raw, Target};
    use platform::error::ErrorKind;

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    /// The line `program` + `args` with `raw` spliced in.
    fn spliced(program: &str, raw: &str, args: &[&str]) -> platform::error::Result<String> {
        let program = wide(program);
        let args: Vec<Vec<u16>> = args.iter().map(|a| wide(a)).collect();
        let slices: Vec<&[u16]> = args.iter().map(Vec::as_slice).collect();
        let line = winargv::build_command_line_wide(&program, &slices)?;
        let head = winargv::build_command_line_wide(&program, &[])?;
        let out = splice_raw(line, head.len(), &wide(raw), winargv::classify(&program))?;
        Ok(String::from_utf16(&out).expect("utf-16"))
    }

    #[test]
    fn raw_args_follow_the_program_verbatim() {
        let line =
            spliced(r"C:\Windows\System32\cmd.exe", r#"/c echo a"b c"d"#, &[]).expect("splice");
        assert_eq!(line, r#"C:\Windows\System32\cmd.exe /c echo a"b c"d"#);
    }

    #[test]
    fn quoted_arguments_follow_the_raw_text() {
        let line = spliced(r"C:\Program Files\x.exe", "/k  x", &["a b", "c"]).expect("splice");
        assert_eq!(line, r#""C:\Program Files\x.exe" /k  x "a b" c"#);
    }

    #[test]
    fn empty_raw_text_leaves_a_space() {
        assert_eq!(spliced("x.exe", "", &["a"]).expect("splice"), "x.exe  a");
    }

    #[test]
    fn a_batch_file_refuses_raw_text() {
        let err = spliced(r"C:\x.cmd", "a", &[]).expect_err("batch");
        assert_eq!(err.kind, ErrorKind::InvalidInput);
        assert_eq!(winargv::classify(&wide("x.bat")), Target::Batch);
    }

    #[test]
    fn a_nul_in_raw_text_is_refused() {
        let err = spliced("x.exe", "a\0b", &[]).expect_err("nul");
        assert_eq!(err.kind, ErrorKind::InvalidInput);
    }
}
