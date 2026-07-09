//! `ts-daemon`: the tailscaled-equivalent daemon.
//!
//! Phase 3 surface: register with a Headscale control server, connect to
//! DERP, and run the DERP-only WireGuard data plane (`ts-engine`). Two
//! daemons on the same tailnet can ping each other's `100.64.x.y` entirely
//! over the relay — no direct paths (Phase 5), no TUN/root (Phase 4).
//!
//! Usage:
//!   ts-daemon --login-server http://127.0.0.1:8080 --authkey <KEY> \
//!             [--derp-server URL] [--state-dir DIR] [--hostname NAME] \
//!             [--ping 100.64.0.2]

mod backend;

use std::net::Ipv4Addr;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use backend::DaemonBackend;
use ts_engine::{Engine, EngineConfig, PingError};
use ts_key::NodeState;

const USAGE: &str = "\
usage: ts-daemon --login-server <url> --authkey <key> [options]

required:
  --login-server <url>   Headscale/control base URL (http://host:port)
  --authkey <key>        preauth key to register with

options:
  --derp-server <url>    DERP relay base URL (default: same as --login-server)
  --state-dir <dir>      identity state directory (default: ./ts-rs-state)
  --hostname <name>      node hostname (default: system hostname)
  --tun <name>           create a TUN device (needs CAP_NET_ADMIN); real OS
                         traffic then rides the tailnet
  --hosts-file <path>    write MagicDNS peer names into this hosts file
  --direct               enable direct-path discovery (magicsock/disco);
                         peers upgrade from DERP to direct UDP when possible
  --stun <host:port>     STUN server for reflexive-endpoint discovery
                         (for NAT traversal; implies --direct)
  --socket <path>        serve the LocalAPI on this Unix socket so ts-cli can
                         query status/prefs/ping (default:
                         /var/run/tailscale/tailscaled.sock)
  --ping <ip>            userspace-ping a peer's tailnet IP, then exit
                         (no-TUN mode only)";

/// Default LocalAPI socket path (matches the Go daemon and ts-cli default).
const DEFAULT_SOCKET: &str = "/var/run/tailscale/tailscaled.sock";

struct Args {
    login_server: String,
    derp_server: Option<String>,
    authkey: String,
    state_dir: PathBuf,
    hostname: Option<String>,
    tun: Option<String>,
    hosts_file: Option<PathBuf>,
    direct: bool,
    stun: Option<String>,
    socket: PathBuf,
    ping: Option<Ipv4Addr>,
}

fn parse_args() -> Result<Args, String> {
    let mut login_server = None;
    let mut derp_server = None;
    let mut authkey = None;
    let mut state_dir = PathBuf::from("ts-rs-state");
    let mut hostname = None;
    let mut tun = None;
    let mut hosts_file = None;
    let mut direct = false;
    let mut stun = None;
    let mut socket = PathBuf::from(DEFAULT_SOCKET);
    let mut ping = None;

    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        let mut next = || it.next().ok_or_else(|| format!("{arg} requires a value"));
        match arg.as_str() {
            "--login-server" => login_server = Some(next()?),
            "--derp-server" => derp_server = Some(next()?),
            "--authkey" => authkey = Some(next()?),
            "--state-dir" => state_dir = PathBuf::from(next()?),
            "--hostname" => hostname = Some(next()?),
            "--tun" => tun = Some(next()?),
            "--hosts-file" => hosts_file = Some(PathBuf::from(next()?)),
            "--direct" => direct = true,
            "--stun" => stun = Some(next()?),
            "--socket" => socket = PathBuf::from(next()?),
            "--ping" => {
                let v = next()?;
                ping = Some(v.parse().map_err(|_| format!("invalid --ping IP {v:?}"))?);
            }
            "-h" | "--help" => return Err(String::new()),
            other => return Err(format!("unknown argument {other:?}")),
        }
    }
    Ok(Args {
        login_server: login_server.ok_or("--login-server is required")?,
        derp_server,
        authkey: authkey.ok_or("--authkey is required")?,
        state_dir,
        hostname,
        tun,
        hosts_file,
        direct,
        stun,
        socket,
        ping,
    })
}

