//! CLI for rusty-croc, mirroring croc's subcommands.
//!
//! `relay`, `ping`, `send`, and `receive` are functional and wire-compatible
//! with stock croc v10, including the local-network path, reconnect-and-resume,
//! zip/text/stdin sending, throttling, proxies, and `--git` (see MIGRATION.md
//! for what remains).

use clap::{Args, Parser, Subcommand};
use rusty_croc::{croc, models, tcp, utils};

#[derive(Parser)]
#[command(
    name = "rusty-croc",
    version,
    about = "Rust port of croc — securely transfer things from one computer to another"
)]
struct Cli {
    /// Toggle debug logging
    #[arg(long, global = true)]
    debug: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Args, Clone)]
struct TransferFlags {
    /// Address of the relay
    #[arg(long)]
    relay: Option<String>,
    /// Password for the relay
    #[arg(long)]
    pass: Option<String>,
    /// Curve to use for PAKE (p256, p384, p521, siec)
    #[arg(long)]
    curve: Option<String>,
    /// Disable compression
    #[arg(long)]
    no_compress: bool,
    /// Disable multiplexing (use a single transfer connection)
    #[arg(long)]
    no_multi: bool,
    /// Force local-network-only transfer
    #[arg(long)]
    local: bool,
    /// Disable the local-network path
    #[arg(long)]
    no_local: bool,
    /// Save these settings (relay, pass, curve) for future runs
    #[arg(long)]
    remember: bool,
    /// SOCKS5 proxy for non-local relays, e.g. 127.0.0.1:9050 (or $SOCKS5_PROXY)
    #[arg(long)]
    socks5: Option<String>,
    /// HTTP CONNECT proxy for non-local relays (or $HTTP_PROXY)
    #[arg(long)]
    connect: Option<String>,
    /// Resolve the relay hostname via public DNS servers (for broken/censored
    /// local DNS)
    #[arg(long)]
    internal_dns: bool,
}

impl TransferFlags {
    fn socks5_proxy(&self) -> String {
        self.socks5
            .clone()
            .or_else(|| std::env::var("SOCKS5_PROXY").ok())
            .unwrap_or_default()
    }
    fn http_proxy(&self) -> String {
        self.connect
            .clone()
            .or_else(|| std::env::var("HTTP_PROXY").ok())
            .unwrap_or_default()
    }
}

/// Settings persisted by `--remember` (our own file, so stock croc's config
/// is never touched).
#[derive(serde::Serialize, serde::Deserialize, Default)]
struct RememberedConfig {
    relay: Option<String>,
    pass: Option<String>,
    curve: Option<String>,
}

fn config_path(kind: &str) -> std::path::PathBuf {
    let base = std::env::var("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            std::path::PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into()))
                .join(".config")
        });
    base.join("rusty-croc").join(format!("{kind}.json"))
}

/// Resolve relay/pass/curve from flags > remembered config > defaults, and
/// persist them when --remember was given.
fn resolve_flags(flags: &TransferFlags, kind: &str) -> (String, String, String) {
    let remembered: RememberedConfig = std::fs::read(config_path(kind))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    let relay = flags
        .relay
        .clone()
        .or(remembered.relay)
        .unwrap_or_else(|| format!("{}:{}", models::DEFAULT_RELAY, models::DEFAULT_PORT));
    let pass = flags
        .pass
        .clone()
        .or(remembered.pass)
        .unwrap_or_else(|| models::DEFAULT_PASSPHRASE.to_string());
    let curve = flags
        .curve
        .clone()
        .or(remembered.curve)
        .unwrap_or_else(|| "p256".to_string());
    if flags.remember {
        let cfg = RememberedConfig {
            relay: Some(relay.clone()),
            pass: Some(pass.clone()),
            curve: Some(curve.clone()),
        };
        let path = config_path(kind);
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(b) = serde_json::to_vec_pretty(&cfg) {
            if std::fs::write(&path, b).is_ok() {
                eprintln!("Saved settings to {}", path.display());
            }
        }
    }
    (relay, pass, curve)
}

