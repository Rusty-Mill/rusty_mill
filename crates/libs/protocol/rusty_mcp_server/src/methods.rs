//! The feature methods of a [`Connection`]: tools, prompts, resources and
//! completion. Each is a thin shell over the registry in [`Server`]; the
//! generic work (revision, pagination, cache hints, argument checks, error
//! codes) lives here so every feature behaves the same.

use crate::ask::{Ask, ToolOutcome};
use crate::changes::{ChangeEvent, ChangeKinds};
use crate::connection::{CallContext, CancelToken, Connection};
use crate::page::{self, Kind};
use crate::server::{ToolKind, MAX_COMPLETION_VALUES};
#[cfg(feature = "request-state")]
use crate::state::StateError;
use crate::tasks::{CreateError, Entry, TaskContext};
use rusty_json::Value;
use rusty_mcp_proto::rpc::params as decode_params;
use rusty_mcp_proto::subscribe::{
    self, AcknowledgedParams, ListenParams, ListenResult, ResourceUpdatedParams,
};
use rusty_mcp_proto::task::{self, GetTaskResult, TaskAckResult, TaskIdParams, UpdateTaskParams};
use rusty_mcp_proto::{
    CacheScope, CallToolParams, CompleteParams, CompleteResult, CreateTaskResult, ErrorCode,
    ErrorData, GetPromptParams, InputRequiredResult, ListPromptsResult,
    ListResourceTemplatesResult, ListResourcesResult, ListToolsResult, Message, PaginatedParams,
    Paging, ProtocolVersion, ReadResourceParams, Reference, RequestId, ResultType,
    SubscriptionFilter, Wire,
};
use std::ops::Range;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::Ordering;
use std::sync::mpsc::RecvTimeoutError;
use std::sync::Arc;
use std::time::Duration;

/// How often an open listener looks up from waiting for events to check
/// whether it was cancelled or the connection is closing.
const LISTEN_POLL: Duration = Duration::from_millis(50);

const SUBSCRIPTION_ID: &str = "io.modelcontextprotocol/subscriptionId";

fn invalid_params(why: impl std::fmt::Display) -> ErrorData {
    ErrorData::new(ErrorCode::INVALID_PARAMS, why.to_string())
}

fn not_offered(what: &str) -> ErrorData {
    ErrorData::new(
        ErrorCode::METHOD_NOT_FOUND,
        format!("this server has no {what}"),
    )
}

/// Fill in `resultType` for a revision that has one.
fn complete_type(version: &ProtocolVersion, slot: &mut Option<ResultType>) {
    if version.is_stateless() && slot.is_none() {
        *slot = Some(ResultType(ResultType::COMPLETE.to_owned()));
    }
}

impl Connection {
    /// The common front of every list method: the revision in force, the
    /// slice of the list the request asks for, and the paging members of the
    /// reply (next cursor, and the cache hints a stateless revision carries).
    fn list_window(
        &self,
        params: &Option<Value>,
        kind: Kind,
        len: usize,
    ) -> Result<(Range<usize>, Paging), ErrorData> {
        let (version, _) = self.version_for(params)?;
        let p: PaginatedParams = decode_params(params).map_err(invalid_params)?;
        let (start, end, next_cursor) =
            page::window(kind, p.cursor.as_deref(), len, self.server().page_size)
                .ok_or_else(|| invalid_params("invalid cursor"))?;
        let mut paging = Paging {
            next_cursor,
            ..Paging::default()
        };
        if version.is_stateless() {
            paging.ttl_ms = Some(0);
            paging.cache_scope = Some(CacheScope::Public);
        }
        Ok((start..end, paging))
    }

    pub(crate) fn tools_list(&self, params: &Option<Value>) -> Result<Value, ErrorData> {
        let tools = &self.server().tools;
        if tools.is_empty() {
            return Err(not_offered("tools"));
        }
        let (range, paging) = self.list_window(params, Kind::Tools, tools.len())?;
        Ok(ListToolsResult {
            tools: tools[range].iter().map(|t| t.tool.clone()).collect(),
            paging,
        }
        .to_value())
    }

