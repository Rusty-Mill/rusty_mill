//! A registry for `completion/complete`.
//!
//! Completion is what turns `db://tables/{table}` from something a user has to
//! already know into something a client can offer suggestions for. The server
//! holds the list of tables; without this method there is no way to ask for it.
//!
//! `rmcp` ships the wire types but no router, the same gap
//! [`crate::resources::ResourceRegistry`] fills for resources.
//!
//! ```
//! use rmcp::model::Reference;
//! use rusty_mcp::completion::CompletionRegistry;
//!
//! let completions = CompletionRegistry::new()
//!     // A fixed list, known at startup.
//!     .with_values(
//!         Reference::for_prompt("explain-error"),
//!         "language",
//!         ["rust", "python", "typescript"],
//!     )
//!     // Computed per request, so it can depend on what the user has typed
//!     // into the other arguments already.
//!     .with_completer(
//!         Reference::for_resource("db://tables/{table}"),
//!         "table",
//!         |_req| async move { Ok(vec!["users".to_string(), "orders".to_string()]) },
//!     );
//! # let _ = completions;
//! ```
//!
//! Then wire it into the handler, and advertise the capability — a client that
//! is not told the server completes will never ask:
//!
//! ```ignore
//! impl ServerHandler for MyServer {
//!     fn get_info(&self) -> ServerInfo {
//!         rusty_mcp::server_info("my-server", "0.1.0",
//!             ServerCapabilities::builder().enable_prompts().enable_completions().build())
//!     }
//!     rusty_mcp::forward_completion_methods!(completions);
//! }
//! ```
//!
//! # This runs while someone is typing
//!
//! Completion sits on the interactive path — a client calls it per keystroke,
//! or close to it. A completer that queries a database on every call will be
//! felt directly as input lag, so cache the candidate list rather than
//! recomputing it. Filtering the cached list is what this module does for you.

use std::{collections::BTreeMap, future::Future, pin::Pin, sync::Arc};

use rmcp::model::{
    ArgumentInfo, CompleteRequestParams, CompleteResult, CompletionInfo, ErrorData, Reference,
};

/// Future returned by a completer.
pub type CompleteFuture = Pin<Box<dyn Future<Output = Result<Vec<String>, ErrorData>> + Send>>;

type CompleteFn = Arc<dyn Fn(CompletionRequest) -> CompleteFuture + Send + Sync>;

/// What a completer is asked for.
#[derive(Debug, Clone, Default)]
pub struct CompletionRequest {
    /// What the user has typed for this argument so far. Often empty — that is
    /// a client asking "what are my options?", not an error.
    pub value: String,
    /// Arguments the user has already filled in, from the request's `context`.
    ///
    /// This is what makes a dependent completion possible: a `column` argument
    /// can look at the `table` already chosen and offer only its columns.
    pub arguments: BTreeMap<String, String>,
}

impl CompletionRequest {
    /// An already-resolved argument, by name.
    pub fn argument(&self, name: &str) -> Option<&str> {
        self.arguments.get(name).map(String::as_str)
    }
}

/// Which kind of thing a completion is registered against.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum RefKind {
    Prompt,
    Resource,
}

/// A reference plus an argument name — what a request is looked up by.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Key {
    kind: RefKind,
    /// A prompt name, or a resource template URI.
    target: String,
    argument: String,
}

impl Key {
    fn from_request(reference: &Reference, argument: &ArgumentInfo) -> Self {
        let (kind, target) = match reference {
            Reference::Prompt(prompt) => (RefKind::Prompt, prompt.name.clone()),
            Reference::Resource(resource) => (RefKind::Resource, resource.uri.clone()),
            // `Reference` is `#[non_exhaustive]`. A variant added by a future
            // rmcp is one this registry cannot have a registration for, so it
            // falls through to "no completions" rather than failing the call.
            _ => (RefKind::Prompt, String::new()),
        };

        Self {
            kind,
            target,
            argument: argument.name.clone(),
        }
    }

    /// How this reads in a diagnostic.
    fn describe(&self) -> String {
        let kind = match self.kind {
            RefKind::Prompt => "prompt",
            RefKind::Resource => "resource",
        };
        format!("{kind} `{}` argument `{}`", self.target, self.argument)
    }
}

enum Source {
    /// Candidates fixed at registration.
    Values(Vec<String>),
    /// Candidates produced per request.
    Dynamic(CompleteFn),
}

