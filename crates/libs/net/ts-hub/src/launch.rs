//! The hub launcher, shared by `ts-daemon hub` and `rusty-mux-server --hub`:
//! runs the first-party coordination server from flags and two files.
//!
//! ```text
//! <prog> --listen ADDR:PORT [--tls-cert PEM --tls-key PEM]
//!               [--stun ADDR:PORT] [--state-dir DIR] [--node-ttl SECS]
//!               [--keys FILE] [--policy FILE] [--advertise HOST]
//!               [--replica-secret FILE] [--advertise-standby URL]...
//! <prog> --standby URL --replica-secret FILE [--tls-ca PEM] [--state-dir DIR]
//! <prog> --listen ADDR:PORT --promote URL [--force] [--state-dir DIR] ...
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
  --replica-secret <f>   let a standby that presents this secret (random, 16+ characters,
                         e.g. `openssl rand -hex 32`; file mode 600) copy the hub's state; needs TLS unless
                         --listen is a loopback address

  --advertise-standby <url>
                         tell nodes this standby's base URL (repeatable), so they
                         reconnect to it after a promotion; it must serve control
                         and DERP on that URL

failover (docs/projects/mux/MESH.md, stage 4):
  --standby <url>        do not serve: follow the active hub at <url> and keep
                         its state in --state-dir (needs --replica-secret;
                         <url> is https:// or loopback; --tls-ca pins its CA)
  --promote <url>        serve from the copy a standby made, as the new active
                         hub. Refuses while the hub at <url> still answers,
                         unless --force. Never automatic.
SIGHUP re-reads --keys and --policy.";

#[derive(Debug)]
struct Args {
    listen: Option<SocketAddr>,
    tls: Option<(PathBuf, PathBuf)>,
    stun: Option<SocketAddr>,
    state_dir: PathBuf,
    node_ttl: Option<u64>,
    keys: Option<PathBuf>,
    policy: Option<PathBuf>,
    advertise: Option<String>,
    standbys: Vec<String>,
    replica_secret: Option<PathBuf>,
    standby: Option<String>,
    promote: Option<String>,
    force: bool,
    tls_ca: Option<PathBuf>,
}

/// What the command line asked for.
#[derive(Debug)]
enum Cli {
    Run(Box<Args>),
    Help,
}

fn parse_args(args: impl Iterator<Item = String>) -> Result<Cli, String> {
    let (mut listen, mut cert, mut key, mut stun) = (None, None, None, None);
    let mut state_dir = PathBuf::from("ts-hub-state");
    let (mut node_ttl, mut keys, mut policy, mut advertise) = (None, None, None, None);
    let (mut replica_secret, mut standby, mut promote, mut tls_ca) = (None, None, None, None);
    let mut force = false;
    let mut standbys = Vec::new();
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
            "--replica-secret" => replica_secret = Some(PathBuf::from(next()?)),
            "--standby" => standby = Some(next()?),
            "--promote" => promote = Some(next()?),
            "--force" => force = true,
            "--advertise-standby" => standbys.push(next()?),
            "--tls-ca" => tls_ca = Some(PathBuf::from(next()?)),
            "-h" | "--help" => return Ok(Cli::Help),
            other => return Err(format!("unknown argument {other:?}")),
        }
    }
    let tls = match (cert, key) {
        (Some(c), Some(k)) => Some((c, k)),
        (None, None) => None,
        _ => return Err("--tls-cert and --tls-key go together".into()),
    };
    let args = Args {
        listen,
        tls,
        stun,
        state_dir,
        node_ttl,
        keys,
        policy,
        advertise,
        standbys,
        replica_secret,
        standby,
        promote,
        force,
        tls_ca,
    };
    args.check()?;
    Ok(Cli::Run(Box::new(args)))
}

impl Args {
    /// Flag combinations that make no sense or would be unsafe.
    fn check(&self) -> Result<(), String> {
        if let Some(url) = &self.standby {
            if self.promote.is_some() {
                return Err("--standby and --promote are different roles".into());
            }
            if self.listen.is_some() {
                return Err(
                    "a standby does not serve: drop --listen (use --promote to serve)".into(),
                );
            }
            if self.replica_secret.is_none() {
                return Err("--standby needs --replica-secret".into());
            }
            return needs_secure_link(url, "--standby");
        }
        if self.listen.is_none() {
            return Err("--listen is required".into());
        }
        if self.force && self.promote.is_none() {
            return Err("--force goes with --promote".into());
        }
        if self.tls_ca.is_some() {
            return Err("--tls-ca goes with --standby".into());
        }
        // Nodes send their pre-auth key to these on every reconnect.
        for url in &self.standbys {
            needs_secure_link(url, "--advertise-standby")?;
        }
        if let Some(url) = &self.promote {
            needs_secure_link(url, "--promote")?;
        }
        // The secret rides in a header: never over plain HTTP off this host.
        let loopback = self.listen.is_some_and(|a| a.ip().is_loopback());
        if self.replica_secret.is_some() && self.tls.is_none() && !loopback {
            return Err(
                "--replica-secret needs --tls-cert/--tls-key, or a loopback --listen".into(),
            );
        }
        Ok(())
    }
}