    pub(crate) fn tools_call(
        &self,
        id: &RequestId,
        params: &Option<Value>,
        token: &CancelToken,
    ) -> Result<Value, ErrorData> {
        let (version, meta) = self.version_for(params)?;
        let call: CallToolParams = decode_params(params).map_err(invalid_params)?;
        let index = self
            .server()
            .tools
            .iter()
            .position(|t| t.tool.name == call.name)
            .ok_or_else(|| invalid_params(format!("unknown tool {:?}", call.name)))?;
        #[allow(unused_mut)]
        let mut ctx = self.call_context(id, version.clone(), meta, token);
        let name = call.name.clone();
        #[cfg(feature = "request-state")]
        {
            if let Some(token) = &call.request_state {
                let (round, state) = self
                    .server()
                    .state
                    .open(&state_scope(&name), token)
                    .map_err(|e| {
                        invalid_params(match e {
                            StateError::Expired => "requestState expired; start the call again",
                            StateError::Invalid => "invalid requestState",
                        })
                    })?;
                ctx.state = Some(state);
                ctx.round = round;
            }
        }
        let ToolKind::Inline(handler) = &self.server().tools[index].kind else {
            return self.start_task(&ctx, index, call);
        };
        match handler(&ctx, call)? {
            ToolOutcome::Done(mut result) => {
                complete_type(&version, &mut result.result_type);
                Ok(result.to_value())
            }
            ToolOutcome::Ask(ask) => self.input_required(&ctx, &name, ask),
        }
    }

    /// Answer a call to a task tool. A client that declared the extension gets
    /// a task: the handler starts on a thread of its own and the task is
    /// announced. Any other client gets the handler's result in the reply.
    fn start_task(
        &self,
        ctx: &CallContext,
        index: usize,
        call: CallToolParams,
    ) -> Result<Value, ErrorData> {
        let declared = ctx.protocol_version().is_stateless()
            && ctx
                .client_capabilities()
                .and_then(|c| c.extensions.as_ref())
                .is_some_and(|e| e.get(task::EXTENSION_ID).is_some());
        if !declared {
            // The client cannot poll, so answer it the ordinary way.
            let ToolKind::Task(handler) = &self.server().tools[index].kind else {
                return Err(ErrorData::new(ErrorCode::INTERNAL_ERROR, "not a task tool"));
            };
            let mut result = handler(&TaskContext::inline(ctx.cancel_token()), call)?;
            complete_type(ctx.protocol_version(), &mut result.result_type);
            return Ok(result.to_value());
        }
        let entry = self.server().tasks.create().map_err(|e| {
            ErrorData::new(
                ErrorCode::INTERNAL_ERROR,
                match e {
                    CreateError::Full => "too many tasks",
                    CreateError::NoRandom => "could not make a task id",
                },
            )
        })?;
        let announcement = entry.announcement();
        let server = self.server_arc();
        let worker = Arc::clone(&entry);
        let spawned = std::thread::Builder::new().spawn(move || {
            let outcome = catch_unwind(AssertUnwindSafe(|| {
                let ToolKind::Task(handler) = &server.tools[index].kind else {
                    return Err(ErrorData::new(ErrorCode::INTERNAL_ERROR, "not a task tool"));
                };
                handler(&TaskContext::new(Arc::clone(&worker)), call)
            }))
            .unwrap_or_else(|_| Err(ErrorData::new(ErrorCode::INTERNAL_ERROR, "internal error")));
            worker.finish(outcome);
        });
        if spawned.is_err() {
            entry.cancel();
            return Err(ErrorData::new(
                ErrorCode::INTERNAL_ERROR,
                "could not start the task",
            ));
        }
        Ok(CreateTaskResult {
            task: announcement,
            meta: None,
        }
        .to_value())
    }

    /// The task `id` names, for a client that may use tasks at all.
    fn task_named(&self, params: &Option<Value>, id: &str) -> Result<Arc<Entry>, ErrorData> {
        if !self.server().has_task_tools() {
            return Err(not_offered("tasks"));
        }
        let (version, _) = self.version_for(params)?;
        if !version.is_stateless() {
            return Err(ErrorData::new(
                ErrorCode::INVALID_REQUEST,
                "tasks need protocol revision 2026-07-28",
            ));
        }
        self.server()
            .tasks
            .get(id)
            .ok_or_else(|| invalid_params("unknown task"))
    }

    pub(crate) fn tasks_get(&self, params: &Option<Value>) -> Result<Value, ErrorData> {
        let p: TaskIdParams = decode_params(params).map_err(invalid_params)?;
        let entry = self.task_named(params, &p.task_id)?;
        Ok(GetTaskResult {
            task: entry.snapshot(),
            meta: None,
        }
        .to_value())
    }