/// Serves `completion/complete`.
///
/// Cheap to clone; sources sit behind an `Arc`, which matters under Streamable
/// HTTP where a handler is built per request.
#[derive(Clone, Default)]
pub struct CompletionRegistry {
    inner: Arc<BTreeMap<Key, Source>>,
}

impl std::fmt::Debug for CompletionRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CompletionRegistry")
            .field("registrations", &self.inner.len())
            .finish()
    }
}

impl CompletionRegistry {
    /// An empty registry. Every request gets an empty completion.
    pub fn new() -> Self {
        Self::default()
    }

    /// Take the map apart so a `with_*` method can extend it.
    ///
    /// Callers hold the only handle while building, so this never clones.
    fn into_map(self) -> BTreeMap<Key, Source> {
        Arc::try_unwrap(self.inner).unwrap_or_default()
    }

    /// Complete an argument from a fixed list.
    ///
    /// A later registration for the same reference and argument replaces an
    /// earlier one.
    pub fn with_values<I, V>(self, reference: Reference, argument: &str, values: I) -> Self
    where
        I: IntoIterator<Item = V>,
        V: Into<String>,
    {
        let key = Key::from_request(&reference, &ArgumentInfo::new(argument, ""));
        let mut map = self.into_map();
        map.insert(
            key,
            Source::Values(values.into_iter().map(Into::into).collect()),
        );
        Self {
            inner: Arc::new(map),
        }
    }

    /// Complete an argument from a list computed per request.
    ///
    /// The completer returns **candidates**, not a filtered result — prefix
    /// matching, the 100-item cap and the `hasMore` flag are applied here.
    /// Returning everything is the intended shape.
    pub fn with_completer<F, Fut>(self, reference: Reference, argument: &str, completer: F) -> Self
    where
        F: Fn(CompletionRequest) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Vec<String>, ErrorData>> + Send + 'static,
    {
        let key = Key::from_request(&reference, &ArgumentInfo::new(argument, ""));
        let mut map = self.into_map();
        map.insert(
            key,
            Source::Dynamic(Arc::new(move |req| Box::pin(completer(req)))),
        );
        Self {
            inner: Arc::new(map),
        }
    }

    /// Registrations pointing at a prompt or template that does not exist.
    ///
    /// Returns a description of each, empty when everything resolves.
    ///
    /// A typo in a prompt name is otherwise invisible: the registration is
    /// accepted, the client asks under the real name, and the answer is an
    /// empty list — indistinguishable from "nothing to suggest". Checking at
    /// registration time would mean this registry holding a reference to the
    /// prompt router and the resource registry, inverting the composition
    /// everything else here uses, so the check is a call you make at wiring
    /// time instead:
    ///
    /// ```
    /// # use rmcp::model::Reference;
    /// # use rusty_mcp::completion::CompletionRegistry;
    /// let completions = CompletionRegistry::new()
    ///     .with_values(Reference::for_prompt("summarize"), "tone", ["formal"]);
    ///
    /// let dangling = completions.dangling(&["summarize"], &[]);
    /// assert!(dangling.is_empty(), "completions point at nothing: {dangling:?}");
    /// ```
    pub fn dangling(&self, prompts: &[&str], resource_templates: &[&str]) -> Vec<String> {
        self.inner
            .keys()
            .filter(|key| {
                let known = match key.kind {
                    RefKind::Prompt => prompts,
                    RefKind::Resource => resource_templates,
                };
                !known.contains(&key.target.as_str())
            })
            .map(Key::describe)
            .collect()
    }

