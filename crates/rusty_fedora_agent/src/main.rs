//! `rusty_fedora_agent`: an unprivileged local agent exposing scoped
//! systemd/dnf/config-file control over a small local HTTP API. Runs on
//! the Fedora/systemd host it manages (e.g. baileyai); `rusty_homelab_mcp`'s
//! `fedora` module is the typed REST client that talks to it. See
//! `README.md` for the API and `deploy/` for the privilege-scoping files
//! (systemd unit, polkit rule, sudoers entry, allowlist config) meant to
//! be reviewed and applied by hand -- this binary does not apply them
//! itself.
//!
//! Linux-only, like the `systemctl`/`journalctl`/`dnf` it shells out to --
//! excluded from `windows-latest` CI in `.github/workflows/ci.yml`
//! (`windows-exclude: rusty_fedora_agent`) rather than cfg-gated into a
//! portable shim, the same treatment `rusty_stream` (built on `io_uring`)
//! already gets there.

mod allowlist;
mod config_files;
mod dnf;
mod domain;
mod error;
mod http;
mod ports;
mod process_util;
mod systemd;

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use clap::Parser;
use platform::process::Spawner;
use platform_linux::LinuxSpawner;

use allowlist::{Allowlist, AllowlistConfig};
use config_files::ConfigStore;
use dnf::DnfController;
use http::AgentState;
use systemd::SystemdAdapter;

/// Unprivileged local agent for scoped systemd/dnf/config-file control.
#[derive(Debug, Parser)]
struct Cli {
    /// Address to bind the HTTP API to. Must be a private/Tailscale
    /// address reachable only from wherever `rusty_homelab_mcp` runs --
    /// never `0.0.0.0` (see README.md).
    #[arg(
        long,
        env = "RUSTY_FEDORA_AGENT_BIND",
        default_value = "127.0.0.1:8765"
    )]
    bind: String,

    /// Path to the allowlist config file: which systemd units, dnf
    /// packages, and config-file path prefixes this agent may act on.
    #[arg(
        long,
        env = "RUSTY_FEDORA_AGENT_ALLOWLIST",
        default_value = "/etc/rusty-fedora-agent/allowlist.toml"
    )]
    allowlist: PathBuf,
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    let allowlist_config = match AllowlistConfig::load(&cli.allowlist) {
        Ok(config) => config,
        Err(err) => {
            eprintln!(
                "failed to load allowlist config from {}: {err}",
                cli.allowlist.display()
            );
            return ExitCode::FAILURE;
        }
    };
    let allowlist = Arc::new(Allowlist::new(allowlist_config));
    let spawner: Arc<dyn Spawner + Send + Sync> = Arc::new(LinuxSpawner);

    let state = AgentState {
        systemd: SystemdAdapter::new(spawner.clone(), allowlist.clone()),
        dnf: DnfController::new(spawner, allowlist.clone()),
        config: ConfigStore::new(allowlist),
    };

    eprintln!("rusty_fedora_agent listening on {}", cli.bind);
    if let Err(err) = http::serve(&cli.bind, state) {
        eprintln!("server error: {err}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
