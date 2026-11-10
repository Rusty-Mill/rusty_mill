//! Windows ConPTY backend, the rollback to our own host (`[console] host =
//! "conpty"`).
//!
//! Spawns the shell (`%COMSPEC%`, default `cmd.exe`) on a
//! [pseudoconsole](https://learn.microsoft.com/windows/console/pseudoconsoles)
//! through platform-windows' [`WindowsPty`], the wrapper rusty_mux_server
//! uses, with the same command line our own host gets (`own::shell_command`).
//! The master side is exposed as a [`BackendHandle`], mirroring the Unix PTY
//! backend. This module also owns the host console's raw mode and size,
//! which both Windows backends share.
//!
//! This module is only compiled on Windows (gated in `backend/mod.rs`).
//!
//! NOTE: run and verified on Windows 11 (build 26200) — shell spawn, child
//! `TERM`/`COLORTERM`, bidirectional relay, and OSC title capture all work.
//! Host resize propagation also works: there is no `SIGWINCH` equivalent to
//! wire, so `resize_poll` in `src/runtime/reactor.rs` polls the console size
//! on a timer and calls [`BackendHandle::set_winsize`] on change.

use std::io;
use std::path::Path;
use std::sync::Arc;

use crate::backend::cmdline::{self, CmdRules, Reader};
use crate::backend::own::{GRACE, POLL, exited_within, shell_command, to_io};
use crate::backend::{Backend, BackendHandle};
use crate::sync::Mutex;
use platform::process::{Child as _, ExitStatus, Signal};
use platform::pty::PtyMaster as _;
use platform::term::WinSize;
use platform_windows::{WindowsChild, WindowsPty, WindowsPtyMaster, winargv};

use windows_sys::Win32::System::Console::{
    CONSOLE_SCREEN_BUFFER_INFO, ENABLE_PROCESSED_OUTPUT, GetConsoleMode,
    GetConsoleScreenBufferInfo, GetStdHandle, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, SetConsoleMode,
};

/// Unit type implementing [`Backend`](crate::backend::Backend) for Windows.
pub struct WindowsBackend;

/// Host console modes captured when raw mode was enabled, restored on exit.
/// Tuple is `(stdin_mode, stdout_mode)`.
static ORIGINAL_MODES: Mutex<Option<(u32, u32)>> = Mutex::new(None);

/// `GetConsoleMode` fails with `ERROR_INVALID_HANDLE` (raw OS error 6)
/// whenever `GetStdHandle(STD_INPUT_HANDLE)`/`GetStdHandle(STD_OUTPUT_HANDLE)`
/// aren't real, attached console handles — always true when stdin/stdout are
/// piped or redirected. That's an inherent limitation (a terminal emulator's
/// raw-mode host handling requires a real console to put in raw mode), not
/// a transient or unusual failure, but the bare `io::Error::last_os_error()`
/// ("The handle is invalid") gave no hint why. This turns it into an
/// actionable message while preserving the original error for diagnostics.
fn not_a_real_console_error() -> std::io::Error {
    let underlying = std::io::Error::last_os_error();
    std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        format!(
            "rusty_term must run attached to a real console: stdin/stdout appear to be \
             piped, redirected, or otherwise non-interactive, so GetConsoleMode failed \
             ({underlying})"
        ),
    )
}

impl Backend for WindowsBackend {
    fn spawn_shell(
        &self,
        cols: u16,
        rows: u16,
        shell: Option<&str>,
        args: &[String],
        cwd: Option<&Path>,
    ) -> Result<Box<dyn BackendHandle>, io::Error> {
        let cwd = match cwd {
            Some(dir) => dir.to_path_buf(),
            None => std::env::current_dir()?,
        };
        let cmd = shell_command(&shell_or_default(shell), args, cwd.into_os_string())?;
        let (master, child) = WindowsPty
            .spawn_concrete(&cmd, WinSize { rows, cols })
            .map_err(to_io)?;
        Ok(Box::new(ConptyHandle {
            master: Arc::new(master),
            child: Some(Arc::new(Mutex::new(child))),
        }))
    }

