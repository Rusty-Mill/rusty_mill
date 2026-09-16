//! End-to-end `rusty-adk` example.
//!
//! Four scenes, each exercising a different part of the ADK 2.0 architecture:
//!
//! 1. an [`LlmAgent`] calling a tool and answering;
//! 2. a workflow graph routing between agents on an emitted route;
//! 3. a fan-out/join graph aggregating parallel branches;
//! 4. a human-in-the-loop graph that suspends and resumes.
//!
//! It runs offline against `MockModel` by default, so `cargo run -p
//! weather-agent` works with no API key. Set `GOOGLE_API_KEY` or
//! `ANTHROPIC_API_KEY` to run scene 1 against a live model instead.

use futures::StreamExt;
use rusty_adk::prelude::*;
use serde_json::json;
use std::sync::Arc;

/// Retrieves the current weather for a city.
///
/// The doc comment becomes the tool description the model reads, and the
/// parameter schema is derived from this signature.
#[adk_tool(crate = ::rusty_adk::tools)]
async fn get_weather(city: String, unit: Option<String>) -> Result<serde_json::Value> {
    let unit = unit.unwrap_or_else(|| "Celsius".to_string());
    let temperature = match city.to_lowercase().as_str() {
        "paris" => 21,
        "tokyo" => 26,
        "oslo" => 4,
        _ => 18,
    };
    Ok(rusty_adk::tools::success(json!({
        "city": city,
        "temperature": temperature,
        "unit": unit,
        "report": format!("It is {temperature} degrees {unit} in {city}."),
    })))
}

/// Records the user's preferred unit for later turns.
#[adk_tool(crate = ::rusty_adk::tools)]
async fn remember_unit(unit: String, ctx: &ToolContext) -> Result<serde_json::Value> {
    // `user:` state follows the user across all of their sessions.
    ctx.set_state("user:preferred_unit", unit.clone());
    Ok(rusty_adk::tools::success(json!({"remembered": unit})))
}

#[tokio::main]
async fn main() -> Result<()> {
    println!("== 1. An agent that calls a tool ==\n");
    agent_with_tool().await?;

    println!("\n== 2. A graph that routes to a specialist ==\n");
    routing_graph().await?;

    println!("\n== 3. A graph that fans out and joins ==\n");
    fan_out_graph().await?;

    println!("\n== 4. A graph that pauses for a human ==\n");
    human_in_the_loop().await?;

    Ok(())
}

/// Builds the model for scene 1: a live one when a key is present, otherwise a
/// scripted mock so the example always runs.
fn weather_model() -> Arc<dyn Model> {
    #[cfg(feature = "live")]
    {
        if let Ok(model) = GeminiModel::from_env("gemini-flash-latest") {
            println!("(using live Gemini)");
            return Arc::new(model);
        }
        if let Ok(model) = AnthropicModel::from_env("claude-opus-5") {
            println!("(using live Claude)");
            return Arc::new(model);
        }
    }
    println!("(no API key set — using MockModel)");
    Arc::new(
        MockModel::new()
            .push_call_json("get_weather", json!({"city": "Paris"}))
            .push_text("It is 21 degrees Celsius in Paris."),
    )
}

/// Scene 1: an agent, a tool, and the runner.
async fn agent_with_tool() -> Result<()> {
    let agent = LlmAgent::builder("weather_agent")
        .model(weather_model())
        .description("Answers questions about the weather.")
        .instruction(
            "You answer weather questions. Use the get_weather tool for any city \
             the user asks about, then state the result in one sentence.",
        )
        .tools([get_weather_tool(), remember_unit_tool()])
        .output_key("last_report")
        .build()?;

    let services = Services::new(Arc::new(InMemorySessionService::new()));
    let runner = Runner::new("weather_app", agent.shared(), services);
    let session = runner.create_session("user-1", None).await?;

    let mut stream = runner.run(
        &session.user_id,
        &session.id,
        Content::user_text("What's the weather in Paris?"),
        None,
    );

    while let Some(event) = stream.next().await {
        let event = event?;
        print_event(&event);
    }

    // The runner committed the agent's state write before the run ended.
    let saved = runner.session("user-1", &session.id).await?.unwrap();
    if let Some(report) = saved.state.get("last_report") {
        println!("  [state] last_report = {report}");
    }
    Ok(())
}

/// Scene 2: a router node picks a specialist agent by emitting a route.
async fn routing_graph() -> Result<()> {
    let triage = RouterNode::new("triage", NodeConfig::default(), |ctx| {
        let input = ctx.input.clone();
        Box::pin(async move {
            let text = input
                .as_ref()
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_lowercase();
            let route = if text.contains("refund") || text.contains("charge") {
                "BILLING"
            } else {
                "TECHNICAL"
            };
            println!("  [triage] routing to {route}");
            Ok((json!(text), vec![route.to_string()]))
        })
    })
    .shared();

    let specialist = |name: &'static str, reply: &'static str| {
        AgentNode::new(
            LlmAgent::builder(name)
                .model(Arc::new(MockModel::new().push_text(reply)))
                .build()
                .unwrap()
                .shared(),
        )
        .shared()
    };

    let graph = Graph::new(
        vec![
            constant_node("intake", json!("I want a refund for a double charge")),
            triage,
            specialist(
                "billing",
                "I have issued a refund for the duplicate charge.",
            ),
            specialist("technical", "I have opened a support ticket."),
        ],
        EdgeBuilder::new()
            .start("intake")
            .add("intake", "triage")
            .add_route("triage", "billing", Route::string("BILLING"))
            .add_route("triage", "technical", Route::string("TECHNICAL"))
            .build(),
    )?;

    run_graph(&graph, None).await
}

