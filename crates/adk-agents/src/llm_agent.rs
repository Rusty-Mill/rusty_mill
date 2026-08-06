//! [`LlmAgent`] — an agent that reasons with a model and calls tools.

use adk_core::{
    AdkError, Args, Content, Event, FunctionResponse, InvocationContext, Part, Result, Role,
    Schema, State, StreamingMode,
};
use adk_models::{GenerateContentConfig, LlmRequest, LlmResponse, SharedModel};
use adk_tools::{invoke_tool, resolve_tools, SharedTool, ToolContext, ToolSource};
use async_trait::async_trait;
use futures::stream::BoxStream;
use futures::StreamExt;
use std::collections::BTreeSet;
use std::sync::Arc;

use crate::agent::{Agent, SharedAgent};
use crate::callbacks::{CallbackContext, Callbacks};

/// Whether the agent sees prior conversation history.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum IncludeContents {
    /// Send the session's history with each request.
    #[default]
    Default,
    /// Send only the current turn, for stateless operation.
    None,
}

/// An agent powered by a language model.
///
/// # Example
///
/// ```
/// # tokio_test::block_on(async {
/// use adk_agents::{Agent, LlmAgent};
/// use adk_models::MockModel;
/// use std::sync::Arc;
///
/// let agent = LlmAgent::builder("capital_agent")
///     .model(Arc::new(MockModel::new().push_text("Paris")))
///     .instruction("Answer with just the capital city.")
///     .description("Provides capital cities.")
///     .output_key("found_capital")
///     .build()
///     .unwrap();
///
/// assert_eq!(agent.name(), "capital_agent");
/// # });
/// ```
pub struct LlmAgent {
    name: String,
    description: String,
    model: SharedModel,
    instruction: String,
    global_instruction: Option<String>,
    tools: Vec<ToolSource>,
    sub_agents: Vec<SharedAgent>,
    output_key: Option<String>,
    input_schema: Option<Schema>,
    output_schema: Option<Schema>,
    generate_content_config: GenerateContentConfig,
    include_contents: IncludeContents,
    callbacks: Callbacks,
    max_tool_iterations: u32,
}

impl LlmAgent {
    /// Starts building an agent.
    pub fn builder(name: impl Into<String>) -> LlmAgentBuilder {
        LlmAgentBuilder::new(name)
    }

    /// The model this agent uses.
    pub fn model(&self) -> &SharedModel {
        &self.model
    }

    /// The state key this agent's final response is stored under, if any.
    pub fn output_key(&self) -> Option<&str> {
        self.output_key.as_deref()
    }

    /// Wraps this agent for sharing.
    pub fn shared(self) -> SharedAgent {
        Arc::new(self)
    }

    /// Tags an event with this agent's branch path before it is yielded.
    ///
    /// The branch is what keeps parallel branches separable in the session
    /// history, so every event an agent produces carries it.
    fn stamp(ctx: &InvocationContext, mut event: Event) -> Event {
        if event.branch.is_none() {
            event.branch = ctx.branch.clone();
        }
        event
    }

    /// Renders the system instruction, substituting `{key}` from state.
    ///
    /// An unknown placeholder is left as written rather than replaced with an
    /// empty string: silently dropping it would hand the model a sentence with
    /// a hole in it, which is far harder to notice than the literal braces.
    fn render_instruction(&self, state: &State) -> Option<String> {
        let mut sections = Vec::new();
        if let Some(global) = &self.global_instruction {
            sections.push(substitute(global, state));
        }
        if !self.instruction.is_empty() {
            sections.push(substitute(&self.instruction, state));
        }
        (!sections.is_empty()).then(|| sections.join("\n\n"))
    }

    /// Builds the model request for this turn.
    async fn build_request(
        &self,
        ctx: &InvocationContext,
        tools: &[SharedTool],
    ) -> Result<LlmRequest> {
        let contents = match self.include_contents {
            IncludeContents::Default => ctx.with_session(|s| s.contents()),
            IncludeContents::None => ctx.with_session(|s| {
                s.events
                    .iter()
                    .filter(|e| e.invocation_id == ctx.invocation_id && !e.is_partial())
                    .filter_map(|e| e.content.clone())
                    .filter(|c| !c.parts.is_empty())
                    .collect()
            }),
        };

        if let Some(schema) = &self.input_schema {
            if let Some(last) = contents.iter().rev().find(|c| c.role == Role::User) {
                let text = last.text();
                let parsed: serde_json::Value = serde_json::from_str(&text).map_err(|e| {
                    AdkError::validation(
                        format!("{}.input", self.name),
                        format!("input must be JSON matching the declared schema: {e}"),
                    )
                })?;
                schema.validate(&parsed, "input")?;
            }
        }

        let declarations = tools.iter().filter_map(|t| t.declaration()).collect();
        let instruction = ctx.with_state(|state| self.render_instruction(state));

        let mut request = LlmRequest::new(self.model.name())
            .with_contents(contents)
            .with_tools(declarations)
            .with_config(self.generate_content_config.clone());
        request.system_instruction = instruction;
        request.response_schema = self.output_schema.clone();
        Ok(request)
    }