    fn set_raw_mode(&self, enabled: bool) -> Result<(), std::io::Error> {
        unsafe {
            let hin = GetStdHandle(STD_INPUT_HANDLE);
            let hout = GetStdHandle(STD_OUTPUT_HANDLE);
            if enabled {
                let mut in_mode: u32 = 0;
                let mut out_mode: u32 = 0;
                if GetConsoleMode(hin, &mut in_mode) == 0
                    || GetConsoleMode(hout, &mut out_mode) == 0
                {
                    return Err(not_a_real_console_error());
                }
                *ORIGINAL_MODES.lock() = Some((in_mode, out_mode));
                // Raw input: no line buffering, echo, or Ctrl-C handling, but do
                // decode VT input sequences. Output: enable VT processing. The
                // core transformation is shared with rusty_lines' Windows
                // backend via rusty_win32::console::raw_mode_core (see its own
                // doc comment); ENABLE_PROCESSED_OUTPUT on top is this
                // backend's own addition.
                let (new_in, core_out) = rusty_win32::console::raw_mode_core(in_mode, out_mode);
                let new_out = core_out | ENABLE_PROCESSED_OUTPUT;
                if SetConsoleMode(hin, new_in) == 0 || SetConsoleMode(hout, new_out) == 0 {
                    return Err(std::io::Error::last_os_error());
                }
            } else if let Some((in_mode, out_mode)) = ORIGINAL_MODES.lock().take() {
                SetConsoleMode(hin, in_mode);
                SetConsoleMode(hout, out_mode);
            }
        }
        Ok(())
    }

    fn terminal_size(&self) -> Option<(u16, u16)> {
        unsafe {
            let hout = GetStdHandle(STD_OUTPUT_HANDLE);
            let mut info: CONSOLE_SCREEN_BUFFER_INFO = std::mem::zeroed();
            if GetConsoleScreenBufferInfo(hout, &mut info) == 0 {
                return None;
            }
            let w = info.srWindow;
            let cols = (w.Right - w.Left + 1) as u16;
            let rows = (w.Bottom - w.Top + 1) as u16;
            if cols > 0 && rows > 0 {
                Some((cols, rows))
            } else {
                None
            }
        }
    }
}

/// The configured shell, else `%COMSPEC%`, else `cmd.exe`. Shared with the
/// own-host backend.
pub(super) fn shell_or_default(shell: Option<&str>) -> String {
    shell
        .map(str::to_owned)
        .or_else(|| std::env::var("COMSPEC").ok())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "cmd.exe".to_string())
}

/// Appends the launcher `args` to `line`, the command line of a program
/// read by `reader` whose own parameters start at `params`, each after a
/// space and quoted for that reader (see `cmdline`'s module doc). For
/// cmd.exe and batch files an argument cmd can't take safely is refused
/// with `InvalidInput`: a `%`, CR, LF or NUL, a switch before cmd's own,
/// or what the shell's [`CmdRules`] rule out.
pub(super) fn push_launcher_args(
    line: &mut Vec<u16>,
    params: usize,
    reader: Reader,
    args: &[String],
) -> std::io::Result<()> {
    if reader == Reader::Crt {
        for arg in args {
            line.push(u16::from(b' '));
            push_quoted_arg(arg, line);
        }
        return Ok(());
    }
    let mut rules = cmdline::cmd_rules(line, params, reader);
    let mut switched = rules.switched;
    for arg in args {
        let wide: Vec<u16> = arg.encode_utf16().collect();
        if !switched && reader == Reader::Cmd {
            // An option before the switch is cmd's own, as `/v:on` is.
            rules.delayed |= cmdline::delayed_expansion(&wide);
        }
        if !switched && cmdline::is_command_switch(arg) {
            switched = true;
        } else if !switched && cmdline::has_command_switch(&wide) {
            return Err(cmd_refused(
                arg,
                &"it holds a switch that would start a command",
            ));
        }
        line.push(u16::from(b' '));
        push_cmd_arg(arg, rules, line)?;
    }
    if rules.wrap && !args.is_empty() {
        cmdline::keep_command_quotes(line, params);
    }
    Ok(())
}

