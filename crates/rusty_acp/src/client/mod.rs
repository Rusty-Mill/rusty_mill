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

mod telemetry;

use std::time::Duration;

use eventsource_stream::Eventsource;
use futures_util::{Stream, StreamExt, TryStreamExt};
use reqwest::{Client, Method, RequestBuilder, Response, StatusCode};
use serde::de::DeserializeOwned;

#[cfg(feature = "trace")]
use crate::trace::{TraceContext, TRACEPARENT_HEADER};

use crate::{
    types::{
        AgentManifest, AgentName, AgentsListResponse, Error, Event, Message, Run, RunCreateRequest,
        RunEventsListResponse, RunId, RunMode, RunResumeRequest, Session, SessionId,
    },
    AcpError, Result, EVENTS_FROM_HEADER,
};

/// Default interval between polls in [`AcpClient::wait_for_run`].
pub const DEFAULT_POLL_INTERVAL: Duration = Duration::from_millis(250);

/// Default ceiling on how long [`AcpClient::wait_for_run`] keeps polling.
pub const DEFAULT_WAIT_TIMEOUT: Duration = Duration::from_secs(300);

/// How many session-history URLs are dereferenced at once.
///
/// A fixed number rather than a builder setting. Every knob is another thing to
/// get wrong, and this crate has declined a second one before — `retention`
/// deliberately covers runs and sessions with a single window. Nobody has a
/// reason to want a different value yet; when somebody does, that reason is
/// what should design the setting.
///
/// Eight, because the constraint is the *other* end. History URLs may point at
/// several different servers, and a client fetching several sessions multiplies
/// whatever this is; a number small enough to be polite to one host is the right
/// default, and it already turns a two-hundred-turn conversation from two
/// hundred sequential round trips into twenty-five.
pub const HISTORY_CONCURRENCY: usize = 8;

/// How a dropped event stream is resumed.
///
/// A streaming run can outlive the connection carrying it: proxies time idle
/// connections out, load balancers recycle them, and the replica executing the
/// run can die. The event log is durable and ordered, so a stream that drops
/// can be picked up from the last event the client saw rather than restarted or
/// abandoned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReconnectPolicy {
    /// How many times in a row to reconnect before giving up.
    ///
    /// Reset by any event that arrives, so a long run that drops repeatedly but
    /// keeps making progress is not cut off by the ceiling. Zero disables
    /// resumption.
    pub max_attempts: u32,
    /// Delay before the first reconnection, doubling with each consecutive
    /// failure.
    pub initial_backoff: Duration,
    /// Ceiling the backoff doubles up to.
    pub max_backoff: Duration,
}

impl ReconnectPolicy {
    /// Never resume: a dropped stream simply ends.
    pub fn disabled() -> Self {
        Self { max_attempts: 0, ..Self::default() }
    }

    /// The delay before the reconnection following `attempts` failures.
    fn backoff_for(&self, attempts: u32) -> Duration {
        let factor = 2u32.saturating_pow(attempts.min(16));
        self.initial_backoff.saturating_mul(factor).min(self.max_backoff)
    }
}

impl Default for ReconnectPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 5,
            initial_backoff: Duration::from_millis(200),
            max_backoff: Duration::from_secs(5),
        }
    }
}

/// How transient failures are retried.
///
/// The deployment this crate is aimed at is several replicas behind a load
/// balancer, and replicas are expected to die — that is the whole point of the
/// leases and the reaper. A client pointed at such a fleet should not fold the
/// first time it meets a member going away. A connection reset, a 502 from a
/// balancer mid-deploy and a 503 from a replica still starting all have the
/// same obvious answer: try again in a moment.
///
/// # What is retried
///
/// Transport failures where no response arrived — connect errors and timeouts —
/// and the statuses that mean *not now*: 429, 502, 503 and 504. `Retry-After`
/// is honoured when the server sends it.
///
/// **500 is deliberately excluded.** It is what an ACP server returns when the
/// agent itself failed, which a second attempt reproduces rather than resolves;
/// retrying it turns one failure into several and delays the error the caller
/// needs to see. That is the opposite trade-off from 503, which is why the set
/// is enumerated rather than written as "any 5xx".
///
/// # What is not retried by default
///
/// Creating a run, and resuming an awaiting one. A request that timed out may
/// well have been received and started, and ACP has no idempotency key that
/// would let a second attempt collapse into the first — so a retry can leave
/// two runs behind, each with the side effects of one. Reads and cancellation
/// have no such hazard: reading twice costs a round trip, and cancelling a run
/// that is already cancelling is a no-op.
///
/// Callers who know their agents are idempotent, or who would rather have a
/// duplicate run than a failed submission, can set
/// [`retry_run_submission`](RetryPolicy::retry_run_submission).
///
/// ```
/// # use std::time::Duration;
/// # use rusty_acp::client::{AcpClient, RetryPolicy};
/// # fn demo() -> Result<(), rusty_acp::AcpError> {
/// let client = AcpClient::builder("http://localhost:8000")
///     .retry(RetryPolicy { max_retries: 5, ..RetryPolicy::default() })
///     .build()?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct RetryPolicy {
    /// How many times to re-send a failed request. Zero disables retrying —
    /// and with it the polling helpers' tolerance of blips mid-wait.
    pub max_retries: u32,
    /// Delay before the first retry, doubling with each further failure.
    pub initial_backoff: Duration,
    /// Ceiling the backoff doubles up to, and the longest `Retry-After` that
    /// will be waited out.
    pub max_backoff: Duration,
    /// What fraction of each delay is randomised, from `0.0` (none) to `1.0`
    /// (the whole delay).
    ///
    /// Without it, clients that failed together retry together: a replica
    /// coming back up is met by the same thundering herd that its predecessor
    /// died to, at intervals the backoff keeps in lockstep.
    pub jitter: f64,
    /// Also retry run creation and resumption, accepting that a retry may
    /// duplicate a run the server had already started.
    pub retry_run_submission: bool,
}

