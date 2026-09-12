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

use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use allowlist::{Allowlist, AllowlistConfig};
use clap::Parser;
use config_files::ConfigStore;
use dnf::DnfController;
use http::AgentState;
use platform::process::Spawner;
use platform_linux::LinuxSpawner;
use systemd::SystemdAdapter;

/// Unprivileged local agent for scoped systemd/dnf/config-file control.
#[derive(Debug, Parser)]
struct Cli {
    /// Address to bind the HTTP API to. Must be a private/Tailscale
    /// address reachable only from wherever `rusty_homelab_mcp` runs --
    /// never `0.0.0.0` (see README.md). Enforced at startup by
    /// [`validate_bind_addr`] unless `--i-know-what-im-doing` is passed.
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

    /// Overrides the `bind` safety check: allows binding to an
    /// unspecified (`0.0.0.0`/`::`) or public address. This agent has no
    /// authentication of its own -- network reachability is the only
    /// access control it has -- so this is a deliberate, explicit opt-in,
    /// not a default anyone should reach for.
    #[arg(long, default_value_t = false)]
    i_know_what_im_doing: bool,
}

/// Whether `ip` is safe to bind this agent's unauthenticated HTTP API to
/// without an explicit override: loopback, or one of the private ranges
/// `rusty_homelab_mcp` is expected to reach it from (RFC 1918 IPv4,
/// Tailscale's `100.64.0.0/10` CGNAT range, or IPv6 unique-local
/// `fc00::/7`). Never an unspecified address (`0.0.0.0`/`::`) or anything
/// publicly routable.
fn is_safe_bind_ip(ip: IpAddr) -> bool {
    if ip.is_loopback() {
        return true;
    }
    if ip.is_unspecified() {
        return false;
    }
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            o[0] == 10
                || (o[0] == 172 && (16..=31).contains(&o[1]))
                || (o[0] == 192 && o[1] == 168)
                || (o[0] == 100 && (64..=127).contains(&o[1]))
        }
        IpAddr::V6(v6) => (v6.segments()[0] & 0xfe00) == 0xfc00,
    }
}

/// Enforces this agent's documented bind-address invariant (see
/// `README.md`): `bind` must parse as a socket address whose IP is
/// loopback or private/Tailscale, unless `override_check` (the
/// `--i-know-what-im-doing` flag) is set. Returns an error message ready
/// to print to stderr on failure.
fn validate_bind_addr(bind: &str, override_check: bool) -> Result<(), String> {
    if override_check {
        return Ok(());
    }
    let addr: SocketAddr = bind
        .parse()
        .map_err(|err| format!("invalid --bind address '{bind}': {err}"))?;
    if is_safe_bind_ip(addr.ip()) {
        Ok(())
    } else {
        Err(format!(
            "refusing to bind to '{bind}': this agent has no authentication \
of its own, so the bind address must be loopback or a private/Tailscale \
address (10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16, 100.64.0.0/10, \
fc00::/7) -- never 0.0.0.0, ::, or a public address. Pass \
--i-know-what-im-doing to override this check."
        ))
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    if let Err(err) = validate_bind_addr(&cli.bind, cli.i_know_what_im_doing) {
        eprintln!("{err}");
        return ExitCode::FAILURE;
    }

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unspecified_bind_is_rejected_without_override() {
        assert!(validate_bind_addr("0.0.0.0:8765", false).is_err());
        assert!(validate_bind_addr("[::]:8765", false).is_err());
    }

    #[test]
    fn unspecified_bind_is_accepted_with_override() {
        assert!(validate_bind_addr("0.0.0.0:8765", true).is_ok());
    }

    #[test]
    fn loopback_bind_is_accepted() {
        assert!(validate_bind_addr("127.0.0.1:8765", false).is_ok());
    }

    #[test]
    fn tailscale_cgnat_bind_is_accepted() {
        assert!(validate_bind_addr("100.64.1.2:8765", false).is_ok());
    }

    #[test]
    fn private_rfc1918_binds_are_accepted() {
        assert!(validate_bind_addr("10.0.0.5:8765", false).is_ok());
        assert!(validate_bind_addr("172.16.0.5:8765", false).is_ok());
        assert!(validate_bind_addr("192.168.1.5:8765", false).is_ok());
    }

    #[test]
    fn public_bind_is_rejected() {
        assert!(validate_bind_addr("8.8.8.8:8765", false).is_err());
    }

    #[test]
    fn unparseable_bind_is_rejected() {
        assert!(validate_bind_addr("not-an-address", false).is_err());
    }
}