/// Appends `arg` for cmd.exe: bare when it is one plain word, else by
/// cmd's rules, as the shell's `rules` allow.
fn push_cmd_arg(arg: &str, rules: CmdRules, out: &mut Vec<u16>) -> std::io::Result<()> {
    if rules.delayed && arg.contains('!') {
        return Err(cmd_refused(arg, &"the shell's /v expands `!`"));
    }
    if cmdline::plain_for_cmd(arg) {
        out.extend(arg.encode_utf16());
        return Ok(());
    }
    if let Some(why) = rules.plain_only {
        return Err(cmd_refused(arg, &why));
    }
    let wide: Vec<u16> = arg.encode_utf16().collect();
    winargv::quote_batch_arg_into(&wide, out).map_err(|e| cmd_refused(arg, &e))
}

/// The error for an argument cmd.exe can't take safely.
fn cmd_refused(arg: &str, why: &dyn std::fmt::Display) -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::InvalidInput,
        format!("rusty_term: cmd.exe can't take the argument {arg:?} safely: {why}"),
    )
}

/// Appends `arg` to `out` (a wide command-line buffer), quoted per the
/// MSVCRT/`CreateProcessW` argument rules — backslashes only escape when
/// immediately followed by a quote — so the child's argv parser splits it
/// back out unchanged. Mirrors the algorithm Windows itself documents.
pub(super) fn push_quoted_arg(arg: &str, out: &mut Vec<u16>) {
    use std::os::windows::ffi::OsStrExt;
    let needs_quotes = arg.is_empty() || arg.chars().any(|c| c == ' ' || c == '\t' || c == '"');
    if !needs_quotes {
        out.extend(std::ffi::OsStr::new(arg).encode_wide());
        return;
    }
    out.push(b'"' as u16);
    let mut chars = arg.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            let mut backslashes = 1;
            while chars.peek() == Some(&'\\') {
                chars.next();
                backslashes += 1;
            }
            let doubled = matches!(chars.peek(), Some('"') | None);
            for _ in 0..(if doubled {
                backslashes * 2
            } else {
                backslashes
            }) {
                out.push(b'\\' as u16);
            }
        } else if c == '"' {
            out.push(b'\\' as u16);
            out.push(b'"' as u16);
        } else {
            let mut buf = [0u16; 2];
            out.extend(c.encode_utf16(&mut buf).iter());
        }
    }
    out.push(b'"' as u16);
}

/// The terminal's side of a ConPTY. The handle `spawn_shell` returns also
/// owns the child; clones share the console only. The last reference closes
/// the console (`WindowsPtyMaster`'s Drop).
struct ConptyHandle {
    master: Arc<WindowsPtyMaster>,
    /// Shared with the exit token's watcher, which polls it.
    child: Option<Arc<Mutex<WindowsChild>>>,
}

impl BackendHandle for ConptyHandle {
    fn read_into(&mut self, buf: &mut [u8]) -> Result<usize, io::Error> {
        self.master.read(buf).map_err(to_io)
    }

