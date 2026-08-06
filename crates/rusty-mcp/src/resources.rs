//! A registry for MCP resources.
//!
//! Tools and prompts each get a router from `rmcp`; resources do not, so a
//! server would otherwise hand-write `resources/list`, `resources/read` and
//! `resources/templates/list` along with their cache hints and not-found
//! handling. [`ResourceRegistry`] is that router.
//!
//! ```no_run
//! use rmcp::model::Resource;
//! use rusty_mcp::resources::ResourceRegistry;
//!
//! let resources = ResourceRegistry::new()
//!     .with_text(
//!         Resource::new("config://app", "app-config").with_mime_type("application/json"),
//!         r#"{"theme":"dark"}"#,
//!     )
//!     .with_reader(
//!         Resource::new("status://health", "health"),
//!         |_req| async move { Ok(vec![rmcp::model::ResourceContents::text("ok", "status://health")]) },
//!     );
//! # let _ = resources;
//! ```
//!
//! Then wire it into the handler:
//!
//! ```ignore
//! impl ServerHandler for MyServer {
//!     fn get_info(&self) -> ServerInfo {
//!         rusty_mcp::server_info("my-server", "0.1.0",
//!             ServerCapabilities::builder().enable_tools().enable_resources().build())
//!     }
//!     rusty_mcp::forward_resource_methods!(resources);
//! }
//! ```

use std::{collections::BTreeMap, future::Future, pin::Pin, sync::Arc};

use rmcp::model::{
    CacheScope, ErrorData, ListResourceTemplatesResult, ListResourcesResult, ReadResourceResult,
    Resource, ResourceContents, ResourceTemplate,
};

/// Future returned by a resource reader.
pub type ReadFuture =
    Pin<Box<dyn Future<Output = Result<Vec<ResourceContents>, ErrorData>> + Send>>;

type ReadFn = Arc<dyn Fn(ReadRequest) -> ReadFuture + Send + Sync>;

/// What a reader is asked for.
#[derive(Debug, Clone)]
pub struct ReadRequest {
    /// The URI the client asked to read.
    pub uri: String,
    /// Variables captured from a URI template, percent-decoded.
    ///
    /// Empty for a concrete resource.
    pub params: BTreeMap<String, String>,
}

impl ReadRequest {
    /// A captured template variable.
    pub fn param(&self, name: &str) -> Option<&str> {
        self.params.get(name).map(String::as_str)
    }
}

enum Source {
    /// Content fixed at registration.
    Static(Vec<ResourceContents>),
    /// Content produced on demand.
    Dynamic(ReadFn),
}

struct Entry {
    resource: Resource,
    source: Source,
}

struct TemplateEntry {
    template: ResourceTemplate,
    parsed: UriTemplate,
    read: ReadFn,
}

/// Serves `resources/list`, `resources/read` and `resources/templates/list`.
///
/// Cheap to clone; entries sit behind an `Arc`, which matters under Streamable
/// HTTP where a handler is built per request.
#[derive(Clone)]
pub struct ResourceRegistry {
    inner: Arc<RegistryInner>,
}

struct RegistryInner {
    entries: Vec<Entry>,
    templates: Vec<TemplateEntry>,
    ttl_ms: Option<u64>,
    cache_scope: CacheScope,
    page_size: usize,
}

impl std::fmt::Debug for ResourceRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResourceRegistry")
            .field("resources", &self.inner.entries.len())
            .field("templates", &self.inner.templates.len())
            .field("ttl_ms", &self.inner.ttl_ms)
            .field("cache_scope", &self.inner.cache_scope)
            .field("page_size", &self.inner.page_size)
            .finish()
    }
}

impl Default for ResourceRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Builder state, so `with_*` can stay chainable without `Arc` churn.
#[derive(Default)]
struct Building {
    entries: Vec<Entry>,
    templates: Vec<TemplateEntry>,
    ttl_ms: Option<u64>,
    cache_scope: Option<CacheScope>,
    page_size: Option<usize>,
}