    pub(crate) fn tasks_update(&self, params: &Option<Value>) -> Result<Value, ErrorData> {
        let p: UpdateTaskParams = decode_params(params).map_err(invalid_params)?;
        let entry = self.task_named(params, &p.task_id)?;
        entry.answer(&p.input_responses).map_err(invalid_params)?;
        Ok(TaskAckResult::default().to_value())
    }

    pub(crate) fn tasks_cancel(&self, params: &Option<Value>) -> Result<Value, ErrorData> {
        let p: TaskIdParams = decode_params(params).map_err(invalid_params)?;
        let entry = self.task_named(params, &p.task_id)?;
        if entry.status().is_terminal() {
            return Err(invalid_params("the task has already ended"));
        }
        entry.cancel();
        Ok(TaskAckResult::default().to_value())
    }

    /// The reply for a handler that wants input: checks the client can be
    /// asked, seals any state, and shapes the `input_required` result.
    fn input_required(&self, ctx: &CallContext, name: &str, ask: Ask) -> Result<Value, ErrorData> {
        if !ctx.protocol_version().is_stateless() {
            return Err(ErrorData::new(
                ErrorCode::INVALID_REQUEST,
                "asking for input mid-call needs protocol revision 2026-07-28",
            ));
        }
        if ask.requests.is_empty() && ask.state.is_none() {
            return Err(ErrorData::new(
                ErrorCode::INTERNAL_ERROR,
                "the tool asked for input but gave no questions and no state",
            ));
        }
        let can_elicit = ctx
            .client_capabilities()
            .is_some_and(|c| c.elicitation.is_some());
        if !ask.requests.is_empty() && !can_elicit {
            return Err(ErrorData::new(
                ErrorCode::MISSING_REQUIRED_CLIENT_CAPABILITY,
                "this tool asks the user questions; the client must declare the elicitation capability",
            ));
        }
        #[allow(unused_mut)]
        let mut result = InputRequiredResult {
            input_requests: (!ask.requests.is_empty()).then_some(ask.requests),
            request_state: None,
            meta: None,
        };
        #[cfg(feature = "request-state")]
        {
            let round = ctx.round.saturating_add(1);
            if ask.state.is_some() && round > self.server().max_input_rounds {
                return Err(invalid_params(format!(
                    "this request exceeded the {} round limit",
                    self.server().max_input_rounds
                )));
            }
            result.request_state = ask
                .state
                .map(|s| self.server().state.seal(&state_scope(name), round, &s));
        }
        #[cfg(not(feature = "request-state"))]
        let _ = name;
        Ok(result.to_value())
    }

    pub(crate) fn prompts_list(&self, params: &Option<Value>) -> Result<Value, ErrorData> {
        let prompts = &self.server().prompts;
        if prompts.is_empty() {
            return Err(not_offered("prompts"));
        }
        let (range, paging) = self.list_window(params, Kind::Prompts, prompts.len())?;
        Ok(ListPromptsResult {
            prompts: prompts[range].iter().map(|p| p.prompt.clone()).collect(),
            paging,
        }
        .to_value())
    }

    pub(crate) fn prompts_get(
        &self,
        id: &RequestId,
        params: &Option<Value>,
        token: &CancelToken,
    ) -> Result<Value, ErrorData> {
        if self.server().prompts.is_empty() {
            return Err(not_offered("prompts"));
        }
        let (version, meta) = self.version_for(params)?;
        let get: GetPromptParams = decode_params(params).map_err(invalid_params)?;
        let entry = self
            .server()
            .prompts
            .iter()
            .find(|p| p.prompt.name == get.name)
            .ok_or_else(|| invalid_params(format!("unknown prompt {:?}", get.name)))?;
        // Arguments the prompt declares required must be given.
        for arg in entry.prompt.arguments.iter().flatten() {
            let given = get
                .arguments
                .as_ref()
                .is_some_and(|a| a.get(&arg.name).is_some_and(|v| !v.is_null()));
            if arg.required == Some(true) && !given {
                return Err(invalid_params(format!(
                    "missing required argument {:?}",
                    arg.name
                )));
            }
        }
        let ctx = self.call_context(id, version.clone(), meta, token);
        let mut result = (entry.handler)(&ctx, get)?;
        complete_type(&version, &mut result.result_type);
        Ok(result.to_value())
    }