    fn write(&mut self, data: &[u8]) -> Result<(), io::Error> {
        // Pipes accept short writes; loop until everything is taken.
        let mut rest = data;
        while !rest.is_empty() {
            let n = self.master.write(rest).map_err(to_io)?;
            if n == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::WriteZero,
                    "ConPTY took no input",
                ));
            }
            rest = &rest[n..];
        }
        Ok(())
    }

    fn try_clone(&self) -> Result<Box<dyn BackendHandle>, io::Error> {
        Ok(Box::new(ConptyHandle {
            master: Arc::clone(&self.master),
            child: None,
        }))
    }

    fn set_winsize(&mut self, cols: u16, rows: u16) -> Result<(), io::Error> {
        self.master.resize(WinSize { rows, cols }).map_err(to_io)
    }

    fn exit_token(&self) -> Option<Box<dyn FnOnce() + Send>> {
        // Only the original holds the child. `WindowsChild` has no
        // blocking wait that leaves it usable, so the watcher polls.
        let child = Arc::clone(self.child.as_ref()?);
        Some(Box::new(move || {
            loop {
                // The guard drops before the sleep, so Drop and
                // `reap_exit_status` can take the lock between polls.
                let status = child.lock().try_wait();
                match status {
                    Ok(None) => std::thread::sleep(POLL),
                    Ok(Some(_)) => return,
                    Err(e) => {
                        eprintln!("rusty_term: can't read the shell's exit status: {e}");
                        return;
                    }
                }
            }
        }))
    }

    fn reap_exit_status(&mut self) -> Option<i32> {
        let child = self.child.as_ref()?;
        match exited_within(&mut child.lock(), GRACE) {
            Some(ExitStatus::Code(code)) => Some(code),
            _ => None,
        }
    }
}

impl Drop for ConptyHandle {
    /// The original hangs up, which asks the child to exit, gives it
    /// [`GRACE`], then kills it if it is still running. Both run on
    /// threads of their own, so Drop returns at once: `hang_up` blocks
    /// while conhost is stuck writing into a full, unread output pipe.
    /// A clone only drops its reference; the last one closes the console
    /// (`WindowsPtyMaster`'s draining Drop).
    fn drop(&mut self) {
        let Some(child) = self.child.take() else {
            return;
        };
        let master = Arc::clone(&self.master);
        // If no thread can be had, the last reference's Drop still closes
        // the console, which tells the child to exit.
        let _ = std::thread::Builder::new()
            .name("conpty-hang-up".into())
            .spawn(move || master.hang_up());
        let end = move || end_survivor(&child);
        if let Err(e) = std::thread::Builder::new()
            .name("conpty-reap".into())
            .spawn(end)
        {
            eprintln!("rusty_term: can't start the shell's reaper: {e}");
        }
    }
}

