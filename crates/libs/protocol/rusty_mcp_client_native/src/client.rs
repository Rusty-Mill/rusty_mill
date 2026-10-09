//! A blocking MCP client over any [`Transport`]: the handshake, calls that
//! wait for their answer while handling whatever else arrives, and typed
//! helpers for the calls consumers make, including driving multi-round-trip
//! input and tasks to a final result.
//!
//! One call is in flight at a time (the client is `&mut`); concurrency is the
//! async facade's job.

use crate::error::ClientError;
use crate::session::{ClientConfig, ClientSession, Incoming};
use crate::transport::{Recv, Transport};
use rusty_json::Value;
use rusty_mcp_proto::task::{method as task_method, TaskPayload};
use rusty_mcp_proto::{
    CallToolResponse, CallToolResult, CompleteParams, CompleteResult, ElicitAction, ElicitParams,
    ElicitResult, ErrorCode, ErrorData, GetPromptParams, GetPromptResult, InputRequest,
    ListPromptsResult, ListResourceTemplatesResult, ListResourcesResult, ListToolsResult, Message,
    PaginatedParams, Prompt, ReadResourceParams, ReadResourceResult, Resource, ResourceTemplate,
    Tool, Wire,
};
use std::time::{Duration, Instant};

/// Most times one call may be sent back for input.
pub const MAX_ROUNDS: u32 = 8;

/// What the application does with what the server sends unprompted.
pub trait Handler {
    /// A notification (progress, list changes, resource updates, ...).
    fn notification(&mut self, _method: &str, _params: Option<&Value>) {}

    /// A request from the server. The default refuses everything.
    ///
    /// # Errors
    /// Whatever should be sent back as the JSON-RPC error.
    fn request(&mut self, method: &str, _params: Option<&Value>) -> Result<Value, ErrorData> {
        Err(ErrorData::new(
            ErrorCode::METHOD_NOT_FOUND,
            format!("this client does not handle {method:?}"),
        ))
    }

    /// The user's answer to an elicitation, whether it arrives as a
    /// multi-round-trip `inputRequests` entry or inside a task. The default
    /// declines.
    fn elicit(&mut self, _params: &ElicitParams) -> ElicitResult {
        ElicitResult {
            action: ElicitAction::Decline,
            content: None,
            meta: None,
        }
    }
}

/// A handler that ignores everything and declines every question.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoHandler;

impl Handler for NoHandler {}

/// A connected client.
pub struct Client<T: Transport, H: Handler = NoHandler> {
    session: ClientSession,
    transport: T,
    handler: H,
    timeout: Duration,
}

fn rpc(e: ErrorData) -> ClientError {
    ClientError::Rpc(e)
}

impl<T: Transport, H: Handler> Client<T, H> {
    /// Shake hands with the server: `server/discover` first when the config
    /// allows a stateless revision, falling back to `initialize` when the
    /// server turns out to be classic. `timeout` bounds each step of the
    /// handshake; afterwards calls wait up to `ClientConfig::call_timeout`.
    ///
    /// # Errors
    /// The transport failed, the server did not answer within `timeout`, or
    /// no revision is shared.
    pub fn connect(
        transport: T,
        config: ClientConfig,
        handler: H,
        timeout: Duration,
    ) -> Result<Self, ClientError> {
        let mut client = Self {
            session: ClientSession::new(config),
            transport,
            handler,
            timeout,
        };
        client.handshake()?;
        client.timeout = client.session.call_timeout();
        Ok(client)
    }

    fn handshake(&mut self) -> Result<(), ClientError> {
        if self.session.prefers_stateless() {
            let (id, message) = self.session.discover_request();
            match self.exchange(id, &message) {
                Ok(result) => {
                    if self.session.on_discover(&result)? {
                        self.announce_version();
                        return Ok(());
                    }
                }
                Err(ClientError::Rpc(e)) if self.session.falls_back_to_initialize(&e) => {}
                Err(e) => return Err(e),
            }
        }
        let (id, message) = self.session.initialize_request()?;
        let result = self.exchange(id, &message)?;
        self.session.on_initialize(&result)?;
        self.announce_version();
        let initialized = self.session.initialized_notification();
        self.transport.send(&initialized)?;
        Ok(())
    }

    fn announce_version(&mut self) {
        if let Some(version) = self.session.negotiated().cloned() {
            self.transport.set_protocol_version(&version);
        }
    }

    /// Wait up to `wait` for something from the server and handle it
    /// (notifications go to the handler, server requests are answered).
    /// Returns whether anything arrived. Use it to receive notifications
    /// between calls, for example resource updates on a classic HTTP session.
    ///
    /// # Errors
    /// Transport failure, or the server closed the connection.
    pub fn pump(&mut self, wait: Duration) -> Result<bool, ClientError> {
        match self.transport.recv(wait)? {
            Recv::Timeout => Ok(false),
            Recv::Closed => Err(ClientError::Closed),
            Recv::Message(m) => {
                match self.session.accept(m) {
                    Incoming::Notification { method, params } => {
                        self.handler.notification(&method, params.as_ref());
                    }
                    Incoming::Request { id, method, params } => {
                        let answer = match self.handler.request(&method, params.as_ref()) {
                            Ok(result) => Message::Response { id, result },
                            Err(error) => Message::error(Some(id), error),
                        };
                        self.transport.send(&answer)?;
                    }
                    Incoming::Response { .. } | Incoming::Stray(_) => {}
                }
                Ok(true)
            }
        }
    }

