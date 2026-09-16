//! An HTTP server that lives *on the tailnet* — no TUN, no root, plain
//! `cargo run`. It registers with a control server, waits for its tailnet IP,
//! and serves a tiny page on port 8080 to any peer that connects.
//!
//! ```console
//! $ cargo run -p ts-net --example serve_http -- \
//!     --login-server http://127.0.0.1:8080 --authkey "$KEY" --hostname rusty-web
//! ts-net: serving http://100.64.0.7:8080/ on the tailnet
//!
//! # from another node on the tailnet:
//! $ curl http://100.64.0.7:8080/
//! Hello from tailscale-rs (ts-net), served with no TUN and no root!
//! ```

#[cfg(target_os = "linux")]
use std::path::PathBuf;
#[cfg(target_os = "linux")]
use std::process::ExitCode;

#[cfg(target_os = "linux")]
use tokio::io::{AsyncReadExt, AsyncWriteExt};
#[cfg(target_os = "linux")]
use ts_net::{Node, NodeConfig, TcpStream};

#[cfg(target_os = "linux")]
const USAGE: &str = "\
usage: serve_http --login-server <url> --authkey <key> [options]

options:
  --derp-server <url>   DERP relay base URL (default: same as --login-server)
  --state-dir <dir>     identity directory (default: ./ts-net-state)
  --hostname <name>     node hostname (default: rusty-web)
  --port <p>            TCP port to serve (default: 8080)
  --direct              enable direct-path discovery
  --stun <host:port>    STUN server for reflexive discovery (implies --direct)";

#[cfg(target_os = "linux")]
struct Args {
    login_server: String,
    derp_server: Option<String>,
    authkey: String,
    state_dir: PathBuf,
    hostname: String,
    port: u16,
    direct: bool,
    stun: Option<String>,
}

#[cfg(target_os = "linux")]
fn parse_args() -> Result<Args, String> {
    let mut login_server = None;
    let mut derp_server = None;
    let mut authkey = None;
    let mut state_dir = PathBuf::from("ts-net-state");
    let mut hostname = "rusty-web".to_string();
    let mut port = 8080u16;
    let mut direct = false;
    let mut stun = None;

    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        let mut next = || it.next().ok_or_else(|| format!("{arg} requires a value"));
        match arg.as_str() {
            "--login-server" => login_server = Some(next()?),
            "--derp-server" => derp_server = Some(next()?),
            "--authkey" => authkey = Some(next()?),
            "--state-dir" => state_dir = PathBuf::from(next()?),
            "--hostname" => hostname = next()?,
            "--port" => port = next()?.parse().map_err(|_| "invalid --port".to_string())?,
            "--direct" => direct = true,
            "--stun" => stun = Some(next()?),
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
        port,
        direct,
        stun,
    })
}

#[cfg(not(target_os = "linux"))]
fn main() {}

#[cfg(target_os = "linux")]
#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    tracing_subscriber_init();
    let args = match parse_args() {
        Ok(a) => a,
        Err(msg) => {
            if !msg.is_empty() {
                eprintln!("serve_http: {msg}\n");
            }
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };

    let node = match Node::new(NodeConfig {
        control_url: args.login_server,
        derp_url: args.derp_server,
        authkey: args.authkey,
        hostname: args.hostname,
        state_dir: args.state_dir,
        enable_direct: args.direct || args.stun.is_some(),
        stun_server: args.stun,
    })
    .await
    {
        Ok(n) => n,
        Err(e) => {
            eprintln!("serve_http: {e}");
            return ExitCode::FAILURE;
        }
    };

    let Some(ip) = node.wait_ip().await else {
        eprintln!("serve_http: timed out waiting for a tailnet IP");
        return ExitCode::FAILURE;
    };

    let mut listener = match node.bind(args.port).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("serve_http: bind failed: {e}");
            return ExitCode::FAILURE;
        }
    };
    println!(
        "ts-net: serving http://{ip}:{}/ on the tailnet (no TUN, no root)",
        args.port
    );

    while let Some(stream) = listener.accept().await {
        tokio::spawn(handle(stream));
    }
    ExitCode::SUCCESS
}

#[cfg(target_os = "linux")]
async fn handle(mut stream: TcpStream) {
    let peer = stream.peer_addr();
    // Read the request headers (until the blank line) — enough to be a polite
    // HTTP/1.1 server for a `curl`.
    let mut buf = [0u8; 2048];
    let mut req = Vec::new();
    loop {
        match stream.read(&mut buf).await {
            Ok(0) => break,
            Ok(n) => {
                req.extend_from_slice(&buf[..n]);
                if req.windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }
            Err(_) => return,
        }
    }

    let body = format!(
        "Hello from tailscale-rs (ts-net), served with no TUN and no root!\n\
         You are {peer}.\n"
    );
    let response = format!(
        "HTTP/1.1 200 OK\r\n\
         Content-Type: text/plain; charset=utf-8\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\
         \r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.flush().await;
    let _ = stream.shutdown().await;
}

#[cfg(target_os = "linux")]
fn tracing_subscriber_init() {
    // Best-effort: honor RUST_LOG if the subscriber crate is present. ts-net's
    // example keeps deps minimal, so just print engine logs via eprintln
    // fallback — the engine uses `tracing`, which is a no-op without a
    // subscriber, so this example stays quiet unless wired up by the harness.
}
