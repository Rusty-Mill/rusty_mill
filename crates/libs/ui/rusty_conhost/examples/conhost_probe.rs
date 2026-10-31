//! Console probes (design §8.1): one binary, a subcommand per API. Each
//! prints one result line, `<subcommand> key=value ...`, so a test can
//! run it under ConPTY and under our host and compare the lines.
//!
//! ```text
//! conhost_probe host <program> [args...]   run a program on our host, output to stdout
//! conhost_probe modes                      GetConsoleMode on stdin and stdout
//! conhost_probe set-mode <in|out> <hex>    SetConsoleMode, then GetConsoleMode
//! conhost_probe cp                         GetConsoleCP
//! conhost_probe set-cp <code page>         SetConsoleCP, then GetConsoleCP
//! conhost_probe write <bytes>              write that many `x`s to stdout
//! conhost_probe raw <hex>                  WriteFile those bytes to stdout: a raw
//!                                          write, read in the output code page
//! conhost_probe ctrl-hold <ms>             print `ready pid=<pid>`, then wait; on CTRL_CLOSE
//!                                          print `ctrl-close` and hold the handler <ms>
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
            eprintln!("conhost_probe: {e}");
            2
        }
    };
    std::process::exit(code);
}

#[cfg(not(windows))]
fn main() {
    eprintln!("conhost_probe: the console probes run on Windows only");
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
            "raw" => {
                let bytes = unhex(arg(1)?)?;
                console::write_raw(&bytes).map_err(|e| format!("error={e}"))?;
                Ok(0)
            }
            "ctrl-hold" => {
                let ms = arg(1)?.parse().map_err(|_| "milliseconds")?;
                console::hold_ctrl_close(ms).map_err(|e| format!("error={e}"))?;
                println!("ready pid={}", std::process::id());
                loop {
                    std::thread::park();
                }
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

    fn unhex(text: &str) -> Result<Vec<u8>, String> {
        if !text.len().is_multiple_of(2) {
            return Err("an even number of hex digits".to_owned());
        }
        (0..text.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&text[i..i + 2], 16).map_err(|e| e.to_string()))
            .collect()
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
        use std::io::Write as _;
        use std::sync::atomic::{AtomicU64, Ordering};
        use std::time::Duration;

        use platform_windows::ffi::win32_surface as w;

        /// How long [`on_ctrl`] holds a CTRL_CLOSE.
        static HOLD_MS: AtomicU64 = AtomicU64::new(0);

        /// Announce CTRL_CLOSE through the console, then hold the handler,
        /// as a client that cleans up slowly does.
        extern "system" fn on_ctrl(event: u32) -> i32 {
            if event != w::CTRL_CLOSE_EVENT {
                return 0;
            }
            let mut out = std::io::stdout().lock();
            let _ = writeln!(out, "ctrl-close");
            let _ = out.flush();
            drop(out);
            std::thread::sleep(Duration::from_millis(HOLD_MS.load(Ordering::Relaxed)));
            1
        }

        pub(crate) fn hold_ctrl_close(ms: u64) -> Result<(), i32> {
            HOLD_MS.store(ms, Ordering::Relaxed);
            // SAFETY: `on_ctrl` is a safe `extern "system"` fn of the
            // handler's signature; it touches only stdout and an atomic.
            if unsafe { w::SetConsoleCtrlHandler(Some(on_ctrl), 1) } == 0 {
                return Err(last_error());
            }
            Ok(())
        }

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

        /// `WriteFile` on stdout, which the console serves as a raw
        /// write; std's console stdout would use WriteConsoleW instead.
        pub(crate) fn write_raw(bytes: &[u8]) -> Result<(), i32> {
            use std::os::windows::io::FromRawHandle as _;
            // SAFETY: stdout's handle stays open for the process's life;
            // ManuallyDrop keeps the File from closing it.
            let file = unsafe { std::fs::File::from_raw_handle(std_handle(true)) };
            let mut file = std::mem::ManuallyDrop::new(file);
            file.write_all(bytes)
                .map_err(|e| e.raw_os_error().unwrap_or(-1))
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
