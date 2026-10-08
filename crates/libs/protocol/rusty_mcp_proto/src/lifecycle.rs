//! The two handshakes: classic `initialize` (2024-11-05 to 2025-11-25) and
//! stateless `server/discover` (2026-07-28). A server answers whichever the
//! client opens with; a client tries `server/discover` first and falls back
//! to `initialize` on `-32601` or `-32022`.

use crate::capabilities::{ClientCapabilities, ServerCapabilities};
use crate::codec::{
    array, decode_all, encode_all, field, object, opt_string, opt_value, string, Obj, Wire,
};
use crate::page::{decode_cache_scope, CacheScope, ResultType};
use crate::rpc::{ErrorCode, ErrorData};
use crate::version::{Implementation, ProtocolVersion};
use crate::{Error, Result};
use rusty_json::Value;

/// Method names of the handshakes and liveness.
pub mod method {
    /// Classic handshake, request.
    pub const INITIALIZE: &str = "initialize";
    /// Classic handshake, the client's follow-up notification.
    pub const INITIALIZED: &str = "notifications/initialized";
    /// Liveness check, no parameters.
    pub const PING: &str = "ping";
    /// Stateless handshake: ask for versions and capabilities.
    pub const DISCOVER: &str = "server/discover";
}

/// Where a stateless server's identity travels in `DiscoverResult._meta`.
const SERVER_INFO_KEY: &str = "io.modelcontextprotocol/serverInfo";

/// Parameters of `initialize`.
#[derive(Clone, Debug, PartialEq)]
pub struct InitializeParams {
    /// The newest revision the client speaks.
    pub protocol_version: ProtocolVersion,
    /// What the client offers.
    pub capabilities: ClientCapabilities,
    /// Who the client is.
    pub client_info: Implementation,
    /// Request `_meta`, raw JSON.
    pub meta: Option<Value>,
}

impl Wire for InitializeParams {
    fn to_value(&self) -> Value {
        Obj::new()
            .set("protocolVersion", self.protocol_version.to_value())
            .set("capabilities", self.capabilities.to_value())
            .set("clientInfo", self.client_info.to_value())
            .opt_value("_meta", &self.meta)
            .done()
    }

    fn from_value(v: &Value) -> Result<Self> {
        const W: &str = "InitializeParams";
        object(v, W)?;
        Ok(Self {
            protocol_version: ProtocolVersion::from_value(field(v, W, "protocolVersion")?)?,
            capabilities: ClientCapabilities::from_value(field(v, W, "capabilities")?)?,
            client_info: Implementation::from_value(field(v, W, "clientInfo")?)?,
            meta: opt_value(v, "_meta"),
        })
    }
}

/// Result of `initialize`.
#[derive(Clone, Debug, PartialEq)]
pub struct InitializeResult {
    /// The revision the server picked.
    pub protocol_version: ProtocolVersion,
    /// What the server offers.
    pub capabilities: ServerCapabilities,
    /// Who the server is.
    pub server_info: Implementation,
    /// Guidance for using the server.
    pub instructions: Option<String>,
    /// Result `_meta`, raw JSON.
    pub meta: Option<Value>,
}

impl Wire for InitializeResult {
    fn to_value(&self) -> Value {
        Obj::new()
            .set("protocolVersion", self.protocol_version.to_value())
            .set("capabilities", self.capabilities.to_value())
            .set("serverInfo", self.server_info.to_value())
            .opt("instructions", self.instructions.clone())
            .opt_value("_meta", &self.meta)
            .done()
    }

    fn from_value(v: &Value) -> Result<Self> {
        const W: &str = "InitializeResult";
        object(v, W)?;
        Ok(Self {
            protocol_version: ProtocolVersion::from_value(field(v, W, "protocolVersion")?)?,
            capabilities: ServerCapabilities::from_value(field(v, W, "capabilities")?)?,
            server_info: Implementation::from_value(field(v, W, "serverInfo")?)?,
            instructions: opt_string(v, W, "instructions")?,
            meta: opt_value(v, "_meta"),
        })
    }
}

/// Parameters of `server/discover`: none besides `_meta`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DiscoverParams {
    /// Request `_meta`, raw JSON.
    pub meta: Option<Value>,
}

impl Wire for DiscoverParams {
    fn to_value(&self) -> Value {
        Obj::new().opt_value("_meta", &self.meta).done()
    }