/// Entries returned per page when nothing else is configured.
///
/// Matches the cap the spec puts on completion results, for no deeper reason
/// than that a hundred of anything is a reasonable thing to put in one response.
pub const DEFAULT_PAGE_SIZE: usize = 100;

impl ResourceRegistry {
    /// An empty registry.
    pub fn new() -> Self {
        Self::from_building(Building::default())
    }

    fn from_building(b: Building) -> Self {
        Self {
            inner: Arc::new(RegistryInner {
                entries: b.entries,
                templates: b.templates,
                // Five minutes: long enough to spare clients constant polling,
                // short enough that an edit shows up without a restart.
                ttl_ms: Some(b.ttl_ms.unwrap_or(5 * 60 * 1_000)),
                // Private by default. Resource contents are frequently
                // user-specific, and a shared cache serving one user's data to
                // another is a worse failure than a cache miss.
                cache_scope: b.cache_scope.unwrap_or(CacheScope::Private),
                page_size: b.page_size.unwrap_or(DEFAULT_PAGE_SIZE),
            }),
        }
    }

    /// Take the current contents apart so a `with_*` method can extend them.
    ///
    /// Callers hold the only handle while building, so this never clones.
    fn into_building(self) -> Building {
        let inner = Arc::try_unwrap(self.inner).unwrap_or_else(|shared| RegistryInner {
            entries: Vec::new(),
            templates: Vec::new(),
            ttl_ms: shared.ttl_ms,
            cache_scope: shared.cache_scope,
            page_size: shared.page_size,
        });

        Building {
            entries: inner.entries,
            templates: inner.templates,
            ttl_ms: inner.ttl_ms,
            cache_scope: Some(inner.cache_scope),
            page_size: Some(inner.page_size),
        }
    }

    /// Register a resource whose text content is fixed.
    pub fn with_text(self, resource: Resource, text: impl Into<String>) -> Self {
        let uri = resource.uri.clone();
        self.with_contents(resource, vec![ResourceContents::text(text, uri)])
    }

    /// Register a resource with pre-built contents.
    pub fn with_contents(self, resource: Resource, contents: Vec<ResourceContents>) -> Self {
        let mut b = self.into_building();
        b.entries.push(Entry {
            resource,
            source: Source::Static(contents),
        });
        Self::from_building(b)
    }

    /// Register a resource read on demand.
    pub fn with_reader<F, Fut>(self, resource: Resource, reader: F) -> Self
    where
        F: Fn(ReadRequest) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Vec<ResourceContents>, ErrorData>> + Send + 'static,
    {
        let mut b = self.into_building();
        b.entries.push(Entry {
            resource,
            source: Source::Dynamic(wrap(reader)),
        });
        Self::from_building(b)
    }

    /// Register a templated family of resources.
    ///
    /// The template is RFC 6570 level 1 — `{var}` placeholders only. A variable
    /// never matches across `/`, which keeps `file:///logs/{name}` from
    /// capturing `../../etc/passwd` as a name.
    pub fn with_template<F, Fut>(self, template: ResourceTemplate, reader: F) -> Self
    where
        F: Fn(ReadRequest) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Vec<ResourceContents>, ErrorData>> + Send + 'static,
    {
        let parsed = UriTemplate::parse(&template.uri_template);
        let mut b = self.into_building();
        b.templates.push(TemplateEntry {
            template,
            parsed,
            read: wrap(reader),
        });
        Self::from_building(b)
    }

    /// Freshness hint published on list and read results. `None` means no hint.
    pub fn with_ttl_ms(self, ttl_ms: impl Into<Option<u64>>) -> Self {
        let mut b = self.into_building();
        b.ttl_ms = ttl_ms.into();
        Self::from_building(b)
    }

    /// Whether shared caches may store these responses.
    ///
    /// Leave this `Private` unless the contents are the same for every caller.
    pub fn with_cache_scope(self, scope: CacheScope) -> Self {
        let mut b = self.into_building();
        b.cache_scope = Some(scope);
        Self::from_building(b)
    }

