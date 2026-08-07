//! External MCP policy processors (`policies.mcpGuardrails`).
//!
//! A processor is an MCP-aware policy service the gateway consults over gRPC
//! before forwarding a call and after its result comes back. It is Envoy's
//! `ext_authz` shape moved down to the MCP method layer, with one addition
//! that changes what it is for: a processor can **rewrite** as well as refuse.
//! That is what makes it a guardrail rather than a gate — redacting a secret
//! out of a tool result is not something a yes/no answer can do.
//!
//! # What this build hooks
//!
//! This gateway serves `tools/list` and `tools/call` and nothing else, so
//! those are the two methods there is anything to hook. A processor keyed on
//! `prompts/*` or `resources/read` is not ignored quietly: `Config::lint`
//! reports it against [`agentgateway_config::MCP_SERVED_METHODS`], because a
//! guardrail that never fires looks exactly like one that always passes.
//!
//! # Ordering and short-circuiting
//!
//! Processors run in configuration order. The first refusal ends the chain,
//! and a rewrite is visible to the processors after it — so a chain is a
//! pipeline, not a vote. That is upstream's behaviour and it is the useful
//! one: a redactor followed by a validator should see redacted input.
//!
//! # Rewriting the upstream request
//!
//! A processor's request-phase answer can also change the headers of the
//! upstream HTTP request carrying the call — see [`HeaderChanges`]. That is a
//! per-call change on a connection dialled once at startup, so it travels in
//! the request's extensions; `mutating_client` is the other half.
//!
//! # Failing closed
//!
//! A processor that cannot be reached, times out, or answers something
//! unparseable **refuses the call** unless `failureMode: failOpen` says
//! otherwise. Same reasoning as `extAuthz`: a policy service that is down must
//! not silently become an open door.

mod wire;

use std::time::Duration;

use agentgateway_config::{FailureMode, McpGuardrails, Processor, resolve};
use cel::{Context, Program};
use http::{HeaderMap, HeaderName, HeaderValue};

use crate::rules::Subject;

pub use wire::{
    AuthorizationError, HeaderMutation, McpHeader, McpRequest, McpRequestResult, McpResponse,
    McpResponseResult, authorization_error, request_result, response_result, to_proto_value,
};

/// Budget for one processor call when the config names none.
///
/// Ten seconds, matching upstream. Long compared with `extAuthz`, and
/// deliberately: a guardrail may be running a model over the payload, which is
/// a different kind of work from an allow/deny lookup.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);

/// Failure to build a processor chain from configuration.
#[derive(Debug, thiserror::Error)]
pub enum GuardrailsError {
    /// The processor's address could not be parsed.
    #[error("{at}: `{host}` is not a valid processor address")]
    Host {
        /// Where in the configuration it came from.
        at: String,
        /// The offending text.
        host: String,
    },

    /// A `metadata` expression did not compile.
    #[error("{at}: invalid CEL expression `{expression}`: {source}")]
    Metadata {
        /// Where in the configuration it came from.
        at: String,
        /// The expression that failed.
        expression: String,
        /// Why it failed.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
}

/// What a processor decided.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Carry on unchanged.
    Pass,
    /// Carry on with this JSON body instead.
    Mutated(Vec<u8>),
    /// Refuse, with this JSON-RPC error code, message and optional data.
    Reject {
        /// JSON-RPC error code.
        code: i32,
        /// Message returned to the caller.
        message: String,
        /// Structured payload from the processor, if it sent one.
        data: Option<serde_json::Value>,
    },
}

/// Header changes a processor asked for, resolved and ready to apply.
///
/// Collected across the whole chain: a later processor setting a name an
/// earlier one set wins, and a later `remove` cancels an earlier `set` (and
/// vice versa). That falls out of applying each processor's mutation in turn to
/// the same map, which is what upstream does.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HeaderChanges {
    set: Vec<(HeaderName, HeaderValue)>,
    remove: Vec<HeaderName>,
}

impl HeaderChanges {
    /// Whether a processor asked for anything at all.
    pub fn is_empty(&self) -> bool {
        self.set.is_empty() && self.remove.is_empty()
    }

