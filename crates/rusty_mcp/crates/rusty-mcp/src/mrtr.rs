//! Multi Round-Trip Requests (SEP-2322).
//!
//! When a tool needs something from the client mid-call — a confirmation, a
//! choice, a file list — 2026-07-28 does not let the server send it a request.
//! Instead the server *returns* an [`InputRequiredResult`], the client answers,
//! and the client **retries the original request** carrying its answers plus the
//! `requestState` string the server handed back.
//!
//! That is the whole difficulty: the protocol is stateless, so the retry is a
//! fresh call to a handler that remembers nothing. Everything the server needs
//! to pick up where it left off has to survive a round trip **through the
//! client**.
//!
//! # `requestState` is untrusted
//!
//! The client echoes it back verbatim, so a caller can change it. A server that
//! keeps meaningful data there — a table name, a user id, an amount — and reads
//! it back without verification has handed the client a way to rewrite the
//! server's own memory mid-operation.
//!
//! [`InputGate`] seals it with HMAC-SHA256 and refuses to open anything
//! tampered with. It also:
//!
//! - **binds the state to the tool that created it**, so a state sealed while
//!   confirming one operation cannot be replayed against another;
//! - **expires it**, so an answer cannot be replayed hours later;
//! - **counts rounds**, so a loop between server and client cannot run forever.
//!
//! # Example
//!
//! ```no_run
//! # use rmcp::model::{CallToolResponse, CallToolResult, ContentBlock, ErrorData};
//! # use rusty_mcp::mrtr::{InputGate, Turn};
//! # use serde::{Deserialize, Serialize};
//! #[derive(Serialize, Deserialize)]
//! struct Pending {
//!     table: String,
//! }
//!
//! # fn example(
//! #     gate: &InputGate<Pending>,
//! #     table: String,
//! #     request_state: Option<&str>,
//! #     responses: Option<&rmcp::model::InputResponses>,
//! # ) -> Result<CallToolResponse, ErrorData> {
//! match gate.turn("drop_table", request_state, responses)? {
//!     // First call: ask, and stash what we will need on the way back.
//!     Turn::Fresh => Ok(CallToolResponse::InputRequired(gate.ask(
//!         "drop_table",
//!         &Pending { table },
//!         InputGate::<Pending>::confirm("key", "Really drop the table?"),
//!     )?)),
//!
//!     // Retry: the state is ours, verified, and the answer is alongside it.
//!     Turn::Resumed { state, answers } => {
//!         if answers.accepted("key") {
//!             Ok(CallToolResult::success(vec![ContentBlock::text(
//!                 format!("dropped {}", state.table),
//!             )])
//!             .into())
//!         } else {
//!             Ok(CallToolResult::success(vec![ContentBlock::text("cancelled")]).into())
//!         }
//!     }
//! }
//! # }
//! ```
//!
//! # Scope
//!
//! MRTR carries sampling, elicitation and roots. Sampling and roots are
//! **deprecated** in this revision, so [`InputGate`] gives elicitation the
//! typed helpers and leaves the other two reachable through the raw
//! [`InputRequests`] map rather than encouraging them.

use std::{marker::PhantomData, time::Duration};

use rmcp::model::{
    ElicitRequest, ElicitRequestParams, ElicitResult, ElicitationSchema, ErrorData, InputRequest,
    InputRequests, InputRequiredResult, InputResponses, RequestStateCodec, RequestStateError,
    SealOptions,
};
use serde::{Serialize, de::DeserializeOwned};

/// How long a sealed state stays openable.
///
/// Long enough for a person to read a prompt and decide, short enough that an
/// abandoned exchange cannot be resumed much later.
const DEFAULT_TTL: Duration = Duration::from_secs(300);

/// How many times one call may bounce between server and client.
const DEFAULT_MAX_ROUNDS: usize = 5;

/// Where an incoming call sits in an MRTR exchange.
#[derive(Debug)]
pub enum Turn<S> {
    /// A first call, with nothing to resume from.
    Fresh,
    /// A retry carrying the client's answers and the state we sealed earlier.
    Resumed {
        /// The state this handler sealed on the previous round, verified.
        state: S,
        /// What the client answered.
        answers: Answers,
    },
}

/// Seals and opens the state a tool needs across an MRTR round trip.
///
/// One gate per state type. The signing key must outlive an exchange, so use a
/// stable per-deployment secret rather than a per-process random one — with
/// several instances behind a load balancer, the retry will not land on the
/// instance that issued the state.
pub struct InputGate<S> {
    codec: RequestStateCodec,
    ttl: Option<Duration>,
    max_rounds: usize,
    _marker: PhantomData<fn() -> S>,
}