/// Kill `child` unless it exits within [`GRACE`] of the hang-up.
fn end_survivor(child: &Mutex<WindowsChild>) {
    let mut child = child.lock();
    if exited_within(&mut child, GRACE).is_none()
        && let Err(e) = child.kill_single(Signal::Kill)
    {
        eprintln!("rusty_term: the shell outlived its pseudoconsole and can't be killed: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::{push_launcher_args, push_quoted_arg};
    use crate::backend::cmdline::{locate, reader, split_program};
    use std::io::ErrorKind;

    /// `shell` with `args` appended by the launcher rules both hosts use.
    fn line(shell: &str, args: &[&str]) -> std::io::Result<String> {
        let args: Vec<String> = args.iter().map(|a| (*a).to_owned()).collect();
        let (program, rest) = split_program(shell, |p| locate(p).is_some()).expect("a program");
        let mut line: Vec<u16> = shell.encode_utf16().collect();
        let params = line.len() - rest.encode_utf16().count();
        push_launcher_args(&mut line, params, reader(program), &args)?;
        Ok(String::from_utf16(&line).expect("utf-16"))
    }

    fn refused(shell: &str, args: &[&str]) -> Option<ErrorKind> {
        line(shell, args).err().map(|e| e.kind())
    }

    /// #1279: cmd.exe gets its launcher arguments by cmd's rules.
    #[test]
    fn launcher_args_for_cmd_are_quoted_by_cmds_rules() {
        let injected = r#"x\" & calc & \"y"#;
        assert_eq!(
            line("cmd.exe /c echo", &[injected]).expect("quoted"),
            r#"cmd.exe /c echo "x\"" & calc & \""y""#
        );
        assert_eq!(
            line("cmd.exe /c echo", &["tail one", "/q"]).expect("quoted"),
            r#"cmd.exe /c echo "tail one" /q"#
        );
        assert_eq!(
            line("cmd", &["/c", "x y", "a&b"]).expect("quoted"),
            r#"cmd /c ""x y" "a&b"""#
        );
        assert_eq!(
            refused("cmd /c echo", &["%PATH%"]),
            Some(ErrorKind::InvalidInput)
        );
        assert_eq!(
            refused("cmd /c echo", &["a\nb"]),
            Some(ErrorKind::InvalidInput)
        );
        // After a shell that leaves a quote open, only plain words go on.
        assert_eq!(
            refused(r#"cmd /c echo ""#, &["a b"]),
            Some(ErrorKind::InvalidInput)
        );
        assert_eq!(
            line(r#"cmd /c echo ""#, &["ab"]).expect("plain"),
            r#"cmd /c echo " ab"#
        );
        // Before cmd's own switch, an argument holding one is refused, even
        // quoted (cmd finds it there); the bare switch itself is taken.
        assert_eq!(
            refused("cmd", &["x /c calc"]),
            Some(ErrorKind::InvalidInput)
        );
        assert_eq!(refused("cmd /q", &["a/k"]), Some(ErrorKind::InvalidInput));
        assert_eq!(
            line("cmd", &["/K", "x/c"]).expect("after the switch"),
            "cmd /K x/c"
        );
        assert_eq!(
            line("cmd /c echo", &["a/c"]).expect("after it"),
            "cmd /c echo a/c"
        );
        // A shell written for cmd's quote stripping keeps its text and takes
        // plain words only (review of 3fcd0d4d, P2 1).
        assert_eq!(
            line(r#"cmd /c ""C:\a b\x.exe" -v""#, &["y"]).expect("plain"),
            r#"cmd /c ""C:\a b\x.exe" -v" y"#
        );
        assert_eq!(
            line(r#"cmd /s /c "prog a""#, &["y"]).expect("plain"),
            r#"cmd /s /c "prog a" y"#
        );
        assert_eq!(
            refused(r#"cmd /s /c "prog a""#, &["a&b"]),
            Some(ErrorKind::InvalidInput)
        );
        // %X% may hold a quote (P2 2); /v expands `!` (P2 3).
        assert_eq!(
            refused("cmd /c echo %X%", &["a & calc"]),
            Some(ErrorKind::InvalidInput)
        );
        assert_eq!(
            refused("cmd /v:on /c echo", &["a!PATH!b"]),
            Some(ErrorKind::InvalidInput)
        );
        // Options from the launcher count too (review of d21ea929, P2).
        assert_eq!(
            refused("cmd", &["/v:on", "/c", "echo", "!SECRET!"]),
            Some(ErrorKind::InvalidInput)
        );
        assert_eq!(
            line("cmd", &["/v:off", "/c", "echo", "a!b"]).expect("off"),
            r#"cmd /v:off /c echo "a!b""#
        );
        // A drive-relative cmd is cmd (review of d21ea929, P1).
        assert_eq!(
            line("C:cmd.exe /c echo", &[r#"x\" & calc & \"y"#]).expect("quoted"),
            r#"C:cmd.exe /c echo "x\"" & calc & \""y""#
        );
    }

    #[test]
    fn launcher_args_for_other_programs_keep_msvcrt_quoting() {
        assert_eq!(
            line("pwsh -NoLogo", &[r#"a\"b"#, "%x%"]).expect("quoted"),
            r#"pwsh -NoLogo "a\\\"b" %x%"#
        );
    }

    fn quote(arg: &str) -> String {
        let mut out = Vec::new();
        push_quoted_arg(arg, &mut out);
        String::from_utf16(&out).unwrap()
    }

    #[test]
    fn plain_arg_is_unquoted() {
        assert_eq!(quote("hi"), "hi");
        assert_eq!(quote("-lc"), "-lc");
    }

    #[test]
    fn spaces_and_empty_get_quoted() {
        assert_eq!(quote("echo hi"), "\"echo hi\"");
        assert_eq!(quote(""), "\"\"");
    }

    #[test]
    fn embedded_quote_is_escaped() {
        assert_eq!(quote(r#"say "hi""#), r#""say \"hi\"""#);
    }

    #[test]
    fn trailing_backslashes_before_closing_quote_are_doubled() {
        assert_eq!(quote(r"C:\some dir\"), r#""C:\some dir\\""#);
    }

    #[test]
    fn backslashes_not_before_a_quote_pass_through() {
        assert_eq!(quote(r"C:\some\path"), r"C:\some\path");
    }

    /// Smoke test: a ConPTY child spawns and its output reaches our reader.
    ///
    /// The pipe read is a blocking `ReadFile`, so the reads happen on a
    /// detached thread and the deadline is enforced by polling the shared
    /// buffer; a hung read can never wedge the test harness. It fails without
    /// `STARTF_USESTDHANDLES` in platform-windows' `spawn_concrete` whenever
    /// the test runner's own std handles are redirected
    /// (docs/research/conpty-attach-2026-07.md).
    /// Review of 66a8fb5f: Drop returns at once, and the hang-up or the
    /// kill ends a running child; the exit watcher must not hold the
    /// child's lock while it sleeps, or the reaper never gets it.
    #[test]
    fn dropping_the_original_conpty_handle_ends_a_running_child() {
        use crate::backend::Backend as _;
        use std::time::{Duration, Instant};
        let h = super::WindowsBackend
            .spawn_shell(80, 24, Some("ping.exe -n 60 127.0.0.1"), &[], None)
            .expect("spawn");
        let exited = h.exit_token().expect("exit token");
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            exited();
            let _ = tx.send(());
        });
        let start = Instant::now();
        drop(h);
        assert!(
            start.elapsed() < Duration::from_millis(500),
            "Drop hands the shutdown off"
        );
        rx.recv_timeout(super::GRACE + Duration::from_secs(5))
            .expect("the child ended after the hang-up");
    }

    #[test]
    fn conpty_child_output_reaches_the_reader() {
        use crate::backend::Backend as _;
        use std::sync::{Arc, Mutex};
        use std::time::{Duration, Instant};
        let h = super::WindowsBackend
            .spawn_shell(80, 24, Some("cmd.exe /c echo boot_ok"), &[], None)
            .expect("spawn");
        let out = Arc::new(Mutex::new(Vec::<u8>::new()));
        let mut rh = h.try_clone().expect("clone read handle");
        let out2 = Arc::clone(&out);
        std::thread::spawn(move || {
            loop {
                match rh.read() {
                    Ok(b) if b.is_empty() => break,
                    Ok(b) => out2.lock().unwrap().extend_from_slice(&b),
                    Err(_) => break,
                }
            }
        });
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if out.lock().unwrap().windows(7).any(|w| w == b"boot_ok") {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let buf = out.lock().unwrap().clone();
        panic!("no child output: {:?}", String::from_utf8_lossy(&buf));
    }

    /// End-to-end win32-input-mode (G10) against a real ConPTY child: conhost
    /// requests `?9001` at startup, and a command typed purely as win32 key
    /// records — presses *and* releases, plus an arrow-key history recall —
    /// round-trips through cmd.exe.
    #[cfg(feature = "gui")]
    #[test]
    fn win32_input_records_round_trip_through_a_real_conpty_child() {
        use crate::backend::{Backend as _, BackendHandle};
        use crate::gui::input::encode_win32;
        use std::sync::{Arc, Mutex};
        use std::time::{Duration, Instant};
        use winit::keyboard::{Key, KeyCode, ModifiersState, NamedKey, PhysicalKey};

        let mut h = super::WindowsBackend
            .spawn_shell(120, 30, Some("cmd.exe"), &[], None)
            .expect("spawn cmd.exe under ConPTY");
        let out = Arc::new(Mutex::new(Vec::<u8>::new()));
        let mut rh = h.try_clone().expect("clone read handle");
        let out2 = Arc::clone(&out);
        let reader = std::thread::spawn(move || {
            loop {
                match rh.read() {
                    Ok(b) if b.is_empty() => break,
                    Ok(b) => out2.lock().unwrap().extend_from_slice(&b),
                    Err(_) => break,
                }
            }
        });

        let wait_for = |needle: &[u8], count: usize, why: &str| {
            let deadline = Instant::now() + Duration::from_secs(20);
            loop {
                let n = {
                    let buf = out.lock().unwrap();
                    buf.windows(needle.len()).filter(|w| *w == needle).count()
                };
                if n >= count {
                    return;
                }
                assert!(
                    Instant::now() < deadline,
                    "timed out waiting for {why}; child output so far: {:?}",
                    String::from_utf8_lossy(&out.lock().unwrap())
                );
                std::thread::sleep(Duration::from_millis(50));
            }
        };

        // conhost announces win32-input-mode support by requesting the mode
        // from the hosting terminal as soon as the pseudoconsole starts.
        wait_for(b"\x1b[?9001h", 1, "conhost's ?9001h request");

        fn keycode(c: char) -> KeyCode {
            match c {
                'c' => KeyCode::KeyC,
                'e' => KeyCode::KeyE,
                'h' => KeyCode::KeyH,
                'i' => KeyCode::KeyI,
                'k' => KeyCode::KeyK,
                'o' => KeyCode::KeyO,
                'r' => KeyCode::KeyR,
                't' => KeyCode::KeyT,
                'x' => KeyCode::KeyX,
                '0' => KeyCode::Digit0,
                '1' => KeyCode::Digit1,
                '9' => KeyCode::Digit9,
                ' ' => KeyCode::Space,
                _ => unreachable!("test only types [cehikortx09 1]"),
            }
        }
        let mods = ModifiersState::empty();
        let press_release = |h: &mut Box<dyn BackendHandle>, code: KeyCode, key: &Key| {
            for down in [true, false] {
                let bytes =
                    encode_win32(PhysicalKey::Code(code), key, mods, down).expect("encodable key");
                h.write(&bytes).expect("write to child");
            }
        };
        let type_line = |h: &mut Box<dyn BackendHandle>, line: &str| {
            for c in line.chars() {
                press_release(h, keycode(c), &Key::Character(c.to_string().into()));
            }
            press_release(h, KeyCode::Enter, &Key::Named(NamedKey::Enter));
        };

        // Every keystroke below reaches cmd.exe only as CSI ..._ records.
        type_line(&mut h, "echo rtok9001");
        // Once as the echoed input line, once as the command's output.
        wait_for(b"rtok9001", 2, "echo output typed via win32 records");

        // ArrowUp (an enhanced key) recalls the command from history.
        press_release(&mut h, KeyCode::ArrowUp, &Key::Named(NamedKey::ArrowUp));
        press_release(&mut h, KeyCode::Enter, &Key::Named(NamedKey::Enter));
        wait_for(b"rtok9001", 4, "arrow-up history recall");

        // `exit` typed as records must terminate the child for real.
        let exited = h.exit_token().expect("owning handle has an exit token");
        type_line(&mut h, "exit");
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            exited();
            let _ = tx.send(());
        });
        rx.recv_timeout(Duration::from_secs(20))
            .expect("child exited after `exit` typed via win32 records");
        drop(h); // tears down the pseudoconsole, EOFs the reader
        reader.join().unwrap();
    }
}