    /// How many entries a single list page carries.
    ///
    /// Defaults to [`DEFAULT_PAGE_SIZE`]. Clamped to at least 1, since a page
    /// size of zero would hand back an endless run of empty pages.
    pub fn with_page_size(self, page_size: usize) -> Self {
        let mut b = self.into_building();
        b.page_size = Some(page_size.max(1));
        Self::from_building(b)
    }

    /// Every registered concrete resource.
    pub fn resources(&self) -> Vec<Resource> {
        self.inner
            .entries
            .iter()
            .map(|e| e.resource.clone())
            .collect()
    }

    /// Every registered URI template.
    ///
    /// Useful for checking that a [`crate::completion::CompletionRegistry`]
    /// does not point at a template that was never registered.
    pub fn template_uris(&self) -> Vec<&str> {
        self.inner
            .templates
            .iter()
            .map(|t| t.template.uri_template.as_str())
            .collect()
    }

    /// Serve `resources/list`, starting after `cursor`.
    ///
    /// Pages are ordered by URI rather than by registration order. That is what
    /// makes the cursor survive the list changing between requests: it names a
    /// position in a total order, so an entry added or removed behind the cursor
    /// cannot shift the page boundary or cause an entry to be served twice.
    ///
    /// An unreadable cursor is [`ErrorData::invalid_params`] rather than a
    /// silently empty page — a client that has lost its place should be told.
    pub fn list(&self, cursor: Option<&str>) -> Result<ListResourcesResult, ErrorData> {
        let (items, next) = page(
            &self.inner.entries,
            |e| e.resource.uri.as_str(),
            CursorKind::Resource,
            cursor,
            self.inner.page_size,
        )?;

        let mut result = ListResourcesResult::with_all_items(
            items.into_iter().map(|e| e.resource.clone()).collect(),
        );
        result.next_cursor = next;
        result.cache_scope = Some(self.inner.cache_scope);
        result.ttl_ms = self.inner.ttl_ms;
        Ok(result)
    }

    /// Serve `resources/templates/list`, starting after `cursor`.
    ///
    /// Ordered and cursored exactly as [`ResourceRegistry::list`], but over a
    /// separate sequence — a cursor minted by one is rejected by the other.
    pub fn list_templates(
        &self,
        cursor: Option<&str>,
    ) -> Result<ListResourceTemplatesResult, ErrorData> {
        let (items, next) = page(
            &self.inner.templates,
            |t| t.template.uri_template.as_str(),
            CursorKind::Template,
            cursor,
            self.inner.page_size,
        )?;

        let mut result = ListResourceTemplatesResult::with_all_items(
            items.into_iter().map(|t| t.template.clone()).collect(),
        );
        result.next_cursor = next;
        result.cache_scope = Some(self.inner.cache_scope);
        result.ttl_ms = self.inner.ttl_ms;
        Ok(result)
    }

    /// Serve `resources/read`.
    ///
    /// Concrete resources are matched first, then templates in registration
    /// order. An unmatched URI is a not-found error — which `rmcp` renders as
    /// `-32602` for 2026-07-28 peers and the legacy `-32002` for older ones.
    pub async fn read(&self, uri: &str) -> Result<ReadResourceResult, ErrorData> {
        for entry in &self.inner.entries {
            if entry.resource.uri == uri {
                let contents = match &entry.source {
                    Source::Static(contents) => contents.clone(),
                    Source::Dynamic(read) => {
                        read(ReadRequest {
                            uri: uri.to_string(),
                            params: BTreeMap::new(),
                        })
                        .await?
                    }
                };
                return Ok(self.finish(contents));
            }
        }

        for entry in &self.inner.templates {
            if let Some(params) = entry.parsed.match_uri(uri) {
                let contents = (entry.read)(ReadRequest {
                    uri: uri.to_string(),
                    params,
                })
                .await?;
                return Ok(self.finish(contents));
            }
        }

        Err(ErrorData::resource_not_found(
            format!("no resource matches `{uri}`"),
            None,
        ))
    }

