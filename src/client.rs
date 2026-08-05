//! An HTTP client for talking to any ACP server.
//!
//! ```no_run
//! use rusty_acp::client::AcpClient;
//! use rusty_acp::types::Message;
//!
//! # async fn demo() -> Result<(), rusty_acp::AcpError> {
//! let client = AcpClient::new("http://localhost:8000")?;
//!
//! // Discovery
//! for manifest in client.list_all_agents().await? {
//!     println!("{}: {}", manifest.name, manifest.description);
//! }
//!
//! // Synchronous run
//! let run = client.run_sync("echo", [Message::user("hello")]).await?;
//! println!("{}", run.output_text());
//! # Ok(())
//! # }
//! ```

use std::time::Duration;

use eventsource_stream::Eventsource;
use futures_util::{Stream, StreamExt};
use reqwest::{Client, Method, RequestBuilder, Response, StatusCode};
use serde::de::DeserializeOwned;

use crate::{
    types::{
        AgentManifest, AgentName, AgentsListResponse, Error, Event, Message, Run, RunCreateRequest,
        RunEventsListResponse, RunId, RunMode, RunResumeRequest, Session, SessionId,
    },
    AcpError, Result,
};

/// Default interval between polls in [`AcpClient::wait_for_run`].
pub const DEFAULT_POLL_INTERVAL: Duration = Duration::from_millis(250);

/// Default ceiling on how long [`AcpClient::wait_for_run`] keeps polling.
pub const DEFAULT_WAIT_TIMEOUT: Duration = Duration::from_secs(300);

/// How long to keep polling a run, and how often.
///
/// The timeout exists because a run can outlive the replica executing it. That
/// replica's lease lapses and the run is failed the next time anyone asks about
/// it — which polling triggers — but a server without leases, or an
/// unreachable one, would otherwise leave a caller looping forever.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WaitOptions {
    /// How long to sleep between polls.
    pub poll_interval: Duration,
    /// Give up after this long. `None` polls until the run settles.
    pub timeout: Option<Duration>,
}

impl Default for WaitOptions {
    fn default() -> Self {
        Self { poll_interval: DEFAULT_POLL_INTERVAL, timeout: Some(DEFAULT_WAIT_TIMEOUT) }
    }
}

impl WaitOptions {
    /// Poll at a different interval.
    pub fn poll_every(mut self, poll_interval: Duration) -> Self {
        self.poll_interval = poll_interval;
        self
    }

    /// Give up after `timeout`.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Poll until the run settles, however long that takes.
    pub fn without_timeout(mut self) -> Self {
        self.timeout = None;
        self
    }
}

/// A client for the ACP HTTP API.
///
/// Cloning is cheap: the underlying [`reqwest::Client`] is shared.
#[derive(Debug, Clone)]
pub struct AcpClient {
    http: Client,
    base_url: String,
}

impl AcpClient {
    /// Build a client pointing at `base_url`, e.g. `http://localhost:8000`.
    pub fn new(base_url: impl Into<String>) -> Result<Self> {
        Self::builder(base_url).build()
    }

    /// Start configuring a client.
    pub fn builder(base_url: impl Into<String>) -> AcpClientBuilder {
        AcpClientBuilder { base_url: base_url.into(), http: None, timeout: None }
    }

    /// Build a client from an existing [`reqwest::Client`].
    pub fn with_http_client(base_url: impl Into<String>, http: Client) -> Result<Self> {
        Self::builder(base_url).http_client(http).build()
    }

    /// The base URL the client targets, without a trailing slash.
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// The underlying HTTP client.
    pub fn http_client(&self) -> &Client {
        &self.http
    }

    fn url(&self, path: &str) -> String {
        format!("{}/{}", self.base_url, path.trim_start_matches('/'))
    }

    fn request(&self, method: Method, path: &str) -> RequestBuilder {
        self.http.request(method, self.url(path))
    }

    /// Check that the server is reachable.
    pub async fn ping(&self) -> Result<()> {
        let response = send(self.request(Method::GET, "/ping")).await?;
        check_status(response).await.map(|_| ())
    }

    /// List agents, one page at a time.
    ///
    /// `limit` must be between 1 and 1000; the server defaults to 10.
    pub async fn list_agents(
        &self,
        limit: Option<usize>,
        offset: Option<usize>,
    ) -> Result<Vec<AgentManifest>> {
        let mut request = self.request(Method::GET, "/agents");
        if let Some(limit) = limit {
            request = request.query(&[("limit", limit)]);
        }
        if let Some(offset) = offset {
            request = request.query(&[("offset", offset)]);
        }
        let response: AgentsListResponse = json(send(request).await?).await?;
        Ok(response.agents)
    }

