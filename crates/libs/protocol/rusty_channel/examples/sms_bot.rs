//! An SMS bot (Twilio) in front of any AG-UI agent.
//!
//! ```text
//! TWILIO_ACCOUNT_SID=AC… TWILIO_AUTH_TOKEN=… TWILIO_WEBHOOK_URL=https://bot.example/sms \
//!   AGENT_URL=http://127.0.0.1:8080/api/agent \
//!   cargo run -p rusty_channel --features bot --example sms_bot
//! ```
//!
//! Optional: `BIND` (default `127.0.0.1:3100`), `AGENT_TOKEN` (sent as a
//! bearer token to the agent). In the Twilio console: the number's
//! messaging webhook set to `TWILIO_WEBHOOK_URL` (through a public tunnel),
//! `POST`, whose path is served here at `/sms`. The URL must match what
//! Twilio was given exactly, query string included: it is part of what
//! Twilio signs.

use rusty_channel::bot::Bot;
use rusty_channel::sms::Twilio;

fn main() -> Result<(), String> {
    let twilio = Twilio::new(
        common::env("TWILIO_ACCOUNT_SID")?,
        common::env("TWILIO_AUTH_TOKEN")?,
        common::env("TWILIO_WEBHOOK_URL")?,
    );
    Bot::new(twilio, common::agent()?, "/sms")?.serve(common::bind()?)
}

mod common;
