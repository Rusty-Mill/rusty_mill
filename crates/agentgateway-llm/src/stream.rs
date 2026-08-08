//! Translating Anthropic's streaming events into OpenAI chunks.
//!
//! The two streams are shaped differently enough that this cannot be a
//! per-event mapping:
//!
//! - Anthropic opens with `message_start` carrying the id, model and *prompt*
//!   token count, then emits `content_block_delta` events with the text, then
//!   `message_delta` with the stop reason and *completion* token count.
//! - OpenAI repeats the id and model on **every** chunk, sends the assistant
//!   role once in the first chunk, and terminates with a literal
//!   `data: [DONE]`.
//!
//! So the translator is a small state machine: it remembers the id and model
//! from `message_start` and replays them, and it tracks whether the role has
//! been announced. A stateless mapping would emit chunks with no id, or the
//! role on every one, and clients notice both.
//!
//! Usage is accumulated across the stream rather than read from one event,
//! because neither event carries both halves.
//!
//! # Tool calls need a second index
//!
//! Anthropic numbers *content blocks*, and text and tool calls share that
//! numbering. OpenAI numbers *tool calls*, and its text is not in the list at
//! all. So a response whose first block is text and whose second is a tool
//! call has that call at Anthropic index 1 and OpenAI index 0 — passing the
//! block index through would leave a client assembling arguments into a call
//! that never opened. The translator keeps its own count and a map from one to
//! the other.

use std::collections::BTreeMap;

use serde_json::{Value, json};

use crate::translate::{Usage, finish_reason};

/// Turns an Anthropic event stream into OpenAI chunks.
#[derive(Debug, Default)]
pub struct ChunkTranslator {
    id: String,
    model: String,
    role_sent: bool,
    created: u64,
    usage: Usage,
    /// Anthropic content-block index -> OpenAI tool-call index.
    ///
    /// See the module docs: the two number different things, and a client
    /// assembling arguments keys on OpenAI's.
    tool_indices: BTreeMap<u64, u64>,
}

impl ChunkTranslator {
    /// A translator stamping `created` on every chunk.
    ///
    /// OpenAI's `created` is per-response, and Anthropic sends no timestamp at
    /// all, so it is captured once by the caller rather than read from a clock
    /// per chunk — chunks of one response claiming different creation times
    /// would be a strange thing to hand a client.
    pub fn new(created: u64) -> Self {
        ChunkTranslator {
            created,
            ..Default::default()
        }
    }

    /// Token usage seen so far.
    pub fn usage(&self) -> Usage {
        self.usage
    }