    /// The protocol state: negotiated revision, what the server said about
    /// itself.
    pub fn session(&self) -> &ClientSession {
        &self.session
    }

    /// The handler, to read what it collected.
    pub fn handler(&self) -> &H {
        &self.handler
    }

    /// Send `message` (a request already registered with the session) and
    /// wait for its answer.
    fn exchange(
        &mut self,
        id: rusty_mcp_proto::RequestId,
        message: &Message,
    ) -> Result<Value, ClientError> {
        self.transport.send(message)?;
        let deadline = Instant::now() + self.timeout;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                let cancel = self.session.cancel(&id, Some("timed out"));
                let _ = self.transport.send(&cancel);
                self.session.forget(&id);
                return Err(ClientError::Timeout);
            }
            match self.transport.recv(left)? {
                Recv::Timeout => continue,
                Recv::Closed => return Err(ClientError::Closed),
                Recv::Message(m) => match self.session.accept(m) {
                    Incoming::Response { id: got, result } if got == id => {
                        return result.map_err(rpc);
                    }
                    Incoming::Response { .. } | Incoming::Stray(_) => {}
                    Incoming::Notification { method, params } => {
                        self.handler.notification(&method, params.as_ref());
                    }
                    Incoming::Request { id, method, params } => {
                        let answer = match self.handler.request(&method, params.as_ref()) {
                            Ok(result) => Message::Response { id, result },
                            Err(error) => Message::error(Some(id), error),
                        };
                        self.transport.send(&answer)?;
                    }
                },
            }
        }
    }

    /// Any request, answered with its raw result.
    ///
    /// # Errors
    /// Transport failure, timeout, or the server's error.
    pub fn call(&mut self, method: &str, params: Option<Value>) -> Result<Value, ClientError> {
        let (id, message) = self.session.request(method, params);
        self.exchange(id, &message)
    }

    /// Any notification.
    ///
    /// # Errors
    /// Transport failure.
    pub fn notify(&mut self, method: &str, params: Option<Value>) -> Result<(), ClientError> {
        let message = self.session.notification(method, params);
        self.transport.send(&message)?;
        Ok(())
    }

    /// `ping`.
    ///
    /// # Errors
    /// As [`Client::call`].
    pub fn ping(&mut self) -> Result<(), ClientError> {
        self.call("ping", None).map(|_| ())
    }

    fn typed<R: Wire>(&mut self, method: &str, params: &impl Wire) -> Result<R, ClientError> {
        let result = self.call(method, Some(params.to_value()))?;
        Ok(R::from_value(&result)?)
    }

    fn pages<P, R>(
        &mut self,
        method: &str,
        mut next: impl FnMut(R) -> (Vec<P>, Option<String>),
    ) -> Result<Vec<P>, ClientError>
    where
        R: Wire,
    {
        let mut all = Vec::new();
        let mut cursor: Option<String> = None;
        loop {
            let params = PaginatedParams {
                cursor: cursor.take(),
                meta: None,
            };
            let page: R = self.typed(method, &params)?;
            let (items, following) = next(page);
            all.extend(items);
            match following {
                Some(c) => cursor = Some(c),
                None => return Ok(all),
            }
        }
    }

    /// Every tool, following the pages.
    ///
    /// # Errors
    /// As [`Client::call`].
    pub fn list_tools(&mut self) -> Result<Vec<Tool>, ClientError> {
        self.pages("tools/list", |r: ListToolsResult| {
            (r.tools, r.paging.next_cursor)
        })
    }

    /// Every prompt.
    ///
    /// # Errors
    /// As [`Client::call`].
    pub fn list_prompts(&mut self) -> Result<Vec<Prompt>, ClientError> {
        self.pages("prompts/list", |r: ListPromptsResult| {
            (r.prompts, r.paging.next_cursor)
        })
    }

    /// Every resource.
    ///
    /// # Errors
    /// As [`Client::call`].
    pub fn list_resources(&mut self) -> Result<Vec<Resource>, ClientError> {
        self.pages("resources/list", |r: ListResourcesResult| {
            (r.resources, r.paging.next_cursor)
        })
    }

    /// Every resource template.
    ///
    /// # Errors
    /// As [`Client::call`].
    pub fn list_resource_templates(&mut self) -> Result<Vec<ResourceTemplate>, ClientError> {
        self.pages(
            "resources/templates/list",
            |r: ListResourceTemplatesResult| (r.resource_templates, r.paging.next_cursor),
        )
    }

    /// `prompts/get`.
    ///
    /// # Errors
    /// As [`Client::call`].
    pub fn get_prompt(
        &mut self,
        name: &str,
        arguments: Option<Value>,
    ) -> Result<GetPromptResult, ClientError> {
        let mut params = GetPromptParams::new(name);
        params.arguments = arguments;
        self.typed("prompts/get", &params)
    }

    /// `resources/read`.
    ///
    /// # Errors
    /// As [`Client::call`].
    pub fn read_resource(&mut self, uri: &str) -> Result<ReadResourceResult, ClientError> {
        self.typed("resources/read", &ReadResourceParams::new(uri))
    }

    /// `completion/complete`.
    ///
    /// # Errors
    /// As [`Client::call`].
    pub fn complete(&mut self, params: &CompleteParams) -> Result<CompleteResult, ClientError> {
        self.typed("completion/complete", params)
    }

    /// One `tools/call`, answered as the server answered: a result, a request
    /// for input, or a task.
    ///
    /// # Errors
    /// As [`Client::call`].
    pub fn call_tool_once(
        &mut self,
        name: &str,
        arguments: Option<Value>,
    ) -> Result<CallToolResponse, ClientError> {
        let mut params = rusty_mcp_proto::tool::CallToolParams::new(name);
        params.arguments = arguments;
        self.typed("tools/call", &params)
    }

    /// `tools/call` driven to its end: questions are put to the handler and
    /// the call retried with the answers, a task is polled until it finishes.
    ///
    /// # Errors
    /// As [`Client::call`]; [`ClientError::TooManyRounds`] when the server
    /// keeps asking; [`ClientError::TaskEnded`] when a task fails or is
    /// cancelled.
    pub fn call_tool(
        &mut self,
        name: &str,
        arguments: Option<Value>,
    ) -> Result<CallToolResult, ClientError> {
        let mut responses: Option<Value> = None;
        let mut state: Option<String> = None;
        for _ in 0..MAX_ROUNDS {
            let mut params = rusty_mcp_proto::tool::CallToolParams::new(name);
            params.arguments = arguments.clone();
            params.input_responses = responses.take();
            params.request_state = state.take();
            match self.typed::<CallToolResponse>("tools/call", &params)? {
                CallToolResponse::Complete(result) => return Ok(result),
                CallToolResponse::Task(created) => {
                    return self.finish_task(&created.task.task_id, created.task.poll_interval_ms)
                }
                CallToolResponse::InputRequired(more) => {
                    state = more.request_state;
                    let requests = more.input_requests.unwrap_or_default();
                    responses =
                        Some(self.answer_all(requests.iter().map(|(k, v)| (k.as_str(), v))));
                }
            }
        }
        Err(ClientError::TooManyRounds(MAX_ROUNDS))
    }

    /// The handler's answer to each question, keyed as asked.
    fn answer_all<'a>(
        &mut self,
        requests: impl Iterator<Item = (&'a str, &'a InputRequest)>,
    ) -> Value {
        let mut out = Value::object();
        for (key, request) in requests {
            let answer = match request
                .params
                .as_ref()
                .filter(|_| request.method == "elicitation/create")
                .and_then(|p| ElicitParams::from_value(p).ok())
            {
                Some(params) => self.handler.elicit(&params),
                // Sampling and roots are not offered by this client.
                None => ElicitResult {
                    action: ElicitAction::Decline,
                    content: None,
                    meta: None,
                },
            };
            out.insert(key, answer.to_value());
        }
        out
    }

    /// Poll a task until it ends, answering its questions.
    fn finish_task(
        &mut self,
        task_id: &str,
        poll_ms: Option<u64>,
    ) -> Result<CallToolResult, ClientError> {
        let pause = Duration::from_millis(poll_ms.unwrap_or(100).clamp(10, 1000));
        let deadline = Instant::now() + self.timeout.max(Duration::from_secs(60));
        loop {
            let mut params = Value::object();
            params.insert("taskId", task_id);
            let got = self.call(task_method::GET, Some(params))?;
            let task = rusty_mcp_proto::task::GetTaskResult::from_value(&got)?.task;
            match task.payload() {
                TaskPayload::Completed { result } => {
                    return Ok(CallToolResult::from_value(result)?);
                }
                TaskPayload::Failed { error } => {
                    return Err(ClientError::TaskEnded(error.to_json_string()));
                }
                TaskPayload::Cancelled => {
                    return Err(ClientError::TaskEnded("cancelled".to_owned()));
                }
                TaskPayload::InputRequired { input_requests } => {
                    let answers =
                        self.answer_all(input_requests.iter().map(|(k, v)| (k.as_str(), v)));
                    let mut update = Value::object();
                    update.insert("taskId", task_id);
                    update.insert("inputResponses", answers);
                    self.call(task_method::UPDATE, Some(update))?;
                }
                TaskPayload::Working => {}
            }
            if Instant::now() > deadline {
                return Err(ClientError::Timeout);
            }
            std::thread::sleep(pause);
        }
    }
}
