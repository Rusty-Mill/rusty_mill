//! Offline demo of the spike: a scripted model drives a policy-vetted tool loop.
//!
//! Run: `cargo run -p rk-spike`. No provider/network required.

use std::sync::Arc;

use rk_spike::fake::FakeChatModel;
use rk_spike::kernel::{run_turn, ChatMessage, ModelStep};
use rk_spike::policy::WorkspacePolicy;
use rk_spike::tool::{register_builtins, ToolRegistry};
use serde_json::json;

#[tokio::main]
async fn main() {
    let root = std::env::current_dir().expect("cwd");
    let policy = Arc::new(WorkspacePolicy::new(root.clone()));
    let mut registry = ToolRegistry::new(policy);
    register_builtins(&mut registry);

    // Scripted turns: (1) read an in-workspace file, (2) attempt an out-of-root
    // read that policy must block, (3) finish with text.
    let model = FakeChatModel::new(vec![
        vec![ModelStep::ToolCall {
            name: "read_file".into(),
            args: json!({ "path": "Cargo.toml" }),
        }],
        vec![ModelStep::ToolCall {
            name: "read_file".into(),
            args: json!({ "path": "/etc/passwd" }),
        }],
        vec![ModelStep::Text(
            "Read the manifest; the out-of-workspace read was blocked by policy.".into(),
        )],
    ]);

    let system = "You are a Rusty Keys spike agent.";
    let mut history = vec![ChatMessage::User("Summarize the workspace manifest.".into())];

    match run_turn(&model, &registry, system, &mut history).await {
        Ok(reply) => {
            println!("=== final reply ===\n{reply}\n");
            println!("=== transcript ===");
            for msg in &history {
                match msg {
                    ChatMessage::User(s) => println!("[user] {s}"),
                    ChatMessage::Assistant(s) => println!("[assistant] {s}"),
                    ChatMessage::ToolResult { name, content } => {
                        let first = content.lines().next().unwrap_or("");
                        println!("[tool:{name}] {first}");
                    }
                }
            }
        }
        Err(e) => eprintln!("turn failed: {e}"),
    }
}
