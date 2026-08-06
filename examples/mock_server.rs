//! A richer mock A2A agent: multiple skills, structured and chunked
//! artifacts, a direct-message (no task) reply, a rejected task, and an
//! interrupted (`AUTH_REQUIRED`) task - exercising most of the protocol
//! surface `echo_server` doesn't touch.
//!
//! Skill routing is a dumb keyword match on the message text, chosen in
//! priority order (see `route` below). Run it with:
//!
//! ```sh
//! cargo run --example mock_server --features server
//! ```
//!
//! Then, in another terminal, try each skill (uses the generic
//! `send_message` client example against this server's port):
//!
//! ```sh
//! export A2A_AGENT_URL=http://127.0.0.1:8081
//! cargo run --example send_message --features client -- "what's the weather in Kyoto?"
//! cargo run --example send_message --features client -- "count the words in this sentence please"
//! cargo run --example send_message --features client -- "generate an image of a fox"
//! cargo run --example send_message --features client -- "write me a long report on rust"
//! cargo run --example send_message --features client -- "can you clarify what you need from me"
//! cargo run --example send_message --features client -- "do something totally unsupported"
//! cargo run --example send_message --features client -- "access my secure account"
//! cargo run --example send_message --features client -- "hello there"
//! ```

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::json;

use rusty_a2a::error::Result;
use rusty_a2a::server::{AgentExecutor, AgentServer, EventSink, RequestContext};
use rusty_a2a::types::{AgentCard, AgentInterface, AgentSkill, Artifact, Message, Part, TaskState};

struct MockAgent;

#[async_trait]
impl AgentExecutor for MockAgent {
    async fn execute(&self, ctx: RequestContext, events: EventSink) -> Result<()> {
        let text = ctx.message.text();
        let lower = text.to_lowercase();

        // Priority-ordered keyword routing. First match wins; "echo" is
        // the fallback so the agent always has *something* to say.
        if lower.contains("clarify") {
            clarify(&events);
        } else if lower.contains("unsupported") {
            reject(&events, &text);
        } else if lower.contains("secure") || lower.contains("auth") {
            require_auth(&events);
        } else if lower.contains("weather") {
            weather_report(&events, &text);
        } else if lower.contains("count") || lower.contains("stats") {
            text_stats(&events, &text);
        } else if lower.contains("image") || lower.contains("picture") {
            image_generator(&events, &text).await;
        } else if lower.contains("report") || lower.contains("long") {
            long_report(&events, &text).await;
        } else {
            echo(&events, &text);
        }

        Ok(())
    }
}

/// No task at all: agents may reply with a bare `Message` to ask for
/// clarification before committing to a task (spec Section 3.7).
fn clarify(events: &EventSink) {
    events.message(Message::agent_text(
        "I can help with weather, word counts, images, and long reports - what would you like?",
    ));
}

/// A task the agent declines to perform - a terminal `Rejected` state,
/// distinct from `Failed` (which implies an error rather than a choice).
fn reject(events: &EventSink, text: &str) {
    events.status(TaskState::Working);
    events.status_with_message(
        TaskState::Rejected,
        Some(Message::agent_text(format!(
            "I can't help with that: \"{text}\" isn't something this mock agent supports."
        ))),
    );
}

/// An interrupted task: the agent needs the client to authenticate before
/// it can continue. `AUTH_REQUIRED` is non-terminal - a real agent would
/// resume this same task once the client sends a follow-up message with
/// credentials.
fn require_auth(events: &EventSink) {
    events.status(TaskState::Working);
    events.status_with_message(
        TaskState::AuthRequired,
        Some(Message::agent_text(
            "This action requires authentication. Send a follow-up message on this task with your credentials to continue.",
        )),
    );
}

fn echo(events: &EventSink, text: &str) {
    events.status(TaskState::Working);
    events.status_with_message(
        TaskState::Completed,
        Some(Message::agent_text(format!("you said: {text}"))),
    );
}

/// A single structured-data artifact - the common case for skills that
/// return machine-readable results rather than prose.
fn weather_report(events: &EventSink, text: &str) {
    events.status(TaskState::Working);

    let location = text
        .split_whitespace()
        .last()
        .unwrap_or("your location")
        .trim_matches(|c: char| !c.is_alphanumeric());

    let artifact = Artifact::new(
        "weather-report",
        vec![Part::data(json!({
            "location": location,
            "conditions": "partly cloudy",
            "temperatureC": 19,
            "windKph": 12,
        }))],
    )
    .with_name("Weather Report");
    events.artifact(artifact);

    events.status_with_message(
        TaskState::Completed,
        Some(Message::agent_text(format!(
            "Here's the (fabricated) weather for {location}."
        ))),
    );
}

