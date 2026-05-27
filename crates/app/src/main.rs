//! Thin CLI over `Session::send` (BACKLOG Phase 1).
//!
//! Usage: `rusty-keys "your prompt"`.
//!
//! Config: `RUSTYKEYS_MODEL` (required) is the model name sent to an
//! OpenAI-compatible endpoint. `RUSTYKEYS_BASE_URL` defaults to local ollama
//! (`http://localhost:11434/v1`); `RUSTYKEYS_API_KEY` defaults to `ollama`.

use aisdk::core::capabilities::DynamicModel;
use aisdk::providers::OpenAICompatible;
use anyhow::{Context, Result};
use rk_app::Session;
use rk_config::Config;

#[tokio::main]
async fn main() -> Result<()> {
    let prompt = std::env::args().skip(1).collect::<Vec<_>>().join(" ");
    if prompt.trim().is_empty() {
        eprintln!("usage: rusty-keys \"your prompt\"");
        std::process::exit(2);
    }

    let config = Config::from_env().context("resolving configuration")?;

    let base_url = std::env::var("RUSTYKEYS_BASE_URL")
        .unwrap_or_else(|_| "http://localhost:11434/v1".to_string());
    let api_key = std::env::var("RUSTYKEYS_API_KEY").unwrap_or_else(|_| "ollama".to_string());

    let model = OpenAICompatible::<DynamicModel>::builder()
        .model_name(config.model.clone())
        .base_url(base_url.clone())
        .api_key(api_key)
        .build()
        .context("building model provider")?;

    let session = Session::new(&config, model);

    eprintln!(
        "rusty-keys · model={} · endpoint={} · workspace={} · level={:?} · tools=[{}]",
        config.model,
        base_url,
        config.workspace.display(),
        config.harness_level,
        session.tool_names().join(", "),
    );

    let outcome = session.send(&prompt).await.context("running turn")?;
    println!("{}", outcome.reply);
    // Verification summary (the `/verify` equivalent for the single-shot CLI).
    eprintln!("\n{}", outcome.report.render());
    Ok(())
}