impl RetryPolicy {
    /// Never retry: every failure reaches the caller.
    pub fn disabled() -> Self {
        Self { max_retries: 0, ..Self::default() }
    }

    /// The delay after `attempt` consecutive failures, jitter included.
    fn backoff_for(&self, attempt: u32) -> Duration {
        let factor = 2u32.saturating_pow(attempt.min(16));
        let base = self.initial_backoff.saturating_mul(factor).min(self.max_backoff);
        let jitter = self.jitter.clamp(0.0, 1.0);
        if jitter == 0.0 {
            return base;
        }
        let fixed = base.mul_f64(1.0 - jitter);
        fixed.saturating_add(base.mul_f64(jitter * random_fraction()))
    }
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_retries: 3,
            initial_backoff: Duration::from_millis(100),
            max_backoff: Duration::from_secs(5),
            jitter: 0.5,
            retry_run_submission: false,
        }
    }
}

/// A random fraction in `[0, 1]`.
///
/// Drawn from the random bytes of a v4 UUID rather than from `rand`: `uuid` is
/// already a required dependency and generates v4s from the platform RNG, so
/// this buys decorrelated backoff without adding a crate for four bytes of
/// entropy. Bytes 9 onward are used because 6 and 8 carry the version and
/// variant bits and are not random.
fn random_fraction() -> f64 {
    let bytes = uuid::Uuid::new_v4().into_bytes();
    let raw = u32::from_le_bytes([bytes[9], bytes[10], bytes[11], bytes[12]]);
    f64::from(raw) / f64::from(u32::MAX)
}

/// Whether a request can be sent again after a failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Replay {
    /// Nothing happens beyond the response being produced again: every read,
    /// and cancellation.
    Safe,
    /// May already have taken effect when it failed, so a retry risks doing it
    /// twice.
    Effectful,
}

/// Statuses that mean "not now" rather than "no".
fn retryable_status(status: StatusCode) -> bool {
    matches!(status.as_u16(), 429 | 502 | 503 | 504)
}

/// Whether a send failed in a way that leaves the request safe to repeat.
///
/// Builder and redirect errors are excluded: neither is transient, and a bad
/// URL retried three times is still a bad URL.
fn retryable_transport(error: &reqwest::Error) -> bool {
    if error.is_builder() || error.is_redirect() {
        return false;
    }
    error.is_connect() || error.is_timeout() || error.is_request()
}

/// The delay a server asked for, in either of the two formats `Retry-After`
/// allows: a count of seconds, or an HTTP date.
///
/// A date already in the past yields `None`, which falls back to the policy's
/// own backoff rather than retrying instantly.
fn retry_after(headers: &reqwest::header::HeaderMap) -> Option<Duration> {
    let value = headers.get(reqwest::header::RETRY_AFTER)?.to_str().ok()?.trim();
    if let Ok(seconds) = value.parse::<u64>() {
        return Some(Duration::from_secs(seconds));
    }
    let when = chrono::DateTime::parse_from_rfc2822(value).ok()?;
    (when.with_timezone(&chrono::Utc) - chrono::Utc::now()).to_std().ok()
}

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
    reconnect: ReconnectPolicy,
    retry: RetryPolicy,
    #[cfg(feature = "trace")]
    send_trace_headers: bool,
}

impl AcpClient {
    /// Build a client pointing at `base_url`, e.g. `http://localhost:8000`.
    pub fn new(base_url: impl Into<String>) -> Result<Self> {
        Self::builder(base_url).build()
    }

    /// Start configuring a client.
    pub fn builder(base_url: impl Into<String>) -> AcpClientBuilder {
        AcpClientBuilder {
            base_url: base_url.into(),
            http: None,
            timeout: None,
            reconnect: ReconnectPolicy::default(),
            retry: RetryPolicy::default(),
            #[cfg(feature = "trace")]
            send_trace_headers: true,
        }
    }

