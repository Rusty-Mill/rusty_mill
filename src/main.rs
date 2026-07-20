//! CLI for rusty-croc, mirroring croc's subcommands.
//!
//! `relay` is fully functional and wire-compatible with stock croc clients.
//! `send`/`receive` require the file-transfer engine (migration phase 2 —
//! see MIGRATION.md) and currently exit with a pointer to the roadmap.

use clap::{Parser, Subcommand};
use rusty_croc::{models, tcp, utils};

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

#[derive(Subcommand)]
enum Command {
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
    /// Send file(s) or folder (NOT YET PORTED — see MIGRATION.md)
    Send {
        /// Files or folders to send
        files: Vec<String>,
        /// Codephrase used to connect to relay
        #[arg(long)]
        code: Option<String>,
    },
    /// Receive file(s) or folder (NOT YET PORTED — see MIGRATION.md)
    #[command(alias = "recv")]
    Receive {
        /// Codephrase to receive with
        code: Option<String>,
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

    let result = match cli.command {
        Command::Relay { host, ports, pass } => relay(&host, &ports, &pass),
        Command::Send { .. } => not_ported("send"),
        Command::Receive { .. } => not_ported("receive"),
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

fn not_ported(cmd: &str) -> Result<(), Box<dyn std::error::Error>> {
    // Show that codephrase generation already works, then be honest about
    // the state of the port.
    if cmd == "send" {
        eprintln!("(codephrase would be: {})", utils::get_random_name());
    }
    Err(format!(
        "`{cmd}` is not ported yet — the file-transfer engine is migration phase 2 (see MIGRATION.md). The `relay` and `ping` commands are fully functional."
    )
    .into())
}
