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

            "content_block_delta" => {
                let Some(text) = data
                    .get("delta")
                    .and_then(|d| d.get("text"))
                    .and_then(Value::as_str)
                else {
                    return Vec::new();
                };

                let mut chunks = Vec::new();
                // OpenAI announces the assistant role once, in its own chunk,
                // before any content.
                if !self.role_sent {
                    self.role_sent = true;
                    chunks.push(self.chunk(json!({"role": "assistant"}), Value::Null));
                }
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

            // `message_stop`, `content_block_start`, `content_block_stop` and
            // `ping` have nothing to say in OpenAI's format. The `[DONE]`
            // sentinel is the caller's to emit once the stream ends, since it
            // is not a chunk.
            _ => Vec::new(),
        }
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
}
