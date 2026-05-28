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
use rk_constrain::PlanDecision;

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
    eprintln!("interactive mode — /verify, /mhir, /memory, /task, /plan, /explore, /permissions, /entropy, /cost, /compact, /reflect, /sleep, /groom, /help, /quit");
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
                "commands: /verify  /mhir  /memory  /task  /plan  /explore  /permissions  /entropy  /cost  /compact  /reflect  /sleep  /groom  /help  /quit"
            ),
            "/permissions" => {
                println!(
                    "permission mode: {}  ·  isolation: {}",
                    session.permission_mode(),
                    session.isolation()
                );
            }
            "/entropy" => {
                let recs = session.entropy_recent(10)?;
                if recs.is_empty() {
                    println!("(no entropy findings yet)");
                }
                for r in recs {
                    let delta = r["delta"].as_i64().unwrap_or(0);
                    let n = r["findings"].as_array().map(|a| a.len()).unwrap_or(0);
                    println!(
                        "turn {} :: delta={delta}  ({n} findings)",
                        r["turn_id"].as_str().unwrap_or("?")
                    );
                    for f in r["findings"].as_array().into_iter().flatten() {
                        println!(
                            "  [sev {}] {} — {}",
                            f["severity"].as_u64().unwrap_or(0),
                            f["category"].as_str().unwrap_or("?"),
                            f["description"].as_str().unwrap_or("")
                        );
                    }
                }
            }
            "/cost" => {
                let (used, limit, frac, total, compactions) = session.cost();
                println!(
                    "tokens :: {used} / {limit} ({:.0}%) :: session_total={total} compactions={compactions}",
                    frac * 100.0
                );
            }
            "/compact" => {
                let n = session.compact_now().await?;
                println!("compacted {n} messages into a summary");
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
            line if line.starts_with("/explore ") => {
                let task = line.trim_start_matches("/explore ").trim();
                if !session.explore_enabled() {
                    eprintln!("explore is disabled — set RUSTYKEYS_EXPLORE=1 (cost: ~N+1 turns)");
                } else {
                    match session.explore(task).await {
                        Ok(report) => println!("{report}"),
                        Err(e) => eprintln!("explore failed: {e}"),
                    }
                }
            }
            line if line.starts_with("/plan ") => {
                // Enter plan mode, then run the proposal turn (read-only).
                let text = line.trim_start_matches("/plan ").trim().to_string();
                session.enter_plan_mode();
                eprintln!("[plan mode — writes and bash blocked until approval]");
                run_turn_and_handle(session, &stdin, &text).await?;
            }
            prompt => {
                run_turn_and_handle(session, &stdin, prompt).await?;
            }
        }
    }
    Ok(())
}

/// Run a turn, print the result, and — if the agent requested `exit_plan_mode`
/// — render the plan and collect a Proceed/Reject/Annotate decision. An
/// annotation is re-sent to the agent as a follow-up turn.
async fn run_turn_and_handle<M>(
    session: &Session<M>,
    stdin: &std::io::Stdin,
    prompt: &str,
) -> Result<()>
where
    M: aisdk::core::language_model::LanguageModel
        + aisdk::core::capabilities::TextInputSupport
        + aisdk::core::capabilities::ToolCallSupport
        + Clone,
{
    let mut next = Some(prompt.to_string());
    while let Some(p) = next.take() {
        let outcome = session.send(&p).await.context("running turn")?;
        println!("{}", outcome.reply);
        eprintln!(
            "[{}]",
            if outcome.report.verified {
                "VERIFIED"
            } else {
                "UNVERIFIED"
            }
        );

        if let Some(plan) = session.plan_exit_pending() {
            println!("--- proposed plan ---\n{plan}\n---------------------");
            eprint!("approve? [proceed/reject/annotate <text>]: ");
            let _ = std::io::stderr().flush();
            let mut ans = String::new();
            stdin.read_line(&mut ans)?;
            let ans = ans.trim();
            let decision = if ans.is_empty() || ans.eq_ignore_ascii_case("proceed") {
                PlanDecision::Proceed
            } else if let Some(note) = ans.strip_prefix("annotate ") {
                PlanDecision::Annotate(note.trim().to_string())
            } else {
                PlanDecision::Reject
            };
            match session.resolve_plan_exit(decision) {
                Some(feedback) => next = Some(feedback), // re-propose with feedback
                None => eprintln!(
                    "[plan {}]",
                    if session.is_planning() {
                        "pending"
                    } else {
                        "resolved"
                    }
                ),
            }
        }
    }
    Ok(())
}
