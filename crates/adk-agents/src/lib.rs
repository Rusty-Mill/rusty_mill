//! Agents for the Rust ADK.
//!
//! [`LlmAgent`] is the reasoning agent: it builds model requests, dispatches
//! the tool calls the model asks for, and streams the whole exchange back as
//! events. [`SequentialAgent`], [`ParallelAgent`], and [`LoopAgent`] are the
//! deterministic workflow agents. [`AgentNode`] drops any of them into a
//! workflow graph.
//!
//! ```
//! # tokio_test::block_on(async {
//! use adk_agents::{Agent, LlmAgent};
//! use adk_core::{Content, InvocationContext, RunConfig, Services, Session};
//! use adk_models::MockModel;
//! use adk_sessions::InMemorySessionService;
//! use futures::StreamExt;
//! use std::sync::Arc;
//!
//! let agent = LlmAgent::builder("greeter")
//!     .model(Arc::new(MockModel::new().push_text("Hello!")))
//!     .instruction("Greet the user.")
//!     .output_key("last_greeting")
//!     .build()
//!     .unwrap();
//!
//! let services = Services::new(Arc::new(InMemorySessionService::new()));
//! let mut session = Session::new("s1", "app", "u1");
//! session.events.push(
//!     adk_core::Event::new("inv", "user").with_content(Content::user_text("hi")),
//! );
//! let ctx = InvocationContext::new(session, services, RunConfig::default());
//!
//! let events: Vec<_> = agent.run(&ctx).collect().await;
//! let last = events.last().unwrap().as_ref().unwrap();
//! assert_eq!(last.text(), "Hello!");
//! // The response was staged under the configured output key.
//! assert_eq!(last.actions.state_delta["last_greeting"], "Hello!");
//! # });
//! ```

#![deny(missing_docs)]
#![warn(clippy::all)]

pub mod agent;
pub mod callbacks;
pub mod llm_agent;
pub mod workflow;

pub use agent::{Agent, AgentNode, IntoAgentNode, SharedAgent};
pub use callbacks::{
    AfterAgentCallback, AfterModelCallback, AfterToolCallback, BeforeAgentCallback,
    BeforeModelCallback, BeforeToolCallback, CallbackContext, Callbacks,
};
pub use llm_agent::{IncludeContents, LlmAgent, LlmAgentBuilder};
pub use workflow::{AgentRunOwned, LoopAgent, ParallelAgent, SequentialAgent};

#[cfg(test)]
mod tests {
    use super::*;
    use adk_core::{
        Content, Event, InvocationContext, RunConfig, Schema, Services, Session, StreamingMode,
    };
    use adk_models::{LlmResponse, MockModel};
    use adk_sessions::InMemorySessionService;
    use adk_tools::{FunctionTool, ToolSource};
    use futures::StreamExt;
    use serde_json::json;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    fn context_with(user_text: &str) -> InvocationContext {
        let services = Services::new(Arc::new(InMemorySessionService::new()));
        let mut session = Session::new("s1", "app", "u1");
        session
            .events
            .push(Event::new("inv", "user").with_content(Content::user_text(user_text)));
        InvocationContext::new(session, services, RunConfig::default())
    }

    async fn drain(agent: &dyn Agent, ctx: &InvocationContext) -> Vec<Event> {
        agent
            .run(ctx)
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .map(|e| e.expect("agent run failed"))
            .collect()
    }

    async fn drain_result(
        agent: &dyn Agent,
        ctx: &InvocationContext,
    ) -> adk_core::Result<Vec<Event>> {
        agent.run(ctx).collect::<Vec<_>>().await.into_iter().collect()
    }

    fn weather_tool() -> ToolSource {
        let tool = FunctionTool::new(
            "get_weather",
            "Retrieves the current weather for a city.",
            Schema::object().property("city", Schema::string().describe("The city name.")),
            |args, _ctx| {
                Box::pin(async move {
                    let city = args.get("city").and_then(|v| v.as_str()).unwrap_or("?");
                    Ok(adk_tools::success(json!({"report": format!("Sunny in {city}.")})))
                })
            },
        );
        ToolSource::Tool(tool.shared())
    }

    // ---- basic turn ----

