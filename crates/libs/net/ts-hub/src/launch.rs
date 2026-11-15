//! The hub launcher, shared by `ts-daemon hub` and `rusty-mux-server --hub`:
//! runs the first-party coordination server from flags and two files.
//!
//! ```text
//! <prog> --listen ADDR:PORT [--tls-cert PEM --tls-key PEM]
//!               [--stun ADDR:PORT] [--state-dir DIR] [--node-ttl SECS]
//!               [--keys FILE] [--policy FILE] [--advertise HOST]
//! ```
//!
//! SIGHUP re-reads the keys and policy files. Logs carry addresses and
//! counts, never keys, tokens or relayed bytes.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

use crate::{FileStore, Hub, Limits, Policy};
use rusty_tokio::net::{TcpListener, UdpSocket};
use ts_key::NodeState;

const USAGE: &str = "\
--listen <addr:port> [options]

required:
  --listen <addr:port>   serve /key, the control protocol and DERP here

options:
  --tls-cert <pem>       serve over TLS with this certificate chain
  --tls-key <pem>        ... and this PKCS#8 key (both or neither)
  --stun <addr:port>     also answer STUN binding requests here (UDP)
  --state-dir <dir>      hub identity and node table (default: ./ts-hub-state)
  --node-ttl <secs>      registrations expire after this long (default: never)
  --keys <file>          pre-auth keys, one `KEY LOGIN [reusable] [name=SERVER]`
                         per line; must not be readable by group or others
  --policy <file>        `FROM -> TO` rules (default: none, nobody reaches anyone)
  --advertise <host>     DERP/STUN host to put in every netmap
SIGHUP re-reads --keys and --policy.";

#[derive(Debug)]
struct Args {
    listen: SocketAddr,
    tls: Option<(PathBuf, PathBuf)>,
    stun: Option<SocketAddr>,
    state_dir: PathBuf,
    node_ttl: Option<u64>,
    keys: Option<PathBuf>,
    policy: Option<PathBuf>,
    advertise: Option<String>,
}

fn parse_args(args: impl Iterator<Item = String>) -> Result<Args, String> {
    let (mut listen, mut cert, mut key, mut stun) = (None, None, None, None);
    let mut state_dir = PathBuf::from("ts-hub-state");
    let (mut node_ttl, mut keys, mut policy, mut advertise) = (None, None, None, None);
    let mut it = args;
    while let Some(arg) = it.next() {
        let mut next = || it.next().ok_or_else(|| format!("{arg} requires a value"));
        let addr = |v: String| {
            v.parse::<SocketAddr>()
                .map_err(|_| format!("bad address {v:?}"))
        };
        match arg.as_str() {
            "--listen" => listen = Some(addr(next()?)?),
            "--tls-cert" => cert = Some(PathBuf::from(next()?)),
            "--tls-key" => key = Some(PathBuf::from(next()?)),
            "--stun" => stun = Some(addr(next()?)?),
            "--state-dir" => state_dir = PathBuf::from(next()?),
            "--node-ttl" => {
                let v = next()?;
                node_ttl = Some(v.parse().map_err(|_| format!("bad --node-ttl {v:?}"))?);
            }
            "--keys" => keys = Some(PathBuf::from(next()?)),
            "--policy" => policy = Some(PathBuf::from(next()?)),
            "--advertise" => advertise = Some(next()?),
            "-h" | "--help" => return Err(String::new()),
            other => return Err(format!("unknown argument {other:?}")),
        }
    }
    let tls = match (cert, key) {
        (Some(c), Some(k)) => Some((c, k)),
        (None, None) => None,
        _ => return Err("--tls-cert and --tls-key go together".into()),
    };
    Ok(Args {
        listen: listen.ok_or("--listen is required")?,
        tls,
        stun,
        state_dir,
        node_ttl,
        keys,
        policy,
        advertise,
    })
}

/// Runs the hub until SIGTERM or Ctrl-C. `prog` is how the user ran it
/// (`"ts-daemon hub"`), for the usage and error lines; `args` are the flags
/// after it.
pub async fn main(prog: &str, args: impl Iterator<Item = String>) -> ExitCode {
    let args = match parse_args(args) {
        Ok(a) => a,
        Err(msg) => {
            if !msg.is_empty() {
                eprintln!("{prog}: {msg}\n");
            }
            eprintln!("usage: {prog} {USAGE}");
            return ExitCode::from(2);
        }
    };
    match run(args).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            rusty_log::error!("hub: {e}");
            ExitCode::FAILURE
        }
    }
}

type BoxError = Box<dyn std::error::Error>;

