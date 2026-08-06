//! The ADK runtime event loop.
//!
//! [`Runner`] is the orchestrator. It loads the session, appends the user's
//! message, drives the root agent or graph, and — for every event the agent
//! yields — commits that event's [`EventActions`](adk_core::EventActions)
//! through the session service before forwarding it on.
//!
//! That ordering is the contract ADK specifies: a state write is only
//! guaranteed persistent once the event carrying its delta has been processed.
//! Code that resumes after yielding an event can rely on its state changes
//! having landed.
//!
//! ```
//! # tokio_test::block_on(async {
//! use adk_agents::LlmAgent;
//! use adk_core::{Content, Services};
//! use adk_models::MockModel;
//! use adk_runner::Runner;
//! use adk_sessions::InMemorySessionService;
//! use futures::StreamExt;
//! use std::sync::Arc;
//!
//! let agent = LlmAgent::builder("greeter")
//!     .model(Arc::new(MockModel::new().push_text("Hello!")))
//!     .output_key("last_greeting")
//!     .build()
//!     .unwrap();
//!
//! let services = Services::new(Arc::new(InMemorySessionService::new()));
//! let runner = Runner::new("my_app", agent.shared(), services);
//!
//! let session = runner.create_session("u1", None).await.unwrap();
//! let events: Vec<_> = runner
//!     .run(&session.user_id, &session.id, Content::user_text("hi"), None)
//!     .collect()
//!     .await;
//!
//! assert_eq!(events.last().unwrap().as_ref().unwrap().text(), "Hello!");
//!
//! // The agent's state write was committed by the runner, not just staged.
//! let saved = runner.session("u1", &session.id).await.unwrap().unwrap();
//! assert_eq!(saved.state.get("last_greeting").unwrap(), "Hello!");
//! # });
//! ```

#![deny(missing_docs)]
#![warn(clippy::all)]

use adk_agents::SharedAgent;
use adk_core::{
    AdkError, Content, Event, InvocationContext, Result, RunConfig, Services, Session, State,
};
use adk_graph::{Graph, ResumeRequest};
use futures::stream::BoxStream;
use futures::StreamExt;
use std::sync::Arc;

/// What the runner drives.
pub enum RootExecutor {
    /// A single agent and its sub-agents.
    Agent(SharedAgent),
    /// A workflow graph.
    Graph(Arc<Graph>),
}

impl From<SharedAgent> for RootExecutor {
    fn from(agent: SharedAgent) -> Self {
        RootExecutor::Agent(agent)
    }
}

impl From<Arc<Graph>> for RootExecutor {
    fn from(graph: Arc<Graph>) -> Self {
        RootExecutor::Graph(graph)
    }
}

/// Orchestrates one application's runs.
pub struct Runner {
    app_name: String,
    root: RootExecutor,
    services: Services,
}

impl Runner {
    /// Builds a runner for `app_name`.
    pub fn new(
        app_name: impl Into<String>,
        root: impl Into<RootExecutor>,
        services: Services,
    ) -> Self {
        Self {
            app_name: app_name.into(),
            root: root.into(),
            services,
        }
    }

    /// The application name.
    pub fn app_name(&self) -> &str {
        &self.app_name
    }

    /// The service bundle.
    pub fn services(&self) -> &Services {
        &self.services
    }

    /// Creates a session for `user_id`.
    pub async fn create_session(&self, user_id: &str, state: Option<State>) -> Result<Session> {
        self.services
            .session
            .create_session(&self.app_name, user_id, state, None)
            .await
    }

    /// Loads a session.
    pub async fn session(&self, user_id: &str, session_id: &str) -> Result<Option<Session>> {
        self.services
            .session
            .get_session(&self.app_name, user_id, session_id)
            .await
    }