/// `https://`, or `http://` only to this host.
fn needs_secure_link(url: &str, flag: &str) -> Result<(), String> {
    let Some((tls, _, authority)) = ts_dial::split(url) else {
        return Err(format!("{flag}: bad URL {url:?}"));
    };
    let host = ts_dial::server_name(&authority);
    if tls || matches!(host, "localhost" | "127.0.0.1" | "::1") {
        return Ok(());
    }
    Err(format!(
        "{flag}: {url} is plain HTTP; use https:// (the link carries pre-auth keys)"
    ))
}

/// Runs the hub until SIGTERM or Ctrl-C. `prog` is how the user ran it
/// (`"ts-daemon hub"`), for the usage and error lines; `args` are the flags
/// after it.
pub async fn main(prog: &str, args: impl Iterator<Item = String>) -> ExitCode {
    let args = match parse_args(args) {
        Ok(Cli::Run(a)) => *a,
        Ok(Cli::Help) => {
            println!("usage: {prog} {USAGE}");
            return ExitCode::SUCCESS;
        }
        Err(msg) => {
            eprintln!("{prog}: {msg}\n\nusage: {prog} {USAGE}");
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
    match (args.standby.clone(), args.listen) {
        (Some(url), _) => standby(&args, &url).await,
        (None, Some(listen)) => serve(args, listen).await,
        (None, None) => Err("--listen is required".into()),
    }
}

/// The trust for a link to the active hub: its CA when given, else the
/// system's.
fn link_trust(args: &Args) -> Result<ts_dial::Trust, BoxError> {
    match &args.tls_ca {
        None => Ok(ts_dial::Trust::System),
        Some(path) => Ok(ts_dial::trust_from_pem(&std::fs::read_to_string(path)?)?),
    }
}

/// Follows the active hub and serves nobody, until stopped.
async fn standby(args: &Args, url: &str) -> Result<(), BoxError> {
    let secret = secret_of(args)?.ok_or("--standby needs --replica-secret")?;
    let trust = link_trust(args)?;
    std::fs::create_dir_all(&args.state_dir)?;
    rusty_log::info!(
        "standby: following {url}, keeping state in {}",
        args.state_dir.display()
    );
    println!("standby: following {url}");
    rusty_tokio::select! {
        ended = crate::replica::follow(url, &secret, &args.state_dir, &trust) => {
            match ended {
                Err(e) => Err(format!("standby stopped: {e}").into()),
                Ok(never) => match never {},
            }
        }
        _ = stop() => {
            rusty_log::info!("standby: stopping");
            Ok(())
        }
    }
}

/// The replication secret from its file, if one was given.
fn secret_of(args: &Args) -> Result<Option<String>, BoxError> {
    let Some(path) = &args.replica_secret else {
        return Ok(None);
    };
    Ok(Some(read_secret(path)?.trim().to_string()))
}

/// Returns on SIGTERM or Ctrl-C.
async fn stop() {
    use rusty_tokio::signal::unix::{SignalKind, signal};
    match signal(SignalKind::terminate()) {
        Ok(mut term) => {
            rusty_tokio::select! {
                _ = rusty_tokio::signal::ctrl_c() => {}
                _ = term.recv() => {}
            }
        }
        Err(_) => {
            rusty_tokio::signal::ctrl_c().await.ok();
        }
    }
}

/// Refuses to promote a standby that never copied anything (it would start a
/// new hub with a new key) or one whose active hub still answers.
async fn check_promotion(args: &Args, url: &str) -> Result<(), BoxError> {
    if !args.state_dir.join("nodes.json").exists() {
        return Err(format!(
            "{} has no copy of a hub's state: run a --standby first",
            args.state_dir.display()
        )
        .into());
    }
    if !args.force {
        crate::replica::is_down(url)
            .await
            .map_err(|why| format!("{why}; stop it first (or --force)"))?;
    }
    Ok(())
}

async fn serve(args: Args, listen: SocketAddr) -> Result<(), BoxError> {
    if let Some(url) = &args.promote {
        check_promotion(&args, url).await?;
        rusty_log::warn!("hub: promoted from a standby copy; the old active must stay down");
    }
    std::fs::create_dir_all(&args.state_dir)?;
    let identity = NodeState::load_or_generate(&args.state_dir)?.machine;
    let store = FileStore::open(args.state_dir.join("nodes.json"), args.node_ttl)?;
    let hub = Hub::with_identity(store, Limits::default(), identity);
    if let Some(secret) = secret_of(&args)? {
        hub.enable_replication(&secret)?;
    }
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
    let listener = TcpListener::bind(listen).await?;
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
    if !args.standbys.is_empty() {
        hub.advertise_standbys(args.standbys.clone());
    }
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
    let mut serving = serving;
    rusty_tokio::select! {
        _ = signals(&hub, &args) => {
            serving.abort();
            rusty_log::info!("hub: shutting down");
            Ok(())
        }
        // The accept loop only ends on a failure: say so and exit non-zero
        // rather than sit there looking healthy.
        ended = &mut serving => match ended {
            Ok(Err(e)) => Err(format!("the listener failed: {e}").into()),
            Ok(Ok(())) => Err("the listener stopped".into()),
            Err(e) => Err(format!("the listener task failed: {e}").into()),
        },
    }
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

/// Applies the keys file and the policy file. Both are read and checked
/// first and the keys are applied all or nothing, so a bad file of either
/// kind leaves the old rules whole.
fn reload(
    hub: &Hub<FileStore>,
    keys: Option<&Path>,
    policy: Option<&Path>,
) -> Result<(), BoxError> {
    let policy = match policy {
        Some(path) => Some(Policy::parse(&std::fs::read_to_string(path)?)?),
        None => None,
    };
    if let Some(path) = keys {
        hub.load_keys(&read_secret(path)?)?;
    }
    if let Some(policy) = policy {
        hub.set_policy(policy);
    }
    Ok(())
}

/// Reads a file of secrets, refusing one that group or others can read.
fn read_secret(path: &Path) -> Result<String, BoxError> {
    use std::io::Read;
    use std::os::unix::fs::PermissionsExt;
    // Open once and check that handle, so the file cannot be swapped between
    // the check and the read.
    let mut file = std::fs::File::open(path)?;
    if file.metadata()?.permissions().mode() & 0o077 != 0 {
        return Err(format!("{} is readable by others (chmod 600)", path.display()).into());
    }
    let mut text = String::new();
    file.read_to_string(&mut text)?;
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Args, String> {
        match parse_args(args.iter().map(|s| s.to_string()))? {
            Cli::Run(a) => Ok(*a),
            Cli::Help => Err("help".into()),
        }
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
    fn failover_flags_go_together_and_refuse_a_plain_link() {
        let run = |a: &[&str]| parse(a).map(drop);
        // A standby serves nothing and needs the secret and a safe link.
        assert!(run(&["--standby", "https://hub:1", "--replica-secret", "s"]).is_ok());
        assert!(run(&["--standby", "http://127.0.0.1:1", "--replica-secret", "s"]).is_ok());
        assert!(run(&["--standby", "http://hub.example:1", "--replica-secret", "s"]).is_err());
        assert!(run(&["--standby", "https://hub:1"]).is_err());
        assert!(
            run(&[
                "--standby",
                "https://hub:1",
                "--replica-secret",
                "s",
                "--listen",
                "127.0.0.1:0"
            ])
            .is_err()
        );
        // Promotion serves, so it needs --listen; --force only goes with it.
        assert!(run(&["--promote", "https://hub:1"]).is_err());
        assert!(
            run(&[
                "--listen",
                "127.0.0.1:0",
                "--promote",
                "https://hub:1",
                "--force"
            ])
            .is_ok()
        );
        assert!(run(&["--listen", "127.0.0.1:0", "--force"]).is_err());
        // Nodes send their pre-auth key to advertised standbys.
        let adv = |u: &'static str| run(&["--listen", "127.0.0.1:0", "--advertise-standby", u]);
        assert!(adv("https://standby.example:8443").is_ok());
        assert!(adv("http://127.0.0.1:9").is_ok());
        assert!(adv("http://standby.example:9").is_err());
        assert!(
            run(&[
                "--listen",
                "127.0.0.1:0",
                "--standby",
                "https://h:1",
                "--promote",
                "https://h:1"
            ])
            .is_err()
        );
        // Promotion does not talk TLS to the old active, so a CA would be ignored.
        assert!(
            run(&[
                "--listen",
                "127.0.0.1:0",
                "--promote",
                "https://hub:1",
                "--tls-ca",
                "ca"
            ])
            .is_err()
        );
        assert!(
            run(&[
                "--standby",
                "https://hub:1",
                "--replica-secret",
                "s",
                "--tls-ca",
                "ca"
            ])
            .is_ok()
        );
        // The secret never goes over plain HTTP off this host.
        assert!(run(&["--listen", "0.0.0.0:80", "--replica-secret", "s"]).is_err());
        assert!(run(&["--listen", "127.0.0.1:80", "--replica-secret", "s"]).is_ok());
        assert!(
            run(&[
                "--listen",
                "0.0.0.0:443",
                "--tls-cert",
                "c",
                "--tls-key",
                "k",
                "--replica-secret",
                "s"
            ])
            .is_ok()
        );
    }

    #[test]
    fn help_is_not_an_error() {
        let cli = parse_args(["--help".to_string()].into_iter());
        assert!(matches!(cli, Ok(Cli::Help)));
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