    /// List every agent, paging until the server returns a short page.
    pub async fn list_all_agents(&self) -> Result<Vec<AgentManifest>> {
        const PAGE: usize = 100;
        let mut all = Vec::new();
        let mut offset = 0;
        loop {
            let page = self.list_agents(Some(PAGE), Some(offset)).await?;
            let received = page.len();
            all.extend(page);
            if received < PAGE {
                return Ok(all);
            }
            offset += received;
        }
    }

    /// Fetch one agent's manifest.
    pub async fn get_agent(&self, name: impl AsRef<str>) -> Result<AgentManifest> {
        json(send(self.request(Method::GET, &format!("/agents/{}", name.as_ref()))).await?).await
    }

    /// Create a run. The request's mode must not be [`RunMode::Stream`]; use
    /// [`stream_run`](AcpClient::stream_run) for that.
    pub async fn create_run(&self, request: RunCreateRequest) -> Result<Run> {
        if request.mode() == RunMode::Stream {
            return Err(Error::invalid_input(
                "use `stream_run` for `stream` mode; `create_run` returns a single Run",
            )
            .into());
        }
        request.validate()?;
        json(send(self.request(Method::POST, "/runs").json(&request)).await?).await
    }

    /// Run an agent synchronously, blocking until it completes, fails or awaits.
    pub async fn run_sync(
        &self,
        agent_name: impl AsRef<str>,
        input: impl IntoIterator<Item = Message>,
    ) -> Result<Run> {
        let request = RunCreateRequest::new(parse_agent_name(agent_name)?, input);
        self.create_run(request.with_mode(RunMode::Sync)).await
    }

    /// Start a run and return as soon as the server has accepted it.
    ///
    /// Poll [`get_run`](AcpClient::get_run), or use
    /// [`wait_for_run`](AcpClient::wait_for_run).
    pub async fn run_async(
        &self,
        agent_name: impl AsRef<str>,
        input: impl IntoIterator<Item = Message>,
    ) -> Result<Run> {
        let request = RunCreateRequest::new(parse_agent_name(agent_name)?, input);
        self.create_run(request.with_mode(RunMode::Async)).await
    }

    /// Start a run and stream its events as they are emitted.
    ///
    /// The stream ends after the terminal event — `run.completed`,
    /// `run.failed`, `run.cancelled`, `run.awaiting`, or a stream-level
    /// `error`.
    pub async fn stream_run(
        &self,
        request: RunCreateRequest,
    ) -> Result<impl Stream<Item = Result<Event>> + Send + Unpin> {
        request.validate()?;
        let request = RunCreateRequest { mode: Some(RunMode::Stream), ..request };
        let response = send(self.request(Method::POST, "/runs").json(&request)).await?;
        event_stream(check_status(response).await?)
    }

    /// Stream a run of `agent_name` over the given input.
    pub async fn stream(
        &self,
        agent_name: impl AsRef<str>,
        input: impl IntoIterator<Item = Message>,
    ) -> Result<impl Stream<Item = Result<Event>> + Send + Unpin> {
        self.stream_run(RunCreateRequest::new(parse_agent_name(agent_name)?, input)).await
    }

    /// Fetch the current state of a run.
    pub async fn get_run(&self, run_id: RunId) -> Result<Run> {
        json(send(self.request(Method::GET, &format!("/runs/{run_id}"))).await?).await
    }

