/// Where core is and who the client is, as RLBot passes them to a launched process.
///
/// | Variable | Meaning | Fallback |
/// |---|---|---|
/// | `RLBOT_SERVER_ADDR` | full `host:port` of core | built from the two below |
/// | `RLBOT_SERVER_IP` | core's host | `127.0.0.1` |
/// | `RLBOT_SERVER_PORT` | core's port | `23234` |
/// | `RLBOT_AGENT_ID` | this client's id | none (the caller picks one) |
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Environment {
    pub server_addr: String,
    pub agent_id: Option<String>,
}

impl Environment {
    /// Reads the process environment.
    pub fn from_env() -> Environment {
        Environment::from_lookup(|name| std::env::var(name).ok())
    }

    /// Reads variables through `get`; empty values count as unset.
    pub fn from_lookup(get: impl Fn(&str) -> Option<String>) -> Environment {
        let get = |name: &str| get(name).filter(|v| !v.is_empty());
        let server_addr = get("RLBOT_SERVER_ADDR").unwrap_or_else(|| {
            format!(
                "{}:{}",
                get("RLBOT_SERVER_IP").unwrap_or_else(|| "127.0.0.1".into()),
                get("RLBOT_SERVER_PORT").unwrap_or_else(|| "23234".into()),
            )
        });
        Environment {
            server_addr,
            agent_id: get("RLBOT_AGENT_ID"),
        }
    }
}
