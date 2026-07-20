//! CLI for rusty-croc, mirroring croc's subcommands.
//!
//! `relay`, `ping`, `send`, and `receive` are functional and wire-compatible
//! with stock croc v10. Not yet ported: local-network discovery, reconnect,
//! zip-folder mode, throttling, proxies (see MIGRATION.md).

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
    #[arg(long, default_value_t = format!("{}:{}", models::DEFAULT_RELAY, models::DEFAULT_PORT))]
    relay: String,
    /// Password for the relay
    #[arg(long, default_value = models::DEFAULT_PASSPHRASE)]
    pass: String,
    /// Curve to use for PAKE (p256, p384, p521, siec)
    #[arg(long, default_value = "p256")]
    curve: String,
    /// Disable compression
    #[arg(long)]
    no_compress: bool,
    /// Disable multiplexing (use a single transfer connection)
    #[arg(long)]
    no_multi: bool,
}

#[derive(Subcommand)]
enum Command {
    /// Send file(s) or folder
    Send {
        /// Files or folders to send
        #[arg(required = true)]
        files: Vec<String>,
        /// Codephrase used to connect to relay (min 6 characters)
        #[arg(long)]
        code: Option<String>,
        /// Hash algorithm (xxhash, md5)
        #[arg(long, default_value = "xxhash")]
        hash: String,
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
            flags,
        } => {
            let secret = match code.or_else(|| std::env::var("CROC_SECRET").ok().filter(|s| !s.is_empty())) {
                Some(c) => c,
                None => utils::get_random_name(),
            };
            if secret.len() < 6 {
                Err("code is too short (must be at least 6 characters)".into())
            } else {
                let opts = croc::Options {
                    is_sender: true,
                    shared_secret: secret,
                    relay_address: flags.relay,
                    relay_password: flags.pass,
                    curve: flags.curve,
                    hash_algorithm: hash,
                    no_compress: flags.no_compress,
                    no_multiplexing: flags.no_multi,
                    ..Default::default()
                };
                croc::Client::send(opts, &files).map_err(|e| e as Box<dyn std::error::Error>)
            }
        }
        Command::Receive {
            code,
            yes,
            overwrite,
            flags,
        } => {
            let secret = code.or_else(|| std::env::var("CROC_SECRET").ok().filter(|s| !s.is_empty()));
            match secret {
                None => Err("enter a code (argument or CROC_SECRET env var)".into()),
                Some(s) if s.len() < 6 => {
                    Err("code is too short (must be at least 6 characters)".into())
                }
                Some(s) => {
                    let opts = croc::Options {
                        is_sender: false,
                        shared_secret: s,
                        relay_address: flags.relay,
                        relay_password: flags.pass,
                        curve: flags.curve,
                        no_prompt: yes,
                        overwrite,
                        no_compress: flags.no_compress,
                        no_multiplexing: flags.no_multi,
                        ..Default::default()
                    };
                    croc::Client::receive(opts).map_err(|e| e as Box<dyn std::error::Error>)
                }
            }
        }
        Command::Relay { host, ports, pass } => relay(&host, &ports, &pass),
        Command::Ping { address } => tcp::ping_server(&address).map(|()| {
            println!("pong from {address}");
        }),
    };
    if let Err(e) = result {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

fn relay(host: &str, ports: &str, pass: &str) -> Result<(), Box<dyn std::error::Error>> {
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
    let transfer_ports = ports[1..].join(",");
    for port in &ports[1..] {
        let host = host.to_string();
        let port = port.clone();
        let pass = pass.to_string();
        std::thread::spawn(move || {
            if let Err(e) = tcp::RelayServer::new(&host, &port, &pass, "").run() {
                log::error!("relay port {port} failed: {e}");
                std::process::exit(1);
            }
        });
    }
    tcp::RelayServer::new(host, &ports[0], pass, &transfer_ports).run()?;
    Ok(())
}
