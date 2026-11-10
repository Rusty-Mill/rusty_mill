use rusty_term::{backend, config, core, runtime, shells, term};

use std::path::PathBuf;
use std::sync::Arc;

use rusty_term::sync::Mutex;

use backend::Backend;
#[cfg(unix)]
use backend::UnixBackend;
#[cfg(windows)]
use backend::{OwnBackend, WindowsBackend};
use config::{Config, LaunchMode, flag_value};
use core::Grid;

/// Splits CLI args at the first `--`, `-e`, or `--command` token: everything
/// before is rusty_term's own flags; everything after is the child command to
/// run (naner's `CustomShell.ExecutablePath` + `Arguments`) — `[program,
/// args...]`. Keeping the child's tokens out of the "our flags" scan means a
/// child argument that happens to read like one of our flags (even literally
/// `--config`) is never misread as ours.
fn split_command(args: &[String]) -> (&[String], Option<(String, Vec<String>)>) {
    match args
        .iter()
        .position(|a| a == "--" || a == "-e" || a == "--command")
    {
        Some(pos) => {
            let rest = &args[pos + 1..];
            let command = rest.split_first().map(|(p, a)| (p.clone(), a.to_vec()));
            (&args[..pos], command)
        }
        None => (args, None),
    }
}

/// Every flag rusty_term itself understands, paired with whether it consumes
/// a following argument (`--cwd /tmp` is two tokens; `--fullscreen` is one).
/// Used both for `--help`'s listing and for flagging typos below — kept as
/// one list so the two can't drift apart.
const KNOWN_FLAGS: &[(&str, bool)] = &[
    ("--help", false),
    ("-h", false),
    ("--version", false),
    ("--list-shells", false),
    ("--config", true),
    ("--cwd", true),
    ("--starting-directory", true),
    ("--title", true),
    ("--fullscreen", false),
    ("--maximized", false),
    ("--opacity", true),
    ("--profile", true),
    ("--session", true),
    ("--gui", false),
    ("--gpu", false),
    ("--single-instance", false),
    ("--setup", false),
];

fn print_help() {
    println!("rusty_term {}", env!("CARGO_PKG_VERSION"));
    println!("A terminal emulator written from scratch in Rust.");
    println!();
    println!("USAGE:");
    println!("    rusty_term [FLAGS] [-- <command> [args...]]");
    println!();
    println!("FLAGS:");
    println!("    -h, --help                    Print this help message and exit");
    println!("        --version                 Print the version and exit");
    println!("        --list-shells             Print detected shells and exit");
    println!("        --config <path>           Load config from this path");
    println!("        --cwd <dir>                Starting directory for the shell");
    println!("        --starting-directory <dir> Alias for --cwd");
    println!("        --title <title>           Window/tab title override");
    println!("        --fullscreen              Ignored since local mode retired");
    println!("        --maximized               Ignored since local mode retired");
    println!("        --opacity <0.0-1.0>       Ignored since local mode retired");
    println!("        --profile <name>          Apply a named [profile.<name>] bundle");
    println!("        --session <path>          Ignored since local mode retired");
    println!("        --gui                     Open the window: runs rusty-mux-gui, passing");
    println!("                                  --gpu, --setup, --config, --profile, --cwd");
    println!("                                  and the command");
    println!("        --gpu                     Use the wgpu GPU renderer (with --gui)");
    println!("        --single-instance         Ignored since local mode retired");
    println!("        --setup                   Open first-run setup (with --gui)");
    println!();
    println!("Anything after `--`, `-e`, or `--command` is run as the child command,");
    println!("replacing the configured shell, e.g. `rusty_term -- htop`.");
}