    /// Headers to add or overwrite on the upstream call.
    pub fn set(&self) -> &[(HeaderName, HeaderValue)] {
        &self.set
    }

    /// Header names to drop from the upstream call.
    pub fn remove(&self) -> &[HeaderName] {
        &self.remove
    }

    /// Fold one processor's answer into the running set.
    ///
    /// Names and values a processor sends that HTTP cannot represent are
    /// skipped with a warning rather than failing the call: a malformed header
    /// is a bug in the processor, and refusing every request because of one is
    /// a worse outcome than dropping it and saying so.
    ///
    /// Repeated `set` entries for one name are joined with `", "`. The protocol
    /// says they form a list replacing the existing header, and a single field
    /// line with comma-separated values is how HTTP spells that — the one
    /// exception being `Set-Cookie`, which cannot be folded and which has no
    /// business on a request anyway.
    fn merge(&mut self, mutation: HeaderMutation) {
        // Within one mutation the first entry for a name overwrites and the
        // rest append, so a processor can send a list. `written` is marked only
        // after a successful write, so a skipped malformed first entry does not
        // turn a later valid one into a stray append.
        let mut written: Vec<HeaderName> = Vec::new();

        for header in mutation.set {
            let Ok(name) = HeaderName::try_from(header.key.as_str()) else {
                tracing::warn!(key = %header.key, "a processor sent an invalid header name; skipping");
                continue;
            };
            let Ok(value) = HeaderValue::from_bytes(&header.value) else {
                tracing::warn!(key = %name, "a processor sent an invalid header value; skipping");
                continue;
            };

            self.remove.retain(|dropped| *dropped != name);
            let appending = written.contains(&name);
            match self.set.iter_mut().find(|(existing, _)| *existing == name) {
                Some((_, existing)) if appending => *existing = join(existing, &value),
                Some((_, existing)) => *existing = value,
                None => self.set.push((name.clone(), value)),
            }
            if !appending {
                written.push(name);
            }
        }

        for key in mutation.remove {
            let Ok(name) = HeaderName::try_from(key.as_str()) else {
                tracing::warn!(%key, "a processor asked to remove an invalid header name; skipping");
                continue;
            };
            self.set.retain(|(existing, _)| *existing != name);
            if !self.remove.contains(&name) {
                self.remove.push(name);
            }
        }
    }
}

/// Join two header values into one comma-separated field line.
fn join(first: &HeaderValue, second: &HeaderValue) -> HeaderValue {
    let mut bytes = first.as_bytes().to_vec();
    bytes.extend_from_slice(b", ");
    bytes.extend_from_slice(second.as_bytes());
    HeaderValue::from_bytes(&bytes).unwrap_or_else(|_| second.clone())
}

impl From<HeaderChanges> for crate::mutating_client::HeaderOverride {
    fn from(changes: HeaderChanges) -> Self {
        crate::mutating_client::HeaderOverride {
            set: changes.set,
            remove: changes.remove,
        }
    }
}

/// Values a processor attached to a call, for whoever is watching.
///
/// `McpRequestResult.metadata` is an arbitrary bag a processor fills in — the
/// classification it made, the rule it matched, the tenant it resolved. This
/// gateway puts it on the request's telemetry span, so a decision a guardrail
/// took in-band is visible in the trace afterwards rather than only in the
/// processor's own logs.
///
/// Upstream stashes the same bag for downstream CEL filters to read. Those do
/// not exist here, and a bag nothing can read is worse than no bag at all.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Annotations(Vec<(String, serde_json::Value)>);

/// How many keys one call may carry onto a span.
///
/// A processor is an external service, and an unbounded key count would let it
/// grow every span the gateway exports. Excess is dropped with a warning
/// rather than truncating silently.
const MAX_ANNOTATIONS: usize = 32;

/// How long one annotation value may be once rendered.
const MAX_ANNOTATION_BYTES: usize = 1024;

