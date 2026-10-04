//! [`ChatModel`] adapters (ADR-0005 §8).
//!
//! - [`OpenAiModel`]: any OpenAI-compatible `/chat/completions` endpoint
//!   over plain HTTP, such as a local Ollama. Token usage comes from the
//!   response's `usage` field; a response without it is an error, because
//!   an unmetered call would defeat the budget.
//! - [`ScriptedModel`]: canned replies with fixed token costs, for tests and
//!   CI, which have no model.

use std::cell::Cell;
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use rsi_core::{ChatModel, Completion, Message, ModelId};
use rusty_http::body::response_framing;
use rusty_http::head::RequestHead;
use rusty_http::sync::SyncTransport;
use rusty_http::{HeaderMap, Method, Url, Version};
use rusty_json::Value;

use crate::error::RuntimeError;

/// The largest response head accepted.
const MAX_HEAD_BYTES: usize = 64 * 1024;
/// The largest response body accepted.
const MAX_BODY_BYTES: u64 = 16 << 20;

/// An OpenAI-compatible chat endpoint over plain HTTP.
pub struct OpenAiModel {
    id: ModelId,
    endpoint: Url,
    api_key: Option<String>,
    timeout: Duration,
}

impl std::fmt::Debug for OpenAiModel {
    // Hand-written so the API key can never reach a log.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenAiModel")
            .field("id", &self.id)
            .field("endpoint", &self.endpoint.absolute_form())
            .field("api_key", &self.api_key.as_ref().map(|_| "<redacted>"))
            .field("timeout", &self.timeout)
            .finish()
    }
}

impl OpenAiModel {
    /// A client for model `id` at `base_url` (for example
    /// `http://127.0.0.1:11434/v1`), sending `api_key` as a bearer token
    /// when set, and giving up on a call after `timeout`.
    ///
    /// # Errors
    /// [`RuntimeError::Model`] for an unparsable URL or an `https` one:
    /// TLS is not wired in yet, so a remote endpoint needs a local proxy.
    pub fn new(
        id: ModelId,
        base_url: &str,
        api_key: Option<String>,
        timeout: Duration,
    ) -> Result<Self, RuntimeError> {
        let mut endpoint = Url::parse(base_url)
            .map_err(|e| RuntimeError::Model(format!("endpoint {base_url}: {e}")))?;
        if endpoint.scheme != "http" {
            return Err(RuntimeError::Model(format!(
                "endpoint {base_url}: only http:// is supported"
            )));
        }
        endpoint.path = format!("{}/chat/completions", endpoint.path.trim_end_matches('/'));
        Ok(Self {
            id,
            endpoint,
            api_key,
            timeout,
        })
    }

    fn post(&self, body: &[u8]) -> Result<Vec<u8>, RuntimeError> {
        let fail = |what: String| RuntimeError::Model(format!("{what} ({})", self.endpoint.host));
        let address = (self.endpoint.host.as_str(), self.endpoint.port)
            .to_socket_addrs()
            .map_err(|e| fail(format!("resolving: {e}")))?
            .next()
            .ok_or_else(|| fail("resolving: no address".into()))?;
        let stream = TcpStream::connect_timeout(&address, self.timeout)
            .map_err(|e| fail(format!("connecting: {e}")))?;
        stream
            .set_read_timeout(Some(self.timeout))
            .and_then(|()| stream.set_write_timeout(Some(self.timeout)))
            .map_err(|e| fail(format!("setting timeouts: {e}")))?;
        let mut headers = HeaderMap::new();
        let header = |headers: &mut HeaderMap, name: &str, value: &str| {
            headers
                .insert(name, value)
                .map_err(|e| fail(format!("header {name}: {e}")))
        };
        header(&mut headers, "Host", &self.endpoint.host_header())?;
        header(&mut headers, "Content-Type", "application/json")?;
        header(&mut headers, "Content-Length", &body.len().to_string())?;
        header(&mut headers, "Connection", "close")?;
        if let Some(key) = &self.api_key {
            headers
                .insert_sensitive("Authorization", &format!("Bearer {key}"))
                .map_err(|_| fail("the API key is not a valid header value".into()))?;
        }
        let head = RequestHead {
            method: Method::Post,
            target: self.endpoint.request_target(),
            version: Version::Http11,
            headers,
        };
        let mut transport = SyncTransport::new(stream);
        transport
            .write_request_head(&head)
            .and_then(|()| transport.write_body(body))
            .map_err(|e| fail(format!("sending: {e}")))?;
        let response = transport
            .read_response_head(MAX_HEAD_BYTES)
            .map_err(|e| fail(format!("reading the response: {e}")))?;
        let framing = response_framing(&response.headers, &Method::Post, response.status)
            .map_err(|e| fail(format!("response framing: {e}")))?;
        let body = match framing {
            rusty_http::body::Framing::ContentLength(len) => {
                transport.read_content_length_body(len, MAX_BODY_BYTES)
            }
            rusty_http::body::Framing::Close => transport.read_close_delimited_body(MAX_BODY_BYTES),
            other => transport.read_body(other),
        }
        .map_err(|e| fail(format!("reading the body: {e}")))?;
        if !response.status.is_success() {
            let text = String::from_utf8_lossy(&body);
            let preview: String = text.chars().take(300).collect();
            return Err(fail(format!(
                "HTTP {}: {preview}",
                response.status.as_u16()
            )));
        }
        Ok(body)
    }
}