/// Every `--`-prefixed token in `args` that isn't in [`KNOWN_FLAGS`], skipping
/// the value token of flags that consume one. Pulled out as a pure function
/// (rather than inlined into an eprintln loop) so the scan logic — which flag
/// takes a value, `--flag=value` vs. `--flag value` — is unit-testable
/// without capturing stderr.
fn unrecognized_flags(args: &[String]) -> Vec<&str> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if let Some(name) = a.split('=').next().filter(|_| a.starts_with("--")) {
            match KNOWN_FLAGS.iter().find(|(f, _)| *f == name) {
                Some((_, takes_value)) => {
                    // `--flag=value` already carries its value in this token;
                    // only a separate `--flag value` pair consumes the next one.
                    if *takes_value && !a.contains('=') {
                        i += 1;
                    }
                }
                None => out.push(a.as_str()),
            }
        }
        i += 1;
    }
    out
}

/// Warn (to stderr) about any unrecognized flag. Typos like `--fullscren`
/// would otherwise be silently ignored rather than doing what the user
/// asked.
fn warn_unrecognized_flags(args: &[String]) {
    for a in unrecognized_flags(args) {
        eprintln!("rusty_term: unrecognized flag `{a}` (ignored)");
    }
}

/// Layer every CLI-only override onto the config file's `Config`: `--cwd`/
/// `--starting-directory` (G1, first match wins), `--title` (G3),
/// `--maximized`/`--fullscreen` (G6, fullscreen taking priority if both are
/// given), `--opacity`, `--profile`, `--session`, and the trailing `command`
/// override (`-- prog arg...` / `-e` / `--command`) — which replaces `shell`
/// outright rather than merely supplying args, so it wins over both the
/// config file and shell auto-detection.
///
/// Pulled out as a pure function (rather than inlined into `main`'s
/// eprintln-as-you-go style) so this whole layering pass — order of
/// precedence included — is unit-testable without a real process/config
/// file. Returns any warnings (bad `--opacity`, unknown `--profile`) instead
/// of printing them directly, for the same reason.
fn apply_cli_overrides(
    config: &mut Config,
    args: &[String],
    command: Option<(String, Vec<String>)>,
) -> Vec<String> {
    let mut warnings = Vec::new();
    if let Some(dir) = cwd(args) {
        config.cwd = Some(PathBuf::from(dir));
    }
    if let Some(title) = flag_value(args, "--title") {
        config.title = Some(title.to_string());
    }
    if args.iter().any(|a| a == "--fullscreen") {
        config.launch_mode = Some(LaunchMode::Fullscreen);
    } else if args.iter().any(|a| a == "--maximized") {
        config.launch_mode = Some(LaunchMode::Maximized);
    }
    if let Some(raw) = flag_value(args, "--opacity") {
        match raw.parse::<f32>() {
            Ok(v) => config.opacity = Some(v.clamp(0.0, 1.0)),
            Err(_) => warnings.push(format!(
                "--opacity `{raw}` is not a number between 0.0 and 1.0; ignored"
            )),
        }
    }
    // `--profile <name>`: layer the named `[profile.<name>]` bundle onto the
    // top-level config, so both front-ends inherit its shell/cwd/theme.
    // `Config::apply_profile` is the single source of truth for this so a
    // live config reload can reapply the same override (see
    // `runtime::reactor::watch_config` / `gui::window::reload_config`)
    // instead of silently reverting it to the file's top-level defaults.
    if let Some(name) = flag_value(args, "--profile")
        && let Some(w) = config.apply_profile(name)
    {
        warnings.push(w);
    }
    if let Some(path) = flag_value(args, "--session") {
        config.session = Some(PathBuf::from(path));
    }
    if let Some((prog, cmd_args)) = command {
        config.shell = Some(prog);
        config.command_args = cmd_args;
    }
    warnings
}

/// The window's flags that `rusty-mux-gui` takes as rusty_term did.
const MUX_GUI_FLAGS: &[&str] = &["--gpu", "--setup", "--config", "--profile", "--cwd"];