    pub(crate) fn resources_list(&self, params: &Option<Value>) -> Result<Value, ErrorData> {
        let s = self.server();
        if s.resources.is_empty() && s.templates.is_empty() {
            return Err(not_offered("resources"));
        }
        let (range, paging) = self.list_window(params, Kind::Resources, s.resources.len())?;
        Ok(ListResourcesResult {
            resources: s.resources[range]
                .iter()
                .map(|r| r.resource.clone())
                .collect(),
            paging,
        }
        .to_value())
    }

    pub(crate) fn templates_list(&self, params: &Option<Value>) -> Result<Value, ErrorData> {
        let s = self.server();
        if s.resources.is_empty() && s.templates.is_empty() {
            return Err(not_offered("resources"));
        }
        let (range, paging) = self.list_window(params, Kind::Templates, s.templates.len())?;
        Ok(ListResourceTemplatesResult {
            resource_templates: s.templates[range]
                .iter()
                .map(|t| t.template.clone())
                .collect(),
            paging,
        }
        .to_value())
    }

    pub(crate) fn resources_read(
        &self,
        id: &RequestId,
        params: &Option<Value>,
        token: &CancelToken,
    ) -> Result<Value, ErrorData> {
        let s = self.server();
        if s.resources.is_empty() && s.templates.is_empty() {
            return Err(not_offered("resources"));
        }
        let (version, meta) = self.version_for(params)?;
        let read: ReadResourceParams = decode_params(params).map_err(invalid_params)?;
        let ctx = self.call_context(id, version.clone(), meta, token);
        let mut result = if let Some(r) = s.resources.iter().find(|r| r.resource.uri == read.uri) {
            (r.handler)(&ctx, read)?
        } else if let Some((t, vars)) = s
            .templates
            .iter()
            .find_map(|t| t.matcher.matches(&read.uri).map(|vars| (t, vars)))
        {
            (t.handler)(&ctx, &vars, read)?
        } else {
            let mut data = Value::object();
            data.insert("uri", read.uri.as_str());
            return Err(ErrorData {
                code: ErrorCode::RESOURCE_NOT_FOUND,
                message: format!("resource not found: {}", read.uri),
                data: Some(data),
            });
        };
        complete_type(&version, &mut result.result_type);
        if version.is_stateless() {
            // Not cacheable unless the handler says so, and then only by the
            // requesting user's client: a read may depend on who asks.
            result.ttl_ms.get_or_insert(0);
            result.cache_scope.get_or_insert(CacheScope::Private);
        }
        Ok(result.to_value())
    }

    pub(crate) fn complete(
        &self,
        id: &RequestId,
        params: &Option<Value>,
        token: &CancelToken,
    ) -> Result<Value, ErrorData> {
        let s = self.server();
        let Some(completer) = &s.completer else {
            return Err(not_offered("completions"));
        };
        let (version, meta) = self.version_for(params)?;
        let req: CompleteParams = decode_params(params).map_err(invalid_params)?;
        // The reference must name something this server offers.
        match &req.reference {
            Reference::Prompt { name, .. } => {
                let prompt = s
                    .prompts
                    .iter()
                    .find(|p| &p.prompt.name == name)
                    .ok_or_else(|| invalid_params(format!("unknown prompt {name:?}")))?;
                let declared = prompt.prompt.arguments.iter().flatten();
                if !declared.clone().any(|a| a.name == req.argument.name) {
                    return Err(invalid_params(format!(
                        "prompt {name:?} has no argument {:?}",
                        req.argument.name
                    )));
                }
            }
            Reference::Resource { uri } => {
                if !s.templates.iter().any(|t| &t.template.uri_template == uri) {
                    return Err(invalid_params(format!("unknown resource template {uri:?}")));
                }
            }
        }
        let ctx = self.call_context(id, version.clone(), meta, token);
        let mut completion = completer(&ctx, req)?;
        if completion.values.len() > MAX_COMPLETION_VALUES {
            completion
                .total
                .get_or_insert(completion.values.len() as u64);
            completion.values.truncate(MAX_COMPLETION_VALUES);
            completion.has_more = Some(true);
        }
        let mut result = CompleteResult {
            result_type: None,
            completion,
            meta: None,
        };
        complete_type(&version, &mut result.result_type);
        Ok(result.to_value())
    }
}