impl Annotations {
    /// Whether a processor sent anything.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The key/value pairs, in the order they were accepted.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &serde_json::Value)> {
        self.0.iter().map(|(k, v)| (k.as_str(), v))
    }

    /// Build a bag directly, for tests that need one without a processor.
    #[cfg(test)]
    pub(crate) fn for_test(value: serde_json::Value) -> Self {
        let serde_json::Value::Object(map) = value else {
            panic!("a metadata bag is an object");
        };
        Annotations(map.into_iter().collect())
    }

    /// Fold one processor's answer into the running set.
    ///
    /// Later processors win on a key collision, which is upstream's rule and
    /// the one that makes a chain read as a pipeline: the last thing to say
    /// something about a call is the most informed.
    fn merge(&mut self, metadata: prost_types::Struct) {
        for (key, value) in metadata.fields {
            let Some(value) = wire::proto_to_json(value) else {
                tracing::debug!(
                    key,
                    "a processor sent an annotation with no JSON form; skipping"
                );
                continue;
            };

            let value = match &value {
                serde_json::Value::String(text) if text.len() > MAX_ANNOTATION_BYTES => {
                    tracing::warn!(
                        key,
                        len = text.len(),
                        "a processor sent an oversized annotation; truncating"
                    );
                    let mut end = MAX_ANNOTATION_BYTES;
                    while !text.is_char_boundary(end) {
                        end -= 1;
                    }
                    serde_json::Value::String(text[..end].to_string())
                }
                _ => value,
            };

            let existing = self.0.iter().position(|(name, _)| name == &key);
            match existing {
                Some(at) => self.0[at].1 = value,
                None if self.0.len() < MAX_ANNOTATIONS => self.0.push((key, value)),
                None => {
                    tracing::warn!(
                        key,
                        max = MAX_ANNOTATIONS,
                        "a processor sent more annotations than a span will carry; dropping"
                    );
                }
            }
        }
    }
}

/// What a processor's request-phase answer amounts to.
#[derive(Debug, Clone, PartialEq)]
pub struct RequestDecision {
    /// Pass, a rewritten body, or a refusal.
    pub outcome: Outcome,
    /// Header changes to apply to the upstream call. Empty on a refusal:
    /// side effects from a request that never happens would surprise.
    pub headers: HeaderChanges,
    /// Values to put on the call's telemetry span. Empty on a refusal, for
    /// the same reason.
    pub annotations: Annotations,
}

/// JSON-RPC codes for refusals that have no standard MCP equivalent.
///
/// The application-defined server-error range is -32000 to -32099. `-32002` is
/// skipped because `rmcp` already assigns it to `RESOURCE_NOT_FOUND`, and
/// upstream skips it for the same reason.
const PERMISSION_DENIED: i32 = -32001;
const RESOURCE_EXHAUSTED: i32 = -32003;
const INVALID_REQUEST: i32 = -32600;
const INTERNAL_ERROR: i32 = -32603;

/// A route's compiled processor chain.
#[derive(Debug, Default)]
pub struct Guardrails {
    processors: Vec<Compiled>,
}

struct Compiled {
    config: Processor,
    endpoint: String,
    timeout: Duration,
    fail_open: bool,
    /// `metadata` expressions, compiled once at startup.
    metadata: Vec<(String, Program)>,
}

impl std::fmt::Debug for Compiled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Processor")
            .field("endpoint", &self.endpoint)
            .field("fail_open", &self.fail_open)
            .finish_non_exhaustive()
    }
}