/// `rusty-mux-gui`'s arguments for a `rusty_term --gui` launch, and a
/// warning for each flag it has no use for or that lacks its value.
/// `--cwd` or else `--starting-directory` becomes one `--cwd`, and of a
/// repeated flag the first counts, as for rusty_term's own window. Unknown
/// flags were warned about already, and words that aren't flags aren't
/// rusty_term's. The child command follows `--`.
fn mux_gui_args(
    args: &[String],
    command: Option<&(String, Vec<String>)>,
) -> (Vec<String>, Vec<String>) {
    let mut out = Vec::new();
    let mut warnings = Vec::new();
    let mut given: Vec<&str> = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let (name, inline) = match a.split_once('=') {
            Some((name, value)) if name.starts_with("--") => (name, Some(value)),
            _ => (a.as_str(), None),
        };
        let Some(&(name, takes_value)) = KNOWN_FLAGS.iter().find(|(f, _)| *f == name) else {
            continue;
        };
        let value = match (takes_value, inline) {
            (false, _) => None,
            (true, Some(value)) => Some(value.to_string()),
            (true, None) => match it.next() {
                Some(value) => Some(value.clone()),
                None => {
                    warnings.push(format!("`{name}` needs a value; ignored"));
                    continue;
                }
            },
        };
        let (flag, value) = match name {
            "--gui" => continue,
            "--cwd" | "--starting-directory" => ("--cwd", cwd(args).map(str::to_string)),
            _ => (name, value),
        };
        if !MUX_GUI_FLAGS.contains(&flag) {
            warnings.push(format!("`{a}` does not apply to the mux window; ignored"));
            continue;
        }
        if given.contains(&flag) {
            continue;
        }
        given.push(flag);
        out.push(flag.to_string());
        out.extend(value);
    }
    if let Some((program, rest)) = command {
        out.push("--".into());
        out.push(program.clone());
        out.extend(rest.iter().cloned());
    }
    (out, warnings)
}

/// The folder `--cwd` or else `--starting-directory` names (G1).
fn cwd(args: &[String]) -> Option<&str> {
    flag_value(args, "--cwd").or_else(|| flag_value(args, "--starting-directory"))
}

/// Whether `path` is a file this user may run: on Unix one with an execute
/// bit, so a copy that isn't falls back to `PATH`.
fn runnable(path: &std::path::Path) -> bool {
    let Ok(meta) = path.metadata() else {
        return false;
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.is_file() && meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        meta.is_file()
    }
}

/// Runs `rusty-mux-gui` for a `--gui` launch: the copy beside this program
/// if there is one, else the one on `PATH`. On Unix it replaces this
/// process; on Windows this waits for it and exits as it does.
fn mux_gui(args: &[String], command: Option<(String, Vec<String>)>) -> std::io::Result<()> {
    let (forward, warnings) = mux_gui_args(args, command.as_ref());
    for warning in warnings {
        eprintln!("rusty_term: {warning}");
    }
    let name = if cfg!(windows) {
        "rusty-mux-gui.exe"
    } else {
        "rusty-mux-gui"
    };
    let program = std::env::current_exe()
        .ok()
        .and_then(|exe| Some(exe.parent()?.join(name)))
        .filter(|path| runnable(path))
        .unwrap_or_else(|| PathBuf::from(name));
    let mut child = std::process::Command::new(&program);
    child.args(&forward);
    let failed = |e: std::io::Error| {
        std::io::Error::new(e.kind(), format!("starting {}: {e}", program.display()))
    };
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        Err(failed(child.exec()))
    }
    #[cfg(not(unix))]
    {
        let status = child.status().map_err(failed)?;
        std::process::exit(status.code().unwrap_or(1));
    }
}

