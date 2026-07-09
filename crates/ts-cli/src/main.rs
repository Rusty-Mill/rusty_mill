//! `ts-cli`: talk to a running tailscaled (official or, later, ts-daemon)
//! over its LocalAPI Unix socket.
//!
//! Phase-1 command surface: `status [--json]`, `up`, `down`,
//! `ping <tailscale-ip>`. Argument parsing is hand-rolled (four subcommands
//! don't justify a dependency; see DESIGN.md).

mod localapi;
mod render;

use std::net::IpAddr;
use std::path::PathBuf;
use std::process::ExitCode;

use localapi::{DEFAULT_SOCKET, LocalApi};

const USAGE: &str = "\
usage: ts-cli [--socket <path>] <command> [flags]

commands:
  status [--json]   show tailnet state and peers
  up                set the backend to running (WantRunning=true)
  down              stop the backend (WantRunning=false)
  ping <ip>         disco-ping a peer by Tailscale IP

--socket defaults to /var/run/tailscale/tailscaled.sock";

enum Command {
    Status { json: bool },
    Up,
    Down,
    Ping { ip: IpAddr },
}

struct Args {
    socket: PathBuf,
    command: Command,
}

fn parse_args(argv: &[String]) -> Result<Args, String> {
    let mut socket = PathBuf::from(DEFAULT_SOCKET);
    let mut it = argv.iter().peekable();
    while let Some(arg) = it.peek() {
        match arg.as_str() {
            "--socket" => {
                it.next();
                let path = it.next().ok_or("--socket requires a path")?;
                socket = PathBuf::from(path);
            }
            "-h" | "--help" => return Err(String::new()),
            _ => break,
        }
    }
    let cmd = it.next().ok_or("missing command")?;
    let rest: Vec<&String> = it.collect();
    let command = match cmd.as_str() {
        "status" => match rest.as_slice() {
            [] => Command::Status { json: false },
            [flag] if flag.as_str() == "--json" => Command::Status { json: true },
            _ => return Err("status accepts only --json".into()),
        },
        "up" | "down" => {
            if !rest.is_empty() {
                return Err(format!("{cmd} takes no arguments"));
            }
            if cmd == "up" {
                Command::Up
            } else {
                Command::Down
            }
        }
        "ping" => match rest.as_slice() {
            [ip] => Command::Ping {
                ip: ip
                    .parse()
                    .map_err(|_| format!("invalid IP address {ip:?}"))?,
            },
            _ => return Err("usage: ping <tailscale-ip>".into()),
        },
        other => return Err(format!("unknown command {other:?}")),
    };
    Ok(Args { socket, command })
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let args = match parse_args(&argv) {
        Ok(args) => args,
        Err(msg) => {
            if !msg.is_empty() {
                eprintln!("ts-cli: {msg}\n");
            }
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };

    let api = LocalApi::new(args.socket);
    let result = match args.command {
        Command::Status { json } => cmd_status(&api, json).await,
        Command::Up => cmd_set_running(&api, true).await,
        Command::Down => cmd_set_running(&api, false).await,
        Command::Ping { ip } => cmd_ping(&api, ip).await,
    };
    match result {
        Ok(code) => code,
        Err(err) => {
            eprintln!("ts-cli: {err}");
            ExitCode::FAILURE
        }
    }
}

async fn cmd_status(api: &LocalApi, json: bool) -> Result<ExitCode, localapi::Error> {
    if json {
        // Byte-faithful passthrough of tailscaled's own JSON.
        let raw = api.status_raw().await?;
        let mut out = String::from_utf8_lossy(&raw).into_owned();
        if !out.ends_with('\n') {
            out.push('\n');
        }
        print!("{out}");
        return Ok(ExitCode::SUCCESS);
    }
    let st = api.status().await?;
    print!("{}", render::status(&st));
    // Mirror the Go CLI: non-Running state is a nonzero exit.
    Ok(if st.backend_state == "Running" {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

async fn cmd_set_running(api: &LocalApi, want: bool) -> Result<ExitCode, localapi::Error> {
    let masked = ts_types::MaskedPrefs {
        want_running: Some(want),
    };
    api.edit_prefs(&masked).await?;
    let st = api.status().await?;
    match st.backend_state.as_str() {
        "Running" if want => {}
        "Stopped" if !want => {}
        "NeedsLogin" => {
            eprintln!(
                "ts-cli: backend needs login{}",
                if st.auth_url.is_empty() {
                    String::new()
                } else {
                    format!("; visit {}", st.auth_url)
                }
            );
            return Ok(ExitCode::FAILURE);
        }
        other => eprintln!("ts-cli: backend state: {other}"),
    }
    Ok(ExitCode::SUCCESS)
}

async fn cmd_ping(api: &LocalApi, ip: IpAddr) -> Result<ExitCode, localapi::Error> {
    let pr = api.ping(ip).await?;
    if !pr.err.is_empty() {
        eprintln!("ping {ip} failed: {}", pr.err);
        return Ok(ExitCode::FAILURE);
    }
    println!("{}", render::pong(&pr));
    Ok(ExitCode::SUCCESS)
}
