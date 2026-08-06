//! The ADK 2.0 workflow graph engine.
//!
//! ADK 2.0 replaced hierarchical agent execution with a graph engine: agents,
//! tools, and plain functions are all [`Node`]s evaluated within a workflow
//! graph, and data flows between them through
//! [`Event::output`](adk_core::Event::output) rather than through session state.
//!
//! ```
//! # tokio_test::block_on(async {
//! use adk_core::{InvocationContext, RunConfig, Services, Session};
//! use adk_graph::{chain, FunctionNode, Graph, NodeConfig, NodeOutcome};
//! use adk_sessions::InMemorySessionService;
//! use futures::StreamExt;
//! use serde_json::json;
//! use std::sync::Arc;
//!
//! // Two nodes: one produces a city, the next upper-cases it.
//! let city = FunctionNode::new("city", NodeConfig::default(), |_ctx| {
//!     Box::pin(async { Ok(NodeOutcome::output(json!("Tokyo"))) })
//! })
//! .shared();
//!
//! let shout = FunctionNode::new("shout", NodeConfig::default(), |ctx| {
//!     let input = ctx.input.clone();
//!     Box::pin(async move {
//!         let name = input.as_ref().and_then(|v| v.as_str()).unwrap_or("");
//!         Ok(NodeOutcome::output(json!(name.to_uppercase())))
//!     })
//! })
//! .shared();
//!
//! let graph = Graph::new(vec![city, shout], chain(["city", "shout"])).unwrap();
//!
//! let services = Services::new(Arc::new(InMemorySessionService::new()));
//! let ctx = InvocationContext::new(Session::new("s", "app", "u"), services, RunConfig::default());
//!
//! let events: Vec<_> = graph.run(ctx, None).collect().await;
//! let last = events.last().unwrap().as_ref().unwrap();
//! assert_eq!(last.output, Some(json!("TOKYO")));
//! # });
//! ```

#![deny(missing_docs)]
#![warn(clippy::all)]

pub mod edge;
pub mod executor;
pub mod node;
pub mod nodes;

pub use edge::{chain, concat, Edge, EdgeBuilder, Route};
pub use executor::{Graph, PendingInterrupt, ResumeRequest, PENDING_STATE_KEY};
pub use node::{Node, NodeConfig, NodeContext, NodeOutcome, SharedNode, START};
pub use nodes::{constant_node, FunctionNode, JoinNode, NodeFn, RouterNode};

#[cfg(test)]
mod tests {
    use super::*;
    use adk_core::{AdkError, Event, InvocationContext, RunConfig, Services, Session};
    use adk_sessions::InMemorySessionService;
    use futures::StreamExt;
    use serde_json::{json, Value};
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    fn context() -> InvocationContext {
        let services = Services::new(Arc::new(InMemorySessionService::new()));
        InvocationContext::new(
            Session::new("s1", "app", "u1"),
            services,
            RunConfig::default(),
        )
    }

    fn value_node(name: &'static str, value: Value) -> SharedNode {
        constant_node(name, value)
    }

    async fn collect(graph: &Graph, ctx: InvocationContext) -> Vec<Event> {
        graph
            .run(ctx, None)
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .map(|e| e.expect("run failed"))
            .collect()
    }

    async fn collect_result(
        graph: &Graph,
        ctx: InvocationContext,
    ) -> adk_core::Result<Vec<Event>> {
        graph.run(ctx, None).collect::<Vec<_>>().await.into_iter().collect()
    }

    // ---- validation ----

    #[test]
    fn an_edge_to_an_unknown_node_is_rejected() {
        let err = Graph::new(
            vec![value_node("a", json!(1))],
            vec![Edge::new(START, "a"), Edge::new("a", "ghost")],
        )
        .unwrap_err();
        assert!(err.to_string().contains("ghost"), "got: {err}");
    }

    #[test]
    fn a_graph_without_an_entry_point_is_rejected() {
        let err = Graph::new(vec![value_node("a", json!(1))], vec![]).unwrap_err();
        assert!(err.to_string().contains("entry point"), "got: {err}");
    }

