//! Example MCP server built on `rusty_mcp_server`.
//!
//! ```text
//! cargo run -p rusty-mcp-demo                                        # stdio
//! cargo run -p rusty-mcp-demo -- --transport http --bind 127.0.0.1:8080
//! ```
//!
//! The server itself is in [`demo`]; this file is the command line.

mod demo;

use rusty_mcp_server::{HttpConfig, bind_http, serve_stdio};
use rusty_serve::Limits;
use std::net::SocketAddr;
use std::process::ExitCode;
use std::sync::Arc;

const USAGE: &str = "\
Usage: rusty-mcp-demo [OPTIONS]

  --transport stdio|http   what to serve on        [MCP_TRANSPORT, default stdio]
  --bind ADDR              HTTP address            [MCP_BIND, default 127.0.0.1:8080]
  --path PATH              HTTP endpoint path      [MCP_PATH, default /mcp]
  --allowed-host HOST      accepted Host, repeat   [MCP_ALLOWED_HOSTS, comma list; default loopback]
  --allowed-origin ORIGIN  accepted Origin, repeat [MCP_ALLOWED_ORIGINS, comma list; default none]
  --max-body-bytes N       request body cap        [MCP_MAX_BODY_BYTES, default 4194304]
  -h, --help               this text
";

#[derive(Debug, PartialEq, Eq)]
enum Transport {
    Stdio,
    Http,
}

#[derive(Debug, PartialEq, Eq)]
struct Args {
    transport: Transport,
    bind: SocketAddr,
    path: String,
    allowed_hosts: Option<Vec<String>>,
    allowed_origins: Option<Vec<String>>,
    max_body_bytes: u64,
}

