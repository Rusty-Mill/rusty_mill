//! Shared by the bot examples: the agent and the bind address from the
//! environment.

use std::net::SocketAddr;

use rusty_agui::HttpAgent;

pub fn env(name: &str) -> Result<String, String> {
    std::env::var(name).map_err(|_| format!("{name} is not set"))
}

/// The agent at `AGENT_URL`, with `AGENT_TOKEN` as a bearer token if set.
pub fn agent() -> Result<HttpAgent, String> {
    let mut agent = HttpAgent::new(&env("AGENT_URL")?).map_err(|e| e.to_string())?;
    if let Ok(token) = std::env::var("AGENT_TOKEN") {
        agent = agent.header("Authorization", &format!("Bearer {token}"));
    }
    Ok(agent)
}

/// `BIND`, default `127.0.0.1:3100`.
pub fn bind() -> Result<SocketAddr, String> {
    let bind = std::env::var("BIND").unwrap_or_else(|_| "127.0.0.1:3100".into());
    bind.parse()
        .map_err(|_| format!("BIND {bind:?} is not an address"))
}
