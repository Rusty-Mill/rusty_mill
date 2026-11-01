//! `rusty-mux-server [--udp ADDR[:PORT]] [--udp-idle SECS] [--config FILE]
//! <socket>`: serves rusty_mux on a Unix socket. Each new block runs
//! `$SHELL` (or `/bin/sh`) in `$HOME` (or `/`), unless the config file's
//! `server` section names a command or directory. The file is `--config`,
//! else `$RUSTY_MUX_CONFIG`, else `$XDG_CONFIG_HOME/rusty_mux/config.json`
//! (or `~/.config/rusty_mux/config.json`); a missing file sets nothing and
//! a bad one stops the server before it starts. `reload-config` reads it
//! again, keeping what it had if the file has gone bad.
//!
//! With `--udp` the server also listens for UDP on `ADDR`, port 60001 unless
//! given, and offers each client an association there to move its
//! connections onto (docs/projects/mux/UDP-TRANSPORT.md). A silent
//! association, and its clients, last `--udp-idle` seconds (a day unless
//! given).
//!
//! `rusty-mux-server --login MAP [--runtime DIR] <socket>` runs login
//! mode's front door instead (Linux, normally as root): any local user may
//! connect, and `MAP` says which account each may use; each account gets a
//! server of its own under `DIR` (`/run/rusty-mux` unless given). SIGHUP
//! reads `MAP` again. The front door starts each account's server with
//! `--forwarded-by UID`, which serves a peer of that uid only after the
//! preface naming who it verified. See `login.rs`.
//!
//! On Windows (10 1809 or later, for ConPTY; AF_UNIX alone needs 1803)
//! each block runs `%COMSPEC%` (or `cmd.exe`) in `%USERPROFILE%` (or `C:\`)
//! on a pseudo console. Put the socket under `%LOCALAPPDATA%`, for example
//! `rusty-mux-server %LOCALAPPDATA%\rusty-mux.sock`; the server limits the
//! file to the current user. `RUSTY_CONSOLE_HOST=own` hosts the console
//! ourselves (rusty_conhost) instead of ConPTY; `conpty` or unset keeps
//! ConPTY, and any other value stops the server before it starts.
//! Ctrl+C or Ctrl+Break stops the server cleanly: it ends every block's
//! program and waits up to 6 s for the consoles it hosts to shut down,
//! which includes trying to end the other programs still attached.
//! Closing its console window, or killing it, does not wait.

use std::ffi::OsString;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use rusty_mux_server::{Config, ConsoleHost, Udp, serve_until};
use rusty_tokio::io::{UdpSocket, UnixListener};
use rusty_tokio::{Builder, LocalOptions};

const USAGE: &str =
    "usage: rusty-mux-server [--udp ADDR[:PORT]] [--udp-idle SECS] [--config FILE] <socket>
       rusty-mux-server --login MAP [--runtime DIR] <socket>";
/// Where login mode puts each account's server unless `--runtime` says.
const RUNTIME: &str = "/run/rusty-mux";
/// The UDP port when `--udp` names an address only.
const UDP_PORT: u16 = 60001;
/// How long a silent association lasts unless `--udp-idle` says.
const UDP_IDLE: Duration = Duration::from_secs(24 * 60 * 60);

/// The command line, checked.
#[derive(Debug, PartialEq, Eq)]
struct Args {
    socket: PathBuf,
    udp: Option<SocketAddr>,
    idle: Duration,
    /// `--config`, if given.
    config: Option<PathBuf>,
    mode: Mode,
}

/// Which server this is.
#[derive(Debug, PartialEq, Eq)]
enum Mode {
    /// A user's own server: today's default.
    Personal,
    /// An account's server behind the front door, whose uid this is.
    Forwarded(u32),
    /// The front door: the map file and the runtime directory.
    Login { map: PathBuf, runtime: PathBuf },
}

fn main() -> ExitCode {
    let args = match parse(std::env::args_os().skip(1)) {
        Ok(args) => args,
        Err(err) => {
            eprintln!("rusty-mux-server: {err}\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    let console_host = match ConsoleHost::from_env() {
        Ok(host) => host,
        Err(err) => {
            eprintln!("rusty-mux-server: {err}");
            return ExitCode::from(2);
        }
    };
    match run(args, console_host) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("rusty-mux-server: {err}");
            ExitCode::FAILURE
        }
    }
}