    /// Executes every tool call in `response`, returning the response parts.
    async fn run_tools(
        &self,
        ctx: &InvocationContext,
        tools: &[SharedTool],
        response: &LlmResponse,
        call_event_id: &str,
    ) -> Result<(Vec<Part>, adk_core::EventActions, BTreeSet<String>)> {
        let callback_ctx = CallbackContext::new(ctx.clone(), &self.name);
        let mut parts = Vec::new();
        let mut actions = adk_core::EventActions::default();
        let mut long_running = BTreeSet::new();

        let calls: Vec<_> = response
            .content
            .as_ref()
            .map(|c| c.function_calls().into_iter().cloned().collect())
            .unwrap_or_default();

        for call in calls {
            let call_id = call.id.clone().unwrap_or_else(|| adk_core::new_id("call"));
            let Some(tool) = tools.iter().find(|t| t.name() == call.name) else {
                // Report an unknown tool back to the model rather than failing
                // the run: the model can correct itself on the next turn.
                parts.push(Part::FunctionResponse(FunctionResponse {
                    id: Some(call_id),
                    name: call.name.clone(),
                    response: adk_tools::error(format!("unknown tool '{}'", call.name)),
                }));
                continue;
            };

            let tool_ctx = ToolContext::new(ctx.clone())
                .with_function_call_id(&call_id)
                .with_function_call_event_id(call_event_id);

            let result = match &self.callbacks.before_tool {
                Some(cb) => match cb(&callback_ctx, &call.name, &call.args).await {
                    Some(replacement) => Ok(replacement),
                    None => invoke_tool(tool.as_ref(), call.args.clone(), &tool_ctx).await,
                },
                None => invoke_tool(tool.as_ref(), call.args.clone(), &tool_ctx).await,
            };

            let mut value = match result {
                Ok(value) => value,
                Err(AdkError::ConfirmationRequired { .. }) => {
                    // The tool needs approval. Carry the request out on this
                    // event and tell the model the call is pending.
                    let tool_actions = tool_ctx.actions();
                    actions
                        .requested_tool_confirmations
                        .extend(tool_actions.requested_tool_confirmations);
                    actions
                        .requested_auth_configs
                        .extend(tool_actions.requested_auth_configs);
                    adk_tools::pending(serde_json::json!({
                        "message": "awaiting user confirmation",
                    }))
                }
                Err(err) => adk_tools::error(err.to_string()),
            };

            if let Some(cb) = &self.callbacks.after_tool {
                if let Some(replacement) = cb(&callback_ctx, &call.name, &call.args, &value).await {
                    value = replacement;
                }
            }

            // Fold the tool's own action requests into the outgoing event.
            let tool_actions = tool_ctx.actions();
            if tool_actions.skip_summarization {
                actions.skip_summarization = true;
            }
            if tool_actions.escalate {
                actions.escalate = true;
            }
            if let Some(target) = tool_actions.transfer_to_agent {
                actions.transfer_to_agent = Some(target);
            }
            actions.artifact_delta.extend(tool_actions.artifact_delta);

            if tool.is_long_running() {
                long_running.insert(call_id.clone());
            }

            parts.push(Part::FunctionResponse(FunctionResponse {
                id: Some(call_id),
                name: call.name.clone(),
                response: value,
            }));
        }

        Ok((parts, actions, long_running))
    }
}

/// Replaces `{key}` placeholders with state values.
fn substitute(template: &str, state: &State) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;

    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        match after.find('}') {
            Some(close) => {
                let key = &after[..close];
                match state.get(key) {
                    Some(value) => match value {
                        serde_json::Value::String(s) => out.push_str(s),
                        other => out.push_str(&other.to_string()),
                    },
                    // Leave an unresolved placeholder visible.
                    None => {
                        out.push('{');
                        out.push_str(key);
                        out.push('}');
                    }
                }
                rest = &after[close + 1..];
            }
            None => {
                out.push_str(&rest[open..]);
                return out;
            }
        }
    }
    out.push_str(rest);
    out
}