    /// Handles one user message, streaming the events it produces.
    ///
    /// Each event is committed through the session service before it is
    /// forwarded, so a consumer that observes an event can rely on its state
    /// delta and artifact delta already being persisted.
    pub fn run<'a>(
        &'a self,
        user_id: &'a str,
        session_id: &'a str,
        new_message: Content,
        run_config: Option<RunConfig>,
    ) -> BoxStream<'a, Result<Event>> {
        self.run_inner(user_id, session_id, Some(new_message), run_config, None)
    }

    /// Resumes a run that a graph node suspended for human input.
    ///
    /// Only meaningful for a graph root; an agent root has no suspension point.
    pub fn resume<'a>(
        &'a self,
        user_id: &'a str,
        session_id: &'a str,
        resume: ResumeRequest,
        run_config: Option<RunConfig>,
    ) -> BoxStream<'a, Result<Event>> {
        self.run_inner(user_id, session_id, None, run_config, Some(resume))
    }

    fn run_inner<'a>(
        &'a self,
        user_id: &'a str,
        session_id: &'a str,
        new_message: Option<Content>,
        run_config: Option<RunConfig>,
        resume: Option<ResumeRequest>,
    ) -> BoxStream<'a, Result<Event>> {
        Box::pin(async_stream::try_stream! {
            let mut session = self
                .services
                .session
                .get_session(&self.app_name, user_id, session_id)
                .await?
                .ok_or_else(|| AdkError::SessionNotFound(session_id.to_string()))?;

            let ctx = InvocationContext::new(
                session.clone(),
                self.services.clone(),
                run_config.unwrap_or_default(),
            );

            // The user's turn is committed before the agent sees it, so the
            // history the agent reads already contains the message.
            if let Some(content) = new_message {
                let user_event =
                    Event::new(&ctx.invocation_id, "user").with_content(content);
                self.services
                    .session
                    .append_event(&mut session, user_event.clone())
                    .await?;
                ctx.with_session_mut(|s| *s = session.clone());
                yield user_event;
            }

            let mut stream: BoxStream<'_, Result<Event>> = match &self.root {
                RootExecutor::Agent(agent) => agent.run(&ctx),
                RootExecutor::Graph(graph) => graph.run(ctx.clone(), resume),
            };

            while let Some(event) = stream.next().await {
                let event = event?;

                // Commit first, forward second: this is the ordering the ADK
                // runtime guarantees.
                self.services
                    .session
                    .append_event(&mut session, event.clone())
                    .await?;

                // Reflect the commit back into the running context so the agent
                // observes it when it resumes after the yield. Apply the event's
                // delta rather than overwriting the whole State: a tool that ran
                // during this step may already have staged writes that have not
                // yet ridden out on an event, and replacing State wholesale
                // would silently discard them.
                if !event.is_partial() {
                    let delta = event.actions.state_delta.clone();
                    ctx.with_state_mut(|state| state.commit(delta));
                }

                yield event;
            }

            // Temporary state exists only for the invocation that wrote it.
            ctx.with_state_mut(State::clear_temp);
        })
    }

    /// Runs to completion and returns the final response text, if any.
    ///
    /// A convenience for callers that do not need the intermediate events.
    pub async fn run_to_completion(
        &self,
        user_id: &str,
        session_id: &str,
        new_message: Content,
        run_config: Option<RunConfig>,
    ) -> Result<Option<String>> {
        let mut stream = self.run(user_id, session_id, new_message, run_config);
        let mut last = None;
        while let Some(event) = stream.next().await {
            let event = event?;
            if event.author != "user" && event.is_final_response() {
                let text = event.text();
                if !text.is_empty() {
                    last = Some(text);
                }
            }
        }
        Ok(last)
    }
}