fn parse(mut args: impl Iterator<Item = OsString>) -> Result<Args, String> {
    let (mut socket, mut udp, mut idle, mut config) = (None, None, UDP_IDLE, None);
    let (mut login, mut runtime, mut forwarded_by) = (None, None, None);
    while let Some(arg) = args.next() {
        let mut value = |flag: &str| {
            args.next()
                .and_then(|v| v.into_string().ok())
                .ok_or(format!("{flag} needs a value"))
        };
        match arg.to_str() {
            Some("--udp") => udp = Some(udp_addr(&value("--udp")?)?),
            Some("--config") => config = Some(PathBuf::from(value("--config")?)),
            Some("--login") => login = Some(PathBuf::from(value("--login")?)),
            Some("--runtime") => runtime = Some(PathBuf::from(value("--runtime")?)),
            Some("--forwarded-by") => {
                let uid = value("--forwarded-by")?;
                let parsed = uid
                    .parse()
                    .map_err(|_| format!("--forwarded-by {uid}: not a uid"))?;
                forwarded_by = Some(parsed);
            }
            Some("--udp-idle") => {
                let secs = value("--udp-idle")?;
                let secs = secs
                    .parse()
                    .ok()
                    .filter(|s| *s > 0)
                    .ok_or(format!("--udp-idle {secs}: not a whole number of seconds"))?;
                idle = Duration::from_secs(secs);
            }
            _ if socket.is_some() => return Err("one socket only".into()),
            _ => socket = Some(PathBuf::from(arg)),
        }
    }
    let socket = socket.ok_or("no socket given")?;
    let mode = match (login, runtime, forwarded_by) {
        (None, None, None) => Mode::Personal,
        (None, None, Some(_)) if udp.is_some() => {
            return Err("an account's server takes no --udp yet".into());
        }
        (None, None, Some(uid)) => Mode::Forwarded(uid),
        (None, Some(_), _) => return Err("--runtime needs --login".into()),
        (Some(_), _, Some(_)) => return Err("--login and --forwarded-by are two modes".into()),
        (Some(map), runtime, None) => {
            if udp.is_some() || config.is_some() {
                return Err("the front door takes no --udp or --config".into());
            }
            let runtime = runtime.unwrap_or_else(|| PathBuf::from(RUNTIME));
            Mode::Login { map, runtime }
        }
    };
    if mode != Mode::Personal && !cfg!(target_os = "linux") {
        return Err("login mode runs on Linux only".into());
    }
    Ok(Args {
        socket,
        udp,
        idle,
        config,
        mode,
    })
}

/// `ADDR:PORT`, or `ADDR` alone for [`UDP_PORT`]; an IPv6 address with a
/// port goes in brackets.
fn udp_addr(value: &str) -> Result<SocketAddr, String> {
    value
        .parse::<SocketAddr>()
        .or_else(|_| {
            value
                .parse::<IpAddr>()
                .map(|ip| SocketAddr::new(ip, UDP_PORT))
        })
        .map_err(|_| format!("--udp {value}: not an address"))
}

fn run(args: Args, console_host: ConsoleHost) -> std::io::Result<()> {
    #[cfg(target_os = "linux")]
    if let Mode::Login { map, runtime } = args.mode {
        return run_login(args.socket, map, runtime);
    }
    let config_file = args
        .config
        .or_else(|| rusty_mux_script::default_path(|name| std::env::var_os(name)));
    let config = default_config(console_host, config_file);
    let socket = args.socket;
    let runtime = Builder::new_current_thread().build_local(&LocalOptions::default())?;
    runtime.block_on(async {
        let listener = UnixListener::bind(&socket)?;
        // No authentication in Phase 1: only this user may connect.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600))?;
        }
        // The Windows form of 0600: one entry, full control for this user.
        #[cfg(windows)]
        platform_windows::fs::restrict_to_owner(&socket).map_err(|e| {
            std::io::Error::other(format!("restricting {}: {e:?}", socket.display()))
        })?;
        let udp = match args.udp {
            Some(addr) => Some(Udp {
                socket: UdpSocket::bind(addr).await?,
                idle: args.idle,
            }),
            None => None,
        };
        match args.mode {
            #[cfg(target_os = "linux")]
            Mode::Forwarded(uid) => {
                rusty_mux_server::serve_forwarded(listener, config, uid, stop_requested()?).await
            }
            _ => serve_until(listener, config, udp, stop_requested()?).await,
        }
    })
}

/// The front door: any local user may connect, so the socket is open to
/// all; the map decides who gets in. SIGHUP reads the map again.
#[cfg(target_os = "linux")]
fn run_login(socket: PathBuf, map: PathBuf, runtime: PathBuf) -> std::io::Result<()> {
    use rusty_tokio::signal::unix::{SignalKind, signal};

    // Before the runtime starts a thread: binding sets the umask.
    let listener = rusty_mux_server::bind_front_door(&socket)?;
    let server = std::env::current_exe()?;
    let login = rusty_mux_server::Login {
        map,
        runtime,
        server,
        limits: rusty_mux_server::Limits::default(),
    };
    let runtime = Builder::new_current_thread().build_local(&LocalOptions::default())?;
    runtime.block_on(async {
        let mut hangups = signal(SignalKind::hangup())?;
        let listener = UnixListener::from_std(listener)?;
        let (reload, reloads) = rusty_tokio::sync::mpsc::unbounded_channel();
        rusty_tokio::task::spawn_local(async move {
            while hangups.recv().await.is_some() {
                // The answer is logged by the front door; nobody waits here.
                let (answer, _) = rusty_tokio::sync::oneshot::channel();
                if reload.send(answer).is_err() {
                    break;
                }
            }
        });
        rusty_mux_server::serve_login(listener, login, reloads).await
    })
}

