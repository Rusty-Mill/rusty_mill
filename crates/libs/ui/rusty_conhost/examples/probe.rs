//! Console probes (design §8.1): one binary, a subcommand per API. Each
//! prints one result line, `<subcommand> key=value ...`, so a test can
//! run it under ConPTY and under our host and compare the lines.
//!
//! ```text
//! probe host <program> [args...]   run a program on our host, output to stdout
//! probe modes                      GetConsoleMode on stdin and stdout
//! probe set-mode <in|out> <hex>    SetConsoleMode, then GetConsoleMode
//! probe cp                         GetConsoleCP
//! probe set-cp <code page>         SetConsoleCP, then GetConsoleCP
//! probe write <bytes>              write that many `x`s to stdout
//! ```
//!
//! `host` is also how fixtures are recorded: run it with
//! `RUSTY_CONHOST_RECORD` set.

#[cfg(windows)]
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = match probe::run(&args) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("probe: {e}");
            2
        }
    };
    std::process::exit(code);
}

#[cfg(not(windows))]
fn main() {
    eprintln!("probe: the console probes run on Windows only");
    std::process::exit(2);
}

#[cfg(windows)]
mod probe {
    use std::io::Write as _;

    use platform::process::{Child, Command};
    use platform::pty::PtyMaster;
    use platform::term::WinSize;
    use rusty_conhost::OwnPty;

    pub(crate) fn run(args: &[String]) -> Result<i32, String> {
        let arg = |i: usize| args.get(i).map(String::as_str).ok_or("missing argument");
        match arg(0)? {
            "host" => host(&args[1..]),
            "modes" => {
                let (input, output) = (console::mode(false), console::mode(true));
                println!("modes in={} out={}", hex(input), hex(output));
                Ok(0)
            }
            "set-mode" => {
                let output = arg(1)? == "out";
                let mode = u32::from_str_radix(arg(2)?, 16).map_err(|e| e.to_string())?;
                let set = console::set_mode(output, mode);
                println!(
                    "set-mode {} after={}",
                    result(set),
                    hex(console::mode(output))
                );
                Ok(0)
            }
            "cp" => {
                println!("cp in={}", console::cp());
                Ok(0)
            }
            "set-cp" => {
                let cp = arg(1)?.parse().map_err(|_| "a code page")?;
                let set = console::set_cp(cp);
                println!("set-cp {} after={}", result(set), console::cp());
                Ok(0)
            }
            "write" => {
                let n: usize = arg(1)?.parse().map_err(|_| "a byte count")?;
                let mut out = std::io::stdout().lock();
                for chunk in vec![b'x'; n].chunks(4096) {
                    out.write_all(chunk).map_err(|e| e.to_string())?;
                }
                out.flush().map_err(|e| e.to_string())?;
                Ok(0)
            }
            other => Err(format!("unknown probe {other:?}")),
        }
    }

    /// Run `argv` on our host and copy its output to stdout.
    fn host(argv: &[String]) -> Result<i32, String> {
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

    fn hex(mode: Result<u32, i32>) -> String {
        match mode {
            Ok(m) => format!("{m:#x}"),
            Err(e) => format!("error{e}"),
        }
    }

    fn result(r: Result<(), i32>) -> String {
        match r {
            Ok(()) => "ok".to_owned(),
            Err(e) => format!("error={e}"),
        }
    }

    /// The console calls, through platform-windows' admitted surface.
    /// Each returns the Win32 error on failure.
    #[allow(unsafe_code)]
    mod console {
        use platform_windows::ffi::win32_surface as w;

        fn last_error() -> i32 {
            std::io::Error::last_os_error().raw_os_error().unwrap_or(-1)
        }

        fn std_handle(output: bool) -> w::HANDLE {
            let which = if output {
                w::STD_OUTPUT_HANDLE
            } else {
                w::STD_INPUT_HANDLE
            };
            // SAFETY: GetStdHandle takes a constant and returns a handle
            // value (possibly null or invalid); nothing is dereferenced.
            unsafe { w::GetStdHandle(which) }
        }

        pub(crate) fn mode(output: bool) -> Result<u32, i32> {
            let mut mode = 0;
            // SAFETY: `mode` is a valid, writable u32 for the call.
            if unsafe { w::GetConsoleMode(std_handle(output), &mut mode) } == 0 {
                return Err(last_error());
            }
            Ok(mode)
        }

        pub(crate) fn set_mode(output: bool, mode: u32) -> Result<(), i32> {
            // SAFETY: plain values in, a BOOL out.
            if unsafe { w::SetConsoleMode(std_handle(output), mode) } == 0 {
                return Err(last_error());
            }
            Ok(())
        }

        pub(crate) fn cp() -> u32 {
            // SAFETY: no arguments; returns the input code page.
            unsafe { w::GetConsoleCP() }
        }

        pub(crate) fn set_cp(cp: u32) -> Result<(), i32> {
            // SAFETY: a plain value in, a BOOL out.
            if unsafe { w::SetConsoleCP(cp) } == 0 {
                return Err(last_error());
            }
            Ok(())
        }
    }
}