    fn finish(&self, contents: Vec<ResourceContents>) -> ReadResourceResult {
        let mut result = ReadResourceResult::new(contents);
        result.cache_scope = Some(self.inner.cache_scope);
        result.ttl_ms = self.inner.ttl_ms;
        result
    }
}

fn wrap<F, Fut>(reader: F) -> ReadFn
where
    F: Fn(ReadRequest) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<Vec<ResourceContents>, ErrorData>> + Send + 'static,
{
    Arc::new(move |req| Box::pin(reader(req)))
}

/// Which sequence a cursor belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CursorKind {
    Resource,
    Template,
}

impl CursorKind {
    /// A one-byte tag, so a cursor issued for one sequence is rejected rather
    /// than quietly indexing into the other.
    fn tag(self) -> u8 {
        match self {
            Self::Resource => b'r',
            Self::Template => b't',
        }
    }
}

/// Cursor format version, so a cursor held across a deploy is refused rather
/// than reinterpreted under a new encoding.
const CURSOR_VERSION: u8 = b'1';

fn encode_cursor(kind: CursorKind, key: &str) -> String {
    use base64::Engine as _;

    let mut payload = vec![CURSOR_VERSION, kind.tag()];
    payload.extend_from_slice(key.as_bytes());
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(payload)
}

fn decode_cursor(kind: CursorKind, cursor: &str) -> Result<String, ErrorData> {
    use base64::Engine as _;

    // Deliberately one message for every failure. Which byte was wrong is of no
    // use to a caller and only helps someone probing the encoding.
    let invalid =
        || ErrorData::invalid_params("the pagination cursor is not one this server issued", None);

    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(cursor)
        .map_err(|_| invalid())?;

    let [version, tag, key @ ..] = bytes.as_slice() else {
        return Err(invalid());
    };
    if *version != CURSOR_VERSION || *tag != kind.tag() {
        return Err(invalid());
    }

    String::from_utf8(key.to_vec()).map_err(|_| invalid())
}

/// One page of `items`, ordered by `key`, resuming after `cursor`.
///
/// The cursor carries a **key**, not an index. That is the whole trick: a
/// fabricated cursor names some position in the key space rather than an offset
/// into a slice, so there is no such thing as an out-of-range read to guard
/// against, and an entry deleted between requests cannot shift the entries
/// after it onto a page they were already served on.
fn page<'a, T>(
    items: &'a [T],
    key: impl Fn(&'a T) -> &'a str,
    kind: CursorKind,
    cursor: Option<&str>,
    page_size: usize,
) -> Result<(Vec<&'a T>, Option<String>), ErrorData> {
    let after = cursor.map(|c| decode_cursor(kind, c)).transpose()?;

    let mut ordered: Vec<(&str, &T)> = items.iter().map(|item| (key(item), item)).collect();
    ordered.sort_by(|a, b| a.0.cmp(b.0));

    // Strictly after: the cursor names the last entry already served.
    let start = match &after {
        Some(after) => ordered.partition_point(|(k, _)| *k <= after.as_str()),
        None => 0,
    };

    let remaining = &ordered[start..];
    let taken = remaining.len().min(page_size);

    // A cursor only when something is actually left. Emitting one on the last
    // page costs the client a whole extra round trip to learn nothing.
    let next = (remaining.len() > taken).then(|| encode_cursor(kind, remaining[taken - 1].0));

    Ok((
        remaining[..taken].iter().map(|(_, item)| *item).collect(),
        next,
    ))
}

/// An RFC 6570 level-1 URI template, compiled for matching.
#[derive(Debug, Clone, PartialEq)]
struct UriTemplate {
    parts: Vec<Part>,
}

#[derive(Debug, Clone, PartialEq)]
enum Part {
    Literal(String),
    Var(String),
}