#[derive(Subcommand)]
enum Command {
    /// Send file(s), folder, or text
    Send {
        /// Files or folders to send ("-" reads stdin)
        files: Vec<String>,
        /// Codephrase used to connect to relay (min 6 characters)
        #[arg(long)]
        code: Option<String>,
        /// Hash algorithm (xxhash, imohash, highway, md5)
        #[arg(long, default_value = "xxhash")]
        hash: String,
        /// Send some text instead of a file
        #[arg(long, short = 't')]
        text: Option<String>,
        /// Zip each folder into a single archive before sending
        #[arg(long)]
        zip: bool,
        /// Throttle the upload speed, e.g. 500K, 10M, 1G
        #[arg(long, default_value = "")]
        throttle: String,
        /// Show the receive code as a QR code
        #[arg(long, alias = "qrcode")]
        qr: bool,
        /// Exclude files whose path contains any of these comma-separated strings
        #[arg(long, default_value = "")]
        exclude: String,
        /// Respect .gitignore (don't send ignored files)
        #[arg(long)]
        git: bool,
        #[command(flatten)]
        flags: TransferFlags,
    },
    /// Receive file(s) or folder
    #[command(alias = "recv")]
    Receive {
        /// Codephrase (or set CROC_SECRET)
        code: Option<String>,
        /// Accept file transfer without prompting
        #[arg(long)]
        yes: bool,
        /// Overwrite existing files without prompting
        #[arg(long)]
        overwrite: bool,
        /// Connect directly to this sender address (ip:port), skipping discovery
        #[arg(long, default_value = "")]
        ip: String,
        #[command(flatten)]
        flags: TransferFlags,
    },
    /// Start your own relay (compatible with stock croc clients)
    Relay {
        /// Host of the relay
        #[arg(long, default_value = "")]
        host: String,
        /// Ports of the relay (first is the main port, the rest are
        /// advertised as transfer ports)
        #[arg(long, default_value = "9009,9010,9011,9012,9013")]
        ports: String,
        /// Password to access the relay
        #[arg(long, default_value = models::DEFAULT_PASSPHRASE)]
        pass: String,
        /// TEST ONLY: sever all piped connections once this many bytes have
        /// crossed the relay (simulates a network blip for reconnect tests)
        #[arg(long, default_value_t = 0, hide = true)]
        test_sever_after: u64,
    },
    /// Check that a relay is reachable (sends ping, expects pong)
    Ping {
        /// Relay address, e.g. localhost:9009
        address: String,
    },
}