    #[test]
    fn duplicate_node_names_are_rejected() {
        let err = Graph::new(
            vec![value_node("a", json!(1)), value_node("a", json!(2))],
            vec![Edge::new(START, "a")],
        )
        .unwrap_err();
        assert!(err.to_string().contains("duplicate"), "got: {err}");
    }

    // ---- data flow ----

    #[tokio::test]
    async fn output_flows_from_one_node_to_the_next() {
        let double = FunctionNode::new("double", NodeConfig::default(), |ctx| {
            let input = ctx.input.clone();
            Box::pin(async move {
                let n = input.as_ref().and_then(Value::as_i64).unwrap_or(0);
                Ok(NodeOutcome::output(json!(n * 2)))
            })
        })
        .shared();

        let graph = Graph::new(
            vec![value_node("seed", json!(21)), double],
            chain(["seed", "double"]),
        )
        .unwrap();

        let events = collect(&graph, context()).await;
        assert_eq!(events.last().unwrap().output, Some(json!(42)));
    }

    #[tokio::test]
    async fn events_carry_node_info_with_step_and_predecessor() {
        let graph = Graph::new(
            vec![value_node("a", json!(1)), value_node("b", json!(2))],
            chain(["a", "b"]),
        )
        .unwrap();

        let events = collect(&graph, context()).await;
        let b = events.iter().find(|e| e.author == "b").unwrap();
        let info = b.node_info.as_ref().unwrap();
        assert_eq!(info.name, "b");
        assert_eq!(info.step, Some(1));
        assert_eq!(info.predecessor.as_deref(), Some("a"));
    }

    #[tokio::test]
    async fn a_typed_node_reports_an_input_type_mismatch() {
        let typed = FunctionNode::typed::<i64, i64, _>(
            "needs_int",
            NodeConfig::default(),
            |n, _ctx| Box::pin(async move { Ok(n + 1) }),
        )
        .shared();

        let graph = Graph::new(
            vec![value_node("text", json!("not a number")), typed],
            chain(["text", "needs_int"]),
        )
        .unwrap();

        let err = collect_result(&graph, context()).await.unwrap_err();
        assert!(err.to_string().contains("needs_int.input"), "got: {err}");
    }

    // ---- routing ----

    fn router(name: &'static str, route: &'static str) -> SharedNode {
        RouterNode::new(name, NodeConfig::default(), move |_ctx| {
            Box::pin(async move { Ok((json!("payload"), vec![route.to_string()])) })
        })
        .shared()
    }

    #[tokio::test]
    async fn a_matching_route_selects_its_branch() {
        let graph = Graph::new(
            vec![
                router("classify", "BUG"),
                value_node("bug", json!("bug handled")),
                value_node("support", json!("support handled")),
            ],
            EdgeBuilder::new()
                .start("classify")
                .add_route("classify", "bug", Route::string("BUG"))
                .add_route("classify", "support", Route::string("CUSTOMER_SUPPORT"))
                .build(),
        )
        .unwrap();

        let events = collect(&graph, context()).await;
        assert!(events.iter().any(|e| e.author == "bug"));
        assert!(!events.iter().any(|e| e.author == "support"));
    }

    #[tokio::test]
    async fn an_unmatched_route_falls_back_to_default() {
        let graph = Graph::new(
            vec![
                router("classify", "SOMETHING_ELSE"),
                value_node("bug", json!(1)),
                value_node("fallback", json!(2)),
            ],
            EdgeBuilder::new()
                .start("classify")
                .add_route("classify", "bug", Route::string("BUG"))
                .add_default("classify", "fallback")
                .build(),
        )
        .unwrap();

        let events = collect(&graph, context()).await;
        assert!(events.iter().any(|e| e.author == "fallback"));
    }

    #[tokio::test]
    async fn an_unmatched_route_with_no_default_is_an_error() {
        let graph = Graph::new(
            vec![router("classify", "NOPE"), value_node("bug", json!(1))],
            EdgeBuilder::new()
                .start("classify")
                .add_route("classify", "bug", Route::string("BUG"))
                .build(),
        )
        .unwrap();

        let err = collect_result(&graph, context()).await.unwrap_err();
        assert!(matches!(err, AdkError::NoRoute { .. }), "got: {err}");
    }

