//! A Microsoft Teams bot in front of any AG-UI agent.
//!
//! ```text
//! TEAMS_APP_ID=… TEAMS_APP_PASSWORD=… AGENT_URL=http://127.0.0.1:8080/api/agent \
//!   cargo run -p rusty_channel --features bot --example teams_bot
//! ```
//!
//! Optional: `BIND` (default `127.0.0.1:3100`), `AGENT_TOKEN` (sent as a
//! bearer token to the agent). In Azure: a Bot resource with the Teams
//! channel enabled and its messaging endpoint pointed at `/teams/messages`
//! (through a public tunnel). The Bot Framework's signing keys are fetched
//! once at start; restart the bot when they rotate.

use rusty_channel::bot::{self, Bot};
use rusty_channel::teams::{self, Teams};
use rusty_oauth::jwks::JwkSet;
use rusty_tls::{TlsConnector, TrustPolicy};

fn main() -> Result<(), String> {
    let tls = TlsConnector::new(&TrustPolicy::System).map_err(|e| e.to_string())?;
    let (status, body) = bot::get(&tls, teams::KEYS_URL)?;
    if status != 200 {
        return Err(format!("fetching {}: {status}", teams::KEYS_URL));
    }
    let keys = JwkSet::parse(&String::from_utf8_lossy(&body)).map_err(|e| e.to_string())?;
    let teams = Teams::new(
        common::env("TEAMS_APP_ID")?,
        common::env("TEAMS_APP_PASSWORD")?,
        keys,
    );
    Bot::new(teams, common::agent()?, "/teams/messages")?.serve(common::bind()?)
}

mod common;