    /// Translate one Anthropic event into zero or more OpenAI chunks.
    ///
    /// Zero is normal: `content_block_start`, `ping` and `content_block_stop`
    /// have no OpenAI counterpart, and inventing an empty chunk for each would
    /// just be noise on the wire.
    pub fn event(&mut self, event: &str, data: &Value) -> Vec<Value> {
        match event {
            "message_start" => {
                let message = data.get("message").unwrap_or(&Value::Null);
                self.id = message
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or("chatcmpl-unknown")
                    .to_string();
                self.model = message
                    .get("model")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                // The prompt count arrives here and nowhere else.
                if let Some(input) = message
                    .get("usage")
                    .and_then(|u| u.get("input_tokens"))
                    .and_then(Value::as_u64)
                {
                    self.usage.prompt = input;
                }
                Vec::new()
            }

            // A tool call opens here rather than in a delta, and it is the
            // only place its id and name are ever sent. A text block opening
            // has nothing to say.
            "content_block_start" => {
                let block = data.get("content_block").unwrap_or(&Value::Null);
                if block.get("type").and_then(Value::as_str) != Some("tool_use") {
                    return Vec::new();
                }
                let Some(block_index) = data.get("index").and_then(Value::as_u64) else {
                    return Vec::new();
                };

                let call_index = self.tool_indices.len() as u64;
                self.tool_indices.insert(block_index, call_index);

                let mut chunks = self.announce_role();
                // Arguments open empty and arrive as deltas, which is what an
                // OpenAI client expects: it concatenates them itself.
                chunks.push(self.chunk(
                    json!({"tool_calls": [{
                        "index": call_index,
                        "id": block.get("id").and_then(Value::as_str).unwrap_or_default(),
                        "type": "function",
                        "function": {
                            "name": block.get("name").and_then(Value::as_str).unwrap_or_default(),
                            "arguments": "",
                        },
                    }]}),
                    Value::Null,
                ));
                chunks
            }

            "content_block_delta" => {
                let delta = data.get("delta").unwrap_or(&Value::Null);

                // A tool call's arguments arrive as `input_json_delta`
                // fragments, which are not valid JSON on their own -- they are
                // pieces of a string the client concatenates.
                if let Some(partial) = delta.get("partial_json").and_then(Value::as_str) {
                    let Some(call_index) = data
                        .get("index")
                        .and_then(Value::as_u64)
                        .and_then(|block| self.tool_indices.get(&block).copied())
                    else {
                        // A fragment for a call that never opened would be
                        // assembled into the wrong one, so it is dropped.
                        return Vec::new();
                    };
                    let mut chunks = self.announce_role();
                    chunks.push(self.chunk(
                        json!({"tool_calls": [{
                            "index": call_index,
                            "function": {"arguments": partial},
                        }]}),
                        Value::Null,
                    ));
                    return chunks;
                }

                let Some(text) = delta.get("text").and_then(Value::as_str) else {
                    return Vec::new();
                };

                let mut chunks = self.announce_role();
                chunks.push(self.chunk(json!({"content": text}), Value::Null));
                chunks
            }

            "message_delta" => {
                if let Some(output) = data
                    .get("usage")
                    .and_then(|u| u.get("output_tokens"))
                    .and_then(Value::as_u64)
                {
                    self.usage.completion = output;
                }
                let reason = data
                    .get("delta")
                    .and_then(|d| d.get("stop_reason"))
                    .and_then(Value::as_str);
                // The final chunk carries an empty delta and the reason, which
                // is how an OpenAI client learns the turn is over.
                vec![self.chunk(json!({}), finish_reason(reason))]
            }

            // `message_stop`, `content_block_stop` and `ping` have nothing to
            // say in OpenAI's format. The `[DONE]` sentinel is the caller's to
            // emit once the stream ends, since it is not a chunk.
            _ => Vec::new(),
        }
    }

    /// The role chunk, if it has not been sent yet.
    ///
    /// OpenAI announces the assistant role once, in its own chunk, before any
    /// content -- and a response that opens with a tool call rather than text
    /// still has to announce it.
    fn announce_role(&mut self) -> Vec<Value> {
        if self.role_sent {
            return Vec::new();
        }
        self.role_sent = true;
        vec![self.chunk(json!({"role": "assistant"}), Value::Null)]
    }

    fn chunk(&self, delta: Value, finish: Value) -> Value {
        json!({
            "id": self.id,
            "object": "chat.completion.chunk",
            "created": self.created,
            "model": self.model,
            "choices": [{"index": 0, "delta": delta, "finish_reason": finish}],
        })
    }
}

/// A minimal server-sent-events parser.
///
/// Accumulates bytes and yields `(event, data)` pairs as complete events
/// arrive. It exists because the gateway sits between two SSE speakers and has
/// to re-frame the stream; pulling in a full SSE client for `event:`/`data:`
/// line handling would be more dependency than problem.
#[derive(Debug, Default)]
pub struct EventParser {
    buffer: String,
}

impl EventParser {
    /// Feed a chunk of the response body, returning any complete events.
    pub fn push(&mut self, bytes: &[u8]) -> Vec<(String, Value)> {
        self.buffer.push_str(&String::from_utf8_lossy(bytes));
        let mut events = Vec::new();

        // Events are separated by a blank line. Anything after the last one is
        // an incomplete event and stays buffered for the next chunk -- which
        // is the whole reason this holds state rather than parsing per chunk.
        while let Some(end) = find_separator(&self.buffer) {
            let (block, rest) = self.buffer.split_at(end.0);
            let block = block.to_string();
            self.buffer = rest[end.1..].to_string();

            if let Some(event) = parse_block(&block) {
                events.push(event);
            }
        }

        events
    }
}