/// Scene 3: two branches run concurrently and a join aggregates them.
async fn fan_out_graph() -> Result<()> {
    let branch = |name: &'static str, value: &'static str| {
        FunctionNode::new(name, NodeConfig::default(), move |ctx| {
            Box::pin(async move {
                ctx.emit_message(format!("gathering {value}..."))?;
                Ok(NodeOutcome::output(json!(value)))
            })
        })
        .shared()
    };

    let summarize = FunctionNode::new("summarize", NodeConfig::default(), |ctx| {
        let input = ctx.input.clone();
        Box::pin(async move {
            // A join hands its successor a map keyed by predecessor node name.
            let parts = input.unwrap_or(json!({}));
            Ok(NodeOutcome::output(json!(format!(
                "forecast={} radar={}",
                parts["forecast"].as_str().unwrap_or("?"),
                parts["radar"].as_str().unwrap_or("?"),
            ))))
        })
    })
    .shared();

    let graph = Graph::new(
        vec![
            constant_node("dispatch", json!("Oslo")),
            branch("forecast", "sunny"),
            branch("radar", "clear"),
            JoinNode::new("gather").shared(),
            summarize,
        ],
        EdgeBuilder::new()
            .start("dispatch")
            .add_fan_out("dispatch", ["forecast", "radar"])
            .add_fan_in("gather", ["forecast", "radar"])
            .add("gather", "summarize")
            .build(),
    )?;

    run_graph(&graph, None).await
}

/// Scene 4: a node suspends the workflow, then the run is resumed.
async fn human_in_the_loop() -> Result<()> {
    let approve = FunctionNode::new("approve_refund", NodeConfig::default(), |ctx| {
        let ctx = ctx.clone();
        Box::pin(async move {
            // First pass: suspend. After resume: this returns the answer.
            let answer = ctx.resume_or_request_input(
                "Approve a refund of 250?",
                Some(json!({"amount": 250})),
            )?;
            Ok(NodeOutcome::output(answer))
        })
    })
    .shared();

    let apply = FunctionNode::new("apply", NodeConfig::default(), |ctx| {
        let input = ctx.input.clone();
        Box::pin(async move {
            let approved = input
                .as_ref()
                .and_then(|v| v.get("approved"))
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            Ok(NodeOutcome::output(json!(if approved {
                "refund issued"
            } else {
                "refund declined"
            })))
        })
    })
    .shared();

    let graph = Graph::new(vec![approve, apply], chain(["approve_refund", "apply"]))?;

    let services = Services::new(Arc::new(InMemorySessionService::new()));
    let runner = Runner::new("refunds", Arc::new(graph), services);
    let session = runner.create_session("user-1", None).await?;

    // First run: the graph suspends and asks.
    let mut interrupt_id = None;
    let mut stream = runner.run(
        &session.user_id,
        &session.id,
        Content::user_text("Please refund my duplicate charge."),
        None,
    );
    while let Some(event) = stream.next().await {
        let event = event?;
        if let Some(request) = &event.request_input {
            println!("  [paused] {} payload={:?}", request.hint, request.payload);
            interrupt_id = Some(request.interrupt_id.clone());
        }
        print_event(&event);
    }
    drop(stream);

    // Second run: the human answers and the graph picks up where it left off.
    let Some(interrupt_id) = interrupt_id else {
        println!("  (no suspension occurred)");
        return Ok(());
    };
    println!("  [human] approving...");

    let mut stream = runner.resume(
        &session.user_id,
        &session.id,
        ResumeRequest::new(interrupt_id, json!({"approved": true})),
        None,
    );
    while let Some(event) = stream.next().await {
        print_event(&event?);
    }
    Ok(())
}

/// Runs a graph against a fresh session and prints its events.
async fn run_graph(graph: &Graph, message: Option<&str>) -> Result<()> {
    let services = Services::new(Arc::new(InMemorySessionService::new()));
    let mut session = Session::new("s1", "example", "user-1");
    if let Some(text) = message {
        session
            .events
            .push(Event::new("inv", "user").with_content(Content::user_text(text)));
    }
    let ctx = InvocationContext::new(session, services, RunConfig::default());

    let mut stream = graph.run(ctx, None);
    while let Some(event) = stream.next().await {
        print_event(&event?);
    }
    Ok(())
}

/// Prints one event in a readable one-line form.
fn print_event(event: &Event) {
    let text = event.text();
    if !text.is_empty() {
        let marker = if event.is_partial() { "~" } else { " " };
        println!("{marker} [{}] {text}", event.author);
    }

    for call in event.function_calls() {
        println!(
            "  [{}] -> calls {}({})",
            event.author,
            call.name,
            Value(&call.args)
        );
    }
    for response in event.function_responses() {
        println!(
            "  [{}] <- {} returned {}",
            event.author, response.name, response.response
        );
    }
    if let Some(output) = &event.output {
        println!("  [{}] output = {output}", event.author);
    }
    if let Some(code) = &event.error_code {
        println!(
            "  [{}] ERROR {code}: {}",
            event.author,
            event.error_message.clone().unwrap_or_default()
        );
    }
}

/// Compact rendering of a tool-call argument map.
struct Value<'a>(&'a rusty_adk::core::Args);

impl std::fmt::Display for Value<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let rendered: Vec<String> = self.0.iter().map(|(k, v)| format!("{k}={v}")).collect();
        f.write_str(&rendered.join(", "))
    }
}