impl Guardrails {
    /// Compile a route's `mcpGuardrails` policy.
    ///
    /// Processors naming `backend` or `service` rather than `host` are dropped
    /// rather than guessed at — this build has no backend registry to resolve
    /// them against. [`Guardrails::lint`] reports each one, so the drop is
    /// loud rather than silent.
    pub fn new(config: &McpGuardrails, at: &str) -> Result<Self, GuardrailsError> {
        let mut processors = Vec::new();

        for (i, processor) in config.processors.iter().enumerate() {
            let Some(host) = processor.host.as_deref() else {
                continue;
            };

            let endpoint = if host.contains("://") {
                host.to_string()
            } else {
                format!("http://{host}")
            };
            if endpoint.parse::<http::Uri>().is_err() {
                return Err(GuardrailsError::Host {
                    at: format!("{at}.processors[{i}]"),
                    host: host.to_string(),
                });
            }

            let mut metadata = Vec::with_capacity(processor.metadata.len());
            for (key, expression) in &processor.metadata {
                let program =
                    Program::compile(expression).map_err(|source| GuardrailsError::Metadata {
                        at: format!("{at}.processors[{i}].metadata.{key}"),
                        expression: expression.clone(),
                        source: Box::new(source),
                    })?;
                metadata.push((key.clone(), program));
            }

            processors.push(Compiled {
                config: processor.clone(),
                endpoint,
                timeout: processor
                    .timeout
                    .map(Duration::from)
                    .unwrap_or(DEFAULT_TIMEOUT),
                fail_open: matches!(processor.failure_mode, Some(FailureMode::FailOpen)),
                metadata,
            });
        }

        Ok(Guardrails { processors })
    }

    /// Whether any processor runs the request side of `method`.
    pub fn runs_request(&self, method: &str) -> bool {
        self.processors
            .iter()
            .any(|p| resolve(method, &p.config.methods).runs_request())
    }

    /// Whether any processor runs the response side of `method`.
    pub fn runs_response(&self, method: &str) -> bool {
        self.processors
            .iter()
            .any(|p| resolve(method, &p.config.methods).runs_response())
    }

    /// Whether this route carries any processors at all.
    pub fn is_empty(&self) -> bool {
        self.processors.is_empty()
    }

    /// Run the request side of the chain.
    ///
    /// `params` is the JSON-RPC `params` as raw JSON, or `None` for a method
    /// that has none. A mutation on a method with no params is discarded with
    /// a log line rather than treated as an error, matching upstream: for
    /// `tools/list` there is nothing to rewrite on the way in, and filtering a
    /// listing is response-phase work.
    pub async fn check_request(
        &self,
        call: CallContext<'_>,
        backends: &[String],
        params: Option<&[u8]>,
    ) -> RequestDecision {
        let method = call.method;
        let mut current = params.map(<[u8]>::to_vec);
        let mut outcome = Outcome::Pass;
        let mut headers = HeaderChanges::default();
        let mut annotations = Annotations::default();

        for processor in &self.processors {
            if !resolve(method, &processor.config.methods).runs_request() {
                continue;
            }

            let request = McpRequest {
                service_names: backends.to_vec(),
                method: method.to_string(),
                metadata_context: processor.metadata_context(call),
                // Each processor sees what the one before it produced, so a
                // chain composes rather than each link seeing the original.
                mcp_request: current.clone(),
                headers: collect_headers(processor, call.headers),
            };

            let result = match processor.call_request(request).await {
                Ok(result) => result,
                Err(status) => match processor.on_failure("checkRequest", &status) {
                    Some(outcome) => {
                        return RequestDecision {
                            outcome,
                            headers: HeaderChanges::default(),
                            annotations: Annotations::default(),
                        };
                    }
                    None => continue,
                },
            };

            match result.result {
                Some(request_result::Result::Pass(_)) | None => {}
                Some(request_result::Result::Mutated(body)) => {
                    if current.is_none() {
                        tracing::debug!(
                            method,
                            "a processor rewrote a request that carries no params; discarding"
                        );
                    } else {
                        current = Some(body.clone());
                        outcome = Outcome::Mutated(body);
                    }
                }
                // Header changes are dropped along with everything else on a
                // refusal: a request that never happens should leave no trace
                // on the one that replaces it.
                Some(request_result::Result::Error(error)) => {
                    return RequestDecision {
                        outcome: reject(error),
                        headers: HeaderChanges::default(),
                        annotations: Annotations::default(),
                    };
                }
            }

            // Honoured on pass and on a rewrite alike -- including for a
            // rewrite this gateway discarded, because the header change was
            // never about the body.
            if let Some(mutation) = result.header_mutation {
                headers.merge(mutation);
            }
            if let Some(metadata) = result.metadata {
                annotations.merge(metadata);
            }
        }

        RequestDecision {
            outcome,
            headers,
            annotations,
        }
    }

