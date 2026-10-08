//! A typed view of a request's `_meta`. 2026-07-28 clients send the
//! protocol version, their identity and their capabilities with *every*
//! request instead of once at connect.

use crate::capabilities::ClientCapabilities;
use crate::codec::{Obj, Wire};
use crate::notify::ProgressToken;
use crate::version::{Implementation, ProtocolVersion};
use crate::{Error, Result};
use rusty_json::Value;

const PROGRESS_TOKEN: &str = "progressToken";
const PROTOCOL_VERSION: &str = "io.modelcontextprotocol/protocolVersion";
const CLIENT_INFO: &str = "io.modelcontextprotocol/clientInfo";
const CLIENT_CAPABILITIES: &str = "io.modelcontextprotocol/clientCapabilities";
const LOG_LEVEL: &str = "io.modelcontextprotocol/logLevel";

/// The members of a request `_meta` this crate understands. Every other
/// key (trace context, vendor keys) rides along in `extra` untouched, so
/// decoding and re-encoding loses nothing.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RequestMeta {
    /// Ask the receiver to report progress under this token.
    pub progress_token: Option<ProgressToken>,
    /// The revision this request speaks (stateless protocol).
    pub protocol_version: Option<ProtocolVersion>,
    /// Who is calling (stateless protocol).
    pub client_info: Option<Implementation>,
    /// What the caller offers (stateless protocol).
    pub client_capabilities: Option<ClientCapabilities>,
    /// Minimum log level wanted, as sent.
    pub log_level: Option<String>,
    /// All other members, a JSON object.
    pub extra: Value,
}

impl RequestMeta {
    /// Metadata with nothing set.
    pub fn new() -> Self {
        Self {
            extra: Value::object(),
            ..Self::default()
        }
    }
}

fn known(key: &str) -> bool {
    matches!(
        key,
        PROGRESS_TOKEN | PROTOCOL_VERSION | CLIENT_INFO | CLIENT_CAPABILITIES | LOG_LEVEL
    )
}

impl Wire for RequestMeta {
    fn to_value(&self) -> Value {
        let mut out = match &self.extra {
            Value::Object(_) => self.extra.clone(),
            _ => Value::object(),
        };
        let known = Obj::new()
            .opt_value(
                PROGRESS_TOKEN,
                &self.progress_token.as_ref().map(Wire::to_value),
            )
            .opt_value(
                PROTOCOL_VERSION,
                &self.protocol_version.as_ref().map(Wire::to_value),
            )
            .opt_value(CLIENT_INFO, &self.client_info.as_ref().map(Wire::to_value))
            .opt_value(
                CLIENT_CAPABILITIES,
                &self.client_capabilities.as_ref().map(Wire::to_value),
            )
            .opt(LOG_LEVEL, self.log_level.clone())
            .done();
        if let Some(map) = known.as_object() {
            for (k, v) in map.iter() {
                out.insert(k.as_str(), v.clone());
            }
        }
        out
    }

    fn from_value(v: &Value) -> Result<Self> {
        const W: &str = "request _meta";
        let map = v
            .as_object()
            .ok_or_else(|| Error::decode(W, "not an object"))?;
        let mut me = RequestMeta::new();
        for (k, val) in map.iter() {
            match k.as_str() {
                PROGRESS_TOKEN => me.progress_token = Some(ProgressToken::from_value(val)?),
                PROTOCOL_VERSION => me.protocol_version = Some(ProtocolVersion::from_value(val)?),
                CLIENT_INFO => me.client_info = Some(Implementation::from_value(val)?),
                CLIENT_CAPABILITIES => {
                    me.client_capabilities = Some(ClientCapabilities::from_value(val)?)
                }
                LOG_LEVEL => {
                    me.log_level = Some(
                        val.as_str()
                            .map(String::from)
                            .ok_or_else(|| Error::decode(W, "log level is not a string"))?,
                    )
                }
                _ => {}
            }
            if !known(k) {
                me.extra.insert(k.as_str(), val.clone());
            }
        }
        Ok(me)
    }
}