/// Find the end of the first event and the length of its separator.
///
/// Both `\n\n` and `\r\n\r\n` appear in the wild, and a parser that knows only
/// one silently buffers a whole stream that never arrives.
fn find_separator(buffer: &str) -> Option<(usize, usize)> {
    let lf = buffer.find("\n\n").map(|at| (at, 2));
    let crlf = buffer.find("\r\n\r\n").map(|at| (at, 4));
    match (lf, crlf) {
        (Some(lf), Some(crlf)) => Some(if lf.0 <= crlf.0 { lf } else { crlf }),
        (Some(one), None) | (None, Some(one)) => Some(one),
        (None, None) => None,
    }
}

fn parse_block(block: &str) -> Option<(String, Value)> {
    let mut event = String::new();
    let mut data = String::new();

    for line in block.lines() {
        let line = line.trim_end_matches('\r');
        if let Some(value) = line.strip_prefix("event:") {
            event = value.trim().to_string();
        } else if let Some(value) = line.strip_prefix("data:") {
            // Multiple `data:` lines in one event concatenate with newlines,
            // per the SSE spec.
            if !data.is_empty() {
                data.push('\n');
            }
            data.push_str(value.trim());
        }
    }

    if event.is_empty() && data.is_empty() {
        return None;
    }
    let parsed = serde_json::from_str(&data).unwrap_or(Value::Null);
    Some((event, parsed))
}

/// Format a value as one SSE `data:` frame.
pub fn frame(value: &Value) -> String {
    format!("data: {value}\n\n")
}

/// The sentinel that ends an OpenAI stream.
pub const DONE: &str = "data: [DONE]\n\n";

#[cfg(test)]
mod tests {
    use super::*;

    fn translator() -> ChunkTranslator {
        ChunkTranslator::new(1_700_000_000)
    }

    #[test]
    fn message_start_emits_nothing_but_captures_identity() {
        // OpenAI has no "stream started" chunk; the id and model it carries
        // are replayed on every later chunk instead.
        let mut t = translator();
        let chunks = t.event(
            "message_start",
            &json!({"message": {"id": "msg_1", "model": "claude-sonnet-4",
                                "usage": {"input_tokens": 9}}}),
        );
        assert!(chunks.is_empty());
        assert_eq!(t.usage().prompt, 9);
    }

    #[test]
    fn the_first_delta_announces_the_role_once() {
        let mut t = translator();
        t.event(
            "message_start",
            &json!({"message": {"id": "msg_1", "model": "m"}}),
        );

        let first = t.event(
            "content_block_delta",
            &json!({"delta": {"type": "text_delta", "text": "Hi"}}),
        );
        assert_eq!(first.len(), 2, "a role chunk then a content chunk");
        assert_eq!(first[0]["choices"][0]["delta"]["role"], "assistant");
        assert_eq!(first[1]["choices"][0]["delta"]["content"], "Hi");

        let second = t.event(
            "content_block_delta",
            &json!({"delta": {"type": "text_delta", "text": " there"}}),
        );
        assert_eq!(second.len(), 1, "the role is announced only once");
        assert_eq!(second[0]["choices"][0]["delta"]["content"], " there");
    }

    #[test]
    fn every_chunk_repeats_the_id_and_model() {
        // Anthropic sends them once; OpenAI clients expect them on each chunk.
        let mut t = translator();
        t.event(
            "message_start",
            &json!({"message": {"id": "msg_1", "model": "claude-sonnet-4"}}),
        );
        let chunks = t.event("content_block_delta", &json!({"delta": {"text": "x"}}));
        for chunk in chunks {
            assert_eq!(chunk["id"], "msg_1");
            assert_eq!(chunk["model"], "claude-sonnet-4");
            assert_eq!(chunk["object"], "chat.completion.chunk");
            assert_eq!(chunk["created"], 1_700_000_000u64);
        }
    }