    fn from_value(v: &Value) -> Result<Self> {
        object(v, "DiscoverParams")?;
        Ok(Self {
            meta: opt_value(v, "_meta"),
        })
    }
}

/// Result of `server/discover`.
#[derive(Clone, Debug, PartialEq)]
pub struct DiscoverResult {
    /// `resultType`, normally `complete`.
    pub result_type: ResultType,
    /// Every revision the server implements.
    pub supported_versions: Vec<ProtocolVersion>,
    /// What the server offers.
    pub capabilities: ServerCapabilities,
    /// Guidance for using the server.
    pub instructions: Option<String>,
    /// How long the answer stays fresh, in milliseconds.
    pub ttl_ms: u64,
    /// Who may cache it.
    pub cache_scope: CacheScope,
    /// Result `_meta`, raw JSON; carries the server's identity.
    pub meta: Option<Value>,
}

impl DiscoverResult {
    /// A non-cacheable, private answer.
    pub fn new(supported_versions: Vec<ProtocolVersion>, capabilities: ServerCapabilities) -> Self {
        Self {
            result_type: ResultType(ResultType::COMPLETE.to_owned()),
            supported_versions,
            capabilities,
            instructions: None,
            ttl_ms: 0,
            cache_scope: CacheScope::Private,
            meta: None,
        }
    }

    /// Record the server's identity in `_meta`.
    pub fn with_server_info(mut self, info: &Implementation) -> Self {
        let mut meta = self.meta.take().unwrap_or_else(Value::object);
        if meta.is_object() {
            meta.insert(SERVER_INFO_KEY, info.to_value());
        }
        self.meta = Some(meta);
        self
    }

    /// The server's identity from `_meta`, when present and well-formed.
    pub fn server_info(&self) -> Option<Implementation> {
        let info = self.meta.as_ref()?.get(SERVER_INFO_KEY)?;
        Implementation::from_value(info).ok()
    }
}

impl Wire for DiscoverResult {
    fn to_value(&self) -> Value {
        Obj::new()
            .set("resultType", self.result_type.0.as_str())
            .set("supportedVersions", encode_all(&self.supported_versions))
            .set("capabilities", self.capabilities.to_value())
            .opt("instructions", self.instructions.clone())
            .set("ttlMs", self.ttl_ms)
            .opt("cacheScope", CacheScope::encode(Some(self.cache_scope)))
            .opt_value("_meta", &self.meta)
            .done()
    }

    fn from_value(v: &Value) -> Result<Self> {
        const W: &str = "DiscoverResult";
        object(v, W)?;
        let ttl_ms = field(v, W, "ttlMs")?
            .as_i64()
            .map(|t| t.max(0).unsigned_abs())
            .ok_or_else(|| Error::decode(W, "\"ttlMs\" is not an integer"))?;
        Ok(Self {
            result_type: ResultType(string(v, W, "resultType")?),
            supported_versions: decode_all(array(v, W, "supportedVersions")?)?,
            capabilities: ServerCapabilities::from_value(field(v, W, "capabilities")?)?,
            instructions: opt_string(v, W, "instructions")?,
            ttl_ms,
            cache_scope: decode_cache_scope(v)?
                .ok_or_else(|| Error::decode(W, "missing \"cacheScope\""))?,
            meta: opt_value(v, "_meta"),
        })
    }
}

/// Classic negotiation (server side): answer with the requested revision
/// when it is served, else with the newest one served. The client then
/// decides whether it can live with that.
///
/// Returns `None` only when `supported` is empty.
pub fn negotiate_classic(
    requested: &ProtocolVersion,
    supported: &[ProtocolVersion],
) -> Option<ProtocolVersion> {
    if supported.contains(requested) {
        return Some(requested.clone());
    }
    supported.iter().max().cloned()
}

/// Stateless negotiation (client side): the newest revision both lists
/// contain, or `None` when they share none.
pub fn pick_common(
    preferred: &[ProtocolVersion],
    server_supported: &[ProtocolVersion],
) -> Option<ProtocolVersion> {
    preferred
        .iter()
        .filter(|v| server_supported.contains(v))
        .max()
        .cloned()
}

/// Whether a failed `server/discover` means "this server is classic":
/// method not found, or an unsupported protocol revision. Any other error
/// is a real failure and must not trigger the `initialize` fallback.
pub fn falls_back_to_initialize(error: &ErrorData) -> bool {
    error.code == ErrorCode::METHOD_NOT_FOUND
        || error.code == ErrorCode::UNSUPPORTED_PROTOCOL_VERSION
}
