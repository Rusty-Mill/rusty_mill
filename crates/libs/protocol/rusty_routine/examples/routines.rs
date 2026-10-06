//! Run a routines file against an AG-UI agent.
//!
//! ```text
//! ROUTINES=routines.json AGENT_URL=http://127.0.0.1:8080/api/agent \
//!   cargo run -p rusty_routine --features run --example routines
//! ```
//!
//! `routines.json` is an array of `{"name", "cron", "prompt", "maxFailures"?}`
//! (cron in UTC). Optional: `AGENT_TOKEN`, sent as a bearer token so the
//! gateway sees the routine as its requester. Each run's reply is printed
//! as one line; a routine that fails `maxFailures` times in a row (default
//! 3) is disabled until the process restarts.

use rusty_agui::HttpAgent;

fn env(name: &str) -> Result<String, String> {
    std::env::var(name).map_err(|_| format!("{name} is not set"))
}

fn main() -> Result<(), String> {
    let path = env("ROUTINES")?;
    let json = std::fs::read_to_string(&path).map_err(|e| format!("{path}: {e}"))?;
    let routines =
        rusty_routine::load(&json, rusty_routine::run::now()).map_err(|e| e.to_string())?;
    let mut agent = HttpAgent::new(&env("AGENT_URL")?).map_err(|e| e.to_string())?;
    if let Ok(token) = std::env::var("AGENT_TOKEN") {
        agent = agent.header("Authorization", &format!("Bearer {token}"));
    }
    for r in &routines {
        println!(
            "[{}] next at {}",
            r.name,
            r.next_run
                .map(|t| t.to_string())
                .unwrap_or_else(|| "never".into())
        );
    }
    rusty_routine::run::run(routines, &agent, &mut std::io::stdout());
    Ok(())
}