    #[test]
    fn message_delta_carries_the_finish_reason_and_completion_tokens() {
        let mut t = translator();
        t.event(
            "message_start",
            &json!({"message": {"id": "m", "model": "m", "usage": {"input_tokens": 4}}}),
        );
        let chunks = t.event(
            "message_delta",
            &json!({"delta": {"stop_reason": "max_tokens"}, "usage": {"output_tokens": 7}}),
        );

        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0]["choices"][0]["finish_reason"], "length");
        assert_eq!(
            t.usage(),
            Usage {
                prompt: 4,
                completion: 7
            }
        );
    }

    #[test]
    fn events_without_an_openai_counterpart_emit_nothing() {
        let mut t = translator();
        for event in [
            "ping",
            "content_block_start",
            "content_block_stop",
            "message_stop",
        ] {
            assert!(
                t.event(event, &json!({})).is_empty(),
                "{event} should not produce a chunk"
            );
        }
    }

    #[test]
    fn the_parser_yields_complete_events_only() {
        let mut parser = EventParser::default();

        // A partial event must stay buffered rather than parse as truncated
        // JSON -- this is the whole reason the parser holds state.
        assert!(
            parser
                .push(b"event: message_start\ndata: {\"mes")
                .is_empty()
        );

        let events = parser.push(b"sage\": {\"id\": \"m\"}}\n\n");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].0, "message_start");
        assert_eq!(events[0].1["message"]["id"], "m");
    }

    #[test]
    fn several_events_in_one_chunk_all_come_out() {
        let mut parser = EventParser::default();
        let events = parser.push(b"event: a\ndata: {\"n\": 1}\n\nevent: b\ndata: {\"n\": 2}\n\n");
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].0, "a");
        assert_eq!(events[1].1["n"], 2);
    }

    #[test]
    fn crlf_separated_events_parse_too() {
        // Both spellings appear in the wild, and knowing only one buffers a
        // whole stream that never arrives.
        let mut parser = EventParser::default();
        let events = parser.push(b"event: a\r\ndata: {\"n\": 1}\r\n\r\n");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].1["n"], 1);
    }

    #[test]
    fn a_frame_is_terminated_the_way_sse_requires() {
        assert_eq!(frame(&json!({"a": 1})), "data: {\"a\":1}\n\n");
        assert!(DONE.ends_with("\n\n"));
    }

    /// The events Anthropic sends for one tool call, in order.
    fn tool_call_events() -> Vec<(&'static str, Value)> {
        vec![
            (
                "message_start",
                json!({"message": {"id": "msg_1", "model": "claude-sonnet-4",
                                   "usage": {"input_tokens": 10}}}),
            ),
            (
                "content_block_start",
                json!({"index": 0, "content_block": {
                    "type": "tool_use", "id": "toolu_1", "name": "get_weather", "input": {}}}),
            ),
            (
                "content_block_delta",
                json!({"index": 0, "delta": {
                    "type": "input_json_delta", "partial_json": "{\"city\":"}}),
            ),
            (
                "content_block_delta",
                json!({"index": 0, "delta": {
                    "type": "input_json_delta", "partial_json": "\"Oslo\"}"}}),
            ),
            (
                "message_delta",
                json!({"delta": {"stop_reason": "tool_use"}, "usage": {"output_tokens": 5}}),
            ),
        ]
    }

    fn translate(events: Vec<(&str, Value)>) -> Vec<Value> {
        let mut translator = ChunkTranslator::new(1);
        events
            .iter()
            .flat_map(|(event, data)| translator.event(event, data))
            .collect()
    }

    #[test]
    fn a_streamed_tool_call_opens_with_its_id_and_name() {
        // The only place either is ever sent.
        let chunks = translate(tool_call_events());
        let opening = &chunks[1]["choices"][0]["delta"]["tool_calls"][0];
        assert_eq!(opening["index"], 0);
        assert_eq!(opening["id"], "toolu_1");
        assert_eq!(opening["type"], "function");
        assert_eq!(opening["function"]["name"], "get_weather");
        assert_eq!(
            opening["function"]["arguments"], "",
            "arguments open empty and arrive as deltas"
        );
    }

    #[test]
    fn the_role_is_announced_even_when_a_response_opens_with_a_tool_call() {
        let chunks = translate(tool_call_events());
        assert_eq!(chunks[0]["choices"][0]["delta"]["role"], "assistant");
        assert_eq!(
            chunks
                .iter()
                .filter(|c| c["choices"][0]["delta"].get("role").is_some())
                .count(),
            1,
            "and only once"
        );
    }

    #[test]
    fn argument_fragments_are_forwarded_for_the_client_to_concatenate() {
        // They are not valid JSON on their own; assembling them here would
        // mean holding the whole call back until it closed.
        let chunks = translate(tool_call_events());
        let arguments: String = chunks
            .iter()
            .filter_map(|c| {
                c["choices"][0]["delta"]["tool_calls"][0]["function"]["arguments"].as_str()
            })
            .collect();
        assert_eq!(arguments, r#"{"city":"Oslo"}"#);
    }

    #[test]
    fn a_tool_call_after_text_is_numbered_by_call_not_by_content_block() {
        // Anthropic numbers content blocks and text shares the numbering;
        // OpenAI numbers only tool calls. Passing the block index through
        // would leave a client assembling arguments into a call that never
        // opened.
        let chunks = translate(vec![
            (
                "message_start",
                json!({"message": {"id": "msg_1", "model": "m"}}),
            ),
            (
                "content_block_delta",
                json!({"index": 0, "delta": {"text": "Looking that up."}}),
            ),
            (
                "content_block_start",
                json!({"index": 1, "content_block": {
                    "type": "tool_use", "id": "toolu_1", "name": "f", "input": {}}}),
            ),
            (
                "content_block_delta",
                json!({"index": 1, "delta": {"partial_json": "{}"}}),
            ),
        ]);

        let calls: Vec<&Value> = chunks
            .iter()
            .filter(|c| c["choices"][0]["delta"].get("tool_calls").is_some())
            .collect();
        assert_eq!(calls.len(), 2);
        for call in calls {
            assert_eq!(
                call["choices"][0]["delta"]["tool_calls"][0]["index"], 0,
                "the first tool call is OpenAI index 0 even at block index 1"
            );
        }
    }

    #[test]
    fn two_tool_calls_get_their_own_indices() {
        let chunks = translate(vec![
            (
                "content_block_start",
                json!({"index": 0, "content_block": {
                    "type": "tool_use", "id": "a", "name": "one", "input": {}}}),
            ),
            (
                "content_block_start",
                json!({"index": 1, "content_block": {
                    "type": "tool_use", "id": "b", "name": "two", "input": {}}}),
            ),
            (
                "content_block_delta",
                json!({"index": 1, "delta": {"partial_json": "{}"}}),
            ),
        ]);

        let indices: Vec<u64> = chunks
            .iter()
            .filter_map(|c| c["choices"][0]["delta"]["tool_calls"][0]["index"].as_u64())
            .collect();
        assert_eq!(indices, vec![0, 1, 1]);
    }

    #[test]
    fn a_fragment_for_a_call_that_never_opened_is_dropped() {
        // Forwarding it would have the client assemble arguments into the
        // wrong call.
        let chunks = translate(vec![(
            "content_block_delta",
            json!({"index": 7, "delta": {"partial_json": "{}"}}),
        )]);
        assert!(chunks.is_empty(), "{chunks:?}");
    }

    #[test]
    fn a_tool_use_stop_reason_reaches_the_client_as_tool_calls() {
        let chunks = translate(tool_call_events());
        let last = chunks.last().expect("a final chunk");
        assert_eq!(last["choices"][0]["finish_reason"], "tool_calls");
    }
}