/// Resolves on Ctrl+C or Ctrl+Break, the server's clean stops on Windows.
/// The Ctrl+Break listener is installed now, so a failure stops the server
/// from starting; Ctrl+C's shares its handler.
#[cfg(windows)]
fn stop_requested() -> std::io::Result<impl Future<Output = ()>> {
    let mut ctrl_break = rusty_tokio::signal::windows::ctrl_break()?;
    Ok(async move {
        let ctrl_c = async {
            if let Err(err) = rusty_tokio::signal::ctrl_c().await {
                eprintln!("rusty-mux-server: listening for Ctrl+C: {err}");
                std::future::pending::<()>().await;
            }
        };
        rusty_tokio::select! {
            () = ctrl_c => {}
            _ = ctrl_break.recv() => {}
        }
    })
}

/// Never: on Linux a signal ends the server as before, and the kernel
/// hangs up every block's PTY with it.
#[cfg(not(windows))]
fn stop_requested() -> std::io::Result<impl Future<Output = ()>> {
    Ok(std::future::pending())
}

#[cfg(not(windows))]
fn default_config(console_host: ConsoleHost, config_file: Option<PathBuf>) -> Config {
    Config {
        program: std::env::var_os("SHELL").unwrap_or_else(|| OsString::from("/bin/sh")),
        args: Vec::new(),
        cwd: std::env::var_os("HOME").unwrap_or_else(|| OsString::from("/")),
        console_host,
        config_file,
    }
}

#[cfg(windows)]
fn default_config(console_host: ConsoleHost, config_file: Option<PathBuf>) -> Config {
    Config {
        program: std::env::var_os("COMSPEC").unwrap_or_else(|| OsString::from("cmd.exe")),
        args: Vec::new(),
        cwd: std::env::var_os("USERPROFILE").unwrap_or_else(|| OsString::from("C:\\")),
        console_host,
        config_file,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_str(args: &[&str]) -> Result<Args, String> {
        parse(args.iter().map(OsString::from))
    }

    #[test]
    fn udp_is_off_unless_asked_and_the_port_defaults() {
        let plain = parse_str(&["s.sock"]).unwrap();
        assert_eq!(plain.udp, None);
        assert_eq!(plain.idle, UDP_IDLE);
        let addr = parse_str(&["--udp", "0.0.0.0", "s.sock"]).unwrap();
        assert_eq!(addr.udp, Some("0.0.0.0:60001".parse().unwrap()));
        let v6 = parse_str(&["s.sock", "--udp", "[::1]:7000", "--udp-idle", "90"]).unwrap();
        assert_eq!(v6.udp, Some("[::1]:7000".parse().unwrap()));
        assert_eq!(v6.idle, Duration::from_secs(90));
        assert_eq!(v6.socket, PathBuf::from("s.sock"));
        assert_eq!(v6.config, None);
        let file = parse_str(&["--config", "c.json", "s.sock"]).unwrap();
        assert_eq!(file.config, Some(PathBuf::from("c.json")));
        assert_eq!(file.mode, Mode::Personal);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn login_mode_and_forwarding_are_asked_for() {
        let front = parse_str(&["--login", "/etc/m", "s.sock"]).unwrap();
        let expected = Mode::Login {
            map: "/etc/m".into(),
            runtime: RUNTIME.into(),
        };
        assert_eq!(front.mode, expected);
        let elsewhere = parse_str(&["--login", "m", "--runtime", "/tmp/r", "s"]).unwrap();
        let expected = Mode::Login {
            map: "m".into(),
            runtime: "/tmp/r".into(),
        };
        assert_eq!(elsewhere.mode, expected);
        let behind = parse_str(&["--forwarded-by", "0", "s"]).unwrap();
        assert_eq!(behind.mode, Mode::Forwarded(0));
    }

    #[test]
    fn a_bad_command_line_is_refused() {
        for bad in [
            &[][..],
            &["a", "b"],
            &["--udp"],
            &["--udp", "nowhere", "s"],
            &["--udp-idle", "0", "s"],
            &["--udp-idle", "1h", "s"],
            &["s", "--config"],
            &["--login", "m"],
            &["--runtime", "r", "s"],
            &["--login", "m", "--forwarded-by", "0", "s"],
            &["--login", "m", "--udp", "0.0.0.0", "s"],
            &["--login", "m", "--config", "c", "s"],
            &["--forwarded-by", "x", "s"],
            &["--forwarded-by", "-1", "s"],
            &["--forwarded-by", "0", "--udp", "0.0.0.0", "s"],
        ] {
            assert!(parse_str(bad).is_err(), "{bad:?}");
        }
    }
}