fn main() {
    let cli = Cli::parse();
    env_logger::Builder::from_default_env()
        .filter_level(if cli.debug {
            log::LevelFilter::Debug
        } else {
            log::LevelFilter::Info
        })
        .init();

    let result: Result<(), Box<dyn std::error::Error>> = match cli.command {
        Command::Send {
            files,
            code,
            hash,
            text,
            zip,
            throttle,
            qr,
            exclude,
            git,
            flags,
        } => send_command(
            files, code, hash, text, zip, throttle, qr, exclude, git, flags,
        ),
        Command::Receive {
            code,
            yes,
            overwrite,
            ip,
            flags,
        } => {
            let mut secret =
                code.or_else(|| std::env::var("CROC_SECRET").ok().filter(|s| !s.is_empty()));
            if secret.is_none() {
                // Interactive prompt, like croc's "Enter receive code:"
                use std::io::IsTerminal;
                if std::io::stdin().is_terminal() {
                    eprint!("Enter receive code: ");
                    let mut line = String::new();
                    if std::io::stdin().read_line(&mut line).is_ok() {
                        let line = line.trim().to_string();
                        if !line.is_empty() {
                            secret = Some(line);
                        }
                    }
                }
            }
            match secret {
                None => Err("enter a code (argument or CROC_SECRET env var)".into()),
                Some(s) if s.len() < 6 => {
                    Err("code is too short (must be at least 6 characters)".into())
                }
                Some(s) => {
                    let (relay, pass, curve) = resolve_flags(&flags, "receive");
                    let opts = croc::Options {
                        is_sender: false,
                        shared_secret: s,
                        relay_address: relay,
                        relay_password: pass,
                        curve,
                        no_prompt: yes,
                        overwrite,
                        no_compress: flags.no_compress,
                        no_multiplexing: flags.no_multi,
                        disable_local: flags.no_local,
                        only_local: flags.local,
                        ip,
                        socks5_proxy: flags.socks5_proxy(),
                        http_proxy: flags.http_proxy(),
                        internal_dns: flags.internal_dns,
                        ..Default::default()
                    };
                    croc::Client::receive(opts).map_err(|e| e as Box<dyn std::error::Error>)
                }
            }
        }
        Command::Relay {
            host,
            ports,
            pass,
            test_sever_after,
        } => relay(&host, &ports, &pass, test_sever_after),
        Command::Ping { address } => tcp::ping_server(&address).map(|()| {
            println!("pong from {address}");
        }),
    };
    if let Err(e) = result {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

#[allow(clippy::too_many_arguments)]
fn send_command(
    mut files: Vec<String>,
    code: Option<String>,
    hash: String,
    text: Option<String>,
    zip: bool,
    throttle: String,
    qr: bool,
    exclude: String,
    git: bool,
    flags: TransferFlags,
) -> Result<(), Box<dyn std::error::Error>> {
    let secret = match code.or_else(|| std::env::var("CROC_SECRET").ok().filter(|s| !s.is_empty()))
    {
        Some(c) => c,
        None => utils::get_random_name(),
    };
    if secret.len() < 6 {
        return Err("code is too short (must be at least 6 characters)".into());
    }
    if qr {
        match qrcode::QrCode::new(secret.as_bytes()) {
            Ok(code) => eprintln!(
                "{}",
                code.render::<qrcode::render::unicode::Dense1x2>()
                    .quiet_zone(true)
                    .build()
            ),
            Err(e) => log::debug!("could not render QR code: {e}"),
        }
    }

    // --text and stdin ("-") become a temp file, like croc's croc-stdin-*.
    let mut temp_path: Option<std::path::PathBuf> = None;
    let sending_text = text.is_some();
    if let Some(t) = text {
        let p = std::path::PathBuf::from(format!("croc-stdin-{}", std::process::id()));
        std::fs::write(&p, t)?;
        files = vec![p.to_string_lossy().to_string()];
        temp_path = Some(p);
    } else if files.len() == 1 && files[0] == "-" {
        use std::io::Read;
        let mut buf = Vec::new();
        std::io::stdin().read_to_end(&mut buf)?;
        let p = std::path::PathBuf::from(format!("croc-stdin-{}", std::process::id()));
        std::fs::write(&p, buf)?;
        files = vec![p.to_string_lossy().to_string()];
        temp_path = Some(p);
    }
    if files.is_empty() {
        return Err("provide files/folders to send, --text, or '-' for stdin".into());
    }

    let (relay, pass, curve) = resolve_flags(&flags, "send");
    let exclude: Vec<String> = exclude
        .split(',')
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty())
        .collect();
    let opts = croc::Options {
        is_sender: true,
        shared_secret: secret,
        relay_address: relay,
        relay_password: pass,
        curve,
        hash_algorithm: hash,
        no_compress: flags.no_compress,
        no_multiplexing: flags.no_multi,
        disable_local: flags.no_local,
        only_local: flags.local,
        throttle_upload: throttle,
        sending_text,
        zip_folder: zip,
        exclude,
        git_ignore: git,
        socks5_proxy: flags.socks5_proxy(),
        http_proxy: flags.http_proxy(),
        internal_dns: flags.internal_dns,
        ..Default::default()
    };
    let result = croc::Client::send(opts, &files).map_err(|e| e as Box<dyn std::error::Error>);
    if let Some(p) = temp_path {
        let _ = std::fs::remove_file(p);
    }
    result
}

fn relay(
    host: &str,
    ports: &str,
    pass: &str,
    test_sever_after: u64,
) -> Result<(), Box<dyn std::error::Error>> {
    let ports: Vec<String> = ports
        .split(',')
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
        .collect();
    if ports.len() < 2 {
        return Err("relay requires at least two ports (main + transfer)".into());
    }
    log::info!(
        "starting rusty-croc relay v{} on ports {}",
        env!("CARGO_PKG_VERSION"),
        ports.join(",")
    );
    let sever =
        (test_sever_after > 0).then(|| std::sync::Arc::new(tcp::SeverState::new(test_sever_after)));
    let transfer_ports = ports[1..].join(",");
    for port in &ports[1..] {
        let host = host.to_string();
        let port = port.clone();
        let pass = pass.to_string();
        let sever = sever.clone();
        std::thread::spawn(move || {
            let mut server = tcp::RelayServer::new(&host, &port, &pass, "");
            if let Some(s) = sever {
                server = server.with_test_sever(s);
            }
            if let Err(e) = server.run() {
                log::error!("relay port {port} failed: {e}");
                std::process::exit(1);
            }
        });
    }
    let mut server = tcp::RelayServer::new(host, &ports[0], pass, &transfer_ports);
    if let Some(s) = sever {
        server = server.with_test_sever(s);
    }
    server.run()?;
    Ok(())
}