    /// Run the response side of the chain.
    ///
    /// `result` is the JSON-RPC `result` as raw JSON. An upstream error skips
    /// this hook entirely — there is no result to inspect, and asking a
    /// guardrail to approve a failure is not a question it can answer.
    pub async fn check_response(
        &self,
        call: CallContext<'_>,
        backends: &[String],
        result: &[u8],
    ) -> Outcome {
        let method = call.method;
        let mut current = result.to_vec();
        let mut outcome = Outcome::Pass;

        for processor in &self.processors {
            if !resolve(method, &processor.config.methods).runs_response() {
                continue;
            }

            let request = McpResponse {
                service_names: backends.to_vec(),
                method: method.to_string(),
                metadata_context: processor.metadata_context(call),
                mcp_response: current.clone(),
            };

            let answer = match processor.call_response(request).await {
                Ok(answer) => answer,
                Err(status) => match processor.on_failure("checkResponse", &status) {
                    Some(reject) => return reject,
                    None => continue,
                },
            };

            match answer.result {
                Some(response_result::Result::Pass(_)) | None => {}
                Some(response_result::Result::Mutated(body)) => {
                    current = body.clone();
                    outcome = Outcome::Mutated(body);
                }
                Some(response_result::Result::Error(error)) => return reject(error),
            }
        }

        outcome
    }
}

impl Compiled {
    /// Evaluate this processor's `metadata` expressions for one call.
    ///
    /// The context is `request` (method and headers), `jwt` (the verified
    /// token's claims) and `mcp` — the subject the call is about, in the same
    /// shape `mcpAuthorization.rules` uses.
    ///
    /// `mcp` is ours rather than upstream's: upstream evaluates these
    /// expressions against the HTTP request alone and reserves the MCP context
    /// for RBAC. Adding it is additive — no expression that worked before
    /// changes meaning — and it is what lets a processor be handed the prompt
    /// or resource it is being asked about without parsing `mcp_request`.
    ///
    /// An expression that fails to evaluate is skipped rather than failing the
    /// call: metadata is context for the processor, not a decision, and a
    /// missing `jwt` claim on an unauthenticated route should not take a
    /// guardrail offline. The processor sees the key absent and decides. That
    /// is also what makes `mcp.prompt.name` harmless on a `prompts/list`,
    /// which fans out and has no single subject.
    fn metadata_context(&self, call: CallContext<'_>) -> Option<prost_types::Struct> {
        if self.metadata.is_empty() {
            return None;
        }

        let mut context = Context::default();
        let request = serde_json::json!({
            "method": call.method,
            "headers": call
                .headers
                .iter()
                .map(|(name, value)| {
                    (
                        name.as_str().to_string(),
                        serde_json::Value::String(
                            String::from_utf8_lossy(value.as_bytes()).into_owned(),
                        ),
                    )
                })
                .collect::<serde_json::Map<_, _>>(),
        });
        context.add_variable("request", request).ok()?;
        if let Some(claims) = call.claims {
            context.add_variable("jwt", claims.clone()).ok()?;
        }
        // Bound exactly as `mcpAuthorization.rules` binds it, and by the same
        // type, so the two cannot drift into disagreeing about what a subject
        // is. Only the subject's own key exists, so `mcp.prompt.name` on a
        // tool call does not resolve and its key is dropped.
        if let (Some(subject), Some(target)) = (call.subject, call.target) {
            let mcp = serde_json::json!({
                subject.key(): { "name": subject.name(), "target": target },
            });
            context.add_variable("mcp", mcp).ok()?;
        }

        let fields = self
            .metadata
            .iter()
            .filter_map(|(key, program)| match program.execute(&context) {
                Ok(value) => {
                    wire::cel_to_json(value).map(|json| (key.clone(), wire::to_proto_value(json)))
                }
                Err(err) => {
                    tracing::debug!(key, %err, "a guardrail metadata expression failed; skipping");
                    None
                }
            })
            .collect();

        Some(prost_types::Struct { fields })
    }