    // ---- fan-out and join ----

    #[tokio::test]
    async fn a_join_aggregates_every_branch_keyed_by_node() {
        let graph = Graph::new(
            vec![
                value_node("dispatch", json!("go")),
                value_node("a", json!("alpha")),
                value_node("b", json!("beta")),
                JoinNode::new("gather").shared(),
            ],
            EdgeBuilder::new()
                .start("dispatch")
                .add_fan_out("dispatch", ["a", "b"])
                .add_fan_in("gather", ["a", "b"])
                .build(),
        )
        .unwrap();

        let events = collect(&graph, context()).await;
        let joined = events
            .iter()
            .find(|e| e.author == "gather")
            .unwrap()
            .output
            .clone()
            .unwrap();
        assert_eq!(joined, json!({"a": "alpha", "b": "beta"}));
    }

    #[tokio::test]
    async fn a_join_that_never_fills_is_reported_rather_than_silently_skipped() {
        // `b` is routed around, so `gather` only ever sees one predecessor.
        let graph = Graph::new(
            vec![
                router("dispatch", "ONLY_A"),
                value_node("a", json!(1)),
                value_node("b", json!(2)),
                JoinNode::new("gather").shared(),
            ],
            EdgeBuilder::new()
                .start("dispatch")
                .add_route("dispatch", "a", Route::string("ONLY_A"))
                .add_route("dispatch", "b", Route::string("ALSO_B"))
                .add_fan_in("gather", ["a", "b"])
                .build(),
        )
        .unwrap();

        let err = collect_result(&graph, context()).await.unwrap_err();
        assert!(err.to_string().contains("gather"), "got: {err}");
    }

    // ---- retries ----

