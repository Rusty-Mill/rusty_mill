//! Thin CLI over `Session::send`.
//!
//! - `rusty-keys "your prompt"` — single-shot: run one turn, print the reply and
//!   its verification verdict.
//! - `rusty-keys` (no args) — interactive REPL with `/verify`, `/mhir`, `/help`,
//!   `/quit`.
//!
//! Config: `RUSTYKEYS_MODEL` (required) is the model name sent to an
//! OpenAI-compatible endpoint. `RUSTYKEYS_BASE_URL` defaults to local ollama
//! (`http://localhost:11434/v1`); `RUSTYKEYS_API_KEY` defaults to `ollama`.

use std::io::Write;

use aisdk::core::capabilities::DynamicModel;
use aisdk::providers::OpenAICompatible;
use anyhow::{Context, Result};
use rk_app::Session;
use rk_config::Config;

#[tokio::main]
async fn main() -> Result<()> {
    let config = Config::from_env().context("resolving configuration")?;

    let base_url = std::env::var("RUSTYKEYS_BASE_URL")
        .unwrap_or_else(|_| "http://localhost:11434/v1".to_string());
    let api_key = std::env::var("RUSTYKEYS_API_KEY").unwrap_or_else(|_| "ollama".to_string());

    let model = OpenAICompatible::<DynamicModel>::builder()
        .model_name(config.model.clone())
        .base_url(base_url.clone())
        .api_key(api_key.clone())
        .build()
        .context("building model provider")?;

    let mut session = Session::new(&config, model).context("building session")?;

    // Semantic recall when RUSTYKEYS_EMBED_MODEL is set; lexical otherwise.
    if let Some(embed_model) = &config.embed_model {
        let em = OpenAICompatible::<DynamicModel>::builder()
            .model_name(embed_model.clone())
            .base_url(base_url.clone())
            .api_key(api_key)
            .build()
            .context("building embedding provider")?;
        session = session.with_embedder(std::sync::Arc::new(rk_app::AiSdkEmbedder::new(em)));
    }

    eprintln!(
        "rusty-keys · model={} · endpoint={} · workspace={} · level={:?} · tools=[{}]",
        config.model,
        base_url,
        config.workspace.display(),
        config.harness_level,
        session.tool_names().join(", "),
    );

    let prompt = std::env::args().skip(1).collect::<Vec<_>>().join(" ");
    if prompt.trim().is_empty() {
        repl(&session).await
    } else {
        let outcome = session.send(&prompt).await.context("running turn")?;
        println!("{}", outcome.reply);
        eprintln!("\n{}", outcome.report.render());
        Ok(())
    }
}

async fn repl<M>(session: &Session<M>) -> Result<()>
where
    M: aisdk::core::language_model::LanguageModel
        + aisdk::core::capabilities::TextInputSupport
        + aisdk::core::capabilities::ToolCallSupport
        + Clone,
{
    eprintln!("interactive mode — /verify, /mhir, /memory, /task, /permissions, /reflect, /sleep, /groom, /help, /quit");
    let stdin = std::io::stdin();
    loop {
        eprint!("› ");
        let _ = std::io::stderr().flush();
        let mut line = String::new();
        if stdin.read_line(&mut line)? == 0 {
            break; // EOF
        }
        let line = line.trim();
        match line {
            "" => continue,
            "/quit" | "/exit" => break,
            "/help" => eprintln!(
                "commands: /verify  /mhir  /memory  /task  /permissions  /reflect  /sleep  /groom  /help  /quit"
            ),
            "/permissions" => {
                println!(
                    "permission mode: {}  ·  isolation: {}",
                    session.permission_mode(),
                    session.isolation()
                );
            }
            "/verify" => {
                session.note_manual_verify()?;
                match session.last_report() {
                    Some(r) => println!("{}", r.render()),
                    None => println!("(no turn yet)"),
                }
            }
            "/mhir" => {
                let m = session.mhir()?;
                println!(
                    "M-HIR {:.3} = {} avoidable / {} turns (excluded: {} unavoidable, {} benign)",
                    m.rate, m.n_interventions, m.n_turns, m.n_unavoidable, m.n_benign,
                );
            }
            "/memory" => {
                let mems = session.memory_recent(10).await?;
                if mems.is_empty() {
                    println!("(no memories yet)");
                }
                for m in mems {
                    let v = if m.validated { "✓" } else { " " };
                    println!("[{}{}] {}: {}", m.mem_type.as_str(), v, m.title, m.body);
                }
            }
            "/reflect" => {
                let s = session.reflect().await?;
                println!("reflected: +{} created, {} updated", s.created, s.updated);
            }
            "/sleep" => {
                let s = session.sleep().await?;
                println!(
                    "slept: +{} created, {} updated, {} pruned, {} groomed",
                    s.created, s.updated, s.pruned, s.groomed
                );
            }
            "/groom" => {
                let s = session.groom().await?;
                println!("groomed: {} ops", s.groomed);
            }
            "/task" => {
                let t = session.task_state();
                println!("task [{:?}]: {}", t.status, t.goal);
                for c in &t.success_criteria {
                    println!("  - {c}");
                }
            }
            line if line.starts_with("/task ") => {
                // `/task <goal> | <criterion> | <criterion>` — `|`-separated.
                let rest = line.trim_start_matches("/task ").trim();
                let mut parts = rest.split('|').map(str::trim);
                let goal = parts.next().unwrap_or("").to_string();
                let criteria: Vec<String> = parts
                    .filter(|c| !c.is_empty())
                    .map(str::to_string)
                    .collect();
                session.set_task(&goal, criteria, Vec::new());
                println!("task set: {goal}");
            }
            prompt => {
                let outcome = session.send(prompt).await.context("running turn")?;
                println!("{}", outcome.reply);
                eprintln!(
                    "[{}]",
                    if outcome.report.verified {
                        "VERIFIED"
                    } else {
                        "UNVERIFIED"
                    }
                );
            }
        }
    }
    Ok(())
}