// Hand-written so the state type need not be `Clone`; `PhantomData<fn() -> S>`
// means a gate never actually holds an `S`.
impl<S> Clone for InputGate<S> {
    fn clone(&self) -> Self {
        Self {
            codec: self.codec.clone(),
            ttl: self.ttl,
            max_rounds: self.max_rounds,
            _marker: PhantomData,
        }
    }
}

impl<S> std::fmt::Debug for InputGate<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InputGate")
            .field("ttl", &self.ttl)
            .field("max_rounds", &self.max_rounds)
            .finish_non_exhaustive()
    }
}

/// What actually gets sealed: the caller's state plus our own bookkeeping.
#[derive(Serialize, serde::Deserialize)]
struct Envelope<S> {
    /// Which round this is, so a runaway exchange terminates.
    round: usize,
    /// The caller's state.
    state: S,
}

impl<S> InputGate<S>
where
    S: Serialize + DeserializeOwned,
{
    /// A gate signing with `key`.
    ///
    /// Use at least 32 bytes of real entropy; the key is the only thing
    /// stopping a client from forging state.
    pub fn new(key: impl Into<Vec<u8>>) -> Self {
        Self {
            codec: RequestStateCodec::new_unchecked(key),
            ttl: Some(DEFAULT_TTL),
            max_rounds: DEFAULT_MAX_ROUNDS,
            _marker: PhantomData,
        }
    }

    /// How long a sealed state stays valid. `None` disables expiry.
    pub fn with_ttl(mut self, ttl: impl Into<Option<Duration>>) -> Self {
        self.ttl = ttl.into();
        self
    }

    /// How many rounds one exchange may take before the gate refuses.
    pub fn with_max_rounds(mut self, max_rounds: usize) -> Self {
        self.max_rounds = max_rounds.max(1);
        self
    }

    /// Classify an incoming call.
    ///
    /// `tool` must be the same name passed to [`InputGate::ask`]; it is
    /// authenticated, so a state sealed for one tool will not open for another.
    ///
    /// A call carrying answers but no state — or a state that fails
    /// verification, has expired, or belongs to a different tool — is an
    /// `InvalidParams` error rather than a silent fall back to [`Turn::Fresh`].
    /// Starting over would quietly discard whatever the client just answered.
    pub fn turn(
        &self,
        tool: &str,
        request_state: Option<&str>,
        responses: Option<&InputResponses>,
    ) -> Result<Turn<S>, ErrorData> {
        match (request_state, responses) {
            (None, None) => Ok(Turn::Fresh),

            // No answers yet: treat as a fresh start. A client may legitimately
            // re-send the original call without having answered.
            (Some(_), None) => Ok(Turn::Fresh),

            (None, Some(_)) => Err(ErrorData::invalid_params(
                "input responses arrived without the matching request state",
                None,
            )),

            (Some(sealed), Some(responses)) => {
                let envelope: Envelope<S> = self.open(tool, sealed)?;

                if envelope.round >= self.max_rounds {
                    return Err(ErrorData::invalid_params(
                        format!("this request exceeded the {} round limit", self.max_rounds),
                        None,
                    ));
                }

                Ok(Turn::Resumed {
                    state: envelope.state,
                    answers: Answers {
                        responses: responses.clone(),
                        round: envelope.round,
                    },
                })
            }
        }
    }

    /// Ask the client for input, sealing `state` for the retry.
    ///
    /// `tool` binds the state, and must match what [`InputGate::turn`] is given.
    pub fn ask(
        &self,
        tool: &str,
        state: &S,
        requests: InputRequests,
    ) -> Result<InputRequiredResult, ErrorData> {
        self.ask_at_round(tool, state, requests, 0)
    }

    /// Ask again from inside a resumed turn, carrying the round count forward.
    ///
    /// Use this rather than [`InputGate::ask`] when an answer leads to another
    /// question; restarting the count would defeat the round limit.
    pub fn ask_again(
        &self,
        tool: &str,
        state: &S,
        requests: InputRequests,
        answers: &Answers,
    ) -> Result<InputRequiredResult, ErrorData> {
        self.ask_at_round(tool, state, requests, answers.round + 1)
    }

    fn ask_at_round(
        &self,
        tool: &str,
        state: &S,
        requests: InputRequests,
        round: usize,
    ) -> Result<InputRequiredResult, ErrorData> {
        if round >= self.max_rounds {
            return Err(ErrorData::internal_error(
                format!("this request exceeded the {} round limit", self.max_rounds),
                None,
            ));
        }

        let envelope = Envelope { round, state };
        let mut options = SealOptions::new().associated_data(tool.as_bytes());
        if let Some(ttl) = self.ttl {
            options = options.ttl(ttl);
        }

        let sealed = self
            .codec
            .seal_json_with(&envelope, &options)
            .map_err(|err| {
                // Only a serialization failure of the caller's own state.
                ErrorData::internal_error(format!("could not seal request state: {err}"), None)
            })?;

        Ok(InputRequiredResult::new(Some(requests), Some(sealed)))
    }

    fn open(&self, tool: &str, sealed: &str) -> Result<Envelope<S>, ErrorData> {
        // Associated data, not options: the TTL travels inside the sealed value
        // and is checked on open.
        self.codec
            .open_json_with(sealed, tool.as_bytes())
            .map_err(|err| match err {
                RequestStateError::Expired => ErrorData::invalid_params(
                    "this request took too long to answer; start again",
                    None,
                ),
                // Tampering, forgery, or a state minted for a different tool all
                // land here. The client is told nothing that would help it
                // distinguish them.
                other => {
                    tracing::warn!(%other, tool, "rejected an unusable request state");
                    ErrorData::invalid_params("the request state is not valid", None)
                }
            })
    }

    /// A yes/no elicitation, keyed by `key`.
    ///
    /// A convenience for the common confirmation case; build [`InputRequests`]
    /// directly for anything richer.
    pub fn confirm(key: impl Into<String>, message: impl Into<String>) -> InputRequests {
        let schema = ElicitationSchema::builder()
            .required_bool_property("confirm", |b| b.description("Whether to proceed."))
            .build_unchecked();

        let mut requests = InputRequests::new();
        requests.insert(
            key.into(),
            InputRequest::Elicitation(ElicitRequest::new(
                ElicitRequestParams::FormElicitationParams {
                    meta: None,
                    message: message.into(),
                    requested_schema: schema,
                },
            )),
        );
        requests
    }
}