impl UriTemplate {
    fn parse(template: &str) -> Self {
        let mut parts = Vec::new();
        let mut literal = String::new();
        let mut rest = template;

        while let Some(open) = rest.find('{') {
            literal.push_str(&rest[..open]);
            match rest[open..].find('}') {
                Some(close) => {
                    if !literal.is_empty() {
                        parts.push(Part::Literal(std::mem::take(&mut literal)));
                    }
                    parts.push(Part::Var(rest[open + 1..open + close].to_string()));
                    rest = &rest[open + close + 1..];
                }
                // Unterminated `{` is treated as a literal rather than
                // silently swallowing the rest of the template.
                None => {
                    literal.push_str(&rest[open..]);
                    rest = "";
                    break;
                }
            }
        }
        literal.push_str(rest);
        if !literal.is_empty() {
            parts.push(Part::Literal(literal));
        }

        Self { parts }
    }

    /// Match `uri`, returning the captured variables percent-decoded.
    fn match_uri(&self, uri: &str) -> Option<BTreeMap<String, String>> {
        let mut params = BTreeMap::new();
        let mut rest = uri;
        let mut parts = self.parts.iter().peekable();

        while let Some(part) = parts.next() {
            match part {
                Part::Literal(lit) => {
                    rest = rest.strip_prefix(lit.as_str())?;
                }
                Part::Var(name) => {
                    // A variable stops at the next literal, or at the end.
                    // Never crossing `/` is what keeps a template variable from
                    // being used to walk out of its namespace.
                    let value = match parts.peek() {
                        Some(Part::Literal(next)) => {
                            let end = rest.find(next.as_str())?;
                            let value = &rest[..end];
                            rest = &rest[end..];
                            value
                        }
                        _ => {
                            let value = rest;
                            rest = "";
                            value
                        }
                    };

                    if value.is_empty() || value.contains('/') {
                        return None;
                    }
                    params.insert(name.clone(), percent_decode(value));
                }
            }
        }

        rest.is_empty().then_some(params)
    }
}

fn percent_decode(value: &str) -> String {
    percent_encoding::percent_decode_str(value)
        .decode_utf8_lossy()
        .into_owned()
}