fn list(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Read the command line over the environment's defaults. `Ok(None)` is
/// `--help`.
fn parse(
    args: impl IntoIterator<Item = String>,
    env: impl Fn(&str) -> Option<String>,
) -> Result<Option<Args>, String> {
    let mut transport = env("MCP_TRANSPORT").unwrap_or_else(|| "stdio".to_owned());
    let mut bind = env("MCP_BIND").unwrap_or_else(|| "127.0.0.1:8080".to_owned());
    let mut path = env("MCP_PATH").unwrap_or_else(|| "/mcp".to_owned());
    let mut hosts = env("MCP_ALLOWED_HOSTS").map(|v| list(&v));
    let mut origins = env("MCP_ALLOWED_ORIGINS").map(|v| list(&v));
    let mut max_body = env("MCP_MAX_BODY_BYTES").unwrap_or_else(|| "4194304".to_owned());
    let mut it = args.into_iter();
    // A repeated flag adds to what the environment gave only once: the
    // command line replaces the environment, then accumulates.
    let (mut hosts_set, mut origins_set) = (false, false);
    while let Some(flag) = it.next() {
        if flag == "-h" || flag == "--help" {
            return Ok(None);
        }
        let mut value = || it.next().ok_or_else(|| format!("{flag} needs a value"));
        match flag.as_str() {
            "--transport" => transport = value()?,
            "--bind" => bind = value()?,
            "--path" => path = value()?,
            "--max-body-bytes" => max_body = value()?,
            "--allowed-host" => {
                let v = value()?;
                if !std::mem::replace(&mut hosts_set, true) {
                    hosts = Some(Vec::new());
                }
                hosts.get_or_insert_with(Vec::new).push(v);
            }
            "--allowed-origin" => {
                let v = value()?;
                if !std::mem::replace(&mut origins_set, true) {
                    origins = Some(Vec::new());
                }
                origins.get_or_insert_with(Vec::new).push(v);
            }
            other => return Err(format!("unknown option {other:?}")),
        }
    }
    Ok(Some(Args {
        transport: match transport.as_str() {
            "stdio" => Transport::Stdio,
            "http" => Transport::Http,
            other => return Err(format!("--transport must be stdio or http, not {other:?}")),
        },
        bind: bind.parse().map_err(|e| format!("--bind {bind:?}: {e}"))?,
        path,
        allowed_hosts: hosts,
        allowed_origins: origins,
        max_body_bytes: max_body
            .parse()
            .map_err(|e| format!("--max-body-bytes {max_body:?}: {e}"))?,
    }))
}

fn run(args: Args) -> Result<(), Box<dyn std::error::Error>> {
    let server = Arc::new(demo::demo_server()?);
    match args.transport {
        Transport::Stdio => serve_stdio(server)?,
        Transport::Http => {
            let mut config = HttpConfig {
                path: args.path,
                ..HttpConfig::default()
            };
            if let Some(hosts) = args.allowed_hosts {
                config.allowed_hosts = hosts;
            }
            if let Some(origins) = args.allowed_origins {
                config.allowed_origins = origins;
            }
            let limits = Limits {
                max_body_bytes: args.max_body_bytes,
                ..Limits::default()
            };
            let http = bind_http(server, args.bind, config, limits)?;
            eprintln!("rusty-mcp-demo: listening on {}", http.local_addr()?);
            http.run()?;
        }
    }
    Ok(())
}

fn main() -> ExitCode {
    let args = match parse(std::env::args().skip(1), |k| std::env::var(k).ok()) {
        Ok(Some(args)) => args,
        Ok(None) => {
            print!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Err(why) => {
            eprintln!("rusty-mcp-demo: {why}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    match run(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(why) => {
            eprintln!("rusty-mcp-demo: {why}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn parse_with(args: &[&str], env: &[(&str, &str)]) -> Result<Option<Args>, String> {
        parse(args.iter().map(|a| (*a).to_owned()), |k| {
            env.iter()
                .find(|(n, _)| *n == k)
                .map(|(_, v)| (*v).to_owned())
        })
    }

    #[test]
    fn defaults_are_stdio_on_loopback() {
        let a = parse_with(&[], &[]).unwrap().unwrap();
        assert_eq!(a.transport, Transport::Stdio);
        assert_eq!(a.bind, "127.0.0.1:8080".parse().unwrap());
        assert_eq!(a.path, "/mcp");
        assert_eq!((a.allowed_hosts, a.allowed_origins), (None, None));
        assert_eq!(a.max_body_bytes, 4 * 1024 * 1024);
    }

    #[test]
    fn the_environment_fills_in_and_flags_override_it() {
        let env = [
            ("MCP_TRANSPORT", "http"),
            ("MCP_BIND", "0.0.0.0:9000"),
            ("MCP_ALLOWED_HOSTS", "a.example, b.example"),
        ];
        let a = parse_with(&[], &env).unwrap().unwrap();
        assert_eq!(a.transport, Transport::Http);
        assert_eq!(a.bind.port(), 9000);
        assert_eq!(a.allowed_hosts.unwrap(), ["a.example", "b.example"]);
        let a = parse_with(
            &["--transport", "stdio", "--allowed-host", "c.example"],
            &env,
        )
        .unwrap()
        .unwrap();
        assert_eq!(a.transport, Transport::Stdio);
        assert_eq!(a.allowed_hosts.unwrap(), ["c.example"], "flags replace env");
    }

    #[test]
    fn repeated_flags_accumulate() {
        let a = parse_with(
            &[
                "--allowed-origin",
                "https://a",
                "--allowed-origin",
                "https://b",
            ],
            &[],
        )
        .unwrap()
        .unwrap();
        assert_eq!(a.allowed_origins.unwrap(), ["https://a", "https://b"]);
    }

    #[test]
    fn help_is_not_an_error() {
        assert_eq!(parse_with(&["--help"], &[]), Ok(None));
    }

    #[test]
    fn bad_input_is_refused_with_a_reason() {
        for (args, needle) in [
            (&["--nope"][..], "unknown option"),
            (&["--bind"], "needs a value"),
            (&["--transport", "carrier-pigeon"], "stdio or http"),
            (&["--bind", "not-an-address"], "--bind"),
            (&["--max-body-bytes", "-1"], "--max-body-bytes"),
        ] {
            let why = parse_with(args, &[]).unwrap_err();
            assert!(why.contains(needle), "{args:?}: {why}");
        }
    }
}