/// What the client sent back.
#[derive(Debug, Clone)]
pub struct Answers {
    responses: InputResponses,
    round: usize,
}

impl Answers {
    /// Which round produced these answers, counting from zero.
    pub fn round(&self) -> usize {
        self.round
    }

    /// Keys the client answered.
    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.responses.keys().map(String::as_str)
    }

    /// The raw JSON answer for `key`.
    pub fn raw(&self, key: &str) -> Option<&serde_json::Value> {
        self.responses.get(key)
    }

    /// Deserialize the answer for `key`.
    ///
    /// Answers are opaque JSON on the wire, so the type is the caller's
    /// assertion about what it asked for.
    pub fn get<T: DeserializeOwned>(&self, key: &str) -> Result<T, ErrorData> {
        let raw = self.raw(key).ok_or_else(|| {
            ErrorData::invalid_params(format!("no answer was provided for `{key}`"), None)
        })?;

        serde_json::from_value(raw.clone()).map_err(|err| {
            ErrorData::invalid_params(format!("the answer for `{key}` was unusable: {err}"), None)
        })
    }

    /// The elicitation result for `key`.
    pub fn elicitation(&self, key: &str) -> Result<ElicitResult, ErrorData> {
        self.get(key)
    }

    /// Whether `key` was answered with an accepted `confirm: true`.
    ///
    /// False for a decline, a cancel, a missing answer, or a malformed one —
    /// anything short of an explicit yes. Defaulting the other way would turn a
    /// dropped connection into consent.
    pub fn accepted(&self, key: &str) -> bool {
        let Ok(result) = self.elicitation(key) else {
            return false;
        };

        if result.action != rmcp::model::ElicitationAction::Accept {
            return false;
        }

        result
            .content
            .as_ref()
            .and_then(|content| content.get("confirm"))
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    const KEY: &[u8] = b"a-test-signing-key-of-sufficient-length";
    const TOOL: &str = "drop_table";

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    struct Pending {
        table: String,
    }

    fn gate() -> InputGate<Pending> {
        InputGate::new(KEY)
    }

    fn pending() -> Pending {
        Pending {
            table: "users".to_string(),
        }
    }

    fn answers(value: serde_json::Value) -> InputResponses {
        let mut responses = InputResponses::new();
        responses.insert("key".to_string(), value);
        responses
    }

    fn accepted() -> InputResponses {
        answers(serde_json::json!({ "action": "accept", "content": { "confirm": true } }))
    }

    fn sealed_state(gate: &InputGate<Pending>) -> String {
        gate.ask(
            TOOL,
            &pending(),
            InputGate::<Pending>::confirm("key", "sure?"),
        )
        .expect("seals")
        .request_state
        .expect("request state is present")
    }

    #[test]
    fn a_call_with_nothing_attached_is_fresh() {
        assert!(matches!(
            gate().turn(TOOL, None, None).expect("classifies"),
            Turn::Fresh
        ));
    }

    #[test]
    fn state_without_answers_is_fresh() {
        let gate = gate();
        let state = sealed_state(&gate);

        assert!(matches!(
            gate.turn(TOOL, Some(&state), None).expect("classifies"),
            Turn::Fresh
        ));
    }

    #[test]
    fn answers_without_state_are_rejected() {
        // Starting over would silently discard what the client just answered.
        assert!(gate().turn(TOOL, None, Some(&accepted())).is_err());
    }

    #[test]
    fn a_sealed_state_round_trips() {
        let gate = gate();
        let state = sealed_state(&gate);

        let Turn::Resumed { state, answers } = gate
            .turn(TOOL, Some(&state), Some(&accepted()))
            .expect("resumes")
        else {
            panic!("expected a resumed turn");
        };

        assert_eq!(state, pending());
        assert!(answers.accepted("key"));
        assert_eq!(answers.round(), 0);
    }

    #[test]
    fn a_tampered_state_is_rejected() {
        let gate = gate();
        let mut state = sealed_state(&gate);
        // Flip the last character of the tag.
        let last = state.pop().expect("non-empty");
        state.push(if last == 'A' { 'B' } else { 'A' });

        assert!(gate.turn(TOOL, Some(&state), Some(&accepted())).is_err());
    }

    #[test]
    fn a_state_sealed_for_another_tool_is_rejected() {
        // The confused-operation case: an answer meant for one confirmation
        // must not authorize a different one.
        let gate = gate();
        let state = sealed_state(&gate);

        assert!(
            gate.turn("drop_database", Some(&state), Some(&accepted()))
                .is_err()
        );
    }

    #[test]
    fn a_state_from_another_key_is_rejected() {
        let issuer = gate();
        let state = sealed_state(&issuer);

        let other: InputGate<Pending> =
            InputGate::new(b"a-completely-different-signing-key".to_vec());
        assert!(other.turn(TOOL, Some(&state), Some(&accepted())).is_err());
    }

    #[test]
    fn an_expired_state_is_rejected() {
        let gate = gate().with_ttl(Duration::from_nanos(1));
        let state = sealed_state(&gate);

        std::thread::sleep(Duration::from_millis(5));
        assert!(gate.turn(TOOL, Some(&state), Some(&accepted())).is_err());
    }

    #[test]
    fn rounds_are_carried_forward_and_bounded() {
        let gate = gate().with_max_rounds(2);

        // Round 0.
        let first = sealed_state(&gate);
        let Turn::Resumed { answers, .. } = gate
            .turn(TOOL, Some(&first), Some(&accepted()))
            .expect("resumes")
        else {
            panic!("expected resumed");
        };
        assert_eq!(answers.round(), 0);

        // Asking again advances to round 1, which is the last one allowed.
        let second = gate
            .ask_again(
                TOOL,
                &pending(),
                InputGate::<Pending>::confirm("key", "again?"),
                &answers,
            )
            .expect("asks again")
            .request_state
            .expect("state");

        let Turn::Resumed { answers, .. } = gate
            .turn(TOOL, Some(&second), Some(&accepted()))
            .expect("resumes")
        else {
            panic!("expected resumed");
        };
        assert_eq!(answers.round(), 1);

        // A third would exceed the limit.
        assert!(
            gate.ask_again(
                TOOL,
                &pending(),
                InputGate::<Pending>::confirm("key", "and again?"),
                &answers,
            )
            .is_err()
        );
    }

    #[test]
    fn only_an_explicit_yes_counts_as_accepted() {
        let gate = gate();
        let state = sealed_state(&gate);

        for (label, response) in [
            ("declined", serde_json::json!({ "action": "decline" })),
            ("cancelled", serde_json::json!({ "action": "cancel" })),
            (
                "accepted but false",
                serde_json::json!({ "action": "accept", "content": { "confirm": false } }),
            ),
            (
                "accepted with nothing",
                serde_json::json!({ "action": "accept" }),
            ),
            ("nonsense", serde_json::json!({ "not": "an elicit result" })),
        ] {
            let Turn::Resumed { answers, .. } = gate
                .turn(TOOL, Some(&state), Some(&answers(response)))
                .expect("resumes")
            else {
                panic!("expected resumed");
            };
            assert!(
                !answers.accepted("key"),
                "{label} must not count as consent"
            );
        }
    }

    #[test]
    fn a_missing_answer_is_not_consent() {
        let gate = gate();
        let state = sealed_state(&gate);

        let Turn::Resumed { answers, .. } = gate
            .turn(TOOL, Some(&state), Some(&accepted()))
            .expect("resumes")
        else {
            panic!("expected resumed");
        };

        assert!(!answers.accepted("a-key-nobody-answered"));
        assert!(
            answers
                .get::<ElicitResult>("a-key-nobody-answered")
                .is_err()
        );
    }

    #[test]
    fn the_confirm_helper_builds_one_elicitation() {
        let requests = InputGate::<Pending>::confirm("key", "Really?");
        assert_eq!(requests.len(), 1);
        assert!(matches!(
            requests.get("key"),
            Some(InputRequest::Elicitation(_))
        ));
    }
}