fn main() -> Result<(), std::io::Error> {
    // Load the config file (`--config <path>` > `$RUSTY_TERM_CONFIG` > the
    // platform default location). Warnings (unknown keys, bad values, an
    // unreadable explicit path) go to stderr — visible with the TUI not yet
    // drawing and harmless behind a detached gui window — and never abort:
    // the terminal always starts, with defaults filling any gap.
    let all_args: Vec<String> = std::env::args().skip(1).collect();
    let (args, command) = split_command(&all_args);

    // `--help`/`-h`/`--version`: the usual "answer and exit before touching
    // anything else" diagnostics, ahead of config loading and `ctl` dispatch
    // so they work even with a broken config or no running instance.
    if args.iter().any(|a| a == "--help" || a == "-h") {
        print_help();
        return Ok(());
    }
    if args.iter().any(|a| a == "--version") {
        println!("rusty_term {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    // `--list-shells`: print what's installed and exit. Runs before config
    // loading so a broken config can't get in the way of the diagnostic.
    if args.iter().any(|a| a == "--list-shells") {
        shells::print_detected();
        return Ok(());
    }

    // `rusty_term ctl`: retired with the GUI's local mode (D159).
    if args.first().map(String::as_str) == Some("ctl") {
        retired_ctl();
    }

    // Typo'd or unknown flags (`--fullscren`, `--tittle foo`) would otherwise
    // be silently swallowed as no-ops; flag them so the user notices before
    // wondering why the option "didn't work".
    warn_unrecognized_flags(args);

    // `--gui`: the window is a rusty_mux client now (term ROADMAP question
    // 1; handoff D146), so hand the launch to `rusty-mux-gui`, which starts
    // a server when none runs.
    if args.iter().any(|a| a == "--gui") {
        return mux_gui(args, command);
    }

    let (mut config, warnings) = Config::load(args);
    for w in &warnings {
        eprintln!("rusty_term: {w}");
    }

    // CLI-only overrides layered on top of the config file: `--cwd` /
    // `--starting-directory` (G1), `--title` (G3), `--maximized` /
    // `--fullscreen` (G6, fullscreen taking priority if both are given), and
    // the trailing `-- prog arg...` / `-e` / `--command` child override (G2)
    // — which replaces `shell` outright rather than merely supplying args, so
    // it wins over both the config file and shell auto-detection below.
    for w in apply_cli_overrides(&mut config, args, command) {
        eprintln!("rusty_term: {w}");
    }

    // No shell configured: ask the detector for a better default than the
    // backend's last resort. On Windows that upgrades cmd.exe to PowerShell
    // when one is installed; on Unix it stays `None` ($SHELL already wins).
    // An explicitly set $COMSPEC is honored over the probe — someone who set
    // it chose their shell, same as $SHELL on Unix.
    if config.shell.is_none() {
        #[cfg(windows)]
        let env_choice = std::env::var("COMSPEC")
            .map(|c| !c.to_ascii_lowercase().ends_with("cmd.exe"))
            .unwrap_or(false);
        #[cfg(not(windows))]
        let env_choice = false;
        if !env_choice {
            config.shell = shells::detect_default();
        }
    }

    #[cfg(unix)]
    let backend: Box<dyn Backend> = Box::new(UnixBackend);
    // `[console] host` picks who serves the shell's console; `own` failing
    // to start is shown, never swapped for ConPTY.
    #[cfg(windows)]
    let backend: Box<dyn Backend> = match config.console_host.unwrap_or_default() {
        config::ConsoleHost::ConPty => Box::new(WindowsBackend),
        config::ConsoleHost::Own => Box::new(OwnBackend::new()),
    };

    // Session files retired with the GUI's local mode (D159).
    if config.session.is_some() {
        eprintln!("rusty_term: `session` is ignored since local mode retired");
    }

    // Start at the host terminal's actual size, falling back to 80x24. The
    // configured cols/rows are a *window* size; the TUI renders into the host
    // terminal, whose size is not ours to choose.
    let (init_cols, init_rows) = backend.terminal_size().unwrap_or((80, 24));
    let grid = Arc::new(Mutex::new(tui_grid(&config, init_cols, init_rows)));

    // The child renders through rusty_term, not the host terminal, so it should
    // see *our* identity and capabilities. We advertise our self-describing
    // `rusty_term` terminfo entry when it's installed, else the near-universal
    // `xterm-256color` (the repertoire we implement), plus truecolor (the
    // renderer always emits 24-bit SGR). Set before spawning so the child
    // inherits it. We're still single-threaded here, so this process-wide env
    // mutation races with nothing.
    unsafe {
        std::env::set_var("TERM", term::resolve_term());
        std::env::set_var("COLORTERM", "truecolor");
    }

    // Hand off to the rusty_tokio runtime — a single async reactor driving the grid
    // and backend (Unix via AsyncFd, Windows by bridging ConPTY's blocking pipes).
    // `run` returns the child shell's own exit status; `main`'s `Result`
    // return only ever maps to 0 or 1, so mirror the child's code exactly via
    // an explicit `process::exit` instead of just propagating the `Result`.
    let code = runtime::run(backend, grid, init_cols, init_rows, config)?;
    std::process::exit(code);
}

/// `rusty_term ctl`: retired with the GUI's local mode (pack D159). The
/// mux has its own control tool, `rusty-mux-ctl`.
/// Says `ctl` is retired and exits 1, with no error dump after it.
fn retired_ctl() -> ! {
    eprintln!("rusty_term: `ctl` was retired with the GUI's local mode; use `rusty-mux-ctl`");
    std::process::exit(1)
}

/// The TUI's grid at `cols` x `rows` with the config's pane defaults:
/// scrollback cap, theme, cursor, and mode 2027 when `grapheme_clustering`
/// is on (set through the parser, so DECRQM reports it and a program can
/// reset it).
fn tui_grid(config: &Config, cols: u16, rows: u16) -> Grid {
    let mut grid = Grid::new(cols as usize, rows as usize);
    if let Some(max) = config.scrollback {
        grid.set_scrollback_max(max);
    }
    grid.apply_theme(&config.theme);
    grid.set_default_cursor(
        config.cursor_style.unwrap_or_default(),
        config.cursor_blink.unwrap_or(false),
    );
    if config.grapheme_clustering == Some(true) {
        core::AnsiParser::new().advance(&mut grid, b"\x1b[?2027h");
    }
    grid
}

#[cfg(test)]
mod tests {
    use super::*;

    /// DECRQM's reply for mode 2027 after `bytes` on a TUI grid.
    fn mode_2027(config: &Config, bytes: &[u8]) -> Vec<u8> {
        let mut grid = tui_grid(config, 20, 2);
        let mut parser = core::AnsiParser::new();
        parser.advance(&mut grid, bytes);
        parser.advance(&mut grid, b"\x1b[?2027$p");
        parser.take_responses()
    }

    #[test]
    fn grapheme_clustering_key_starts_the_tui_grid_in_mode_2027() {
        let off = Config::default();
        assert_eq!(mode_2027(&off, b""), b"\x1b[?2027;2$y", "off by default");
        let on = Config {
            grapheme_clustering: Some(true),
            ..Config::default()
        };
        assert_eq!(mode_2027(&on, b""), b"\x1b[?2027;1$y");
        assert_eq!(
            mode_2027(&on, b"\x1b[?2027l"),
            b"\x1b[?2027;2$y",
            "a program resets it"
        );
        assert_eq!(
            mode_2027(&on, b"\x1bc"),
            b"\x1b[?2027;2$y",
            "RIS turns it off"
        );
    }

    fn strings(args: &[&str]) -> Vec<String> {
        args.iter().map(|a| a.to_string()).collect()
    }

    #[test]
    fn gui_launch_passes_the_mux_windows_flags() {
        let args = strings(&[
            "--gui",
            "--gpu",
            "--config=/c.toml",
            "--profile",
            "work",
            "--starting-directory",
            "/tmp",
            "--setup",
        ]);
        let command = ("htop".to_string(), vec!["-d".to_string()]);
        let (out, warnings) = mux_gui_args(&args, Some(&command));
        assert_eq!(
            out,
            strings(&[
                "--gpu",
                "--config",
                "/c.toml",
                "--profile",
                "work",
                "--cwd",
                "/tmp",
                "--setup",
                "--",
                "htop",
                "-d",
            ])
        );
        assert!(warnings.is_empty(), "{warnings:?}");
    }

    #[test]
    fn gui_launch_names_the_flags_it_drops() {
        let args = strings(&[
            "--gui",
            "--title",
            "x",
            "--fullscreen",
            "--opacity=0.5",
            "--fullscren",
            "word",
        ]);
        let (out, warnings) = mux_gui_args(&args, None);
        assert!(out.is_empty(), "{out:?}");
        assert_eq!(
            warnings,
            [
                "`--title` does not apply to the mux window; ignored",
                "`--fullscreen` does not apply to the mux window; ignored",
                "`--opacity=0.5` does not apply to the mux window; ignored",
            ]
        );
    }

    #[test]
    fn gui_launch_reads_repeats_and_missing_values_as_rusty_term_does() {
        let args = strings(&[
            "--starting-directory",
            "/b",
            "--cwd",
            "/a",
            "--profile=one",
            "--profile",
            "two",
            "--config",
        ]);
        let (out, warnings) = mux_gui_args(&args, None);
        assert_eq!(out, strings(&["--cwd", "/a", "--profile", "one"]));
        assert_eq!(warnings, ["`--config` needs a value; ignored"]);
    }

    #[cfg(unix)]
    #[test]
    fn only_a_file_with_an_execute_bit_is_runnable() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("rt-runnable-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        let file = dir.join("rusty-mux-gui");
        std::fs::write(&file, b"").expect("write");
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).expect("chmod");
        assert!(!runnable(&file));
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        assert!(runnable(&file));
        assert!(!runnable(&dir));
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn s(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn all_known_flags_pass_through_unrecognized_flags_cleanly() {
        let args = s(&[
            "--cwd",
            "/tmp",
            "--title",
            "hi",
            "--fullscreen",
            "--opacity",
            "0.5",
            "--profile",
            "dev",
            "--session",
            "/x",
            "--gui",
            "--gpu",
            "--single-instance",
            "--setup",
            "--list-shells",
            "--help",
            "-h",
            "--version",
        ]);
        assert!(unrecognized_flags(&args).is_empty());
    }

    #[test]
    fn typo_d_flag_is_reported() {
        let args = s(&["--fullscren"]);
        assert_eq!(unrecognized_flags(&args), vec!["--fullscren"]);
    }

    #[test]
    fn value_token_of_a_known_flag_is_not_mistaken_for_a_flag() {
        // `--cwd` consumes the next token; `--title` looks like a flag but is
        // really the *value* of `--cwd` here, so it must not be misdetected as
        // an unknown flag in its own right nor its own value skipped.
        let args = s(&["--cwd", "--title", "--title", "hi"]);
        assert!(unrecognized_flags(&args).is_empty());
    }

    #[test]
    fn equals_form_does_not_swallow_the_next_token() {
        // `--opacity=0.5` carries its value inline, so the following
        // `--fullscreen` is a separate flag, not a consumed value.
        let args = s(&["--opacity=0.5", "--fullscreen"]);
        assert!(unrecognized_flags(&args).is_empty());
    }

    #[test]
    fn multiple_typos_are_all_reported_in_order() {
        let args = s(&["--fulscreen", "--cwd", "/tmp", "--tittle", "x"]);
        assert_eq!(unrecognized_flags(&args), vec!["--fulscreen", "--tittle"]);
    }

    #[test]
    fn split_command_still_hides_child_args_from_the_flag_scan() {
        // A child command that happens to look like our flags (even
        // literally `--config`) must never reach the "our flags" scan.
        let all = s(&["--fullscreen", "--", "--config", "--bogus"]);
        let (ours, command) = split_command(&all);
        assert!(unrecognized_flags(ours).is_empty());
        assert_eq!(command, Some(("--config".to_string(), s(&["--bogus"]))));
    }

    #[test]
    fn cli_overrides_cwd_title_and_opacity() {
        let args = s(&["--cwd", "/work", "--title", "hi", "--opacity", "0.5"]);
        let mut config = Config::default();
        let warnings = apply_cli_overrides(&mut config, &args, None);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(config.cwd, Some(PathBuf::from("/work")));
        assert_eq!(config.title.as_deref(), Some("hi"));
        assert_eq!(config.opacity, Some(0.5));
    }

    #[test]
    fn cli_overrides_prefers_cwd_over_starting_directory() {
        let args = s(&["--cwd", "/a", "--starting-directory", "/b"]);
        let mut config = Config::default();
        apply_cli_overrides(&mut config, &args, None);
        assert_eq!(config.cwd, Some(PathBuf::from("/a")));
    }

    #[test]
    fn cli_overrides_falls_back_to_starting_directory_when_cwd_is_absent() {
        let args = s(&["--starting-directory", "/b"]);
        let mut config = Config::default();
        apply_cli_overrides(&mut config, &args, None);
        assert_eq!(config.cwd, Some(PathBuf::from("/b")));
    }

    #[test]
    fn cli_overrides_fullscreen_wins_over_maximized_when_both_given() {
        let args = s(&["--maximized", "--fullscreen"]);
        let mut config = Config::default();
        apply_cli_overrides(&mut config, &args, None);
        assert_eq!(config.launch_mode, Some(LaunchMode::Fullscreen));
    }

    #[test]
    fn cli_overrides_maximized_alone_applies() {
        let args = s(&["--maximized"]);
        let mut config = Config::default();
        apply_cli_overrides(&mut config, &args, None);
        assert_eq!(config.launch_mode, Some(LaunchMode::Maximized));
    }

    #[test]
    fn cli_overrides_invalid_opacity_warns_and_leaves_opacity_unset() {
        let args = s(&["--opacity", "not-a-number"]);
        let mut config = Config::default();
        let warnings = apply_cli_overrides(&mut config, &args, None);
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("--opacity"), "{warnings:?}");
        assert_eq!(config.opacity, None);
    }

    #[test]
    fn cli_overrides_opacity_is_clamped_to_the_unit_range() {
        let args = s(&["--opacity", "5.0"]);
        let mut config = Config::default();
        assert!(apply_cli_overrides(&mut config, &args, None).is_empty());
        assert_eq!(config.opacity, Some(1.0));
    }

    #[test]
    fn cli_overrides_unknown_profile_warns_and_leaves_config_untouched() {
        let args = s(&["--profile", "nope"]);
        let mut config = Config::default();
        let warnings = apply_cli_overrides(&mut config, &args, None);
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("nope"), "{warnings:?}");
    }

    #[test]
    fn cli_overrides_session_sets_the_session_path() {
        let args = s(&["--session", "/tabs.toml"]);
        let mut config = Config::default();
        apply_cli_overrides(&mut config, &args, None);
        assert_eq!(config.session, Some(PathBuf::from("/tabs.toml")));
    }

    #[test]
    fn cli_overrides_trailing_command_replaces_shell_and_args() {
        let mut config = Config {
            shell: Some("/bin/bash".to_string()),
            ..Default::default()
        };
        let command = Some(("htop".to_string(), s(&["-d", "10"])));
        apply_cli_overrides(&mut config, &[], command);
        assert_eq!(config.shell.as_deref(), Some("htop"));
        assert_eq!(config.command_args, s(&["-d", "10"]));
    }
}
