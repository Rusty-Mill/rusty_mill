//! `rusty-mux-server [--udp ADDR[:PORT]] [--udp-idle SECS] [--config FILE]
//! [<socket>]`: serves rusty_mux on a Unix socket: the one named, else
//! `$RUSTY_MUX_SOCKET`, else the default, `$XDG_RUNTIME_DIR/rusty_mux/default.sock`
//! (`%LOCALAPPDATA%\rusty_mux\default.sock` on Windows), whose directory
//! the server makes private to the user (C69). A server never removes a
//! name it didn't make: if a server already answers there it refuses to
//! start, and anything there that is not a socket stays; a socket nothing
//! answers on is replaced. With `--idle-exit SECS` the server stops
//! cleanly once it has had no block and no client for SECS; it never stops
//! while a block lives. `rusty-mux` and `rusty-mux-gui` start a server this
//! way when nothing answers on the default socket.
//!
//! On Linux the server also takes its socket from systemd (socket
//! activation, `LISTEN_PID`/`LISTEN_FDS`), as the user units in
//! `dist/systemd/` set up; it then binds nothing and leaves the name to
//! systemd. The socket argument, if any, only names the state directory. Each new block runs
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
//! `rusty-mux-server --shared ALLOW [--config FILE] [--state DIR | --no-state]
//! <socket>` (Linux) is one server that also serves the users `ALLOW` lists
//! (C26 shared mode); its socket is open to every local user, and the list
//! decides. SIGHUP reads the list again. See `shared.rs`.
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
//! on a console we host ourselves (rusty_conhost). Put the socket under
//! `%LOCALAPPDATA%`, for example
//! `rusty-mux-server %LOCALAPPDATA%\rusty-mux.sock`; the server limits the
//! file to the current user. `RUSTY_CONSOLE_HOST=conpty` uses Windows'
//! ConPTY instead; `own` or unset keeps our host, and any other value
//! stops the server before it starts.
//! Ctrl+C or Ctrl+Break stops the server cleanly: it ends every block's
//! program and waits up to 6 s for the consoles it hosts to shut down,
//! which includes trying to end the other programs still attached.
//! Closing its console window, or killing it, does not wait.
//!
//! Sessions survive a restart (C45): every few seconds, and on a clean
//! stop, the server saves its sessions, windows, layouts and screens to
//! `--state DIR`, by default `$XDG_STATE_HOME/rusty_mux/<socket name>`
//! (or `~/.local/state/...`; `%LOCALAPPDATA%\rusty_mux\<socket name>` on
//! Windows), owner-only. The next server on that directory restores them,
//! each block with a fresh shell under its saved screen; the programs that
//! were running are not restarted. `--no-state` saves and restores nothing.

use std::ffi::OsString;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use rusty_mux_proto::endpoint;
use rusty_mux_server::{Config, ConsoleHost, Udp, serve_until, serve_until_idle};
use rusty_tokio::io::{UdpSocket, UnixListener};
use rusty_tokio::{Builder, LocalOptions};

const USAGE: &str = "usage: rusty-mux-server [--udp ADDR[:PORT]] [--udp-idle SECS] [--config FILE]
                        [--state DIR | --no-state] [--idle-exit SECS] [<socket>]
       rusty-mux-server --login MAP [--runtime DIR] <socket>
       rusty-mux-server --shared ALLOW [--tailnet ADDR:PORT] [--config FILE]
                        [--state DIR | --no-state] <socket>";
/// Where login mode puts each account's server unless `--runtime` says.
const RUNTIME: &str = "/run/rusty-mux";
/// The UDP port when `--udp` names an address only.
const UDP_PORT: u16 = 60001;
/// How long a silent association lasts unless `--udp-idle` says.
const UDP_IDLE: Duration = Duration::from_secs(24 * 60 * 60);

/// The command line, checked.
#[derive(Debug, PartialEq, Eq)]
struct Args {
    /// The socket named, if any (login mode requires one).
    socket: Option<PathBuf>,
    udp: Option<SocketAddr>,
    idle: Duration,
    /// `--config`, if given.
    config: Option<PathBuf>,
    state: StateDir,
    mode: Mode,
    /// `--idle-exit`, if given.
    idle_exit: Option<Duration>,
}

