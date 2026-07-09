//! `ts-daemon`: the tailscaled-equivalent daemon.
//!
//! Phase 2 surface: register a fresh identity with a Headscale control
//! server and stream the netmap long-poll, printing live updates. There is
//! no data plane yet (Phase 3) and no LocalAPI server yet (Phase 6).
//!
//! Usage:
//!   ts-daemon --login-server http://127.0.0.1:8080 --authkey <KEY> \
//!             [--state-dir DIR] [--hostname NAME]

use std::ops::ControlFlow;
use std::path::PathBuf;
use std::process::ExitCode;

use ts_control::ControlClient;
use ts_key::NodeState;
use ts_types::tailcfg::{Hostinfo, MapResponse};

const USAGE: &str = "\
usage: ts-daemon --login-server <url> --authkey <key> [options]

required:
  --login-server <url>   Headscale/control base URL (http://host:port)
  --authkey <key>        preauth key to register with

options:
  --state-dir <dir>      identity state directory (default: ./ts-rs-state)
  --hostname <name>      node hostname (default: system hostname)
  --once                 register and print one netmap, then exit";

struct Args {
    login_server: String,
    authkey: String,
    state_dir: PathBuf,
    hostname: Option<String>,
    once: bool,
}

fn parse_args() -> Result<Args, String> {
    let mut login_server = None;
    let mut authkey = None;
    let mut state_dir = PathBuf::from("ts-rs-state");
    let mut hostname = None;
    let mut once = false;

    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        let mut next = || it.next().ok_or_else(|| format!("{arg} requires a value"));
        match arg.as_str() {
            "--login-server" => login_server = Some(next()?),
            "--authkey" => authkey = Some(next()?),
            "--state-dir" => state_dir = PathBuf::from(next()?),
            "--hostname" => hostname = Some(next()?),
            "--once" => once = true,
            "-h" | "--help" => return Err(String::new()),
            other => return Err(format!("unknown argument {other:?}")),
        }
    }
    Ok(Args {
        login_server: login_server.ok_or("--login-server is required")?,
        authkey: authkey.ok_or("--authkey is required")?,
        state_dir,
        hostname,
        once,
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
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            tracing::error!("{err}");
            ExitCode::FAILURE
        }
    }
}

async fn run(args: Args) -> Result<(), Box<dyn std::error::Error>> {
    // Load or generate this node's persistent identity.
    let state = NodeState::load_or_generate(&args.state_dir)?;
    let node_key = state.node.public();
    let disco_key = state.disco.public();
    let hostname = args.hostname.unwrap_or_else(default_hostname);
    tracing::info!(%node_key, "loaded node identity");

    let hostinfo = Hostinfo {
        ipn_version: format!("tailscale-rs-{}", env!("CARGO_PKG_VERSION")),
        hostname: hostname.clone(),
        os: std::env::consts::OS.to_string(),
        routable_ips: Vec::new(),
    };

    let client =
        ControlClient::connect(&args.login_server, state.machine.clone(), hostinfo).await?;
    tracing::info!(control_key = %client.control_key(), "fetched control noise key");

    // Register with the preauth key.
    let resp = client.register(node_key, &args.authkey).await?;
    tracing::info!(
        machine_authorized = resp.machine_authorized,
        "registered node with control server"
    );
    println!(
        "registered: node_key={node_key} hostname={hostname} authorized={}",
        resp.machine_authorized
    );

    // Stream the netmap.
    tracing::info!("starting netmap long-poll…");
    let once = args.once;
    let mut printed_self = false;
    client
        .poll_netmap(node_key, disco_key, move |resp| {
            let stop = print_netmap_update(&resp, &mut printed_self);
            if once && stop {
                ControlFlow::Break(())
            } else {
                ControlFlow::Continue(())
            }
        })
        .await?;

    Ok(())
}

/// Prints a human-readable summary of one netmap frame. Returns true once a
/// frame carrying real map data (not just a keep-alive) has been seen.
fn print_netmap_update(resp: &MapResponse, printed_self: &mut bool) -> bool {
    if resp.keep_alive {
        tracing::debug!("keep-alive");
        return false;
    }

    let mut saw_data = false;
    if let Some(node) = &resp.node {
        saw_data = true;
        *printed_self = true;
        let ip = node
            .primary_ip()
            .map(|ip| ip.to_string())
            .unwrap_or_else(|| "?".into());
        println!("self:  {ip:<15} {} (node {})", node.name, node.id.0);
    }
    if let Some(peers) = &resp.peers {
        saw_data = true;
        println!("netmap: {} peer(s)", peers.len());
        for p in peers {
            let ip = p
                .primary_ip()
                .map(|ip| ip.to_string())
                .unwrap_or_else(|| "?".into());
            let online = match p.online {
                Some(true) => "online",
                Some(false) => "offline",
                None => "?",
            };
            println!("  peer: {ip:<15} {:<24} {online}", p.name);
        }
    }
    if let Some(changed) = &resp.peers_changed {
        saw_data = true;
        for p in changed {
            println!("  peer changed: {} ({})", p.name, p.id.0);
        }
    }
    if let Some(removed) = &resp.peers_removed {
        saw_data = true;
        for id in removed {
            println!("  peer removed: node {}", id.0);
        }
    }
    if !resp.domain.is_empty() {
        tracing::info!(domain = %resp.domain, "tailnet domain");
    }
    saw_data
}
