//! Pagination and the members every list result shares.

use crate::codec::{array, decode_all, encode_all, object, opt_string, opt_value, Obj, Wire};
use crate::{Error, Result};
use rusty_json::Value;

/// How a result says it should be parsed (`resultType`). Open on purpose:
/// 2026-07-28 defines `complete`, `input_required` and `task`, and a peer
/// may send more.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResultType(pub String);

impl ResultType {
    /// An ordinary finished result.
    pub const COMPLETE: &'static str = "complete";
    /// The server needs input before it can finish.
    pub const INPUT_REQUIRED: &'static str = "input_required";
    /// The work continues as a task.
    pub const TASK: &'static str = "task";
}

/// Who may reuse a cached result (`cacheScope`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CacheScope {
    /// Any client or intermediary.
    Public,
    /// Only the requesting user's client.
    Private,
}

impl CacheScope {
    fn parse(v: &Value) -> Result<Self> {
        match v.as_str() {
            Some("public") => Ok(CacheScope::Public),
            Some("private") => Ok(CacheScope::Private),
            _ => Err(Error::decode("cacheScope", "not \"public\" or \"private\"")),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            CacheScope::Public => "public",
            CacheScope::Private => "private",
        }
    }
}

/// Parameters of every `*/list` request.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PaginatedParams {
    /// Opaque cursor from the previous page's `nextCursor`.
    pub cursor: Option<String>,
    /// Request `_meta`, kept as raw JSON.
    pub meta: Option<Value>,
}

impl Wire for PaginatedParams {
    fn to_value(&self) -> Value {
        Obj::new()
            .opt("cursor", self.cursor.clone())
            .opt_value("_meta", &self.meta)
            .done()
    }

    fn from_value(v: &Value) -> Result<Self> {
        object(v, "PaginatedParams")?;
        Ok(Self {
            cursor: opt_string(v, "PaginatedParams", "cursor")?,
            meta: opt_value(v, "_meta"),
        })
    }
}

/// The members every list result carries besides its items.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Paging {
    /// `resultType`.
    pub result_type: Option<ResultType>,
    /// Cursor for the next page; `None` on the last.
    pub next_cursor: Option<String>,
    /// 2026-07-28: how long the list stays fresh, in milliseconds.
    pub ttl_ms: Option<u64>,
    /// 2026-07-28: who may cache it.
    pub cache_scope: Option<CacheScope>,
    /// Result `_meta`, kept as raw JSON.
    pub meta: Option<Value>,
}

/// Read `ttlMs`; a negative value is clamped to 0 (immediately stale), as
/// the spec asks clients to do.
pub(crate) fn decode_ttl_ms(v: &Value, what: &'static str) -> Result<Option<u64>> {
    match v.get("ttlMs") {
        None | Some(Value::Null) => Ok(None),
        Some(n) => n
            .as_i64()
            .map(|t| Some(t.max(0).unsigned_abs()))
            .ok_or_else(|| Error::decode(what, "\"ttlMs\" is not an integer")),
    }
}

pub(crate) fn decode_cache_scope(v: &Value) -> Result<Option<CacheScope>> {
    match v.get("cacheScope") {
        None | Some(Value::Null) => Ok(None),
        Some(s) => CacheScope::parse(s).map(Some),
    }
}

impl CacheScope {
    pub(crate) fn encode(scope: Option<CacheScope>) -> Option<&'static str> {
        scope.map(CacheScope::as_str)
    }
}

impl Paging {
    /// Add these members to a list result under construction.
    pub(crate) fn encode(&self, obj: Obj) -> Obj {
        obj.opt("resultType", self.result_type.as_ref().map(|t| t.0.clone()))
            .opt("nextCursor", self.next_cursor.clone())
            .opt("ttlMs", self.ttl_ms)
            .opt("cacheScope", CacheScope::encode(self.cache_scope))
            .opt_value("_meta", &self.meta)
    }

    /// Read the shared members of a list result.
    pub(crate) fn decode(v: &Value, what: &'static str) -> Result<Self> {
        Ok(Self {
            result_type: opt_string(v, what, "resultType")?.map(ResultType),
            next_cursor: opt_string(v, what, "nextCursor")?,
            ttl_ms: decode_ttl_ms(v, what)?,
            cache_scope: decode_cache_scope(v)?,
            meta: opt_value(v, "_meta"),
        })
    }
}

/// A list result: `items` under `key`, plus the shared [`Paging`] members.
pub(crate) fn encode_list<T: Wire>(key: &str, items: &[T], paging: &Paging) -> Value {
    paging.encode(Obj::new().set(key, encode_all(items))).done()
}

/// The inverse of [`encode_list`].
pub(crate) fn decode_list<T: Wire>(
    v: &Value,
    what: &'static str,
    key: &'static str,
) -> Result<(Vec<T>, Paging)> {
    object(v, what)?;
    Ok((decode_all(array(v, what, key)?)?, Paging::decode(v, what)?))
}