#[async_trait]
impl Agent for LlmAgent {
    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn sub_agents(&self) -> &[SharedAgent] {
        &self.sub_agents
    }

    fn run<'a>(&'a self, ctx: &'a InvocationContext) -> BoxStream<'a, Result<Event>> {
        Box::pin(async_stream::try_stream! {
            let ctx = ctx.for_agent(&self.name);
            let callback_ctx = CallbackContext::new(ctx.clone(), &self.name);

            // A before-agent callback that returns content short-circuits the
            // whole agent, which is what makes it usable as a guardrail.
            if let Some(cb) = &self.callbacks.before_agent {
                if let Some(content) = cb(&callback_ctx).await {
                    let mut event = Event::new(&ctx.invocation_id, &self.name).with_content(content);
                    event.actions.state_delta = ctx.take_state_delta();
                    event.turn_complete = Some(true);
                    yield Self::stamp(&ctx, event);
                    return;
                }
            }

            let tools = resolve_tools(&self.tools, &ctx).await?;

            if self.output_schema.is_some() && !tools.is_empty() {
                Err(AdkError::Config(format!(
                    "agent '{}' sets an output schema and also has tools; \
                     most providers reject that combination",
                    self.name
                )))?;
            }

            let mut iteration = 0;
            let mut final_text = String::new();

            loop {
                ctx.check_cancelled()?;
                if iteration >= self.max_tool_iterations {
                    Err(AdkError::LimitExceeded(format!(
                        "agent '{}' exceeded max_tool_iterations ({})",
                        self.name, self.max_tool_iterations
                    )))?;
                }
                iteration += 1;

                let mut request = self.build_request(&ctx, &tools).await?;

                let mut response = match &self.callbacks.before_model {
                    Some(cb) => cb(&callback_ctx, &mut request).await,
                    None => None,
                };

                if response.is_none() {
                    ctx.track_llm_call()?;
                    if ctx.run_config.streaming_mode == StreamingMode::Sse
                        && self.model.supports_streaming()
                    {
                        // Forward chunks as partial events, then aggregate.
                        let mut text = String::new();
                        let mut stream = self.model.generate_content_stream(request.clone());
                        let mut aggregated = LlmResponse {
                            turn_complete: true,
                            ..Default::default()
                        };
                        let mut other_parts: Vec<Part> = Vec::new();
                        while let Some(chunk) = stream.next().await {
                            let chunk = chunk?;
                            if let Some(content) = &chunk.content {
                                for part in &content.parts {
                                    match part {
                                        Part::Text(t) => text.push_str(t),
                                        other => other_parts.push(other.clone()),
                                    }
                                }
                            }
                            if chunk.finish_reason.is_some() {
                                aggregated.finish_reason = chunk.finish_reason.clone();
                            }
                            if chunk.usage.is_some() {
                                aggregated.usage = chunk.usage.clone();
                            }
                            if chunk.partial {
                                yield Self::stamp(&ctx, Event::new(&ctx.invocation_id, &self.name)
                                    .with_content(
                                        chunk.content.clone().unwrap_or_else(|| {
                                            Content::model_text(String::new())
                                        }),
                                    )
                                    .as_partial());
                            }
                        }
                        let mut parts = Vec::new();
                        if !text.is_empty() {
                            parts.push(Part::Text(text));
                        }
                        parts.extend(other_parts);
                        if !parts.is_empty() {
                            aggregated.content = Some(Content::new(Role::Model, parts));
                        }
                        response = Some(aggregated);
                    } else {
                        response = Some(self.model.generate_content(request).await?);
                    }
                }

                let mut response = response.expect("a response was produced above");

                if let Some(cb) = &self.callbacks.after_model {
                    if let Some(replacement) = cb(&callback_ctx, &response).await {
                        response = replacement;
                    }
                }

                if response.is_error() {
                    let mut event = Event::new(&ctx.invocation_id, &self.name).with_error(
                        response.error_code.clone().unwrap_or_else(|| "MODEL_ERROR".into()),
                        response.error_message.clone().unwrap_or_default(),
                    );
                    event.turn_complete = Some(true);
                    yield Self::stamp(&ctx, event);
                    return;
                }

                let has_calls = response.has_function_calls();
                let content = response
                    .content
                    .clone()
                    .unwrap_or_else(|| Content::new(Role::Model, Vec::new()));

                if let Some(schema) = &self.output_schema {
                    let text = content.text();
                    let parsed: serde_json::Value =
                        serde_json::from_str(text.trim()).map_err(|e| {
                            AdkError::validation(
                                format!("{}.output", self.name),
                                format!("response is not valid JSON: {e}"),
                            )
                        })?;
                    schema.validate(&parsed, "output")?;
                }

                // The model's turn: text and/or the calls it wants made.
                let mut model_event = Event::new(&ctx.invocation_id, &self.name)
                    .with_content(content.clone());
                model_event.actions.state_delta = ctx.take_state_delta();

                if !has_calls {
                    final_text = content.text();
                    if let Some(key) = &self.output_key {
                        if !final_text.is_empty() {
                            model_event
                                .actions
                                .state_delta
                                .insert(key.clone(), serde_json::Value::String(final_text.clone()));
                        }
                    }
                    model_event.turn_complete = Some(true);
                    yield Self::stamp(&ctx, model_event);
                    break;
                }

                let call_event_id = model_event.id.clone();
                let (parts, tool_actions, long_running) = self
                    .run_tools(&ctx, &tools, &response, &call_event_id)
                    .await?;
                model_event.long_running_tool_ids = long_running.clone();
                yield Self::stamp(&ctx, model_event);

                let mut tool_event = Event::new(&ctx.invocation_id, &self.name)
                    .with_content(Content::new(Role::User, parts));
                tool_event.actions = tool_actions;
                tool_event.actions.state_delta = ctx.take_state_delta();
                tool_event.long_running_tool_ids = long_running;

                let stop_here = tool_event.actions.skip_summarization
                    || tool_event.actions.escalate
                    || tool_event.actions.transfer_to_agent.is_some()
                    || !tool_event.long_running_tool_ids.is_empty()
                    || !tool_event.actions.requested_tool_confirmations.is_empty();

                // Record the exchange so the next model request sees it.
                ctx.with_session_mut(|session| {
                    session.events.push(
                        Event::new(&ctx.invocation_id, &self.name).with_content(content.clone()),
                    );
                    if let Some(c) = &tool_event.content {
                        session.events.push(
                            Event::new(&ctx.invocation_id, &self.name).with_content(c.clone()),
                        );
                    }
                });

                yield Self::stamp(&ctx, tool_event);

                if stop_here || ctx.should_end_invocation() {
                    break;
                }
            }

            if let Some(cb) = &self.callbacks.after_agent {
                if let Some(content) = cb(&callback_ctx).await {
                    let mut event =
                        Event::new(&ctx.invocation_id, &self.name).with_content(content);
                    event.actions.state_delta = ctx.take_state_delta();
                    event.turn_complete = Some(true);
                    yield Self::stamp(&ctx, event);
                }
            }

            let _ = final_text;
        })
    }
}