    /// Poll a run until it reaches a terminal state or pauses awaiting input.
    ///
    /// Gives up after [`WaitOptions::timeout`], reporting the status it last
    /// saw rather than looping forever.
    ///
    /// ```no_run
    /// # use rusty_acp::client::{AcpClient, WaitOptions};
    /// # use rusty_acp::types::RunId;
    /// # async fn demo(client: AcpClient, run_id: RunId) -> Result<(), rusty_acp::AcpError> {
    /// let run = client.wait_for_run(run_id, WaitOptions::default()).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn wait_for_run(&self, run_id: RunId, options: WaitOptions) -> Result<Run> {
        self.poll_until(run_id, options, |run| run.status.is_terminal() || run.status.is_awaiting())
            .await
    }

    /// Poll a run until `settled` accepts it, or the timeout elapses.
    async fn poll_until(
        &self,
        run_id: RunId,
        options: WaitOptions,
        settled: impl Fn(&Run) -> bool,
    ) -> Result<Run> {
        let deadline = options.timeout.map(|timeout| tokio::time::Instant::now() + timeout);
        loop {
            let run = self.get_run(run_id).await?;
            if settled(&run) {
                return Ok(run);
            }
            if deadline.is_some_and(|deadline| tokio::time::Instant::now() >= deadline) {
                return Err(AcpError::Timeout {
                    run_id: run_id.to_string(),
                    status: run.status.to_string(),
                });
            }
            tokio::time::sleep(options.poll_interval).await;
        }
    }

    /// Resume an awaiting run. The request's mode must not be
    /// [`RunMode::Stream`]; use [`stream_resume`](AcpClient::stream_resume).
    pub async fn resume_run(&self, request: RunResumeRequest) -> Result<Run> {
        if request.mode == RunMode::Stream {
            return Err(Error::invalid_input(
                "use `stream_resume` for `stream` mode; `resume_run` returns a single Run",
            )
            .into());
        }
        let path = format!("/runs/{}", request.run_id);
        json(send(self.request(Method::POST, &path).json(&request)).await?).await
    }

    /// Resume an awaiting run and stream the events that follow.
    pub async fn stream_resume(
        &self,
        request: RunResumeRequest,
    ) -> Result<impl Stream<Item = Result<Event>> + Send + Unpin> {
        let path = format!("/runs/{}", request.run_id);
        let request = RunResumeRequest { mode: RunMode::Stream, ..request };
        let response = send(self.request(Method::POST, &path).json(&request)).await?;
        event_stream(check_status(response).await?)
    }

    /// Request cancellation of a run.
    ///
    /// The server accepts the request and moves the run to `cancelling`; the
    /// returned snapshot may not yet be `cancelled`.
    pub async fn cancel_run(&self, run_id: RunId) -> Result<Run> {
        json(send(self.request(Method::POST, &format!("/runs/{run_id}/cancel"))).await?).await
    }

    /// Cancel a run and poll until it reaches a terminal state.
    ///
    /// Waits for a *terminal* status rather than merely a non-`cancelling` one:
    /// the server accepts a cancellation before applying it — and with several
    /// replicas, the request may be accepted by one replica and applied by
    /// another — so the run can still read `in-progress` right after
    /// [`cancel_run`](AcpClient::cancel_run) returns.
    ///
    /// The final status is usually [`Cancelled`](crate::types::RunStatus::Cancelled),
    /// but a run that finished before the cancellation landed stays
    /// [`Completed`](crate::types::RunStatus::Completed) or
    /// [`Failed`](crate::types::RunStatus::Failed).
    pub async fn cancel_and_wait(&self, run_id: RunId, options: WaitOptions) -> Result<Run> {
        self.cancel_run(run_id).await?;
        self.poll_until(run_id, options, |run| run.status.is_terminal()).await
    }

    /// Fetch the full event log of a run.
    pub async fn list_run_events(&self, run_id: RunId) -> Result<Vec<Event>> {
        let response: RunEventsListResponse =
            json(send(self.request(Method::GET, &format!("/runs/{run_id}/events"))).await?).await?;
        Ok(response.events)
    }

    /// Fetch a session.
    pub async fn get_session(&self, session_id: SessionId) -> Result<Session> {
        json(send(self.request(Method::GET, &format!("/session/{session_id}"))).await?).await
    }

    /// Dereference every URL in a session's history into a [`Message`].
    ///
    /// History URLs may point at other servers; each is fetched with this
    /// client's HTTP client, in order.
    pub async fn fetch_session_history(&self, session: &Session) -> Result<Vec<Message>> {
        let mut messages = Vec::with_capacity(session.history.len());
        for url in &session.history {
            let response = send(self.http.get(url)).await?;
            messages.push(json(response).await?);
        }
        Ok(messages)
    }

    /// Fetch the state document a session points at, decoded as `T`.
    ///
    /// Returns `Ok(None)` when the session has no state URL.
    pub async fn fetch_session_state<T: DeserializeOwned>(
        &self,
        session: &Session,
    ) -> Result<Option<T>> {
        let Some(url) = &session.state else {
            return Ok(None);
        };
        let response = send(self.http.get(url)).await?;
        json(response).await.map(Some)
    }
}