    /// Build a client from an existing [`reqwest::Client`].
    ///
    /// Construct it through [`rusty_acp::reqwest`](crate::reqwest) rather than
    /// your own dependency, so the two cannot resolve to different copies of
    /// the crate and leave you with a type error between two identically named
    /// types.
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
        self.traced(self.http.request(method, self.url(path)))
    }

    /// Attach a `traceparent`, unless the caller has turned it off.
    ///
    /// A fresh trace per call. Without OpenTelemetry there is no ambient
    /// context to inherit — `tracing` spans carry no trace id of their own —
    /// so the honest thing is to start one rather than to pretend to continue
    /// something. It still earns its place: the id this client logs is the one
    /// the server logs, and the one every replica that later touches the run
    /// logs, which is the correlation that did not exist before.
    ///
    /// A caller that *does* have a trace — because something upstream gave it
    /// one — sets the header on its own [`reqwest::Client`] and calls
    /// [`without_trace_headers`](AcpClientBuilder::without_trace_headers), so
    /// there is exactly one `traceparent` on the wire rather than two.
    fn traced(&self, request: RequestBuilder) -> RequestBuilder {
        #[cfg(feature = "trace")]
        if self.send_trace_headers {
            return request.header(TRACEPARENT_HEADER, TraceContext::mint().to_string());
        }
        request
    }

    /// The retry policy in force.
    pub fn retry_policy(&self) -> &RetryPolicy {
        &self.retry
    }

    /// The policy governing a request of this kind, or `None` when it is to be
    /// sent exactly once.
    fn policy_for(&self, replay: Replay) -> Option<&RetryPolicy> {
        if self.retry.max_retries == 0 {
            return None;
        }
        match replay {
            Replay::Safe => Some(&self.retry),
            Replay::Effectful if self.retry.retry_run_submission => Some(&self.retry),
            Replay::Effectful => None,
        }
    }

    /// Send a request, re-sending it while it fails transiently.
    ///
    /// The response is returned without its status being checked, as before —
    /// this decides only whether to *repeat* the request, and leaves what a
    /// status means to [`check_status`]. A retryable status that survives the
    /// last attempt is handed back intact, so the caller sees the server's own
    /// 503 rather than an error invented here.
    async fn send(&self, request: RequestBuilder, replay: Replay) -> Result<Response> {
        // Named before the loop because a `RequestBuilder` is consumed by
        // sending, and a span field cannot be filled in from something that has
        // been moved.
        let method = request.try_clone().map_or("unknown", |probe| {
            probe.build().map_or("unknown", |built| match *built.method() {
                Method::GET => "GET",
                Method::POST => "POST",
                Method::PUT => "PUT",
                Method::DELETE => "DELETE",
                _ => "other",
            })
        });

        let Some(policy) = self.policy_for(replay) else {
            telemetry::request_sent(method);
            return send_once(request).await;
        };

        let mut attempt = 0u32;
        loop {
            // A request whose body cannot be cloned — a stream — cannot be
            // replayed, so it gets the one attempt it can have.
            let Some(this_attempt) = request.try_clone() else {
                telemetry::request_sent(method);
                return send_once(request).await;
            };
            let last_attempt = attempt >= policy.max_retries;

            telemetry::request_sent(method);
            let (delay, reason) = match this_attempt.send().await {
                Ok(response) => {
                    if !retryable_status(response.status()) {
                        return Ok(response);
                    }
                    if last_attempt {
                        // Warn, and only here. Each retry is routine and logged
                        // at debug; running out of them is where a request
                        // actually failed after the client tried to save it,
                        // and is the line an operator wants.
                        tracing::warn!(
                            method,
                            status = response.status().as_u16(),
                            attempts = attempt + 1,
                            "giving up on an acp request after exhausting the retry policy"
                        );
                        telemetry::retries_exhausted(method);
                        return Ok(response);
                    }
                    // A `Retry-After` longer than the ceiling is obeyed by
                    // giving up rather than by ignoring it: a server that asked
                    // for a minute should not be knocked on in five seconds.
                    match retry_after(response.headers()) {
                        Some(asked) if asked > policy.max_backoff => {
                            tracing::debug!(
                                method,
                                ?asked,
                                max_backoff = ?policy.max_backoff,
                                "obeying a Retry-After longer than the ceiling by giving up"
                            );
                            return Ok(response);
                        }
                        Some(asked) => (asked, "retry_after"),
                        None => (policy.backoff_for(attempt), "status"),
                    }
                }
                Err(error) => {
                    if !retryable_transport(&error) {
                        return Err(AcpError::Transport(error.to_string()));
                    }
                    if last_attempt {
                        tracing::warn!(
                            method,
                            %error,
                            attempts = attempt + 1,
                            "giving up on an acp request after exhausting the retry policy"
                        );
                        telemetry::retries_exhausted(method);
                        return Err(AcpError::Transport(error.to_string()));
                    }
                    (policy.backoff_for(attempt), "transport")
                }
            };

            tracing::debug!(method, attempt = attempt + 1, ?delay, reason, "retrying acp request");
            telemetry::retried(reason, delay);
            tokio::time::sleep(delay).await;
            attempt += 1;
        }
    }

    /// Check that the server is reachable.
    pub async fn ping(&self) -> Result<()> {
        let response = self.send(self.request(Method::GET, "/ping"), Replay::Safe).await?;
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
        let response: AgentsListResponse = json(self.send(request, Replay::Safe).await?).await?;
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
        json(
            self.send(
                self.request(Method::GET, &format!("/agents/{}", name.as_ref())),
                Replay::Safe,
            )
            .await?,
        )
        .await
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
        json(
            self.send(self.request(Method::POST, "/runs").json(&request), Replay::Effectful)
                .await?,
        )
        .await
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
        let response = self
            .send(self.request(Method::POST, "/runs").json(&request), Replay::Effectful)
            .await?;
        event_stream(self.clone(), check_status(response).await?)
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
        json(self.send(self.request(Method::GET, &format!("/runs/{run_id}")), Replay::Safe).await?)
            .await
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
    ///
    /// A transient failure mid-wait is treated as *not settled yet* rather than
    /// as an answer. These helpers exist to wait out runs that take minutes,
    /// across a fleet whose members come and go; folding because one poll met a
    /// replica going away would defeat the point, when the next poll a quarter
    /// of a second later would have found the run alive and progressing. The
    /// deadline still bounds the whole thing, so a server that is simply down
    /// ends the wait rather than extending it — and reports the failure it kept
    /// meeting, not a timeout that would hide it.
    ///
    /// Only when retrying is switched off entirely does the first failure
    /// propagate, so that [`RetryPolicy::disabled`] means one thing everywhere.
    async fn poll_until(
        &self,
        run_id: RunId,
        options: WaitOptions,
        settled: impl Fn(&Run) -> bool,
    ) -> Result<Run> {
        let deadline = options.timeout.map(|timeout| tokio::time::Instant::now() + timeout);
        let tolerate_blips = self.retry.max_retries > 0;
        let mut last_status: Option<String> = None;

        loop {
            // Reset every iteration: only a failure the wait *ended* on is
            // worth reporting, not one it recovered from.
            let last_error = match self.get_run(run_id).await {
                Ok(run) => {
                    if settled(&run) {
                        return Ok(run);
                    }
                    last_status = Some(run.status.to_string());
                    None
                }
                Err(error) if tolerate_blips && error.is_transient() => {
                    tracing::debug!(%run_id, %error, "transient failure while waiting; polling on");
                    Some(error)
                }
                Err(error) => return Err(error),
            };

            if deadline.is_some_and(|deadline| tokio::time::Instant::now() >= deadline) {
                return Err(last_error.unwrap_or_else(|| AcpError::Timeout {
                    run_id: run_id.to_string(),
                    // Only reachable with a status in hand: the first poll
                    // either sets one or leaves an error behind.
                    status: last_status.unwrap_or_else(|| "unknown".to_string()),
                }));
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
        json(self.send(self.request(Method::POST, &path).json(&request), Replay::Effectful).await?)
            .await
    }

    /// Resume an awaiting run and stream the events that follow.
    pub async fn stream_resume(
        &self,
        request: RunResumeRequest,
    ) -> Result<impl Stream<Item = Result<Event>> + Send + Unpin> {
        let path = format!("/runs/{}", request.run_id);
        let request = RunResumeRequest { mode: RunMode::Stream, ..request };
        let response =
            self.send(self.request(Method::POST, &path).json(&request), Replay::Effectful).await?;
        event_stream(self.clone(), check_status(response).await?)
    }

    /// Request cancellation of a run.
    ///
    /// The server accepts the request and moves the run to `cancelling`; the
    /// returned snapshot may not yet be `cancelled`.
    pub async fn cancel_run(&self, run_id: RunId) -> Result<Run> {
        json(
            self.send(self.request(Method::POST, &format!("/runs/{run_id}/cancel")), Replay::Safe)
                .await?,
        )
        .await
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

    /// The request that asks a run's log for a live, resumable stream.
    ///
    /// One place rather than two: [`attach`](AcpClient::attach) makes it to
    /// start a stream and [`ResumeState::reconnect`] makes it to pick one back
    /// up, and the two must agree about the headers or resumption works from
    /// one entry point and not the other.
    fn events_request(&self, run_id: RunId, after: Option<u64>) -> RequestBuilder {
        let mut request = self
            .request(Method::GET, &format!("/runs/{run_id}/events"))
            .header(reqwest::header::ACCEPT, "text/event-stream");
        if let Some(after) = after {
            request = request.header("last-event-id", after.to_string());
        }
        request
    }

    /// Stream the events of a run this client did not start.
    ///
    /// The run has to exist; everything already in its log is replayed, then
    /// the stream continues live and ends after the terminal event — exactly
    /// what [`stream_run`](AcpClient::stream_run) yields, for a run somebody
    /// else submitted. Attaching to a run that has already finished replays the
    /// whole log and closes, which is the useful answer rather than an error.
    ///
    /// **More robust than `stream_run`, and worth knowing why.** The run id is
    /// known here before the first byte arrives, where `stream_run` learns it
    /// from the first `run.*` event. Resumption needs that id, so a connection
    /// that drops before any event arrives can be resumed by this method and
    /// cannot be by that one.
    ///
    /// ```no_run
    /// # use futures_util::StreamExt;
    /// # use rusty_acp::client::AcpClient;
    /// # use rusty_acp::types::{Message, RunId};
    /// # async fn demo(client: AcpClient) -> Result<(), rusty_acp::AcpError> {
    /// let started = client.run_async("writer", [Message::user("hello")]).await?;
    ///
    /// let mut events = client.attach(started.run_id).await?;
    /// while let Some(event) = events.next().await {
    ///     println!("{:?}", event?);
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn attach(
        &self,
        run_id: RunId,
    ) -> Result<impl Stream<Item = Result<Event>> + Send + Unpin> {
        self.attach_stream(run_id, None).await
    }

    /// Stream a run's events from after the one indexed `last_seen`.
    ///
    /// For a caller that has already read part of the log — with
    /// [`list_run_events`](AcpClient::list_run_events), or from an earlier
    /// stream — and wants the rest without re-reading what it has. The index is
    /// the SSE `id` the server tags each event with, and the semantics are the
    /// same: *everything after this*, not *everything from this*.
    pub async fn attach_after(
        &self,
        run_id: RunId,
        last_seen: u64,
    ) -> Result<impl Stream<Item = Result<Event>> + Send + Unpin> {
        self.attach_stream(run_id, Some(last_seen)).await
    }

    async fn attach_stream(
        &self,
        run_id: RunId,
        after: Option<u64>,
    ) -> Result<impl Stream<Item = Result<Event>> + Send + Unpin> {
        let response = send_checked(self, self.events_request(run_id, after)).await?;
        Ok(resumable_stream(self.clone(), response, Some(run_id), after))
    }

    /// Fetch a run's event log.
    ///
    /// Returns a [`RunEventLog`] rather than a bare `Vec<Event>` because the
    /// log may not be whole: a server bounding how much of one run it keeps
    /// drops the oldest events, and a short list would otherwise be
    /// indistinguishable from a short run. See
    /// [`is_complete`](RunEventLog::is_complete).
    pub async fn list_run_events(&self, run_id: RunId) -> Result<RunEventLog> {
        let response = self
            .send(self.request(Method::GET, &format!("/runs/{run_id}/events")), Replay::Safe)
            .await?;
        let response = check_status(response).await?;

        // Absent on a server from before the header existed, where the honest
        // answer is "no idea" rather than "nothing was dropped" — so it is
        // reported as `None` and `is_complete` says so.
        let first_index = response
            .headers()
            .get(EVENTS_FROM_HEADER)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse().ok());

        let body: RunEventsListResponse = json(response).await?;
        Ok(RunEventLog { events: body.events, first_index })
    }

    /// Fetch a session.
    pub async fn get_session(&self, session_id: SessionId) -> Result<Session> {
        json(
            self.send(self.request(Method::GET, &format!("/session/{session_id}")), Replay::Safe)
                .await?,
        )
        .await
    }

    /// Dereference every URL in a session's history into a [`Message`].
    ///
    /// The returned messages are in the session's order. The *fetches* are not:
    /// up to [`HISTORY_CONCURRENCY`] are in flight at once, and `buffered`
    /// — rather than `buffer_unordered` — puts the results back in order. The
    /// ordering that matters is of the answer, not of the requests, and the two
    /// were previously the same thing only because the loop was serial.
    ///
    /// Which cost a lot. ACP models history as *dereferenceable URLs* precisely
    /// so a session can span servers, so a long conversation was one round trip
    /// per turn, in sequence, each potentially to a different host: the total
    /// was the sum of every latency rather than the largest. Retries made it
    /// worse, since one slow server's backoff stalled every turn behind it.
    ///
    /// An error abandons the fetch rather than returning a partial history.
    /// A history with a hole in it that reads as complete is the failure the
    /// `410` and `Acp-Events-From` exist to prevent on the event log, and this
    /// is the same shape: a caller cannot tell a short conversation from a
    /// truncated one, and here it would be feeding that difference to an agent.
    pub async fn fetch_session_history(&self, session: &Session) -> Result<Vec<Message>> {
        futures_util::stream::iter(&session.history)
            .map(|url| async move {
                let response = self.send(self.traced(self.http.get(url)), Replay::Safe).await?;
                json(response).await
            })
            .buffered(HISTORY_CONCURRENCY)
            .try_collect()
            .await
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
        let response = self.send(self.traced(self.http.get(url)), Replay::Safe).await?;
        json(response).await.map(Some)
    }
}

/// A run's event log, and where it starts.
///
/// A server that bounds how much of one run's log it keeps drops the oldest
/// events, so the list it returns can be a tail rather than the whole thing. A
/// bare `Vec<Event>` could not say which, and a short log that reads as
/// complete is the silent loss the resumable stream's 410 exists to prevent —
/// left in place on this endpoint until now.
#[derive(Debug, Clone, PartialEq)]
pub struct RunEventLog {
    /// The events the server returned, in order.
    pub events: Vec<Event>,
    /// The index the first returned event sits at in the run's log.
    ///
    /// `Some(0)` means nothing was dropped. `None` means the server did not
    /// say — it predates the header — which is different from saying nothing
    /// was dropped, and is why this is an `Option` rather than defaulting to
    /// zero.
    pub first_index: Option<u64>,
}

impl RunEventLog {
    /// Whether this is known to be the whole log.
    ///
    /// `false` for a log with a trimmed front **and** for one whose server did
    /// not say. A caller that needs certainty gets the cautious answer; one
    /// that only wants the events can ignore this entirely.
    pub fn is_complete(&self) -> bool {
        self.first_index == Some(0)
    }

    /// How many events were dropped from the front, when the server said.
    pub fn dropped(&self) -> Option<u64> {
        self.first_index
    }

    /// The events, discarding what is known about the log's completeness.
    pub fn into_events(self) -> Vec<Event> {
        self.events
    }
}

impl std::ops::Deref for RunEventLog {
    type Target = [Event];

    fn deref(&self) -> &Self::Target {
        &self.events
    }
}

impl IntoIterator for RunEventLog {
    type Item = Event;
    type IntoIter = std::vec::IntoIter<Event>;

    fn into_iter(self) -> Self::IntoIter {
        self.events.into_iter()
    }
}

impl<'a> IntoIterator for &'a RunEventLog {
    type Item = &'a Event;
    type IntoIter = std::slice::Iter<'a, Event>;

    fn into_iter(self) -> Self::IntoIter {
        self.events.iter()
    }
}

/// Builder for [`AcpClient`].
#[derive(Debug, Clone)]
pub struct AcpClientBuilder {
    base_url: String,
    http: Option<Client>,
    timeout: Option<Duration>,
    reconnect: ReconnectPolicy,
    retry: RetryPolicy,
    #[cfg(feature = "trace")]
    send_trace_headers: bool,
}

impl AcpClientBuilder {
    /// Use an existing HTTP client, e.g. one carrying auth headers.
    ///
    /// ```
    /// use rusty_acp::client::AcpClient;
    /// use rusty_acp::reqwest;
    ///
    /// # fn build() -> Result<(), Box<dyn std::error::Error>> {
    /// let mut headers = reqwest::header::HeaderMap::new();
    /// headers.insert("authorization", "Bearer hunter2".parse()?);
    ///
    /// let client = AcpClient::builder("http://localhost:8000")
    ///     .http_client(reqwest::Client::builder().default_headers(headers).build()?)
    ///     .build()?;
    /// # let _ = client;
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// Built through [`rusty_acp::reqwest`](crate::reqwest), so the client
    /// handed over is guaranteed to be the type this expects.
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

    /// How to resume an event stream whose connection drops.
    ///
    /// Defaults to [`ReconnectPolicy::default`]. Pass
    /// [`ReconnectPolicy::disabled`] to let a dropped stream simply end.
    pub fn reconnect(mut self, reconnect: ReconnectPolicy) -> Self {
        self.reconnect = reconnect;
        self
    }

    /// How transient failures are retried.
    ///
    /// Defaults to [`RetryPolicy::default`]. Pass [`RetryPolicy::disabled`] to
    /// surface every failure to the caller — worth doing when the client sits
    /// behind a retrying proxy, or under middleware such as `reqwest-retry`
    /// that would otherwise multiply the attempts together.
    pub fn retry(mut self, retry: RetryPolicy) -> Self {
        self.retry = retry;
        self
    }

    /// Stop attaching a `traceparent` header to outbound requests.
    ///
    /// For a caller that already has a trace and puts the header on its own
    /// [`reqwest::Client`]. Without this the request would carry two, and which
    /// one a server reads is not something to leave to chance.
    ///
    /// Turning it off and setting nothing in its place costs the correlation:
    /// the server mints a trace per request either way, so the ids exist, but
    /// nothing ties this client's own logging to them.
    #[cfg(feature = "trace")]
    pub fn without_trace_headers(mut self) -> Self {
        self.send_trace_headers = false;
        self
    }

    /// Validate the base URL and build the client.
    pub fn build(self) -> Result<AcpClient> {
        // Registering descriptions is idempotent, so doing it per client rather
        // than once globally costs nothing and needs no initialisation call a
        // caller could forget. Same bargain the server makes in its builder.
        telemetry::describe();

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

        Ok(AcpClient {
            http,
            base_url,
            reconnect: self.reconnect,
            retry: self.retry,
            #[cfg(feature = "trace")]
            send_trace_headers: self.send_trace_headers,
        })
    }
}

fn parse_agent_name(name: impl AsRef<str>) -> Result<AgentName> {
    AgentName::new(name.as_ref()).map_err(AcpError::from)
}

async fn send_once(request: RequestBuilder) -> Result<Response> {
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

/// One decoded SSE message: the event, and the log index the server tagged it
/// with.
type IndexedEvent = (Option<u64>, Event);

/// A stream of SSE messages from one HTTP response.
type SseStream = std::pin::Pin<Box<dyn Stream<Item = Result<IndexedEvent>> + Send>>;

/// Adapt an SSE response into decoded events paired with their log index.
fn sse_messages(response: Response) -> SseStream {
    Box::pin(response.bytes_stream().eventsource().map(|item| match item {
        Ok(message) => {
            let event = serde_json::from_str::<Event>(&message.data).map_err(|err| {
                AcpError::Stream(format!(
                    "failed to decode `{}` event: {err}; data: {}",
                    message.event, message.data
                ))
            })?;
            Ok((message.id.trim().parse::<u64>().ok(), event))
        }
        Err(err) => Err(AcpError::Stream(err.to_string())),
    }))
}

/// What a resuming stream needs to pick up where it left off.
struct ResumeState {
    client: AcpClient,
    /// Learned from the first `run.*` event. Until it is known there is nothing
    /// to reconnect *to*, so an early drop cannot be resumed.
    run_id: Option<RunId>,
    /// The last index the server tagged, and so the point to resume after.
    last_index: Option<u64>,
    /// Consecutive failed reconnections; reset by any event that arrives.
    attempts: u32,
    /// Why the most recent reconnection failed, when it failed transiently.
    /// Cleared as soon as one succeeds, so it is only ever the reason the
    /// stream is about to end.
    last_error: Option<AcpError>,
    /// `None` while disconnected, which is what drives a reconnection.
    inner: Option<SseStream>,
    done: bool,
}

impl ResumeState {
    /// Reconnect and attach to the run's log after the last event seen.
    ///
    /// `Ok(true)` means the stream can carry on: either it is attached again,
    /// or this attempt failed in a way another attempt might fix. `Ok(false)`
    /// means resumption is over — switched off, the attempt ceiling spent, or
    /// no event has yet said which run this is.
    async fn reconnect(&mut self) -> Result<bool> {
        let policy = &self.client.reconnect;
        let Some(run_id) = self.run_id else {
            // No event has arrived, so there is no run id to resume against.
            // Distinguished from the ceiling below because they call for
            // different responses: this one is unlucky timing, that one is a
            // policy an operator set.
            telemetry::stream_abandoned("no_run_id");
            return Ok(false);
        };
        if self.attempts >= policy.max_attempts {
            tracing::warn!(
                %run_id,
                attempts = self.attempts,
                last_index = ?self.last_index,
                "giving up on a dropped event stream after exhausting the reconnect policy"
            );
            telemetry::stream_abandoned("exhausted");
            return Ok(false);
        }

        let backoff = policy.backoff_for(self.attempts);
        self.attempts += 1;
        tracing::debug!(
            %run_id,
            attempt = self.attempts,
            ?backoff,
            resume_from = ?self.last_index,
            "reconnecting a dropped event stream"
        );
        telemetry::stream_reconnected();
        tokio::time::sleep(backoff).await;

        let request = self.client.events_request(run_id, self.last_index);

        // A reconnection that cannot be made is the very case resumption exists
        // for, so a transient failure costs an attempt rather than the stream.
        // Anything else — a 404 for a run that has been swept, a bad URL — will
        // not be fixed by asking again, and reaches the caller.
        let response = match send_checked(&self.client, request).await {
            Ok(response) => response,
            Err(error) if error.is_transient() => {
                tracing::debug!(
                    %run_id,
                    %error,
                    attempt = self.attempts,
                    "a reconnection failed transiently; it costs an attempt, not the stream"
                );
                self.last_error = Some(error);
                return Ok(true);
            }
            Err(error) => return Err(error),
        };

        self.last_error = None;
        self.inner = Some(sse_messages(response));
        Ok(true)
    }
}

/// Send a safe request and reject a non-success status, as one fallible step.
///
/// A free function rather than a method on [`ResumeState`]: holding `&self`
/// across the await would make the resulting stream non-`Send`.
async fn send_checked(client: &AcpClient, request: RequestBuilder) -> Result<Response> {
    check_status(client.send(request, Replay::Safe).await?).await
}

/// Adapt an SSE response into a stream of [`Event`]s that survives a dropped
/// connection.
///
/// Ends *after* the terminal event, so callers see the final `run.*` snapshot
/// rather than losing it to the cut-off. Anything else that ends the underlying
/// response — a proxy idle-timeout, a load balancer recycling a connection, the
/// executing replica dying — is a drop rather than an ending, and is retried
/// against the run's durable log under the client's [`ReconnectPolicy`].
fn event_stream(
    client: AcpClient,
    response: Response,
) -> Result<impl Stream<Item = Result<Event>> + Send + Unpin> {
    // Nothing known up front: a run started by this request announces its id in
    // its first `run.*` event, and until then there is nothing to resume to.
    Ok(resumable_stream(client, response, None, None))
}

/// The same stream, told what it is watching.
///
/// `run_id` and `last_index` are what resumption needs. `attach` has both
/// before the response arrives; a freshly submitted run has neither.
fn resumable_stream(
    client: AcpClient,
    response: Response,
    run_id: Option<RunId>,
    last_index: Option<u64>,
) -> impl Stream<Item = Result<Event>> + Send + Unpin {
    let state = ResumeState {
        client,
        run_id,
        last_index,
        attempts: 0,
        last_error: None,
        inner: Some(sse_messages(response)),
        done: false,
    };

    Box::pin(futures_util::stream::unfold(state, |mut state| async move {
        if state.done {
            return None;
        }
        loop {
            let next = match state.inner.as_mut() {
                Some(inner) => inner.next().await,
                None => match state.reconnect().await {
                    Ok(true) => continue,
                    // Resumption is unavailable; the stream ends where it was
                    // cut rather than pretending the run finished. If the last
                    // thing that happened was a failed reconnection, that
                    // failure is *why* it is ending and the caller should see
                    // it — an empty end would read as a run that produced
                    // nothing.
                    Ok(false) => {
                        return match state.last_error.take() {
                            Some(error) => {
                                state.done = true;
                                Some((Err(error), state))
                            }
                            None => None,
                        }
                    }
                    Err(err) => {
                        state.done = true;
                        return Some((Err(err), state));
                    }
                },
            };

            match next {
                Some(Ok((index, event))) => {
                    if index.is_some() {
                        state.last_index = index;
                    }
                    if state.run_id.is_none() {
                        state.run_id = event.run().map(|run| run.run_id);
                    }
                    // A stream that is delivering again has earned a fresh
                    // budget; the ceiling is on consecutive failures.
                    state.attempts = 0;
                    state.done = event.is_terminal();
                    return Some((Ok(event), state));
                }
                // The response ended without a terminal event, or failed
                // mid-flight. Either way the run may still be going, so this is
                // a disconnection rather than the end of the stream.
                Some(Err(_)) | None => state.inner = None,
            }
        }
    }))
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Just the header, which is all the parsing sees.
    fn with_retry_after(value: &str) -> reqwest::header::HeaderMap {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(reqwest::header::RETRY_AFTER, value.parse().unwrap());
        headers
    }

    #[test]
    fn retry_after_reads_a_count_of_seconds() {
        assert_eq!(retry_after(&with_retry_after("7")), Some(Duration::from_secs(7)));
        assert_eq!(retry_after(&with_retry_after("  7 ")), Some(Duration::from_secs(7)));
    }

    /// The other format the header allows. Worth a test because it is the one
    /// that goes through a date parser and could silently stop working.
    #[test]
    fn retry_after_reads_an_http_date() {
        let when = chrono::Utc::now() + chrono::Duration::seconds(30);
        let parsed = retry_after(&with_retry_after(&when.to_rfc2822())).expect("a delay");
        assert!(
            parsed > Duration::from_secs(25) && parsed <= Duration::from_secs(30),
            "expected roughly 30s, got {parsed:?}"
        );
    }

    /// A date already past means "now", and falls back to the policy's own
    /// backoff rather than retrying instantly.
    #[test]
    fn retry_after_ignores_a_date_in_the_past() {
        let when = chrono::Utc::now() - chrono::Duration::seconds(30);
        assert_eq!(retry_after(&with_retry_after(&when.to_rfc2822())), None);
        assert_eq!(retry_after(&with_retry_after("not a date")), None);
    }

    #[test]
    fn backoff_doubles_and_is_capped() {
        let policy = RetryPolicy {
            initial_backoff: Duration::from_millis(100),
            max_backoff: Duration::from_millis(400),
            jitter: 0.0,
            ..RetryPolicy::default()
        };
        assert_eq!(policy.backoff_for(0), Duration::from_millis(100));
        assert_eq!(policy.backoff_for(1), Duration::from_millis(200));
        assert_eq!(policy.backoff_for(2), Duration::from_millis(400));
        assert_eq!(policy.backoff_for(99), Duration::from_millis(400));
    }

    /// Jitter has to stay inside the delay it is jittering — a backoff that can
    /// exceed its own ceiling is not a ceiling.
    #[test]
    fn jitter_stays_within_the_backoff() {
        let policy = RetryPolicy {
            initial_backoff: Duration::from_millis(100),
            jitter: 0.5,
            ..RetryPolicy::default()
        };
        let delays: Vec<_> = (0..64).map(|_| policy.backoff_for(0)).collect();
        for delay in &delays {
            assert!(
                *delay >= Duration::from_millis(50) && *delay <= Duration::from_millis(100),
                "{delay:?} outside the jittered range"
            );
        }
        // The point of jitter is that clients disagree; identical delays would
        // mean the randomness is not reaching the calculation.
        assert!(delays.iter().collect::<std::collections::HashSet<_>>().len() > 1);
    }

    #[test]
    fn no_jitter_is_deterministic() {
        let policy = RetryPolicy { jitter: 0.0, ..RetryPolicy::default() };
        assert_eq!(policy.backoff_for(1), policy.backoff_for(1));
    }
}