/// The part of `requested` this server will send: only announced
/// categories, and only URIs `resources/read` can serve.
fn accept_filter(
    requested: &SubscriptionFilter,
    kinds: &ChangeKinds,
    readable: impl Fn(&str) -> bool,
) -> SubscriptionFilter {
    let wants =
        |asked: Option<bool>, announced: bool| (asked == Some(true) && announced).then_some(true);
    let mut uris: Vec<String> = Vec::new();
    if kinds.resource_updates {
        for uri in requested.resource_subscriptions.iter().flatten() {
            if readable(uri) && !uris.contains(uri) {
                uris.push(uri.clone());
            }
        }
    }
    SubscriptionFilter {
        tools_list_changed: wants(requested.tools_list_changed, kinds.tools_list),
        prompts_list_changed: wants(requested.prompts_list_changed, kinds.prompts_list),
        resources_list_changed: wants(requested.resources_list_changed, kinds.resources_list),
        resource_subscriptions: (!uris.is_empty()).then_some(uris),
    }
}

/// The notification for `event`, if `accepted` includes it, tagged with the
/// subscription it belongs to.
fn notification_for(
    event: &ChangeEvent,
    accepted: &SubscriptionFilter,
    meta: &Value,
) -> Option<Message> {
    let list_changed = |on: Option<bool>, method: &str| {
        (on == Some(true)).then(|| {
            let mut params = Value::object();
            params.insert("_meta", meta.clone());
            Message::Notification {
                method: method.to_owned(),
                params: Some(params),
            }
        })
    };
    match event {
        ChangeEvent::ToolsListChanged => list_changed(
            accepted.tools_list_changed,
            subscribe::method::TOOLS_LIST_CHANGED,
        ),
        ChangeEvent::PromptsListChanged => list_changed(
            accepted.prompts_list_changed,
            subscribe::method::PROMPTS_LIST_CHANGED,
        ),
        ChangeEvent::ResourcesListChanged => list_changed(
            accepted.resources_list_changed,
            subscribe::method::RESOURCES_LIST_CHANGED,
        ),
        ChangeEvent::ResourceUpdated { uri } => accepted
            .resource_subscriptions
            .as_ref()
            .is_some_and(|uris| uris.contains(uri))
            .then(|| {
                Message::notification(
                    subscribe::method::RESOURCE_UPDATED,
                    &ResourceUpdatedParams {
                        uri: uri.clone(),
                        meta: Some(meta.clone()),
                    },
                )
            }),
    }
}