    async fn call_request(&self, request: McpRequest) -> Result<McpRequestResult, tonic::Status> {
        let mut client = self.connect().await?;
        match tokio::time::timeout(self.timeout, client.check_request(request)).await {
            Ok(result) => result,
            Err(_) => Err(tonic::Status::deadline_exceeded("the processor timed out")),
        }
    }

    async fn call_response(
        &self,
        request: McpResponse,
    ) -> Result<McpResponseResult, tonic::Status> {
        let mut client = self.connect().await?;
        match tokio::time::timeout(self.timeout, client.check_response(request)).await {
            Ok(result) => result,
            Err(_) => Err(tonic::Status::deadline_exceeded("the processor timed out")),
        }
    }

    async fn connect(
        &self,
    ) -> Result<wire::client::ExtMcpClient<tonic::transport::Channel>, tonic::Status> {
        let channel = tonic::transport::Endpoint::from_shared(self.endpoint.clone())
            .map_err(|err| tonic::Status::internal(format!("bad processor endpoint: {err}")))?
            .connect_timeout(self.timeout)
            .connect()
            .await
            .map_err(|err| {
                tonic::Status::unavailable(format!("could not reach the processor: {err}"))
            })?;
        Ok(wire::client::ExtMcpClient::new(channel))
    }

    /// What to do when a processor did not answer usably.
    ///
    /// `Some(reject)` refuses the call; `None` carries on to the next
    /// processor, which is what `failOpen` means.
    fn on_failure(&self, rpc: &str, status: &tonic::Status) -> Option<Outcome> {
        tracing::warn!(
            endpoint = %self.endpoint,
            rpc,
            code = ?status.code(),
            message = %status.message(),
            fail_open = self.fail_open,
            "an mcpGuardrails processor could not be consulted"
        );
        if self.fail_open {
            return None;
        }
        Some(Outcome::Reject {
            code: INTERNAL_ERROR,
            message: format!("mcpGuardrails {rpc} failed: {}", status.message()),
            data: None,
        })
    }
}

/// Turn a processor's refusal into a JSON-RPC error.
fn reject(error: AuthorizationError) -> Outcome {
    use authorization_error::Code;
    let code = match Code::try_from(error.code).unwrap_or(Code::Unknown) {
        Code::PermissionDenied => PERMISSION_DENIED,
        Code::ResourceExhausted => RESOURCE_EXHAUSTED,
        Code::Invalid => INVALID_REQUEST,
        Code::Unknown => INTERNAL_ERROR,
    };
    let data = error.mcp_error.as_deref().and_then(|raw| {
        serde_json::from_slice(raw)
            .map_err(|err| {
                tracing::warn!(%err, "ignoring an unparseable mcp_error payload from a processor");
            })
            .ok()
    });
    Outcome::Reject {
        code,
        message: error.reason,
        data,
    }
}

/// What a processor's `metadata` expressions are evaluated against.
#[derive(Debug, Clone, Copy)]
pub struct CallContext<'a> {
    /// The JSON-RPC method.
    pub method: &'a str,
    /// Headers on the HTTP request carrying the call.
    pub headers: &'a HeaderMap,
    /// Claims from the verified token, when the route validated one.
    pub claims: Option<&'a serde_json::Value>,
    /// What the call is about, for the methods that are about one thing.
    ///
    /// `None` for the `*/list` methods, which fan out across every target and
    /// so have no single subject to name.
    pub subject: Option<Subject<'a>>,
    /// The target the subject belongs to, alongside `subject`.
    pub target: Option<&'a str>,
}

/// The headers a processor is shown.
fn collect_headers(processor: &Compiled, headers: &HeaderMap) -> Vec<McpHeader> {
    headers
        .iter()
        .filter(|(name, _)| processor.config.request_headers.allows(name.as_str()))
        .map(|(name, value)| McpHeader {
            key: name.as_str().to_string(),
            value: value.as_bytes().to_vec(),
        })
        .collect()
}

#[cfg(test)]
mod tests;