/// Builder for [`LlmAgent`].
pub struct LlmAgentBuilder {
    name: String,
    description: String,
    model: Option<SharedModel>,
    instruction: String,
    global_instruction: Option<String>,
    tools: Vec<ToolSource>,
    sub_agents: Vec<SharedAgent>,
    output_key: Option<String>,
    input_schema: Option<Schema>,
    output_schema: Option<Schema>,
    generate_content_config: GenerateContentConfig,
    include_contents: IncludeContents,
    callbacks: Callbacks,
    max_tool_iterations: u32,
}

impl LlmAgentBuilder {
    /// Starts a builder for an agent named `name`.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: String::new(),
            model: None,
            instruction: String::new(),
            global_instruction: None,
            tools: Vec::new(),
            sub_agents: Vec::new(),
            output_key: None,
            input_schema: None,
            output_schema: None,
            generate_content_config: GenerateContentConfig::default(),
            include_contents: IncludeContents::Default,
            callbacks: Callbacks::default(),
            max_tool_iterations: 10,
        }
    }

    /// Sets the model.
    pub fn model(mut self, model: SharedModel) -> Self {
        self.model = Some(model);
        self
    }

    /// Sets the agent's task, personality, and constraints.
    ///
    /// `{key}` placeholders are substituted from session state at request time.
    pub fn instruction(mut self, instruction: impl Into<String>) -> Self {
        self.instruction = instruction.into();
        self
    }

    /// Sets an instruction prepended for this agent and its sub-agents.
    pub fn global_instruction(mut self, instruction: impl Into<String>) -> Self {
        self.global_instruction = Some(instruction.into());
        self
    }

    /// Sets the capability summary other agents read when routing.
    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = description.into();
        self
    }

    /// Adds a tool or toolset.
    pub fn tool(mut self, tool: impl Into<ToolSource>) -> Self {
        self.tools.push(tool.into());
        self
    }

    /// Adds several tools.
    pub fn tools<I, T>(mut self, tools: I) -> Self
    where
        I: IntoIterator<Item = T>,
        T: Into<ToolSource>,
    {
        self.tools.extend(tools.into_iter().map(Into::into));
        self
    }

    /// Adds a sub-agent this agent may delegate to.
    pub fn sub_agent(mut self, agent: SharedAgent) -> Self {
        self.sub_agents.push(agent);
        self
    }

    /// Stores the agent's final text response under this state key.
    pub fn output_key(mut self, key: impl Into<String>) -> Self {
        self.output_key = Some(key.into());
        self
    }

    /// Requires the user message to be JSON matching this schema.
    pub fn input_schema(mut self, schema: Schema) -> Self {
        self.input_schema = Some(schema);
        self
    }

    /// Requires the response to be JSON matching this schema.
    ///
    /// Incompatible with tools; [`LlmAgentBuilder::build`] rejects the pair.
    pub fn output_schema(mut self, schema: Schema) -> Self {
        self.output_schema = Some(schema);
        self
    }

    /// Sets sampling and safety configuration.
    pub fn generate_content_config(mut self, config: GenerateContentConfig) -> Self {
        self.generate_content_config = config;
        self
    }

    /// Controls whether conversation history is sent.
    pub fn include_contents(mut self, include: IncludeContents) -> Self {
        self.include_contents = include;
        self
    }

    /// Attaches callbacks.
    pub fn callbacks(mut self, callbacks: Callbacks) -> Self {
        self.callbacks = callbacks;
        self
    }

    /// Caps how many model/tool round trips one turn may take.
    pub fn max_tool_iterations(mut self, max: u32) -> Self {
        self.max_tool_iterations = max;
        self
    }

    /// Finishes the agent.
    ///
    /// Fails when no model is set, or when an output schema is combined with
    /// tools — a pairing most providers reject, caught here rather than at the
    /// first request.
    pub fn build(self) -> Result<LlmAgent> {
        let model = self
            .model
            .ok_or_else(|| AdkError::Config(format!("agent '{}' has no model", self.name)))?;

        if self.output_schema.is_some() && !self.tools.is_empty() {
            return Err(AdkError::Config(format!(
                "agent '{}' sets an output schema and also has tools; \
                 most providers reject that combination",
                self.name
            )));
        }

        Ok(LlmAgent {
            name: self.name,
            description: self.description,
            model,
            instruction: self.instruction,
            global_instruction: self.global_instruction,
            tools: self.tools,
            sub_agents: self.sub_agents,
            output_key: self.output_key,
            input_schema: self.input_schema,
            output_schema: self.output_schema,
            generate_content_config: self.generate_content_config,
            include_contents: self.include_contents,
            callbacks: self.callbacks,
            max_tool_iterations: self.max_tool_iterations,
        })
    }
}