/// Where sessions are saved across restarts.
#[derive(Debug, PartialEq, Eq)]
enum StateDir {
    /// Under the user's state directory, named after the socket.
    Default,
    /// `--state DIR`.
    At(PathBuf),
    /// `--no-state`.
    Off,
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
    /// One server that also serves the users this allowlist names (C26),
    /// and the tailnet peers it names, on `--tailnet`'s address (C27).
    Shared(PathBuf, Option<SocketAddr>),
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
    let mut shared = None;
    let mut tailnet = None;
    let mut state = StateDir::Default;
    let mut idle_exit = None;
    while let Some(arg) = args.next() {
        let mut value = |flag: &str| {
            args.next()
                .and_then(|v| v.into_string().ok())
                .ok_or(format!("{flag} needs a value"))
        };
        match arg.to_str() {
            Some("--udp") => udp = Some(udp_addr(&value("--udp")?)?),
            Some("--config") => config = Some(PathBuf::from(value("--config")?)),
            Some("--state") | Some("--no-state") if state != StateDir::Default => {
                return Err("one of --state and --no-state, once".into());
            }
            Some("--state") => state = StateDir::At(PathBuf::from(value("--state")?)),
            Some("--no-state") => state = StateDir::Off,
            Some("--login") => login = Some(PathBuf::from(value("--login")?)),
            Some("--shared") => shared = Some(PathBuf::from(value("--shared")?)),
            Some("--tailnet") => tailnet = Some(tailnet_addr(&value("--tailnet")?)?),
            Some("--runtime") => runtime = Some(PathBuf::from(value("--runtime")?)),
            Some("--forwarded-by") => {
                let uid = value("--forwarded-by")?;
                let parsed = uid
                    .parse()
                    .map_err(|_| format!("--forwarded-by {uid}: not a uid"))?;
                forwarded_by = Some(parsed);
            }
            Some("--udp-idle") => idle = seconds("--udp-idle", &value("--udp-idle")?)?,
            Some("--idle-exit") => {
                idle_exit = Some(seconds("--idle-exit", &value("--idle-exit")?)?)
            }
            _ if socket.is_some() => return Err("one socket only".into()),
            _ => socket = Some(PathBuf::from(arg)),
        }
    }
    if let Some(allow) = shared {
        if login.is_some() || runtime.is_some() || forwarded_by.is_some() {
            return Err("--shared is a mode of its own".into());
        }
        if udp.is_some() || idle_exit.is_some() {
            return Err("shared mode takes no --udp or --idle-exit yet".into());
        }
        if socket.is_none() {
            return Err("shared mode needs a socket: other users must find it".into());
        }
        if !cfg!(target_os = "linux") {
            return Err("shared mode runs on Linux only".into());
        }
        return Ok(Args {
            socket,
            udp,
            idle,
            config,
            state,
            mode: Mode::Shared(allow, tailnet),
            idle_exit,
        });
    }
    if tailnet.is_some() {
        return Err("--tailnet needs --shared: the allowlist decides who is served".into());
    }
    let mode = match (login, runtime, forwarded_by) {
        (None, None, None) => Mode::Personal,
        (None, None, Some(_)) if udp.is_some() => {
            return Err("an account's server takes no --udp yet".into());
        }
        (None, None, Some(uid)) => Mode::Forwarded(uid),
        (None, Some(_), _) => return Err("--runtime needs --login".into()),
        (Some(_), _, Some(_)) => return Err("--login and --forwarded-by are two modes".into()),
        (Some(map), runtime, None) => {
            if udp.is_some() || config.is_some() || state != StateDir::Default {
                return Err("the front door takes no --udp, --config or --state".into());
            }
            let runtime = runtime.unwrap_or_else(|| PathBuf::from(RUNTIME));
            Mode::Login { map, runtime }
        }
    };
    if mode != Mode::Personal && idle_exit.is_some() {
        return Err("login mode takes no --idle-exit".into());
    }
    if mode != Mode::Personal && socket.is_none() {
        return Err("login mode needs a socket".into());
    }
    if mode != Mode::Personal && !cfg!(target_os = "linux") {
        return Err("login mode runs on Linux only".into());
    }
    Ok(Args {
        socket,
        udp,
        idle,
        config,
        state,
        mode,
        idle_exit,
    })
}

/// A whole, positive number of seconds for `flag`.
fn seconds(flag: &str, secs: &str) -> Result<Duration, String> {
    secs.parse()
        .ok()
        .filter(|s| *s > 0)
        .map(Duration::from_secs)
        .ok_or(format!("{flag} {secs}: not a whole number of seconds"))
}