    #[tokio::test]
    async fn a_plain_answer_is_a_single_final_event() {
        let agent = LlmAgent::builder("a")
            .model(Arc::new(MockModel::new().push_text("Paris")))
            .build()
            .unwrap();

        let events = drain(&agent, &context_with("capital of France?")).await;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].text(), "Paris");
        assert!(events[0].is_final_response());
    }

    #[tokio::test]
    async fn the_instruction_is_sent_as_a_system_instruction() {
        let model = Arc::new(MockModel::new().push_text("ok"));
        let agent = LlmAgent::builder("a")
            .model(model.clone())
            .global_instruction("You are terse.")
            .instruction("Answer the question.")
            .build()
            .unwrap();

        drain(&agent, &context_with("hi")).await;
        let sent = model.recorded_requests()[0].system_instruction.clone().unwrap();
        assert!(sent.contains("You are terse."));
        assert!(sent.contains("Answer the question."));
    }

    #[tokio::test]
    async fn instruction_placeholders_read_session_state() {
        let model = Arc::new(MockModel::new().push_text("ok"));
        let agent = LlmAgent::builder("a")
            .model(model.clone())
            .instruction("Write about: {topic}.")
            .build()
            .unwrap();

        let ctx = context_with("go");
        ctx.set_state("topic", "otters");
        drain(&agent, &ctx).await;

        assert_eq!(
            model.recorded_requests()[0].system_instruction.as_deref(),
            Some("Write about: otters.")
        );
    }

    #[tokio::test]
    async fn output_key_stages_the_final_response_into_state() {
        let agent = LlmAgent::builder("a")
            .model(Arc::new(MockModel::new().push_text("Bonjour")))
            .output_key("greeting")
            .build()
            .unwrap();

        let events = drain(&agent, &context_with("hi")).await;
        assert_eq!(events[0].actions.state_delta["greeting"], "Bonjour");
    }

    // ---- tool calling ----

    #[tokio::test]
    async fn a_tool_call_round_trips_and_then_answers() {
        let model = MockModel::new()
            .push_call_json("get_weather", json!({"city": "Paris"}))
            .push_text("It is sunny in Paris.");

        let agent = LlmAgent::builder("weather")
            .model(Arc::new(model))
            .tool(weather_tool())
            .build()
            .unwrap();

        let events = drain(&agent, &context_with("weather in Paris?")).await;

        // model call -> tool result -> final answer
        assert_eq!(events.len(), 3);
        assert!(!events[0].function_calls().is_empty());
        let response = events[1].function_responses()[0];
        assert_eq!(response.name, "get_weather");
        assert_eq!(response.response["status"], "success");
        assert!(response.response["report"].as_str().unwrap().contains("Paris"));
        assert_eq!(events[2].text(), "It is sunny in Paris.");
    }

    #[tokio::test]
    async fn the_declared_tool_schema_reaches_the_model() {
        let model = Arc::new(MockModel::new().push_text("done"));
        let agent = LlmAgent::builder("a")
            .model(model.clone())
            .tool(weather_tool())
            .build()
            .unwrap();

        drain(&agent, &context_with("hi")).await;
        let declared = &model.recorded_requests()[0].tools;
        assert_eq!(declared.len(), 1);
        assert_eq!(declared[0].name, "get_weather");
        assert!(declared[0].parameters.is_some());
    }

    #[tokio::test]
    async fn an_unknown_tool_is_reported_to_the_model_not_fatal() {
        let model = MockModel::new()
            .push_call_json("nonexistent", json!({}))
            .push_text("I could not do that.");

        let agent = LlmAgent::builder("a")
            .model(Arc::new(model))
            .tool(weather_tool())
            .build()
            .unwrap();

        let events = drain(&agent, &context_with("go")).await;
        let response = events[1].function_responses()[0];
        assert_eq!(response.response["status"], "error");
        assert!(response.response["error_message"]
            .as_str()
            .unwrap()
            .contains("nonexistent"));
        assert_eq!(events.last().unwrap().text(), "I could not do that.");
    }

    #[tokio::test]
    async fn a_failing_tool_becomes_an_error_result_the_model_can_recover_from() {
        let failing = FunctionTool::new(
            "boom",
            "Always fails.",
            Schema::object(),
            |_args, _ctx| Box::pin(async { Err(adk_core::AdkError::tool("boom", "exploded")) }),
        );
        let model = MockModel::new()
            .push_call_json("boom", json!({}))
            .push_text("That tool failed.");

        let agent = LlmAgent::builder("a")
            .model(Arc::new(model))
            .tool(ToolSource::Tool(failing.shared()))
            .build()
            .unwrap();

        let events = drain(&agent, &context_with("go")).await;
        assert_eq!(events[1].function_responses()[0].response["status"], "error");
        assert_eq!(events.last().unwrap().text(), "That tool failed.");
    }

    #[tokio::test]
    async fn skip_summarization_ends_the_turn_at_the_tool_result() {
        let tool = FunctionTool::new("raw", "Returns user-ready output.", Schema::object(), |_a, ctx| {
            let ctx = ctx.clone();
            Box::pin(async move {
                ctx.skip_summarization();
                Ok(adk_tools::success(json!({"data": 1})))
            })
        });
        let model = MockModel::new()
            .push_call_json("raw", json!({}))
            .push_text("this should never be reached");

        let agent = LlmAgent::builder("a")
            .model(Arc::new(model))
            .tool(ToolSource::Tool(tool.shared()))
            .build()
            .unwrap();

        let events = drain(&agent, &context_with("go")).await;
        assert_eq!(events.len(), 2);
        assert!(events[1].actions.skip_summarization);
        assert!(events[1].is_final_response());
    }

    #[tokio::test]
    async fn a_long_running_tool_ends_the_turn_and_records_its_call_id() {
        let tool = FunctionTool::new(
            "start_job",
            "Starts a background job.",
            Schema::object(),
            |_a, _c| Box::pin(async { Ok(adk_tools::pending(json!({"ticket": "t-1"}))) }),
        )
        .long_running();

        let model = MockModel::new().push_call_json("start_job", json!({}));
        let agent = LlmAgent::builder("a")
            .model(Arc::new(model))
            .tool(ToolSource::Tool(tool.shared()))
            .build()
            .unwrap();

        let events = drain(&agent, &context_with("go")).await;
        assert!(!events[1].long_running_tool_ids.is_empty());
        assert!(events[1].is_final_response());
    }

    #[tokio::test]
    async fn a_tool_requiring_confirmation_pauses_instead_of_running() {
        let ran = Arc::new(AtomicU32::new(0));
        let counter = Arc::clone(&ran);
        let tool = FunctionTool::new(
            "reimburse",
            "Reimburses the user.",
            Schema::object().property("amount", Schema::integer()),
            move |_args, _ctx| {
                let counter = Arc::clone(&counter);
                Box::pin(async move {
                    counter.fetch_add(1, Ordering::SeqCst);
                    Ok(adk_tools::success(json!({})))
                })
            },
        )
        .require_confirmation("Approve this reimbursement?");

        let model = MockModel::new().push_call_json("reimburse", json!({"amount": 5000}));
        let agent = LlmAgent::builder("a")
            .model(Arc::new(model))
            .tool(ToolSource::Tool(tool.shared()))
            .build()
            .unwrap();

        let events = drain(&agent, &context_with("refund me")).await;
        let confirmations = &events[1].actions.requested_tool_confirmations;
        assert_eq!(confirmations.len(), 1);
        assert!(confirmations.values().next().unwrap().hint.contains("Approve"));
        // The gate held: the tool body never ran.
        assert_eq!(ran.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn a_runaway_tool_loop_is_capped() {
        // The mock keeps requesting the tool, so only the cap ends the turn.
        let mut model = MockModel::new();
        for _ in 0..10 {
            model = model.push_call_json("get_weather", json!({"city": "Paris"}));
        }

        let agent = LlmAgent::builder("a")
            .model(Arc::new(model))
            .tool(weather_tool())
            .max_tool_iterations(3)
            .build()
            .unwrap();

        let err = drain_result(&agent, &context_with("go")).await.unwrap_err();
        assert!(matches!(err, adk_core::AdkError::LimitExceeded(_)), "got: {err}");
    }

    // ---- schemas ----

    #[test]
    fn an_output_schema_combined_with_tools_is_rejected_at_build_time() {
        let err = LlmAgent::builder("a")
            .model(Arc::new(MockModel::new()))
            .tool(weather_tool())
            .output_schema(Schema::object())
            .build()
            .unwrap_err();
        assert!(err.to_string().contains("output schema"), "got: {err}");
    }

    #[test]
    fn an_agent_without_a_model_is_rejected() {
        assert!(LlmAgent::builder("a").build().is_err());
    }

    #[tokio::test]
    async fn a_response_violating_the_output_schema_is_rejected() {
        let agent = LlmAgent::builder("a")
            .model(Arc::new(MockModel::new().push_text("not json")))
            .output_schema(Schema::object().property("capital", Schema::string()))
            .build()
            .unwrap();

        let err = drain_result(&agent, &context_with("go")).await.unwrap_err();
        assert!(err.to_string().contains("a.output"), "got: {err}");
    }

    #[tokio::test]
    async fn a_response_matching_the_output_schema_passes() {
        let agent = LlmAgent::builder("a")
            .model(Arc::new(
                MockModel::new().push_text(r#"{"capital": "Paris"}"#),
            ))
            .output_schema(Schema::object().property("capital", Schema::string()))
            .build()
            .unwrap();

        assert!(drain_result(&agent, &context_with("go")).await.is_ok());
    }

    // ---- callbacks ----

    #[tokio::test]
    async fn a_before_agent_callback_can_short_circuit_the_agent() {
        let model = Arc::new(MockModel::new().push_text("should not be called"));
        let agent = LlmAgent::builder("a")
            .model(model.clone())
            .callbacks(Callbacks::new().before_agent(|_ctx| {
                Box::pin(async { Some(Content::model_text("blocked by policy")) })
            }))
            .build()
            .unwrap();

        let events = drain(&agent, &context_with("go")).await;
        assert_eq!(events[0].text(), "blocked by policy");
        assert_eq!(model.call_count(), 0);
    }

    #[tokio::test]
    async fn a_before_model_callback_can_replace_the_model_call() {
        let model = Arc::new(MockModel::new().push_text("live answer"));
        let agent = LlmAgent::builder("a")
            .model(model.clone())
            .callbacks(Callbacks::new().before_model(|_ctx, _req| {
                Box::pin(async { Some(LlmResponse::text("cached answer")) })
            }))
            .build()
            .unwrap();

        let events = drain(&agent, &context_with("go")).await;
        assert_eq!(events[0].text(), "cached answer");
        assert_eq!(model.call_count(), 0);
    }

    #[tokio::test]
    async fn a_before_model_callback_can_mutate_the_request() {
        let model = Arc::new(MockModel::new().push_text("ok"));
        let agent = LlmAgent::builder("a")
            .model(model.clone())
            .instruction("original")
            .callbacks(Callbacks::new().before_model(|_ctx, req| {
                Box::pin(async move {
                    req.system_instruction = Some("rewritten".into());
                    None
                })
            }))
            .build()
            .unwrap();

        drain(&agent, &context_with("go")).await;
        assert_eq!(
            model.recorded_requests()[0].system_instruction.as_deref(),
            Some("rewritten")
        );
    }

    #[tokio::test]
    async fn an_after_model_callback_can_replace_the_response() {
        let agent = LlmAgent::builder("a")
            .model(Arc::new(MockModel::new().push_text("raw")))
            .callbacks(Callbacks::new().after_model(|_ctx, _resp| {
                Box::pin(async { Some(LlmResponse::text("filtered")) })
            }))
            .build()
            .unwrap();

        assert_eq!(drain(&agent, &context_with("go")).await[0].text(), "filtered");
    }

    #[tokio::test]
    async fn a_before_tool_callback_can_replace_the_tool_result() {
        let model = MockModel::new()
            .push_call_json("get_weather", json!({"city": "Paris"}))
            .push_text("done");

        let agent = LlmAgent::builder("a")
            .model(Arc::new(model))
            .tool(weather_tool())
            .callbacks(Callbacks::new().before_tool(|_ctx, name, _args| {
                let name = name.to_string();
                Box::pin(async move {
                    (name == "get_weather").then(|| json!({"status": "success", "report": "stubbed"}))
                })
            }))
            .build()
            .unwrap();

        let events = drain(&agent, &context_with("go")).await;
        assert_eq!(events[1].function_responses()[0].response["report"], "stubbed");
    }

    #[tokio::test]
    async fn an_after_tool_callback_can_transform_the_result() {
        let model = MockModel::new()
            .push_call_json("get_weather", json!({"city": "Paris"}))
            .push_text("done");

        let agent = LlmAgent::builder("a")
            .model(Arc::new(model))
            .tool(weather_tool())
            .callbacks(Callbacks::new().after_tool(|_ctx, _name, _args, result| {
                let mut result = result.clone();
                Box::pin(async move {
                    result["redacted"] = json!(true);
                    Some(result)
                })
            }))
            .build()
            .unwrap();

        let events = drain(&agent, &context_with("go")).await;
        assert_eq!(events[1].function_responses()[0].response["redacted"], true);
    }

    // ---- streaming ----

    #[tokio::test]
    async fn streaming_yields_partial_events_then_a_final_one() {
        let services = Services::new(Arc::new(InMemorySessionService::new()));
        let mut session = Session::new("s1", "app", "u1");
        session
            .events
            .push(Event::new("inv", "user").with_content(Content::user_text("hi")));
        let ctx = InvocationContext::new(
            session,
            services,
            RunConfig {
                streaming_mode: StreamingMode::Sse,
                ..Default::default()
            },
        );

        let agent = LlmAgent::builder("a")
            .model(Arc::new(MockModel::new().push_stream(["The ", "answer."])))
            .build()
            .unwrap();

        let events = drain(&agent, &ctx).await;
        let partials: Vec<_> = events.iter().filter(|e| e.is_partial()).collect();
        assert_eq!(partials.len(), 2);
        let final_event = events.last().unwrap();
        assert!(!final_event.is_partial());
        assert_eq!(final_event.text(), "The answer.");
    }

    // ---- workflow agents ----

    fn scripted(name: &str, text: &str) -> SharedAgent {
        LlmAgent::builder(name)
            .model(Arc::new(MockModel::new().push_text(text)))
            .output_key(format!("{name}_out"))
            .build()
            .unwrap()
            .shared()
    }

    #[tokio::test]
    async fn a_sequential_agent_runs_sub_agents_in_order() {
        let agent = SequentialAgent::new("pipeline", vec![scripted("first", "1"), scripted("second", "2")]);
        let events = drain(&agent, &context_with("go")).await;
        let authors: Vec<&str> = events.iter().map(|e| e.author.as_str()).collect();
        assert_eq!(authors, vec!["first", "second"]);
    }

    #[tokio::test]
    async fn a_parallel_agent_tags_each_branch() {
        let agent = ParallelAgent::new("fanout", vec![scripted("a", "1"), scripted("b", "2")]);
        let events = drain(&agent, &context_with("go")).await;
        assert_eq!(events.len(), 2);

        // Each sub-agent stamps its own segment onto the parent's path, so the
        // agent name appears once rather than twice.
        let mut branches: Vec<String> = events.iter().filter_map(|e| e.branch.clone()).collect();
        branches.sort();
        assert_eq!(branches, vec!["fanout.a", "fanout.b"]);
    }

    #[tokio::test]
    async fn a_loop_agent_stops_when_a_sub_agent_escalates() {
        let escalating = FunctionTool::new("finish", "Marks the task complete.", Schema::object(), |_a, ctx| {
            let ctx = ctx.clone();
            Box::pin(async move {
                ctx.escalate();
                Ok(adk_tools::success(json!({"done": true})))
            })
        });

        let worker = LlmAgent::builder("worker")
            .model(Arc::new(
                MockModel::new()
                    .push_call_json("finish", json!({}))
                    .push_text("unused"),
            ))
            .tool(ToolSource::Tool(escalating.shared()))
            .build()
            .unwrap()
            .shared();

        let agent = LoopAgent::new("refine", vec![worker], 10);
        let events = drain(&agent, &context_with("go")).await;
        assert!(events.iter().any(|e| e.actions.escalate));
        // The loop stopped at the escalation rather than running to the cap.
        assert!(events.len() <= 3, "expected an early exit, got {} events", events.len());
    }

    #[tokio::test]
    async fn a_loop_agent_that_never_escalates_hits_its_cap() {
        let agent = LoopAgent::new("spin", vec![scripted("worker", "again")], 3);
        let err = drain_result(&agent, &context_with("go")).await.unwrap_err();
        assert!(err.to_string().contains("max_iterations"), "got: {err}");
    }

    // ---- graph integration ----

    #[tokio::test]
    async fn an_agent_runs_as_a_graph_node_and_its_answer_becomes_the_output() {
        use adk_graph::{chain, Graph};

        let agent = LlmAgent::builder("answerer")
            .model(Arc::new(MockModel::new().push_text("42")))
            .build()
            .unwrap();

        let graph = Graph::new(
            vec![AgentNode::new(agent.shared()).shared()],
            chain(["answerer"]),
        )
        .unwrap();

        let events: Vec<Event> = graph
            .run(context_with("what is the answer?"), None)
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .map(|e| e.unwrap())
            .collect();

        let output = events.last().unwrap().output.clone().unwrap();
        assert_eq!(output, json!("42"));
    }
}