/// The JSON body of a chat-completion request.
fn request_body(model: &ModelId, messages: &[Message], max_tokens: u64) -> String {
    let mut list = Value::array();
    for message in messages {
        let mut item = Value::object();
        item.insert("role", message.role.as_str());
        item.insert("content", message.content.as_str());
        list.push(item);
    }
    let mut body = Value::object();
    body.insert("model", model.as_str());
    body.insert("messages", list);
    body.insert("max_tokens", max_tokens);
    body.insert("stream", false);
    body.to_json_string()
}

/// The reply text and token usage of a chat-completion response.
fn parse_completion(body: &[u8]) -> Result<Completion, RuntimeError> {
    let bad = |what: &str| RuntimeError::Model(format!("unexpected response: {what}"));
    let text = std::str::from_utf8(body).map_err(|_| bad("not UTF-8"))?;
    let json = Value::parse(text).map_err(|e| bad(&e.to_string()))?;
    let reply = json
        .pointer("/choices/0/message/content")
        .and_then(Value::as_str)
        .ok_or_else(|| bad("no choices[0].message.content"))?;
    let tokens = |key: &str| {
        json.pointer(&format!("/usage/{key}"))
            .and_then(Value::as_u64)
            .ok_or_else(|| bad(&format!("no usage.{key}; refusing an unmetered call")))
    };
    Ok(Completion {
        text: reply.to_owned(),
        prompt_tokens: tokens("prompt_tokens")?,
        completion_tokens: tokens("completion_tokens")?,
    })
}

impl ChatModel for OpenAiModel {
    type Error = RuntimeError;

    fn id(&self) -> &ModelId {
        &self.id
    }

    fn complete(&self, messages: &[Message], max_tokens: u64) -> Result<Completion, RuntimeError> {
        let body = request_body(&self.id, messages, max_tokens);
        parse_completion(&self.post(body.as_bytes())?)
    }
}

/// Canned replies, cycled in order, each charged a fixed token cost.
#[derive(Debug)]
pub struct ScriptedModel {
    id: ModelId,
    replies: Vec<String>,
    prompt_tokens: u64,
    completion_tokens: u64,
    calls: Cell<usize>,
}

impl ScriptedModel {
    /// A model that answers with `replies` in turn, charging
    /// `prompt_tokens` plus `completion_tokens` (capped at the call's
    /// `max_tokens`) per call.
    #[must_use]
    pub const fn new(
        id: ModelId,
        replies: Vec<String>,
        prompt_tokens: u64,
        completion_tokens: u64,
    ) -> Self {
        Self {
            id,
            replies,
            prompt_tokens,
            completion_tokens,
            calls: Cell::new(0),
        }
    }

    /// Calls answered so far.
    #[must_use]
    pub fn calls(&self) -> usize {
        self.calls.get()
    }
}

impl ChatModel for ScriptedModel {
    type Error = RuntimeError;

    fn id(&self) -> &ModelId {
        &self.id
    }