/// `--tailnet`'s `ADDR:PORT`: the address must be a tailnet one, the only
/// kind whose peers `tailscaled` can name.
#[cfg(target_os = "linux")]
fn tailnet_addr(text: &str) -> Result<SocketAddr, String> {
    let addr: SocketAddr = text
        .parse()
        .map_err(|_| format!("--tailnet {text}: not ADDR:PORT"))?;
    match rusty_mux_server::tailnet::is_tailnet_addr(addr.ip()) {
        true => Ok(addr),
        false => Err(format!(
            "--tailnet {text}: not a tailnet address (100.64.0.0/10 or fd7a:115c:a1e0::/48)"
        )),
    }
}

#[cfg(not(target_os = "linux"))]
fn tailnet_addr(_text: &str) -> Result<SocketAddr, String> {
    Err("--tailnet runs on Linux only".into())
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
        let socket = args
            .socket
            .ok_or_else(|| std::io::Error::other("no socket"))?;
        return run_login(socket, map, runtime);
    }
    let env = |name: &str| std::env::var_os(name);
    let (socket, _) = endpoint::resolve(args.socket, env).map_err(std::io::Error::other)?;
    // A client starting a server names the default socket, so compare.
    if Some(&socket) == endpoint::default_socket(env).as_ref() {
        default_dir(&socket)?;
    }
    let config_file = args
        .config
        .or_else(|| rusty_mux_script::default_path(|name| std::env::var_os(name)));
    let mut config = default_config(console_host, config_file);
    config.state_dir = match args.state {
        StateDir::Off => None,
        StateDir::At(dir) => Some(dir),
        StateDir::Default => default_state_dir(&socket, |name| std::env::var_os(name)),
    };
    #[cfg(target_os = "linux")]
    if let Mode::Shared(allow, tailnet) = &args.mode {
        return run_shared(&socket, allow.clone(), *tailnet, config);
    }
    // Before the runtime starts a thread: binding sets the umask, and the
    // activation variables are read and removed. No authentication in
    // Phase 1: only this user may connect (0600).
    #[cfg(target_os = "linux")]
    let (listener, owned) = linux_listener(&socket)?;
    let runtime = Builder::new_current_thread().build_local(&LocalOptions::default())?;
    let served = runtime.block_on(async {
        #[cfg(target_os = "linux")]
        let listener = UnixListener::from_std(listener)?;
        // Windows' bind replaces only a socket nothing answers on.
        #[cfg(not(target_os = "linux"))]
        let listener = UnixListener::bind(&socket)?;
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
            _ => match args.idle_exit {
                Some(idle) => {
                    serve_until_idle(listener, config, udp, idle, stop_requested()?).await
                }
                None => serve_until(listener, config, udp, stop_requested()?).await,
            },
        }
    });
    // A clean stop takes the name away, if it is still this server's.
    #[cfg(target_os = "linux")]
    let released = owned.map_or(Ok(()), rusty_mux_server::endpoint::Owned::release);
    #[cfg(not(target_os = "linux"))]
    let released = Ok(());
    served.and(released)
}

/// The listener systemd passed, else `socket` bound under O6's rules with
/// the name this server owns. The activation variables are removed either
/// way, so no block's program inherits them.
#[cfg(target_os = "linux")]
fn linux_listener(
    socket: &std::path::Path,
) -> std::io::Result<(
    std::os::unix::net::UnixListener,
    Option<rusty_mux_server::endpoint::Owned>,
)> {
    use rusty_mux_server::endpoint;
    // SAFETY: called once, at start, before the runtime or any block
    // opens a descriptor, so fd 3 is still systemd's when it names us.
    let inherited = unsafe { endpoint::inherited(|name| std::env::var(name).ok()) };
    for name in ["LISTEN_PID", "LISTEN_FDS", "LISTEN_FDNAMES"] {
        // SAFETY: called before the runtime starts, so no other thread
        // reads the environment.
        unsafe { std::env::remove_var(name) };
    }
    if let Some(listener) = inherited? {
        return Ok((listener, None));
    }
    let (listener, owned) = endpoint::bind(socket, 0o177)?;
    Ok((listener, Some(owned)))
}

/// Makes the default socket's directory: private to this user on Linux
/// (checked like login mode's runtime directory), and under the user's own
/// `%LOCALAPPDATA%` on Windows.
fn default_dir(socket: &std::path::Path) -> std::io::Result<()> {
    let Some(dir) = socket.parent() else {
        return Ok(());
    };
    #[cfg(target_os = "linux")]
    return rusty_mux_server::endpoint::private_dir(dir);
    #[cfg(not(target_os = "linux"))]
    return std::fs::create_dir_all(dir);
}

