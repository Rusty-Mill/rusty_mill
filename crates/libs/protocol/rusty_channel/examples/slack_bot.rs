//! A Slack bot in front of any AG-UI agent.
//!
//! ```text
//! SLACK_SIGNING_SECRET=… SLACK_BOT_TOKEN=xoxb-… AGENT_URL=http://127.0.0.1:8080/api/agent \
//!   cargo run -p rusty_channel --features bot --example slack_bot
//! ```
//!
//! Optional: `BIND` (default `127.0.0.1:3100`), `AGENT_TOKEN` (sent as a
//! bearer token to the agent). Point the Slack app's Events API request
//! URL at `/slack/events` (through a public tunnel) and subscribe it to
//! `app_mention` and `message.im`.

use rusty_channel::bot::Bot;
use rusty_channel::slack::Slack;

fn main() -> Result<(), String> {
    let slack = Slack::new(
        common::env("SLACK_SIGNING_SECRET")?,
        common::env("SLACK_BOT_TOKEN")?,
    );
    Bot::new(slack, common::agent()?, "/slack/events")?.serve(common::bind()?)
}

mod common;