    #[tokio::test]
    async fn a_retryable_failure_is_retried_up_to_the_budget() {
        let attempts = Arc::new(AtomicU32::new(0));
        let counter = Arc::clone(&attempts);
        let flaky = FunctionNode::new(
            "flaky",
            NodeConfig::default().with_retries(3),
            move |_ctx| {
                let counter = Arc::clone(&counter);
                Box::pin(async move {
                    let n = counter.fetch_add(1, Ordering::SeqCst);
                    if n < 2 {
                        return Err(AdkError::model("m", "transient"));
                    }
                    Ok(NodeOutcome::output(json!("recovered")))
                })
            },
        )
        .shared();

        let graph = Graph::new(vec![flaky], chain(["flaky"])).unwrap();
        let events = collect(&graph, context()).await;
        assert_eq!(events.last().unwrap().output, Some(json!("recovered")));
        assert_eq!(attempts.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn a_non_retryable_failure_is_not_retried() {
        let attempts = Arc::new(AtomicU32::new(0));
        let counter = Arc::clone(&attempts);
        let bad = FunctionNode::new("bad", NodeConfig::default().with_retries(5), move |_ctx| {
            let counter = Arc::clone(&counter);
            Box::pin(async move {
                counter.fetch_add(1, Ordering::SeqCst);
                Err(AdkError::validation("x", "always wrong"))
            })
        })
        .shared();

        let graph = Graph::new(vec![bad], chain(["bad"])).unwrap();
        assert!(collect_result(&graph, context()).await.is_err());
        assert_eq!(attempts.load(Ordering::SeqCst), 1);
    }

    // ---- interrupts and resume ----

    fn approval_graph() -> Graph {
        let ask = FunctionNode::new("ask", NodeConfig::default(), |ctx| {
            let ctx = ctx.clone();
            Box::pin(async move {
                let answer = ctx.resume_or_request_input("Approve?", None)?;
                Ok(NodeOutcome::output(answer))
            })
        })
        .shared();

        let after = FunctionNode::new("after", NodeConfig::default(), |ctx| {
            let input = ctx.input.clone();
            Box::pin(async move { Ok(NodeOutcome::output(input.unwrap_or(Value::Null))) })
        })
        .shared();

        Graph::new(vec![ask, after], chain(["ask", "after"])).unwrap()
    }

    #[tokio::test]
    async fn a_node_can_suspend_the_run_for_human_input() {
        let graph = approval_graph();
        let ctx = context();
        let events = collect(&graph, ctx.clone()).await;

        // The run ends at the suspension, with a request the caller can answer.
        let request = events
            .iter()
            .find_map(|e| e.request_input.clone())
            .expect("expected a request for input");
        assert_eq!(request.hint, "Approve?");
        assert!(!events.iter().any(|e| e.author == "after"));

        let pending = Graph::pending_interrupt(&ctx).unwrap();
        assert_eq!(pending.node, "ask");
        assert_eq!(pending.interrupt_id, request.interrupt_id);
    }

    #[tokio::test]
    async fn resuming_re_runs_the_node_and_continues_the_graph() {
        let graph = approval_graph();
        let ctx = context();
        let events = collect(&graph, ctx.clone()).await;
        let interrupt_id = events
            .iter()
            .find_map(|e| e.request_input.as_ref().map(|r| r.interrupt_id.clone()))
            .unwrap();

        let resumed: Vec<Event> = graph
            .run(
                ctx.clone(),
                Some(ResumeRequest::new(interrupt_id, json!("approved"))),
            )
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .map(|e| e.unwrap())
            .collect();

        // The suspended node ran again, this time with the answer in hand.
        assert!(resumed.iter().any(|e| e.author == "ask"));
        let last = resumed.last().unwrap();
        assert_eq!(last.author, "after");
        assert_eq!(last.output, Some(json!("approved")));
    }

    #[tokio::test]
    async fn resuming_with_the_wrong_interrupt_id_is_rejected() {
        let graph = approval_graph();
        let ctx = context();
        collect(&graph, ctx.clone()).await;

        let result: adk_core::Result<Vec<Event>> = graph
            .run(ctx, Some(ResumeRequest::new("wrong-id", json!("x"))))
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .collect();
        assert!(result.unwrap_err().to_string().contains("mismatch"));
    }

    // ---- limits, cancellation, and emitted messages ----

    #[tokio::test]
    async fn a_cycle_is_stopped_by_the_step_budget() {
        let loop_node = value_node("loop", json!(1));
        let graph = Graph::new(
            vec![loop_node],
            vec![Edge::new(START, "loop"), Edge::new("loop", "loop")],
        )
        .unwrap();

        let services = Services::new(Arc::new(InMemorySessionService::new()));
        let ctx = InvocationContext::new(
            Session::new("s", "app", "u"),
            services,
            RunConfig::default().with_max_graph_steps(5),
        );

        let err = collect_result(&graph, ctx).await.unwrap_err();
        assert!(matches!(err, AdkError::LimitExceeded(_)), "got: {err}");
    }

    #[tokio::test]
    async fn cancellation_stops_the_run() {
        let graph = Graph::new(vec![value_node("a", json!(1))], chain(["a"])).unwrap();
        let ctx = context();
        ctx.cancel();
        let err = collect_result(&graph, ctx).await.unwrap_err();
        assert!(matches!(err, AdkError::Cancelled));
    }

    #[tokio::test]
    async fn nodes_can_emit_progress_messages_before_their_output() {
        let chatty = FunctionNode::new("chatty", NodeConfig::default(), |ctx| {
            let ctx = ctx.clone();
            Box::pin(async move {
                ctx.emit_message("Beginning research...")?;
                Ok(NodeOutcome::output(json!("done")))
            })
        })
        .shared();

        let graph = Graph::new(vec![chatty], chain(["chatty"])).unwrap();
        let events = collect(&graph, context()).await;
        assert_eq!(events[0].text(), "Beginning research...");
        assert_eq!(events[1].output, Some(json!("done")));
    }

    #[tokio::test]
    async fn state_written_by_a_node_rides_out_on_its_event() {
        let writer = FunctionNode::new("writer", NodeConfig::default(), |ctx| {
            let ctx = ctx.clone();
            Box::pin(async move {
                ctx.invocation.set_state("attempts", 1);
                Ok(NodeOutcome::empty())
            })
        })
        .shared();

        let graph = Graph::new(vec![writer], chain(["writer"])).unwrap();
        let events = collect(&graph, context()).await;
        assert_eq!(
            events.last().unwrap().actions.state_delta.get("attempts"),
            Some(&json!(1))
        );
    }
}