/// `<state home>/rusty_mux/<socket file name>`, where the state home is
/// `$XDG_STATE_HOME` or `~/.local/state` (`%LOCALAPPDATA%` on Windows);
/// `None`, saving nothing, when neither is known.
fn default_state_dir(
    socket: &std::path::Path,
    env: impl Fn(&str) -> Option<OsString>,
) -> Option<PathBuf> {
    let set = |name: &str| env(name).filter(|v| !v.is_empty()).map(PathBuf::from);
    let home = if cfg!(windows) {
        set("LOCALAPPDATA")?
    } else {
        set("XDG_STATE_HOME").or_else(|| Some(set("HOME")?.join(".local").join("state")))?
    };
    let name = socket.file_name()?;
    Some(home.join("rusty_mux").join(name))
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

/// A shared server (C26): its socket is open to every local user and the
/// allowlist decides who is served. SIGHUP reads the list again.
#[cfg(target_os = "linux")]
fn run_shared(
    socket: &std::path::Path,
    allow: PathBuf,
    tailnet: Option<SocketAddr>,
    config: Config,
) -> std::io::Result<()> {
    use rusty_tokio::signal::unix::{SignalKind, signal};

    // Before the runtime starts a thread: binding sets the umask.
    let (listener, owned) = rusty_mux_server::endpoint::bind(socket, 0o000)?;
    let runtime = Builder::new_current_thread().build_local(&LocalOptions::default())?;
    let served = runtime.block_on(async {
        let mut hangups = signal(SignalKind::hangup())?;
        let listener = UnixListener::from_std(listener)?;
        let (reload, reloads) = rusty_tokio::sync::mpsc::unbounded_channel();
        rusty_tokio::task::spawn_local(async move {
            while hangups.recv().await.is_some() {
                // The server logs the answer; nobody waits here.
                let (answer, _) = rusty_tokio::sync::oneshot::channel();
                if reload.send(answer).is_err() {
                    break;
                }
            }
        });
        let tailnet = match tailnet {
            None => None,
            Some(addr) => {
                let listener = rusty_tokio::io::TcpListener::bind(addr).await?;
                let whois = rusty_mux_server::tailnet::Whois::new(
                    rusty_mux_server::tailnet::DEFAULT_SOCKET,
                );
                Some(rusty_mux_server::Tailnet::new(listener, whois)?)
            }
        };
        let sharing = rusty_mux_server::Sharing { allow, tailnet };
        rusty_mux_server::serve_shared(listener, config, sharing, reloads, stop_requested()?).await
    });
    served.and(owned.release())
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

/// Resolves on SIGTERM or SIGINT, the server's clean stops on Unix (C36b):
/// state is saved, owned blocks are hung up and the name is released, as
/// on Windows. Both handlers are installed now, so a failure stops the
/// server from starting.
#[cfg(not(windows))]
fn stop_requested() -> std::io::Result<impl Future<Output = ()>> {
    use rusty_tokio::signal::unix::{SignalKind, signal};

    let mut terminate = signal(SignalKind::terminate())?;
    let mut interrupt = signal(SignalKind::interrupt())?;
    Ok(async move {
        rusty_tokio::select! {
            _ = terminate.recv() => {}
            _ = interrupt.recv() => {}
        }
    })
}

#[cfg(not(windows))]
fn default_config(console_host: ConsoleHost, config_file: Option<PathBuf>) -> Config {
    Config {
        program: std::env::var_os("SHELL").unwrap_or_else(|| OsString::from("/bin/sh")),
        args: Vec::new(),
        cwd: std::env::var_os("HOME").unwrap_or_else(|| OsString::from("/")),
        console_host,
        grapheme_clustering: false,
        config_file,
        state_dir: None,
    }
}

#[cfg(windows)]
fn default_config(console_host: ConsoleHost, config_file: Option<PathBuf>) -> Config {
    Config {
        program: std::env::var_os("COMSPEC").unwrap_or_else(|| OsString::from("cmd.exe")),
        args: Vec::new(),
        cwd: std::env::var_os("USERPROFILE").unwrap_or_else(|| OsString::from("C:\\")),
        console_host,
        grapheme_clustering: false,
        config_file,
        state_dir: None,
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
        assert_eq!(v6.socket, Some(PathBuf::from("s.sock")));
        assert_eq!(v6.config, None);
        let file = parse_str(&["--config", "c.json", "s.sock"]).unwrap();
        assert_eq!(file.config, Some(PathBuf::from("c.json")));
        assert_eq!(file.mode, Mode::Personal);
        assert_eq!(file.state, StateDir::Default);
        let at = parse_str(&["--state", "/s", "x"]).unwrap();
        assert_eq!(at.state, StateDir::At("/s".into()));
        assert_eq!(
            parse_str(&["--no-state", "x"]).unwrap().state,
            StateDir::Off
        );
    }

    #[test]
    fn the_socket_is_optional_except_in_login_mode() {
        let plain = parse_str(&[]).unwrap();
        assert_eq!(plain.socket, None);
        assert_eq!(plain.mode, Mode::Personal);
        assert_eq!(plain.idle_exit, None);
        let idle = parse_str(&["--idle-exit", "60"]).unwrap();
        assert_eq!(idle.idle_exit, Some(Duration::from_secs(60)));
        assert!(parse_str(&["--login", "m"]).is_err());
        assert!(parse_str(&["--forwarded-by", "0"]).is_err());
    }

    #[test]
    fn the_state_dir_is_named_after_the_socket() {
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            move |name: &str| {
                pairs
                    .iter()
                    .find(|(k, _)| *k == name)
                    .map(|(_, v)| OsString::from(v))
            }
        };
        let dir = |pairs| default_state_dir("/run/u/mux.sock".as_ref(), env(pairs));
        if cfg!(windows) {
            assert_eq!(
                dir(&[("LOCALAPPDATA", "C:\\L")]),
                Some(PathBuf::from("C:\\L\\rusty_mux\\mux.sock"))
            );
            assert_eq!(dir(&[]), None);
            return;
        }
        assert_eq!(
            dir(&[("XDG_STATE_HOME", "/x"), ("HOME", "/h")]),
            Some("/x/rusty_mux/mux.sock".into())
        );
        assert_eq!(
            dir(&[("XDG_STATE_HOME", ""), ("HOME", "/h")]),
            Some("/h/.local/state/rusty_mux/mux.sock".into())
        );
        assert_eq!(dir(&[]), None);
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

    #[cfg(target_os = "linux")]
    #[test]
    fn shared_mode_takes_an_allowlist_and_a_socket() {
        let shared = parse_str(&["--shared", "allow", "--no-state", "s.sock"]).unwrap();
        assert_eq!(shared.mode, Mode::Shared(PathBuf::from("allow"), None));
        assert_eq!(shared.socket, Some(PathBuf::from("s.sock")));
        assert_eq!(shared.state, StateDir::Off);
        for bad in [
            &["--shared", "allow"][..],
            &["--shared"],
            &["--shared", "a", "--udp", "0.0.0.0", "s"],
            &["--shared", "a", "--idle-exit", "5", "s"],
            &["--shared", "a", "--login", "m", "s"],
            &["--shared", "a", "--forwarded-by", "0", "s"],
            &["--shared", "a", "--runtime", "r", "s"],
        ] {
            assert!(parse_str(bad).is_err(), "{bad:?}");
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn the_tailnet_listener_is_shared_mode_and_a_tailnet_address() {
        let ok = parse_str(&["--shared", "a", "--tailnet", "100.64.0.7:7000", "s"]).unwrap();
        assert_eq!(
            ok.mode,
            Mode::Shared(PathBuf::from("a"), Some("100.64.0.7:7000".parse().unwrap()))
        );
        for bad in [
            &["--tailnet", "100.64.0.7:7000", "s"][..],
            &["--shared", "a", "--tailnet", "127.0.0.1:7000", "s"],
            &["--shared", "a", "--tailnet", "0.0.0.0:7000", "s"],
            &["--shared", "a", "--tailnet", "192.168.1.2:7000", "s"],
            &["--shared", "a", "--tailnet", "100.64.0.7", "s"],
            &["--shared", "a", "--tailnet"],
        ] {
            assert!(parse_str(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn a_bad_command_line_is_refused() {
        for bad in [
            &["a", "b"][..],
            &["--forwarded-by", "0"],
            &["--idle-exit", "0"],
            &["--idle-exit", "1m"],
            &["--idle-exit"],
            &["--forwarded-by", "0", "--idle-exit", "5", "s"],
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
            &["--login", "m", "--no-state", "s"],
            &["--state", "a", "--no-state", "s"],
            &["--state"],
            &["--forwarded-by", "x", "s"],
            &["--forwarded-by", "-1", "s"],
            &["--forwarded-by", "0", "--udp", "0.0.0.0", "s"],
        ] {
            assert!(parse_str(bad).is_err(), "{bad:?}");
        }
    }
}