impl std::fmt::Debug for Runner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let root = match &self.root {
            RootExecutor::Agent(a) => format!("agent:{}", a.name()),
            RootExecutor::Graph(_) => "graph".to_string(),
        };
        f.debug_struct("Runner")
            .field("app_name", &self.app_name)
            .field("root", &root)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use adk_agents::{Agent, LlmAgent};
    use adk_core::{Schema, SessionService};
    use adk_models::MockModel;
    use adk_sessions::InMemorySessionService;
    use adk_tools::{FunctionTool, ToolSource};
    use serde_json::json;

    fn services() -> Services {
        Services::new(Arc::new(InMemorySessionService::new()))
    }

    fn runner_with(agent: SharedAgent) -> Runner {
        Runner::new("app", agent, services())
    }

    async fn collect(runner: &Runner, session: &Session, text: &str) -> Vec<Event> {
        runner
            .run(&session.user_id, &session.id, Content::user_text(text), None)
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .map(|e| e.expect("run failed"))
            .collect()
    }

    #[tokio::test]
    async fn the_user_message_is_committed_before_the_agent_runs() {
        let agent = LlmAgent::builder("a")
            .model(Arc::new(MockModel::new().push_text("hi back")))
            .build()
            .unwrap()
            .shared();
        let runner = runner_with(agent);
        let session = runner.create_session("u1", None).await.unwrap();

        let events = collect(&runner, &session, "hello").await;
        assert_eq!(events[0].author, "user");
        assert_eq!(events[0].text(), "hello");

        let saved = runner.session("u1", &session.id).await.unwrap().unwrap();
        assert_eq!(saved.events[0].text(), "hello");
    }

    #[tokio::test]
    async fn state_written_by_an_agent_is_persisted_by_the_runner() {
        let agent = LlmAgent::builder("a")
            .model(Arc::new(MockModel::new().push_text("Bonjour")))
            .output_key("greeting")
            .build()
            .unwrap()
            .shared();
        let runner = runner_with(agent);
        let session = runner.create_session("u1", None).await.unwrap();

        collect(&runner, &session, "hi").await;
        let saved = runner.session("u1", &session.id).await.unwrap().unwrap();
        assert_eq!(saved.state.get("greeting").unwrap(), &json!("Bonjour"));
    }

    #[tokio::test]
    async fn temp_state_does_not_survive_the_invocation() {
        let tool = FunctionTool::new("scratch", "Writes scratch state.", Schema::object(), |_a, ctx| {
            let ctx = ctx.clone();
            Box::pin(async move {
                ctx.set_state("temp:scratch", 1);
                ctx.set_state("durable", 2);
                Ok(adk_tools::success(json!({})))
            })
        });
        let agent = LlmAgent::builder("a")
            .model(Arc::new(
                MockModel::new()
                    .push_call_json("scratch", json!({}))
                    .push_text("done"),
            ))
            .tool(ToolSource::Tool(tool.shared()))
            .build()
            .unwrap()
            .shared();

        let runner = runner_with(agent);
        let session = runner.create_session("u1", None).await.unwrap();
        collect(&runner, &session, "go").await;

        let saved = runner.session("u1", &session.id).await.unwrap().unwrap();
        assert_eq!(saved.state.get("durable"), Some(&json!(2)));
        assert!(saved.state.get("temp:scratch").is_none());
    }

    #[tokio::test]
    async fn history_accumulates_across_turns() {
        let agent = LlmAgent::builder("a")
            .model(Arc::new(
                MockModel::new().push_text("first").push_text("second"),
            ))
            .build()
            .unwrap()
            .shared();
        let runner = runner_with(agent);
        let session = runner.create_session("u1", None).await.unwrap();

        collect(&runner, &session, "one").await;
        collect(&runner, &session, "two").await;

        let saved = runner.session("u1", &session.id).await.unwrap().unwrap();
        // two user turns plus two agent turns
        assert_eq!(saved.events.len(), 4);
        assert_eq!(saved.events[2].text(), "two");
    }

    #[tokio::test]
    async fn run_to_completion_returns_the_final_text() {
        let agent = LlmAgent::builder("a")
            .model(Arc::new(MockModel::new().push_text("the answer")))
            .build()
            .unwrap()
            .shared();
        let runner = runner_with(agent);
        let session = runner.create_session("u1", None).await.unwrap();

        let answer = runner
            .run_to_completion("u1", &session.id, Content::user_text("q"), None)
            .await
            .unwrap();
        assert_eq!(answer.as_deref(), Some("the answer"));
    }

    #[tokio::test]
    async fn an_unknown_session_is_reported() {
        let agent = LlmAgent::builder("a")
            .model(Arc::new(MockModel::new()))
            .build()
            .unwrap()
            .shared();
        let runner = runner_with(agent);

        let result: Result<Vec<Event>> = runner
            .run("u1", "nope", Content::user_text("hi"), None)
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .collect();
        assert!(matches!(result.unwrap_err(), AdkError::SessionNotFound(_)));
    }

    #[tokio::test]
    async fn a_graph_root_runs_and_can_be_resumed() {
        use adk_graph::{chain, FunctionNode, NodeConfig, NodeOutcome};

        let ask = FunctionNode::new("ask", NodeConfig::default(), |ctx| {
            let ctx = ctx.clone();
            Box::pin(async move {
                let answer = ctx.resume_or_request_input("Approve?", None)?;
                Ok(NodeOutcome::output(answer))
            })
        })
        .shared();
        let graph = Arc::new(Graph::new(vec![ask], chain(["ask"])).unwrap());

        let runner = Runner::new("app", graph, services());
        let session = runner.create_session("u1", None).await.unwrap();

        let events = collect(&runner, &session, "start").await;
        let interrupt_id = events
            .iter()
            .find_map(|e| e.request_input.as_ref().map(|r| r.interrupt_id.clone()))
            .expect("expected a suspension");

        let resumed: Vec<Event> = runner
            .resume(
                "u1",
                &session.id,
                ResumeRequest::new(interrupt_id, json!("approved")),
                None,
            )
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .map(|e| e.unwrap())
            .collect();

        assert_eq!(resumed.last().unwrap().output, Some(json!("approved")));
    }

    #[tokio::test]
    async fn every_event_is_recorded_in_session_history() {
        let agent = LlmAgent::builder("a")
            .model(Arc::new(MockModel::new().push_text("done")))
            .build()
            .unwrap()
            .shared();
        let runner = runner_with(agent);
        let session = runner.create_session("u1", None).await.unwrap();

        let streamed = collect(&runner, &session, "go").await;
        let saved = runner
            .services()
            .session
            .get_session("app", "u1", &session.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(saved.events.len(), streamed.len());
    }
}