async fn run(args: Args) -> Result<(), BoxError> {
    std::fs::create_dir_all(&args.state_dir)?;
    let identity = NodeState::load_or_generate(&args.state_dir)?.machine;
    let store = FileStore::open(args.state_dir.join("nodes.json"), args.node_ttl)?;
    let hub = Hub::with_identity(store, Limits::default(), identity);
    // A bad file stops the hub before it serves anyone.
    reload(&hub, args.keys.as_deref(), args.policy.as_deref())?;
    if args.policy.is_none() {
        rusty_log::warn!("hub: no --policy: nodes will see no peers and reach nobody");
    }

    let acceptor = match &args.tls {
        Some((cert, key)) => Some(crate::tls_acceptor(
            &std::fs::read_to_string(cert)?,
            &std::fs::read_to_string(key)?,
        )?),
        None => None,
    };
    let listener = TcpListener::bind(args.listen).await?;
    let local = listener.local_addr()?;
    let stun_port = match args.stun {
        Some(addr) => {
            let socket = UdpSocket::bind(addr).await?;
            let port = socket.local_addr()?.port();
            rusty_tokio::spawn(async move {
                if let Err(e) = crate::stun::serve(socket).await {
                    rusty_log::error!("stun: {e}");
                }
            });
            port
        }
        None => 0,
    };
    if let Some(host) = &args.advertise {
        hub.advertise(host, local.port(), stun_port);
    }
    let scheme = if acceptor.is_some() {
        "TLS"
    } else {
        "plain HTTP"
    };
    rusty_log::info!(
        "hub: serving {scheme} on {local}, stun port {stun_port}, key {}",
        hub.public_key()
    );
    // Scripts and tests wait for this line; its last word is the address.
    println!("hub: listening on {local}");

    let serving = match acceptor {
        Some(acceptor) => rusty_tokio::spawn(hub.clone().serve_tls(listener, acceptor)),
        None => rusty_tokio::spawn(hub.clone().serve(listener)),
    };
    signals(&hub, &args).await;
    serving.abort();
    rusty_log::info!("hub: shutting down");
    Ok(())
}

/// Reloads on SIGHUP; returns on SIGTERM or Ctrl-C.
async fn signals(hub: &Arc<Hub<FileStore>>, args: &Args) {
    use rusty_tokio::signal::unix::{SignalKind, signal};
    let (Ok(mut term), Ok(mut hup)) = (
        signal(SignalKind::terminate()),
        signal(SignalKind::hangup()),
    ) else {
        rusty_log::warn!("hub: cannot handle SIGTERM/SIGHUP; Ctrl-C only");
        rusty_tokio::signal::ctrl_c().await.ok();
        return;
    };
    loop {
        rusty_tokio::select! {
            _ = rusty_tokio::signal::ctrl_c() => return,
            _ = term.recv() => return,
            _ = hup.recv() => match reload(hub, args.keys.as_deref(), args.policy.as_deref()) {
                Ok(()) => rusty_log::info!("hub: reloaded keys and policy"),
                Err(e) => rusty_log::error!("hub: reload failed, keeping the old rules: {e}"),
            },
        }
    }
}

/// Applies the policy file, then the keys file. Each is checked before it
/// changes anything.
fn reload(
    hub: &Hub<FileStore>,
    keys: Option<&Path>,
    policy: Option<&Path>,
) -> Result<(), BoxError> {
    if let Some(path) = policy {
        hub.set_policy(Policy::parse(&std::fs::read_to_string(path)?)?);
    }
    if let Some(path) = keys {
        hub.load_keys(&read_secret(path)?)?;
    }
    Ok(())
}

/// Reads a file of secrets, refusing one that group or others can read.
fn read_secret(path: &Path) -> Result<String, BoxError> {
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(path)?.permissions().mode();
    if mode & 0o077 != 0 {
        return Err(format!("{} is readable by others (chmod 600)", path.display()).into());
    }
    Ok(std::fs::read_to_string(path)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Args, String> {
        parse_args(args.iter().map(|s| s.to_string()))
    }

    #[test]
    fn listen_is_required_and_tls_files_go_together() {
        assert!(parse(&[]).is_err());
        assert!(parse(&["--listen", "127.0.0.1:0"]).is_ok());
        assert!(parse(&["--listen", "127.0.0.1:0", "--tls-cert", "c"]).is_err());
        let a = parse(&["--listen", "[::1]:443", "--tls-cert", "c", "--tls-key", "k"]).unwrap();
        assert!(a.tls.is_some());
    }

    #[test]
    fn bad_values_are_named_not_ignored() {
        assert!(
            parse(&["--listen", "nope"])
                .unwrap_err()
                .contains("bad address")
        );
        assert!(parse(&["--listen", "127.0.0.1:1", "--node-ttl", "x"]).is_err());
        assert!(parse(&["--listen", "127.0.0.1:1", "--wat"]).is_err());
    }

    #[test]
    fn a_keys_file_others_can_read_is_refused() {
        use std::os::unix::fs::PermissionsExt;
        let path = std::env::temp_dir().join(format!("ts-hub-keys-{}", std::process::id()));
        std::fs::write(&path, "k alice\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(read_secret(&path).is_err());
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(read_secret(&path).unwrap(), "k alice\n");
        std::fs::remove_file(&path).ok();
    }
}