/// Implement the three resource methods by forwarding to a
/// [`ResourceRegistry`] field.
///
/// Expands inside an `impl ServerHandler` block:
///
/// ```ignore
/// impl ServerHandler for MyServer {
///     fn get_info(&self) -> ServerInfo { /* ... */ }
///     rusty_mcp::forward_resource_methods!(resources);
/// }
/// ```
#[macro_export]
macro_rules! forward_resource_methods {
    ($field:ident) => {
        async fn list_resources(
            &self,
            request: ::core::option::Option<$crate::__private::PaginatedRequestParams>,
            _context: $crate::__private::RequestContext<$crate::__private::RoleServer>,
        ) -> ::core::result::Result<
            $crate::__private::ListResourcesResult,
            $crate::__private::ErrorData,
        > {
            let cursor = request.as_ref().and_then(|r| r.cursor.as_deref());
            self.$field.list(cursor)
        }

        async fn list_resource_templates(
            &self,
            request: ::core::option::Option<$crate::__private::PaginatedRequestParams>,
            _context: $crate::__private::RequestContext<$crate::__private::RoleServer>,
        ) -> ::core::result::Result<
            $crate::__private::ListResourceTemplatesResult,
            $crate::__private::ErrorData,
        > {
            let cursor = request.as_ref().and_then(|r| r.cursor.as_deref());
            self.$field.list_templates(cursor)
        }

        async fn read_resource(
            &self,
            request: $crate::__private::ReadResourceRequestParams,
            _context: $crate::__private::RequestContext<$crate::__private::RoleServer>,
        ) -> ::core::result::Result<
            $crate::__private::ReadResourceResponse,
            $crate::__private::ErrorData,
        > {
            // `ReadResourceResponse` is the MRTR wrapper; a registry always
            // produces a complete result, never an input request.
            self.$field
                .read(&request.uri)
                .await
                .map(::core::convert::Into::into)
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    fn template(s: &str) -> UriTemplate {
        UriTemplate::parse(s)
    }

    #[test]
    fn parses_literals_and_variables() {
        assert_eq!(
            template("file:///logs/{name}.log").parts,
            vec![
                Part::Literal("file:///logs/".into()),
                Part::Var("name".into()),
                Part::Literal(".log".into()),
            ]
        );
    }

    #[test]
    fn an_unterminated_brace_stays_literal() {
        assert_eq!(
            template("file:///{oops").parts,
            vec![Part::Literal("file:///{oops".into())]
        );
    }

    #[test]
    fn captures_a_variable() {
        let params = template("file:///logs/{name}.log")
            .match_uri("file:///logs/app.log")
            .expect("should match");
        assert_eq!(params.get("name").map(String::as_str), Some("app"));
    }

    #[test]
    fn captures_a_trailing_variable() {
        let params = template("db://tables/{table}")
            .match_uri("db://tables/users")
            .expect("should match");
        assert_eq!(params.get("table").map(String::as_str), Some("users"));
    }

    #[test]
    fn captures_several_variables() {
        let params = template("db://{schema}/tables/{table}")
            .match_uri("db://public/tables/users")
            .expect("should match");
        assert_eq!(params.get("schema").map(String::as_str), Some("public"));
        assert_eq!(params.get("table").map(String::as_str), Some("users"));
    }

    #[test]
    fn a_variable_never_crosses_a_slash() {
        // This is the traversal guard: without it, `name` would capture
        // `../../etc/passwd` and a filesystem-backed reader would obey.
        assert!(
            template("file:///logs/{name}")
                .match_uri("file:///logs/../../etc/passwd")
                .is_none()
        );
        assert!(
            template("file:///logs/{name}.log")
                .match_uri("file:///logs/nested/app.log")
                .is_none()
        );
    }

    #[test]
    fn rejects_an_empty_variable() {
        assert!(
            template("db://tables/{table}")
                .match_uri("db://tables/")
                .is_none()
        );
    }

    #[test]
    fn rejects_a_mismatched_prefix_or_trailing_junk() {
        let t = template("file:///logs/{name}.log");
        assert!(t.match_uri("other:///logs/app.log").is_none());
        assert!(t.match_uri("file:///logs/app.log.bak").is_none());
    }

    #[test]
    fn percent_decodes_captured_values() {
        let params = template("db://tables/{table}")
            .match_uri("db://tables/my%20table")
            .expect("should match");
        assert_eq!(params.get("table").map(String::as_str), Some("my table"));
    }

    #[tokio::test]
    async fn reads_a_static_resource() {
        let registry = ResourceRegistry::new().with_text(
            Resource::new("config://app", "app-config").with_mime_type("application/json"),
            "{}",
        );

        let result = registry.read("config://app").await.expect("should read");
        assert_eq!(result.contents.len(), 1);
        // Cache hints are required on read results under 2026-07-28.
        assert!(result.ttl_ms.is_some());
        assert_eq!(result.cache_scope, Some(CacheScope::Private));
    }

    #[tokio::test]
    async fn lists_resources_and_templates_with_cache_hints() {
        let registry = ResourceRegistry::new()
            .with_text(Resource::new("config://app", "app-config"), "{}")
            .with_template(
                ResourceTemplate::new("db://tables/{table}", "table"),
                |_req| async move { Ok(vec![]) },
            );

        let list = registry.list(None).expect("lists");
        assert_eq!(list.resources.len(), 1);
        assert!(list.ttl_ms.is_some());
        assert!(list.cache_scope.is_some());
        assert!(list.next_cursor.is_none(), "one page holds everything");

        let templates = registry.list_templates(None).expect("lists");
        assert_eq!(templates.resource_templates.len(), 1);
        assert!(templates.ttl_ms.is_some());
    }

    /// A registry with `count` resources named so that URI order and
    /// registration order disagree — otherwise a paginator that ignores its
    /// ordering would still pass.
    fn many(count: usize, page_size: usize) -> ResourceRegistry {
        (0..count)
            .rev()
            .fold(ResourceRegistry::new(), |registry, i| {
                registry.with_text(Resource::new(format!("mem://r{i:03}"), "r"), "x")
            })
            .with_page_size(page_size)
    }

    /// Walk every page, returning the URIs in the order they were served.
    fn drain(registry: &ResourceRegistry) -> Vec<String> {
        let mut uris = Vec::new();
        let mut cursor = None;

        loop {
            let page = registry.list(cursor.as_deref()).expect("lists");
            uris.extend(page.resources.iter().map(|r| r.uri.clone()));
            match page.next_cursor {
                Some(next) => cursor = Some(next),
                None => return uris,
            }
        }
    }

    #[test]
    fn paging_yields_every_entry_exactly_once() {
        let registry = many(25, 10);
        let uris = drain(&registry);

        assert_eq!(uris.len(), 25);

        let unique: std::collections::BTreeSet<_> = uris.iter().collect();
        assert_eq!(unique.len(), 25, "no entry served twice");

        let mut sorted = uris.clone();
        sorted.sort();
        assert_eq!(uris, sorted, "pages come out in URI order");
    }

    #[test]
    fn each_page_is_full_until_the_last_and_the_last_has_no_cursor() {
        let registry = many(25, 10);

        let first = registry.list(None).expect("lists");
        assert_eq!(first.resources.len(), 10);
        let cursor = first.next_cursor.expect("more to come");

        let second = registry.list(Some(&cursor)).expect("lists");
        assert_eq!(second.resources.len(), 10);
        let cursor = second.next_cursor.expect("more to come");

        let third = registry.list(Some(&cursor)).expect("lists");
        assert_eq!(third.resources.len(), 5);
        assert!(third.next_cursor.is_none(), "the last page ends the walk");
    }

    #[test]
    fn an_exact_multiple_does_not_emit_a_trailing_empty_page() {
        // 20 entries at 10 a page. Emitting a cursor on the second page would
        // cost the client a whole round trip to be told there is nothing left.
        let registry = many(20, 10);

        let first = registry.list(None).expect("lists");
        let cursor = first.next_cursor.expect("more to come");
        let second = registry.list(Some(&cursor)).expect("lists");

        assert_eq!(second.resources.len(), 10);
        assert!(second.next_cursor.is_none());
    }

    #[test]
    fn every_page_carries_cache_hints() {
        // Not just the first: a client that caches page two needs the hint too.
        let registry = many(25, 10);
        let mut cursor = None;
        let mut pages = 0;

        loop {
            let page = registry.list(cursor.as_deref()).expect("lists");
            pages += 1;
            assert!(page.ttl_ms.is_some(), "page {pages} has no ttl");
            assert!(page.cache_scope.is_some(), "page {pages} has no scope");
            match page.next_cursor {
                Some(next) => cursor = Some(next),
                None => break,
            }
        }
        assert_eq!(pages, 3);
    }

    #[test]
    fn a_malformed_cursor_is_invalid_params() {
        let registry = many(25, 10);

        for cursor in ["not base64!!", "", "AAAA"] {
            let err = registry.list(Some(cursor)).expect_err("should reject");
            assert_eq!(
                err.code,
                rmcp::model::ErrorCode::INVALID_PARAMS,
                "cursor {cursor:?} should be -32602"
            );
        }
    }

    #[test]
    fn a_cursor_from_one_sequence_is_rejected_by_the_other() {
        let registry = many(25, 10);
        let cursor = registry
            .list(None)
            .expect("lists")
            .next_cursor
            .expect("more to come");

        // Same registry, different sequence. Without the kind tag this would
        // silently seek into the templates by a resource URI.
        let err = registry
            .list_templates(Some(&cursor))
            .expect_err("should reject");
        assert_eq!(err.code, rmcp::model::ErrorCode::INVALID_PARAMS);
    }

    #[test]
    fn removing_the_entry_a_cursor_names_does_not_disturb_the_next_page() {
        // The reason the cursor holds a key and not an index. Take a cursor,
        // then rebuild the registry without the entry it names.
        let registry = many(25, 10);
        let cursor = registry
            .list(None)
            .expect("lists")
            .next_cursor
            .expect("more to come");

        let mut rebuilt = ResourceRegistry::new().with_page_size(10);
        for i in 0..25 {
            if i == 9 {
                // `mem://r009` is exactly what the cursor points at.
                continue;
            }
            rebuilt = rebuilt.with_text(Resource::new(format!("mem://r{i:03}"), "r"), "x");
        }

        let page = rebuilt.list(Some(&cursor)).expect("lists");
        assert_eq!(
            page.resources.first().map(|r| r.uri.as_str()),
            Some("mem://r010"),
            "the walk resumes where it left off even though the anchor is gone"
        );
    }

    #[test]
    fn a_cursor_past_the_end_is_an_empty_final_page() {
        // Not an error: a cursor beyond the last key is indistinguishable from
        // one whose successors were all deleted, and that is a normal end.
        let registry = many(5, 10);
        let cursor = encode_cursor(CursorKind::Resource, "mem://zzz");

        let page = registry.list(Some(&cursor)).expect("lists");
        assert!(page.resources.is_empty());
        assert!(page.next_cursor.is_none());
    }

    #[test]
    fn a_page_size_of_zero_is_clamped() {
        // Zero would page forever, one empty page at a time.
        let registry = many(3, 0);
        assert_eq!(drain(&registry).len(), 3);
    }

    #[test]
    fn templates_paginate_too() {
        let mut registry = ResourceRegistry::new().with_page_size(2);
        for i in 0..5 {
            registry = registry.with_template(
                ResourceTemplate::new(format!("db://t{i}/{{x}}"), "t"),
                |_req| async move { Ok(vec![]) },
            );
        }

        let mut seen = 0;
        let mut cursor = None;
        loop {
            let page = registry.list_templates(cursor.as_deref()).expect("lists");
            seen += page.resource_templates.len();
            match page.next_cursor {
                Some(next) => cursor = Some(next),
                None => break,
            }
        }
        assert_eq!(seen, 5);
    }

    #[tokio::test]
    async fn reads_through_a_template() {
        let registry = ResourceRegistry::new().with_template(
            ResourceTemplate::new("db://tables/{table}", "table"),
            |req: ReadRequest| async move {
                let table = req.param("table").unwrap_or_default().to_string();
                Ok(vec![ResourceContents::text(
                    format!("rows of {table}"),
                    req.uri.clone(),
                )])
            },
        );

        let result = registry.read("db://tables/users").await.expect("reads");
        match &result.contents[0] {
            ResourceContents::TextResourceContents { text, .. } => {
                assert_eq!(text, "rows of users");
            }
            other => panic!("expected text contents, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn an_unknown_uri_is_not_found() {
        let registry =
            ResourceRegistry::new().with_text(Resource::new("config://app", "app-config"), "{}");

        let err = registry
            .read("config://missing")
            .await
            .expect_err("no match");
        // rmcp maps this to -32602 for 2026-07-28 peers and -32002 for older.
        assert_eq!(err.code, rmcp::model::ErrorCode::RESOURCE_NOT_FOUND);
    }

    #[tokio::test]
    async fn concrete_resources_win_over_templates() {
        let registry = ResourceRegistry::new()
            .with_text(Resource::new("db://tables/users", "users"), "exact")
            .with_template(
                ResourceTemplate::new("db://tables/{table}", "table"),
                |_req| async move { Ok(vec![ResourceContents::text("templated", "x")]) },
            );

        let result = registry.read("db://tables/users").await.expect("reads");
        match &result.contents[0] {
            ResourceContents::TextResourceContents { text, .. } => assert_eq!(text, "exact"),
            other => panic!("expected the concrete resource, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_reader_error_propagates() {
        let registry = ResourceRegistry::new()
            .with_reader(Resource::new("flaky://thing", "flaky"), |_req| async move {
                Err(ErrorData::internal_error("backend down", None))
            });

        let err = registry
            .read("flaky://thing")
            .await
            .expect_err("should fail");
        assert!(err.message.contains("backend down"));
    }
}