impl std::fmt::Debug for LlmAgent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LlmAgent")
            .field("name", &self.name)
            .field("model", &self.model.name())
            .field("tools", &self.tools.len())
            .field("sub_agents", &self.sub_agents.len())
            .field("output_key", &self.output_key)
            .finish_non_exhaustive()
    }
}

/// Convenience alias for tool arguments.
pub type ToolArgs = Args;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn substitution_fills_known_placeholders() {
        let mut state = State::new();
        state.set("topic", "otters");
        assert_eq!(
            substitute("Write about: {topic}.", &state),
            "Write about: otters."
        );
    }

    #[test]
    fn substitution_renders_non_string_values() {
        let mut state = State::new();
        state.set("count", 3);
        assert_eq!(substitute("{count} items", &state), "3 items");
    }

    #[test]
    fn an_unknown_placeholder_is_left_visible() {
        // Leaving the braces makes the mistake obvious in the prompt, rather
        // than handing the model a sentence with a silent gap.
        let state = State::new();
        assert_eq!(substitute("Hello {missing}!", &state), "Hello {missing}!");
    }

    #[test]
    fn an_unclosed_brace_is_passed_through() {
        let state = State::new();
        assert_eq!(substitute("50% {of the", &state), "50% {of the");
    }

    #[test]
    fn substitution_handles_several_placeholders() {
        let mut state = State::new();
        state.set("a", "x");
        state.set("b", "y");
        assert_eq!(substitute("{a}-{b}-{a}", &state), "x-y-x");
    }
}
