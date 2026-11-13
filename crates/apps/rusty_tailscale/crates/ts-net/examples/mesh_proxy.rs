//! Spike (throwaway): a local proxy into the tailnet, like `tailscale nc`.
//! Registers an embedded node (no TUN, no root, no admin), listens on a local
//! Unix socket, and for each local connection dials `--to IP:PORT` over the
//! tailnet and relays bytes. A mux client pointed at the socket then talks to
//! a mux server on the mesh.
//!
//! ```console
//! $ MESH_AUTHKEY=... cargo run -p ts-net --example mesh_proxy -- \
//!     --login-server http://127.0.0.1:8080 --to 100.64.0.1:7600 --listen proxy.sock
//! ```

use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::ExitCode;

use rusty_tokio::io::{UnixListener, copy_bidirectional};
use ts_net::{Node, NodeConfig};

const USAGE: &str = "usage: MESH_AUTHKEY=<key> mesh_proxy --login-server <url> --to <ip:port> \
--listen <socket> [--state-dir <dir>] [--hostname <name>]";

struct Args {
    login_server: String,
    to: SocketAddr,
    listen: PathBuf,
    state_dir: PathBuf,
    hostname: String,
}

fn parse() -> Result<Args, String> {
    let (mut login, mut to, mut listen) = (None, None, None);
    let mut state_dir = PathBuf::from("mesh-proxy-state");
    let mut hostname = "mesh-proxy".to_string();
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        let mut value = || it.next().ok_or(format!("{flag} needs a value"));
        match flag.as_str() {
            "--login-server" => login = Some(value()?),
            "--to" => to = Some(value()?.parse().map_err(|_| "--to: not IP:PORT")?),
            "--listen" => listen = Some(PathBuf::from(value()?)),
            "--state-dir" => state_dir = PathBuf::from(value()?),
            "--hostname" => hostname = value()?,
            other => return Err(format!("unknown argument {other:?}")),
        }
    }
    Ok(Args {
        login_server: login.ok_or("--login-server is required")?,
        to: to.ok_or("--to is required")?,
        listen: listen.ok_or("--listen is required")?,
        state_dir,
        hostname,
    })
}

#[rusty_tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let args = match parse() {
        Ok(a) => a,
        Err(msg) => {
            eprintln!("mesh_proxy: {msg}\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    let Ok(authkey) = std::env::var("MESH_AUTHKEY") else {
        eprintln!("mesh_proxy: MESH_AUTHKEY is not set\n{USAGE}");
        return ExitCode::from(2);
    };
    match run(args, authkey).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("mesh_proxy: {e}");
            ExitCode::FAILURE
        }
    }
}

async fn run(args: Args, authkey: String) -> Result<(), Box<dyn std::error::Error>> {
    let node = Node::new(NodeConfig {
        control_url: args.login_server,
        derp_url: None,
        authkey,
        hostname: args.hostname,
        state_dir: args.state_dir,
        enable_direct: true,
        stun_server: None,
    })
    .await?;
    let ip = node
        .wait_ip()
        .await
        .ok_or("timed out waiting for a tailnet IP")?;
    let _ = std::fs::remove_file(&args.listen);
    let listener = UnixListener::bind(&args.listen)?;
    println!(
        "mesh_proxy: {ip} -> {} via {}",
        args.to,
        args.listen.display()
    );
    loop {
        let (mut local, _) = listener.accept().await?;
        let node = node.clone();
        let to = args.to;
        rusty_tokio::spawn(async move {
            let mut remote = match node.connect(to).await {
                Ok(s) => s,
                Err(e) => return eprintln!("mesh_proxy: dial {to}: {e}"),
            };
            let _ = copy_bidirectional(&mut local, &mut remote).await;
        });
    }
}
