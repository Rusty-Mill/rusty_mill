//! `rusty-mux-gui [--gpu] [--config <path>] [--profile <name>] [--cwd <dir>]
//! [--setup] [<socket> [block]] [-- <program> [args...]]`: show a rusty_mux
//! block's window in rusty_term's native window, starting with that block
//! focused. Without a socket it uses `$RUSTY_MUX_SOCKET`, else the default
//! socket (C69, `rusty_mux_proto::endpoint`), starting a server there if
//! none answers (`rusty_mux_client::autostart`). Without a block id it
//! creates a session, window and block first; that block runs the program
//! after `--`, else the profile's or the config's `shell`, in `--cwd`, else
//! the profile's or the config's `cwd`. Splits and new windows run the same
//! shell. The window follows the server's layout and shared focus; typing
//! goes to the focused block, whose input lease this client holds.
//! rusty_term's config file sets fonts and colours; `--gpu` draws with the
//! GPU renderer, and `--setup` opens setup, as they do for rusty_term.

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::ExitCode;

const USAGE: &str = "usage: rusty-mux-gui [--gpu] [--config <path>] [--profile <name>] \
                     [--cwd <dir>] [--setup] [<socket> [block]] [-- <program> [args...]]";

fn main() -> ExitCode {
    let args = match Args::parse(std::env::args_os().skip(1)) {
        Ok(args) => args,
        Err(err) => {
            eprintln!("rusty-mux-gui: {err}\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    let socket = match args.socket.clone() {
        Some(socket) => PathBuf::from(socket),
        None => match default_socket() {
            Ok(socket) => socket,
            Err(err) => {
                eprintln!("rusty-mux-gui: {err}");
                return ExitCode::from(2);
            }
        },
    };
    match run(socket, &args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("rusty-mux-gui: {err}");
            ExitCode::FAILURE
        }
    }
}

/// The command line, as `rusty-mux-gui` reads it. `--gpu`, `--config`
/// and `--setup` are read again by rusty_term's window from the process's
/// arguments; they are checked here so a typo is an error.
#[derive(Debug, Default, PartialEq, Eq)]
struct Args {
    socket: Option<OsString>,
    block: Option<OsString>,
    profile: Option<String>,
    cwd: Option<PathBuf>,
    /// The program after `--` and its arguments.
    command: Option<(String, Vec<String>)>,
}

impl Args {
    fn parse(raw: impl IntoIterator<Item = OsString>) -> Result<Args, String> {
        let mut args = Args::default();
        let mut raw = raw.into_iter();
        let mut positional = Vec::new();
        while let Some(arg) = raw.next() {
            let Some(text) = arg.to_str() else {
                positional.push(arg);
                continue;
            };
            let (flag, inline) = match text.split_once('=') {
                Some((flag, v)) if flag.starts_with("--") => (flag, Some(OsString::from(v))),
                _ => (text, None),
            };
            match flag {
                "--" if inline.is_none() => {
                    args.command = Some(command(raw.by_ref())?);
                    break;
                }
                "--gpu" | "--setup" if inline.is_none() => {}
                "--config" => {
                    value(inline, &mut raw, "--config")?;
                }
                "--profile" => args.profile = Some(utf8(value(inline, &mut raw, "--profile")?)?),
                "--cwd" => args.cwd = Some(PathBuf::from(value(inline, &mut raw, "--cwd")?)),
                _ if text.starts_with('-') => return Err(format!("unknown option `{text}`")),
                _ => positional.push(arg),
            }
        }
        let mut positional = positional.into_iter();
        args.socket = positional.next();
        args.block = positional.next();
        if positional.next().is_some() {
            return Err("too many arguments".into());
        }
        if args.block.is_some() && args.command.is_some() {
            return Err(
                "a program after `--` runs only in a new session, not in a given block".into(),
            );
        }
        Ok(args)
    }
}

/// `flag`'s value: the one after its `=`, else the next argument.
fn value(
    inline: Option<OsString>,
    raw: &mut impl Iterator<Item = OsString>,
    flag: &str,
) -> Result<OsString, String> {
    inline
        .or_else(|| raw.next())
        .ok_or_else(|| format!("`{flag}` needs a value"))
}

fn utf8(arg: OsString) -> Result<String, String> {
    arg.into_string()
        .map_err(|arg| format!("`{}` is not UTF-8", arg.to_string_lossy()))
}

/// The program and arguments after `--`; nothing after it is an error.
fn command(rest: impl Iterator<Item = OsString>) -> Result<(String, Vec<String>), String> {
    let mut rest = rest.map(utf8);
    let program = rest.next().ok_or("`--` needs a program")??;
    let args = rest.collect::<Result<Vec<_>, _>>()?;
    Ok((program, args))
}

/// The socket when none is named: `$RUSTY_MUX_SOCKET`, else the default,
/// on which a server is started if nothing answers (C69).
fn default_socket() -> Result<PathBuf, String> {
    use rusty_mux_proto::endpoint::{Origin, resolve};
    let (socket, origin) = resolve(None, |name| std::env::var_os(name))?;
    #[cfg(any(target_os = "linux", windows))]
    if origin == Origin::Default {
        rusty_mux_client::autostart::ensure(&socket)?;
    }
    #[cfg(not(any(target_os = "linux", windows)))]
    let _ = (origin, Origin::Default);
    Ok(socket)
}

/// What new blocks run: the command line over the profile over the config
/// file, as rusty_term's local mode chose its shells.
fn start(
    args: &Args,
    config: &rusty_term::config::Config,
) -> Result<rusty_mux_gui::source::Start, String> {
    use rusty_mux_proto::{Launch, Program};
    let shell = config
        .shell
        .as_deref()
        .and_then(rusty_term::backend::shell_program)
        .map(|(command, args)| Program { command, args });
    let program = match &args.command {
        Some((command, args)) => Some(Program {
            command: command.clone(),
            args: args.clone(),
        }),
        None => shell.clone(),
    };
    let cwd = match args.cwd.as_ref().or(config.cwd.as_ref()) {
        Some(dir) => Some(
            std::path::absolute(dir)
                .map_err(|e| format!("{}: {e}", dir.display()))?
                .into_os_string()
                .into_string()
                .map_err(|dir| format!("{} is not UTF-8", dir.to_string_lossy()))?,
        ),
        None => None,
    };
    Ok(rusty_mux_gui::source::Start {
        first: Launch {
            program,
            cwd,
            ..Launch::default()
        },
        shell,
    })
}

#[cfg(any(target_os = "linux", windows))]
fn run(socket: PathBuf, args: &Args) -> Result<(), String> {
    use rusty_mux_core::BlockId;
    use rusty_mux_gui::source::attach_with;

    let block = match &args.block {
        Some(id) => {
            let id = id.to_str().ok_or("block id is not UTF-8")?;
            Some(BlockId::from_uuid(
                id.parse().map_err(|e| format!("block id: {e}"))?,
            ))
        }
        None => None,
    };
    // rusty_term reads `--config` from these, before `--`, as its window
    // does.
    let process: Vec<String> = std::env::args_os()
        .skip(1)
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    let own = rusty_term::config::own_args(&process);
    let (mut config, warnings) = rusty_term::config::Config::load(own);
    for w in &warnings {
        eprintln!("rusty-mux-gui: {w}");
    }
    if config.grapheme_clustering == Some(true) {
        eprintln!(
            "rusty-mux-gui: `grapheme_clustering` does not reach mux blocks; set \
             \"grapheme_clustering\": true in the mux config's \"server\" section"
        );
    }
    if let Some(warning) = args
        .profile
        .as_deref()
        .and_then(|p| config.apply_profile(p))
    {
        eprintln!("rusty-mux-gui: {warning}");
    }
    let attached = attach_with(&socket, block, start(args, &config)?)?;
    let events = attached.events;
    rusty_term::gui::run_attached(
        &attached.source,
        attached.first,
        attached.focus,
        &config,
        |remote| events.follow(remote),
    )
    .map_err(|e| e.to_string())
}

#[cfg(not(any(target_os = "linux", windows)))]
fn run(_socket: PathBuf, _args: &Args) -> Result<(), String> {
    Err("the client runs on Linux and Windows only".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Args, String> {
        Args::parse(args.iter().map(OsString::from))
    }

    #[test]
    fn no_arguments_is_the_default_socket() {
        assert_eq!(parse(&[]), Ok(Args::default()));
        assert_eq!(parse(&["--gpu", "--setup"]), Ok(Args::default()));
    }

    #[test]
    fn a_socket_block_or_command_are_read() {
        let args = parse(&[
            "--cwd",
            "/tmp",
            "--profile",
            "work",
            "s.sock",
            "--",
            "htop",
            "-d",
            "5",
        ])
        .expect("parsed");
        assert_eq!(args.socket.as_deref(), Some("s.sock".as_ref()));
        assert_eq!(args.block, None);
        assert_eq!(args.cwd, Some(PathBuf::from("/tmp")));
        assert_eq!(args.profile.as_deref(), Some("work"));
        assert_eq!(
            args.command,
            Some(("htop".into(), vec!["-d".into(), "5".into()]))
        );
        let args = parse(&["--cwd", "/tmp", "s.sock", "b"]).expect("parsed");
        assert_eq!(args.block.as_deref(), Some("b".as_ref()));
    }

    #[test]
    fn a_flag_after_the_separator_is_the_programs() {
        let args = parse(&["--", "ls", "--cwd"]).expect("parsed");
        assert_eq!(args.cwd, None);
        assert_eq!(args.command, Some(("ls".into(), vec!["--cwd".into()])));
    }

    #[test]
    fn bad_command_lines_are_errors() {
        assert!(parse(&["a", "b", "c"]).is_err());
        assert!(parse(&["--cwd"]).is_err());
        assert!(parse(&["--"]).is_err());
        assert!(parse(&["--fullscreen"]).is_err());
        assert!(parse(&["s.sock", "b", "--", "htop"]).is_err());
    }

    #[test]
    fn values_may_follow_an_equals_sign() {
        let args = parse(&["--cwd=/tmp", "--profile=work", "--config=c.toml"]).expect("parsed");
        assert_eq!(args.cwd, Some(PathBuf::from("/tmp")));
        assert_eq!(args.profile.as_deref(), Some("work"));
        assert!(parse(&["--gpu=yes"]).is_err());
    }

    #[test]
    fn a_shell_with_arguments_is_split_into_words() {
        let config = rusty_term::config::Config {
            shell: Some("bash --login -i".into()),
            ..Default::default()
        };
        let made = start(&Args::default(), &config).expect("start");
        let shell = made.shell.expect("shell");
        assert_eq!(shell.command, "bash");
        assert_eq!(shell.args, ["--login", "-i"]);
        assert_eq!(made.first.program, Some(shell));
    }

    #[test]
    fn the_command_line_wins_over_the_config() {
        let config = rusty_term::config::Config {
            shell: Some("fish".into()),
            cwd: Some(PathBuf::from("/srv")),
            ..Default::default()
        };
        let args = parse(&["--cwd", "/tmp", "--", "htop"]).expect("parsed");
        let made = start(&args, &config).expect("start");
        let first = made.first.program.expect("program");
        assert_eq!(first.command, "htop");
        assert_eq!(made.first.cwd, absolute("/tmp"));
        assert_eq!(made.shell.map(|p| p.command), Some("fish".into()));

        let made = start(&Args::default(), &config).expect("start");
        assert_eq!(made.first.program.map(|p| p.command), Some("fish".into()));
        assert_eq!(made.first.cwd, absolute("/srv"));
    }

    /// `path` as `start` makes it absolute: on Windows `/tmp` gains the
    /// current drive.
    fn absolute(path: &str) -> Option<String> {
        let made = std::path::absolute(path).expect("absolute");
        Some(made.to_str().expect("UTF-8").to_owned())
    }
}