impl Connection {
    /// `subscriptions/listen`: a request that stays open, acknowledging what
    /// it was granted and then forwarding matching changes until the client
    /// cancels or hangs up, the connection closes, or the broadcaster is
    /// closed (which ends it with the final result).
    pub(crate) fn listen(
        &self,
        id: &RequestId,
        params: &Option<Value>,
        token: &CancelToken,
    ) -> Result<Value, ErrorData> {
        let s = self.server();
        let Some((changes, kinds)) = &s.changes else {
            return Err(not_offered("subscriptions"));
        };
        let (version, _) = self.version_for(params)?;
        if !version.is_stateless() {
            return Err(ErrorData::new(
                ErrorCode::METHOD_NOT_FOUND,
                "subscriptions/listen needs protocol version 2026-07-28",
            ));
        }
        let req: ListenParams = decode_params(params).map_err(invalid_params)?;
        let accepted = accept_filter(&req.notifications, kinds, |uri| s.can_read(uri));
        // Subscribe before acknowledging, so nothing published in between is lost.
        let subscription = changes.subscribe();
        let mut meta = Value::object();
        meta.insert(SUBSCRIPTION_ID, id.to_value());
        self.notify(Message::notification(
            subscribe::method::ACKNOWLEDGED,
            &AcknowledgedParams {
                meta: Some(meta.clone()),
                notifications: accepted.clone(),
            },
        ));
        loop {
            if token.is_cancelled() || self.is_closing() {
                break;
            }
            match subscription.events.recv_timeout(LISTEN_POLL) {
                Ok(event) => {
                    if let Some(message) = notification_for(&event, &accepted, &meta) {
                        self.notify(message);
                    }
                }
                Err(RecvTimeoutError::Timeout) => {
                    // Events were dropped while this listener was behind: say
                    // everything it follows may have changed.
                    if subscription.lagged.swap(false, Ordering::SeqCst) {
                        for event in resync_events(&accepted) {
                            if let Some(message) = notification_for(&event, &accepted, &meta) {
                                self.notify(message);
                            }
                        }
                    }
                }
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
        Ok(ListenResult::complete(id)
            .with_server_info(&s.info)
            .to_value())
    }
}

/// Everything a listener with `accepted` follows, as events.
fn resync_events(accepted: &SubscriptionFilter) -> Vec<ChangeEvent> {
    let mut events = vec![
        ChangeEvent::ToolsListChanged,
        ChangeEvent::PromptsListChanged,
        ChangeEvent::ResourcesListChanged,
    ];
    events.extend(
        accepted
            .resource_subscriptions
            .iter()
            .flatten()
            .map(|uri| ChangeEvent::ResourceUpdated { uri: uri.clone() }),
    );
    events
}

/// What a sealed `requestState` is bound to: the method and the tool.
#[cfg(feature = "request-state")]
fn state_scope(tool: &str) -> String {
    format!("tools/call\0{tool}")
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn filter(tools: bool, uris: &[&str]) -> SubscriptionFilter {
        SubscriptionFilter {
            tools_list_changed: tools.then_some(true),
            prompts_list_changed: None,
            resources_list_changed: None,
            resource_subscriptions: (!uris.is_empty())
                .then(|| uris.iter().map(|u| (*u).to_owned()).collect()),
        }
    }

    #[test]
    fn only_announced_categories_and_readable_uris_are_accepted() {
        let requested = SubscriptionFilter {
            tools_list_changed: Some(true),
            prompts_list_changed: Some(true),
            resources_list_changed: Some(true),
            resource_subscriptions: Some(vec!["a://1".into(), "b://2".into(), "a://1".into()]),
        };
        let kinds = ChangeKinds {
            tools_list: true,
            resource_updates: true,
            ..ChangeKinds::default()
        };
        let accepted = accept_filter(&requested, &kinds, |u| u.starts_with("a://"));
        assert_eq!(
            accepted,
            filter(true, &["a://1"]),
            "prompts and list changes were not announced"
        );

        // Without announcing updates, subscriptions are dropped entirely.
        let none = accept_filter(&requested, &ChangeKinds::default(), |_| true);
        assert_eq!(none, SubscriptionFilter::default());
        // `false` in a request is not a request.
        let off = SubscriptionFilter {
            tools_list_changed: Some(false),
            ..SubscriptionFilter::default()
        };
        assert_eq!(
            accept_filter(&off, &ChangeKinds::all(), |_| true),
            SubscriptionFilter::default()
        );
    }

    #[test]
    fn notifications_follow_the_accepted_filter_and_carry_the_subscription() {
        let accepted = filter(true, &["a://1"]);
        let mut meta = Value::object();
        meta.insert(SUBSCRIPTION_ID, 9);
        let tools = notification_for(&ChangeEvent::ToolsListChanged, &accepted, &meta).unwrap();
        let Message::Notification { method, params } = tools else {
            panic!("not a notification")
        };
        assert_eq!(method, subscribe::method::TOOLS_LIST_CHANGED);
        assert_eq!(params.unwrap().get("_meta"), Some(&meta));
        assert!(notification_for(&ChangeEvent::PromptsListChanged, &accepted, &meta).is_none());
        assert!(notification_for(&ChangeEvent::ResourcesListChanged, &accepted, &meta).is_none());
        let updated = ChangeEvent::ResourceUpdated {
            uri: "a://1".into(),
        };
        assert!(notification_for(&updated, &accepted, &meta).is_some());
        let other = ChangeEvent::ResourceUpdated {
            uri: "a://2".into(),
        };
        assert!(notification_for(&other, &accepted, &meta).is_none());
    }

    #[test]
    fn a_resync_covers_exactly_what_the_listener_follows() {
        let accepted = filter(true, &["a://1", "a://2"]);
        let mut meta = Value::object();
        meta.insert(SUBSCRIPTION_ID, 1);
        let sent: Vec<Message> = resync_events(&accepted)
            .iter()
            .filter_map(|e| notification_for(e, &accepted, &meta))
            .collect();
        // One tools signal and one update per followed resource; nothing else.
        assert_eq!(sent.len(), 3);
    }
}
