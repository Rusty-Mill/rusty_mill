//! Console probes (design §8.1): one binary, a subcommand per API. Each
//! prints one result line, `#> <subcommand> key=value ...`, so a test can
//! run it under ConPTY and under our host and compare the lines. A
//! failed call shows as `error=<Win32 error>`.
//!
//! ```text
//! conhost_probe host <program> [args...]   run a program on our host, output to stdout
//! conhost_probe write <bytes>              write that many `x`s to stdout
//! conhost_probe raw <hex>                  WriteFile those bytes to stdout: a raw
//!                                          write, read in the output code page
//! conhost_probe lines <raw|console> <n>    numbered lines, `n` bytes or more, in one
//!                                          WriteFile, or in one write_all to std's stdout
//! conhost_probe ctrl-hold <ms>             print `ready pid=<pid>`, then wait; on CTRL_CLOSE
//!                                          print `ctrl-close` and hold the handler <ms>
//! ```
//!
//! The parity probes are listed in [`probes::run`]. Text arguments take
//! `\e \r \n \t \b \s \\ \xHH \u{H..}` escapes, and text in a result line
//! is escaped the same way, so a line never holds a space or a control.
//!
//! `host` is also how fixtures are recorded: run it with
//! `RUSTY_CONHOST_RECORD` set.

#[cfg(windows)]
mod calls;
#[cfg(windows)]
mod probes;
mod text;

#[cfg(windows)]
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = match run(&args) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("conhost_probe: {e}");
            2
        }
    };
    std::process::exit(code);
}

#[cfg(not(windows))]
fn main() {
    // The escapes are plain Rust; keep them checked off Windows too.
    let _ = (text::escape(&[]), text::unescape(""));
    eprintln!("conhost_probe: the console probes run on Windows only");
    std::process::exit(2);
}

/// What precedes a probe's result line.
#[cfg(windows)]
const RESULT: &str = "#> ";

#[cfg(windows)]
fn run(args: &[String]) -> Result<i32, String> {
    use std::io::Write as _;

    let arg = |i: usize| args.get(i).map(String::as_str).ok_or("missing argument");
    match arg(0)? {
        "host" => host(&args[1..]),
        "write" => {
            let n: usize = arg(1)?.parse().map_err(|_| "a byte count")?;
            let mut out = std::io::stdout().lock();
            for chunk in vec![b'x'; n].chunks(4096) {
                out.write_all(chunk).map_err(|e| e.to_string())?;
            }
            out.flush().map_err(|e| e.to_string())?;
            Ok(0)
        }
        "raw" => {
            let bytes = text::unhex(arg(1)?)?;
            calls::write_raw(&bytes).map_err(|e| format!("error={e}"))?;
            Ok(0)
        }
        "lines" => {
            let n: usize = arg(2)?.parse().map_err(|_| "a byte count")?;
            let text: String = (0..)
                .map(|i| format!("line {i:07}\r\n"))
                .take(n.div_ceil(14))
                .collect();
            match arg(1)? {
                "raw" => calls::write_raw(text.as_bytes()).map_err(|e| format!("error={e}"))?,
                // std's console stdout: WriteConsoleW, in as few
                // calls as its version makes (one, in Rust 1.98).
                "console" => {
                    let mut out = std::io::stdout().lock();
                    out.write_all(text.as_bytes()).map_err(|e| e.to_string())?;
                    out.flush().map_err(|e| e.to_string())?;
                }
                other => return Err(format!("lines: unknown writer {other:?}")),
            }
            Ok(0)
        }
        "ctrl-hold" => {
            let ms = arg(1)?.parse().map_err(|_| "milliseconds")?;
            calls::hold_ctrl_close(ms).map_err(|e| format!("error={e}"))?;
            println!("ready pid={}", std::process::id());
            loop {
                std::thread::park();
            }
        }
        _ => {
            let line = probes::run(args)?;
            // On a line of its own, whatever the probe left on the screen,
            // after a marker that no probe's own output holds.
            print!("\r\n{RESULT}{line}\r\n");
            std::io::stdout().flush().map_err(|e| e.to_string())?;
            Ok(0)
        }
    }
}

/// Run `argv` on our host and copy its output to stdout.
#[cfg(windows)]
fn host(argv: &[String]) -> Result<i32, String> {
    use std::io::Write as _;

    use platform::process::{Child, Command};
    use platform::pty::PtyMaster;
    use platform::term::WinSize;
    use rusty_conhost::OwnPty;

    let (program, rest) = argv.split_first().ok_or("host: no program")?;
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    let mut cmd = Command::new(program, cwd);
    cmd.argv = rest.iter().map(Into::into).collect();
    let size = WinSize {
        rows: 30,
        cols: 120,
    };
    let (master, child) = OwnPty
        .spawn_concrete(&cmd, size)
        .map_err(|e| e.to_string())?;
    let mut out = std::io::stdout().lock();
    let mut buf = [0u8; 16 * 1024];
    loop {
        let n = master.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        out.write_all(&buf[..n]).map_err(|e| e.to_string())?;
    }
    out.flush().map_err(|e| e.to_string())?;
    match Box::new(child).wait().map_err(|e| e.to_string())? {
        platform::process::ExitStatus::Code(code) => Ok(code),
        other => Err(format!("host: the child ended with {other:?}")),
    }
}