/// Builder for [`AcpClient`].
#[derive(Debug, Clone)]
pub struct AcpClientBuilder {
    base_url: String,
    http: Option<Client>,
    timeout: Option<Duration>,
}

impl AcpClientBuilder {
    /// Use an existing HTTP client, e.g. one carrying auth headers.
    pub fn http_client(mut self, http: Client) -> Self {
        self.http = Some(http);
        self
    }

    /// Set a per-request timeout.
    ///
    /// Ignored when an HTTP client is supplied; configure the timeout there.
    /// Streaming runs can outlive any timeout, so leave this unset for them.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Validate the base URL and build the client.
    pub fn build(self) -> Result<AcpClient> {
        let base_url = self.base_url.trim_end_matches('/').to_string();
        if !(base_url.starts_with("http://") || base_url.starts_with("https://")) {
            return Err(AcpError::InvalidUrl(format!(
                "base url must start with http:// or https://, got {:?}",
                self.base_url
            )));
        }

        let http = match self.http {
            Some(http) => http,
            None => {
                let mut builder = Client::builder();
                if let Some(timeout) = self.timeout {
                    builder = builder.timeout(timeout);
                }
                builder.build().map_err(|err| AcpError::Transport(err.to_string()))?
            }
        };

        Ok(AcpClient { http, base_url })
    }
}

fn parse_agent_name(name: impl AsRef<str>) -> Result<AgentName> {
    AgentName::new(name.as_ref()).map_err(AcpError::from)
}

async fn send(request: RequestBuilder) -> Result<Response> {
    request.send().await.map_err(|err| AcpError::Transport(err.to_string()))
}

/// Turn a non-success response into an [`AcpError`], preferring the ACP error
/// object when the body carries one.
async fn check_status(response: Response) -> Result<Response> {
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    let body = response.text().await.unwrap_or_default();
    match serde_json::from_str::<Error>(&body) {
        Ok(error) => Err(AcpError::Protocol(error)),
        Err(_) => Err(http_error(status, body)),
    }
}

fn http_error(status: StatusCode, body: String) -> AcpError {
    AcpError::Http { status: status.as_u16(), body }
}

async fn json<T: DeserializeOwned>(response: Response) -> Result<T> {
    let response = check_status(response).await?;
    let body = response.text().await.map_err(|err| AcpError::Transport(err.to_string()))?;
    serde_json::from_str(&body).map_err(|err| {
        AcpError::Serialization(format!("failed to decode response: {err}; body: {body}"))
    })
}

/// Adapt an SSE response into a stream of [`Event`]s.
fn event_stream(response: Response) -> Result<impl Stream<Item = Result<Event>> + Send + Unpin> {
    let stream = response.bytes_stream().eventsource().map(|item| match item {
        Ok(message) => serde_json::from_str::<Event>(&message.data).map_err(|err| {
            AcpError::Stream(format!(
                "failed to decode `{}` event: {err}; data: {}",
                message.event, message.data
            ))
        }),
        Err(err) => Err(AcpError::Stream(err.to_string())),
    });
    Ok(Box::pin(TerminalInclusive { inner: Box::pin(stream), done: false }))
}

/// Ends the stream *after* yielding the terminal event, so callers see the
/// final `run.*` snapshot rather than losing it to the cut-off.
struct TerminalInclusive<S> {
    inner: std::pin::Pin<Box<S>>,
    done: bool,
}

impl<S: Stream<Item = Result<Event>>> Stream for TerminalInclusive<S> {
    type Item = Result<Event>;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        if self.done {
            return std::task::Poll::Ready(None);
        }
        let polled = self.inner.as_mut().poll_next(cx);
        if let std::task::Poll::Ready(Some(Ok(event))) = &polled {
            if event.is_terminal() {
                self.done = true;
            }
        }
        polled
    }
}

/// Collect a run event stream into its final [`Run`] snapshot.
///
/// Returns the run carried by the terminal `run.*` event, or an error if the
/// stream ended without one.
pub async fn collect_run(mut stream: impl Stream<Item = Result<Event>> + Unpin) -> Result<Run> {
    let mut last_run = None;
    while let Some(event) = stream.next().await {
        let event = event?;
        if let Event::Error { error } = &event {
            return Err(AcpError::Protocol(error.clone()));
        }
        if let Some(run) = event.run() {
            last_run = Some(run.clone());
        }
    }
    last_run.ok_or_else(|| AcpError::Stream("stream ended without a run event".to_string()))
}