fn text_stats(events: &EventSink, text: &str) {
    events.status(TaskState::Working);

    let words = text.split_whitespace().count();
    let chars = text.chars().count();

    let artifact = Artifact::new(
        "text-stats",
        vec![Part::data(json!({
            "words": words,
            "characters": chars,
        }))],
    )
    .with_name("Text Statistics");
    events.artifact(artifact);

    events.status_with_message(
        TaskState::Completed,
        Some(Message::agent_text(format!("{words} words, {chars} characters."))),
    );
}

/// Simulates a slower generation pipeline with visible progress messages,
/// then a single artifact referenced by URL rather than inline data.
async fn image_generator(events: &EventSink, text: &str) {
    events.status(TaskState::Working);
    events.status_with_message(
        TaskState::Working,
        Some(Message::agent_text("queued for rendering...")),
    );
    tokio::time::sleep(Duration::from_millis(150)).await;
    events.status_with_message(TaskState::Working, Some(Message::agent_text("rendering...")));
    tokio::time::sleep(Duration::from_millis(150)).await;

    let artifact = Artifact::new(
        "generated-image",
        vec![Part::url("https://example.com/mock/generated-image.png").with_media_type("image/png")],
    )
    .with_name("Generated Image");
    events.artifact(artifact);

    events.status_with_message(
        TaskState::Completed,
        Some(Message::agent_text(format!(
            "Here's your (mock) image for: {text}"
        ))),
    );
}

/// Builds one artifact incrementally across several chunks (spec Section
/// 4.2.2: `append`/`lastChunk`), so a streaming client sees it arrive
/// piece by piece instead of all at once.
async fn long_report(events: &EventSink, text: &str) {
    events.status(TaskState::Working);

    let sections = [
        format!("# Report: {text}\n\n"),
        "## Summary\n\nThis is a mock, incrementally-streamed report.\n\n".to_string(),
        "## Details\n\nEach of these sections was sent as a separate artifact chunk, \
             appended to the same artifact id.\n\n"
            .to_string(),
        "## Conclusion\n\nThat's the whole (fake) report.\n".to_string(),
    ];

    for (i, section) in sections.iter().enumerate() {
        let last = i == sections.len() - 1;
        events.artifact_update(
            Artifact::new("long-report", vec![Part::text(section.clone())]).with_name("Long Report"),
            /* append = */ i > 0,
            /* last_chunk = */ last,
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    events.status_with_message(
        TaskState::Completed,
        Some(Message::agent_text("Report generated in 4 chunks.")),
    );
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let addr = ([127, 0, 0, 1], 8081);
    let interface = AgentInterface::json_rpc("http://127.0.0.1:8081");

    let card = AgentCard::new(
        "Mock Multi-Skill Agent",
        "A mock A2A agent exercising multiple skills, structured/chunked artifacts, \
         direct-message replies, rejected tasks, and interrupted (auth-required) tasks.",
        env!("CARGO_PKG_VERSION"),
        interface,
    )
    .with_streaming(true)
    .with_skill(
        AgentSkill::new(
            "weather-report",
            "Weather Report",
            "Returns a fabricated weather report for a location.",
        )
        .with_tags(["weather", "demo"]),
    )
    .with_skill(
        AgentSkill::new(
            "text-stats",
            "Text Statistics",
            "Counts words and characters in your message.",
        )
        .with_tags(["analysis", "demo"]),
    )
    .with_skill(
        AgentSkill::new(
            "image-generator",
            "Image Generator",
            "Simulates generating an image, with progress updates.",
        )
        .with_tags(["image", "demo"]),
    )
    .with_skill(
        AgentSkill::new(
            "long-report",
            "Long Report Writer",
            "Streams a multi-section report as chunked artifact updates.",
        )
        .with_tags(["report", "demo"]),
    )
    .with_skill(
        AgentSkill::new("echo", "Echo", "Fallback: repeats back your message.").with_tags(["demo", "echo"]),
    );

    let server = AgentServer::new(card, Arc::new(MockAgent));
    println!("Mock agent listening on http://127.0.0.1:8081");
    println!("Agent card: http://127.0.0.1:8081/.well-known/agent-card.json");
    println!("Skills: weather-report, text-stats, image-generator, long-report, echo");
    println!("Also try: \"clarify\", \"unsupported\", \"secure\"/\"auth\" for the non-completion paths");
    server.serve(addr).await
}