    fn complete(&self, _: &[Message], max_tokens: u64) -> Result<Completion, RuntimeError> {
        let call = self.calls.get();
        let text = self
            .replies
            .get(call % self.replies.len().max(1))
            .ok_or_else(|| RuntimeError::Model("the script has no replies".into()))?;
        self.calls.set(call + 1);
        Ok(Completion {
            text: text.clone(),
            prompt_tokens: self.prompt_tokens,
            completion_tokens: self.completion_tokens.min(max_tokens),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;

    use rsi_core::Role;

    use super::*;

    fn id() -> ModelId {
        ModelId::parse("qwen2.5-coder:7b").expect("valid")
    }

    fn messages() -> Vec<Message> {
        vec![
            Message {
                role: Role::System,
                content: "be \"brief\"".into(),
            },
            Message {
                role: Role::User,
                content: "hi\n".into(),
            },
        ]
    }

    #[test]
    fn builds_an_openai_request_body() {
        let body = Value::parse(&request_body(&id(), &messages(), 77)).expect("JSON");
        assert_eq!(
            body.pointer("/model").and_then(Value::as_str),
            Some("qwen2.5-coder:7b")
        );
        assert_eq!(
            body.pointer("/messages/0/content").and_then(Value::as_str),
            Some("be \"brief\"")
        );
        assert_eq!(
            body.pointer("/messages/1/role").and_then(Value::as_str),
            Some("user")
        );
        assert_eq!(
            body.pointer("/max_tokens").and_then(Value::as_u64),
            Some(77)
        );
    }

    #[test]
    fn parses_completions_and_refuses_unmetered_ones() {
        let ok = br#"{"choices":[{"message":{"role":"assistant","content":"hey"}}],
                     "usage":{"prompt_tokens":12,"completion_tokens":3}}"#;
        assert_eq!(
            parse_completion(ok).ok(),
            Some(Completion {
                text: "hey".into(),
                prompt_tokens: 12,
                completion_tokens: 3
            })
        );
        let unmetered = br#"{"choices":[{"message":{"content":"hey"}}]}"#;
        assert!(parse_completion(unmetered).is_err());
        assert!(parse_completion(b"{}").is_err());
        assert!(parse_completion(b"not json").is_err());
    }

    #[test]
    fn refuses_https_and_redacts_the_key() {
        let https = OpenAiModel::new(id(), "https://api.example/v1", None, Duration::from_secs(1));
        assert!(matches!(https, Err(RuntimeError::Model(_))));
        let model = OpenAiModel::new(
            id(),
            "http://127.0.0.1:1/v1/",
            Some("sk-secret".into()),
            Duration::from_secs(1),
        )
        .expect("valid");
        let debug = format!("{model:?}");
        assert!(!debug.contains("sk-secret"), "{debug}");
        assert!(debug.contains("/v1/chat/completions"), "{debug}");
    }

    /// One HTTP exchange against a loopback server playing the endpoint.
    #[test]
    fn talks_to_an_openai_compatible_server() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().expect("accept");
            let mut reader = BufReader::new(stream);
            let mut head = String::new();
            let mut length = 0;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).expect("line");
                if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    length = value.trim().parse().expect("length");
                }
                head.push_str(&line);
                if line == "\r\n" {
                    break;
                }
            }
            let mut body = vec![0u8; length];
            reader.read_exact(&mut body).expect("body");
            let reply = r#"{"choices":[{"message":{"content":"pong"}}],"usage":{"prompt_tokens":5,"completion_tokens":1}}"#;
            write!(
                reader.get_mut(),
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{reply}",
                reply.len()
            )
            .expect("reply");
            (head, String::from_utf8(body).expect("utf8"))
        });
        let model = OpenAiModel::new(
            id(),
            &format!("http://127.0.0.1:{port}/v1"),
            Some("sk-test".into()),
            Duration::from_secs(5),
        )
        .expect("valid");
        let completion = model.complete(&messages(), 9).expect("completes");
        assert_eq!(completion.text, "pong");
        assert_eq!(completion.prompt_tokens + completion.completion_tokens, 6);
        let (head, body) = server.join().expect("server");
        assert!(
            head.starts_with("POST /v1/chat/completions HTTP/1.1\r\n"),
            "{head}"
        );
        assert!(head.contains("Authorization: Bearer sk-test"), "{head}");
        assert!(body.contains("\"max_tokens\":9"), "{body}");
    }

    #[test]
    fn an_unreachable_endpoint_is_a_model_error() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        drop(listener);
        let model = OpenAiModel::new(
            id(),
            &format!("http://127.0.0.1:{port}/v1"),
            None,
            Duration::from_secs(2),
        )
        .expect("valid");
        assert!(matches!(
            model.complete(&messages(), 1),
            Err(RuntimeError::Model(_))
        ));
    }

    #[test]
    fn scripted_replies_cycle_and_respect_max_tokens() {
        let model = ScriptedModel::new(id(), vec!["a".into(), "b".into()], 10, 50);
        let texts: Vec<_> = (0..3)
            .map(|_| model.complete(&messages(), 20).expect("reply"))
            .collect();
        assert_eq!(texts[0].text, "a");
        assert_eq!(texts[2].text, "a");
        assert_eq!(texts[1].completion_tokens, 20);
        assert_eq!(model.calls(), 3);
        let empty = ScriptedModel::new(id(), vec![], 1, 1);
        assert!(empty.complete(&messages(), 1).is_err());
    }
}