fn default_hostname() -> String {
    std::fs::read_to_string("/proc/sys/kernel/hostname")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "ts-rs-node".to_string())
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .with_target(false)
        .init();

    let args = match parse_args() {
        Ok(a) => a,
        Err(msg) => {
            if !msg.is_empty() {
                eprintln!("ts-daemon: {msg}\n");
            }
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };

    match run(args).await {
        Ok(code) => code,
        Err(err) => {
            tracing::error!("{err}");
            ExitCode::FAILURE
        }
    }
}

async fn run(args: Args) -> Result<ExitCode, Box<dyn std::error::Error>> {
    let state = NodeState::load_or_generate(&args.state_dir)?;
    let hostname = args.hostname.unwrap_or_else(default_hostname);
    tracing::info!(node_key = %state.node.public(), hostname = %hostname, "identity loaded");

    let tun_mode = args.tun.is_some();
    let control_url = args.login_server.clone();
    let config = EngineConfig {
        derp_url: args
            .derp_server
            .unwrap_or_else(|| args.login_server.clone()),
        control_url: args.login_server,
        authkey: args.authkey,
        hostname: hostname.clone(),
        tun_name: args.tun,
        magic_dns_hosts: args.hosts_file,
        enable_direct: args.direct || args.stun.is_some(),
        stun_server: args.stun,
        stack_io: None,
    };

    let engine = Engine::start(config, state).await?;

    match args.ping {
        Some(target) => cmd_ping(&engine, target).await,
        None => {
            // Serve the LocalAPI so ts-cli (status/up/down/ping) can drive us.
            let backend = DaemonBackend::new(engine.clone(), control_url, hostname);
            let socket = args.socket.clone();
            tokio::spawn(async move {
                if let Err(e) = ts_localapi::serve(&socket, backend).await {
                    tracing::error!("localapi: {e}");
                }
            });

            let mode = if tun_mode {
                "TUN, relayed via DERP"
            } else {
                "userspace DERP-only"
            };
            println!(
                "ts-daemon: data plane up ({mode}); LocalAPI at {}. Ctrl-C to stop.",
                args.socket.display()
            );
            wait_for_shutdown().await;
            tracing::info!("ts-daemon: shutting down");
            // Best-effort: remove the LocalAPI socket so a restart binds cleanly.
            let _ = std::fs::remove_file(&args.socket);
            Ok(ExitCode::SUCCESS)
        }
    }
}

/// Blocks until the process receives SIGINT (Ctrl-C) or SIGTERM (the signal
/// `systemctl stop` sends), so the daemon shuts down cleanly under an init
/// system as well as interactively.
async fn wait_for_shutdown() {
    use tokio::signal::unix::{SignalKind, signal};
    let mut term = match signal(SignalKind::terminate()) {
        Ok(s) => s,
        Err(e) => {
            // Fall back to Ctrl-C only if we can't install the SIGTERM handler.
            tracing::warn!("ts-daemon: cannot handle SIGTERM ({e}); Ctrl-C only");
            tokio::signal::ctrl_c().await.ok();
            return;
        }
    };
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = term.recv() => {}
    }
}

/// Waits for the target peer to appear in the netmap, then pings it over the
/// relay.
async fn cmd_ping(
    engine: &ts_engine::EngineHandle,
    target: Ipv4Addr,
) -> Result<ExitCode, Box<dyn std::error::Error>> {
    // The netmap arrives shortly after registration; wait for the peer.
    for _ in 0..50 {
        if engine.peer_ips().await.contains(&target) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    // First ping performs the WireGuard handshake, so allow generous time.
    match engine.ping(target, Duration::from_secs(10)).await {
        Ok(rtt) => {
            println!(
                "pong from {target} via DERP relay in {:.1}ms",
                rtt.as_secs_f64() * 1000.0
            );
            Ok(ExitCode::SUCCESS)
        }
        Err(PingError::UnknownPeer(_)) => {
            eprintln!("ts-daemon: {target} is not a known peer (netmap has no such node)");
            Ok(ExitCode::FAILURE)
        }
        Err(e) => {
            eprintln!("ts-daemon: ping failed: {e}");
            Ok(ExitCode::FAILURE)
        }
    }
}