    /// How many completions are registered.
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// Whether nothing is registered.
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Serve `completion/complete`.
    ///
    /// An unregistered reference or argument yields an empty completion rather
    /// than an error. A client asks about arguments speculatively, and "nothing
    /// to suggest" is the ordinary answer, not a fault.
    pub async fn complete(
        &self,
        params: CompleteRequestParams,
    ) -> Result<CompleteResult, ErrorData> {
        let key = Key::from_request(&params.r#ref, &params.argument);

        let Some(source) = self.inner.get(&key) else {
            return Ok(CompleteResult::new(CompletionInfo::default()));
        };

        let candidates = match source {
            Source::Values(values) => values.clone(),
            Source::Dynamic(completer) => {
                let arguments = params
                    .context
                    .and_then(|context| context.arguments)
                    .map(|args| args.into_iter().collect())
                    .unwrap_or_default();

                completer(CompletionRequest {
                    value: params.argument.value.clone(),
                    arguments,
                })
                .await?
            }
        };

        Ok(finish(candidates, &params.argument.value))
    }
}

/// Filter to what the user has typed, then cap.
///
/// Matching is a case-insensitive prefix. Prefix rather than substring because
/// that is what a user typing a name expects to narrow; case-insensitive
/// because they should not have to guess whether the server capitalises.
fn finish(candidates: Vec<String>, typed: &str) -> CompleteResult {
    let typed = typed.to_lowercase();
    let mut matched: Vec<String> = candidates
        .into_iter()
        .filter(|candidate| candidate.to_lowercase().starts_with(&typed))
        .collect();

    matched.sort();
    matched.dedup();

    // `total` is the count *before* the cap, so a client can tell "these are
    // the only three" from "here are 100 of many".
    let total = matched.len();
    let has_more = total > CompletionInfo::MAX_VALUES;
    matched.truncate(CompletionInfo::MAX_VALUES);

    let completion = CompletionInfo::with_pagination(matched, Some(total as u32), has_more)
        // Unreachable: the truncate above is exactly this bound. Degrading to
        // an empty completion beats a panic on the interactive path.
        .unwrap_or_default();

    CompleteResult::new(completion)
}

/// Implement `completion/complete` by forwarding to a [`CompletionRegistry`]
/// field.
///
/// Expands inside an `impl ServerHandler` block:
///
/// ```ignore
/// impl ServerHandler for MyServer {
///     fn get_info(&self) -> ServerInfo { /* ... */ }
///     rusty_mcp::forward_completion_methods!(completions);
/// }
/// ```
#[macro_export]
macro_rules! forward_completion_methods {
    ($field:ident) => {
        async fn complete(
            &self,
            request: $crate::__private::CompleteRequestParams,
            _context: $crate::__private::RequestContext<$crate::__private::RoleServer>,
        ) -> ::core::result::Result<$crate::__private::CompleteResult, $crate::__private::ErrorData>
        {
            self.$field.complete(request).await
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    use rmcp::model::CompletionContext;

    fn request(reference: Reference, argument: &str, value: &str) -> CompleteRequestParams {
        CompleteRequestParams::new(reference, ArgumentInfo::new(argument, value))
    }

    fn registry() -> CompletionRegistry {
        CompletionRegistry::new().with_values(
            Reference::for_prompt("explain-error"),
            "language",
            ["Rust", "python", "typescript", "ruby"],
        )
    }

    #[tokio::test]
    async fn an_empty_value_offers_everything() {
        let result = registry()
            .complete(request(
                Reference::for_prompt("explain-error"),
                "language",
                "",
            ))
            .await
            .expect("completes");

        assert_eq!(result.completion.values.len(), 4);
        assert_eq!(result.completion.total, Some(4));
        assert_eq!(result.completion.has_more, Some(false));
    }

    #[tokio::test]
    async fn matching_is_a_case_insensitive_prefix() {
        let result = registry()
            .complete(request(
                Reference::for_prompt("explain-error"),
                "language",
                "r",
            ))
            .await
            .expect("completes");

        // "Rust" matches despite the capital; "ruby" matches; the others do not.
        assert_eq!(result.completion.values, vec!["Rust", "ruby"]);
    }

    #[tokio::test]
    async fn a_prefix_is_not_a_substring() {
        // "python" contains "th" but does not start with it. Substring matching
        // would make a long candidate list feel random to type against.
        let result = registry()
            .complete(request(
                Reference::for_prompt("explain-error"),
                "language",
                "th",
            ))
            .await
            .expect("completes");

        assert!(result.completion.values.is_empty());
    }

    #[tokio::test]
    async fn an_unregistered_reference_completes_to_nothing() {
        let result = registry()
            .complete(request(
                Reference::for_prompt("nonexistent"),
                "language",
                "",
            ))
            .await
            .expect("no error");

        assert!(result.completion.values.is_empty());
    }

    #[tokio::test]
    async fn an_unregistered_argument_completes_to_nothing() {
        let result = registry()
            .complete(request(
                Reference::for_prompt("explain-error"),
                "not-an-argument",
                "",
            ))
            .await
            .expect("no error");

        assert!(result.completion.values.is_empty());
    }

    #[tokio::test]
    async fn a_prompt_and_a_resource_of_the_same_name_do_not_collide() {
        let registry = CompletionRegistry::new()
            .with_values(Reference::for_prompt("thing"), "x", ["from-prompt"])
            .with_values(Reference::for_resource("thing"), "x", ["from-resource"]);

        let prompt = registry
            .complete(request(Reference::for_prompt("thing"), "x", ""))
            .await
            .expect("completes");
        assert_eq!(prompt.completion.values, vec!["from-prompt"]);

        let resource = registry
            .complete(request(Reference::for_resource("thing"), "x", ""))
            .await
            .expect("completes");
        assert_eq!(resource.completion.values, vec!["from-resource"]);
    }

    #[tokio::test]
    async fn the_hundred_item_cap_is_enforced_with_has_more() {
        let many: Vec<String> = (0..150).map(|i| format!("item-{i:03}")).collect();
        let registry = CompletionRegistry::new().with_values(Reference::for_prompt("p"), "a", many);

        let result = registry
            .complete(request(Reference::for_prompt("p"), "a", ""))
            .await
            .expect("completes");

        // The spec caps a response at 100 values; `total` still reports the
        // real size, which is the only way a client can tell it is seeing part.
        assert_eq!(result.completion.values.len(), CompletionInfo::MAX_VALUES);
        assert_eq!(result.completion.total, Some(150));
        assert_eq!(result.completion.has_more, Some(true));
        assert!(result.completion.validate().is_ok());
    }

    #[tokio::test]
    async fn a_completer_sees_the_already_resolved_arguments() {
        let registry = CompletionRegistry::new().with_completer(
            Reference::for_resource("db://{schema}/{table}"),
            "table",
            |req: CompletionRequest| async move {
                // The dependent case: which tables exist depends on the schema
                // the user already picked.
                Ok(match req.argument("schema") {
                    Some("public") => vec!["users".to_string(), "orders".to_string()],
                    _ => vec![],
                })
            },
        );

        let mut arguments = std::collections::HashMap::new();
        arguments.insert("schema".to_string(), "public".to_string());

        let params = request(
            Reference::for_resource("db://{schema}/{table}"),
            "table",
            "",
        )
        .with_context(CompletionContext::with_arguments(arguments));

        let result = registry.complete(params).await.expect("completes");
        assert_eq!(result.completion.values, vec!["orders", "users"]);
    }

    #[tokio::test]
    async fn a_completer_without_context_gets_an_empty_argument_map() {
        let registry = CompletionRegistry::new().with_completer(
            Reference::for_prompt("p"),
            "a",
            |req: CompletionRequest| async move {
                assert!(req.arguments.is_empty());
                Ok(vec!["ok".to_string()])
            },
        );

        let result = registry
            .complete(request(Reference::for_prompt("p"), "a", ""))
            .await
            .expect("completes");
        assert_eq!(result.completion.values, vec!["ok"]);
    }

    #[tokio::test]
    async fn a_completer_error_propagates() {
        let registry = CompletionRegistry::new().with_completer(
            Reference::for_prompt("p"),
            "a",
            |_req| async move { Err(ErrorData::internal_error("catalog down", None)) },
        );

        let err = registry
            .complete(request(Reference::for_prompt("p"), "a", ""))
            .await
            .expect_err("should fail");
        assert!(err.message.contains("catalog down"));
    }

    #[tokio::test]
    async fn a_later_registration_replaces_an_earlier_one() {
        let registry = CompletionRegistry::new()
            .with_values(Reference::for_prompt("p"), "a", ["old"])
            .with_values(Reference::for_prompt("p"), "a", ["new"]);

        let result = registry
            .complete(request(Reference::for_prompt("p"), "a", ""))
            .await
            .expect("completes");
        assert_eq!(result.completion.values, vec!["new"]);
        assert_eq!(registry.len(), 1);
    }

    #[test]
    fn dangling_finds_a_reference_that_does_not_exist() {
        let registry = CompletionRegistry::new()
            .with_values(Reference::for_prompt("summarize"), "tone", ["formal"])
            .with_values(Reference::for_prompt("sumarize"), "tone", ["formal"])
            .with_values(
                Reference::for_resource("db://tables/{table}"),
                "table",
                ["users"],
            );

        let dangling = registry.dangling(&["summarize"], &["db://tables/{table}"]);

        // The typo, and only the typo.
        assert_eq!(dangling.len(), 1);
        assert!(dangling[0].contains("sumarize"), "got {dangling:?}");
    }

    #[test]
    fn dangling_is_empty_when_everything_resolves() {
        let registry = CompletionRegistry::new().with_values(
            Reference::for_prompt("summarize"),
            "tone",
            ["formal"],
        );
        assert!(registry.dangling(&["summarize"], &[]).is_empty());
    }

    #[test]
    fn an_empty_registry_is_empty() {
        let registry = CompletionRegistry::new();
        assert!(registry.is_empty());
        assert_eq!(registry.len(), 0);
    }
}
