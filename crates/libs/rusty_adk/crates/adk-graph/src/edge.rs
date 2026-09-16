//! Edges and route matchers — how the graph decides where to go next.

use serde::{Deserialize, Serialize};

use crate::node::START;

/// Decides whether an edge is followed, given the routes its source emitted.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Route {
    /// Always follow. This is what a plain chained edge uses.
    Always,
    /// Follow when the source emitted this exact label.
    String(String),
    /// Follow when the source emitted this integer, rendered as a label.
    Int(i64),
    /// Follow when the source emitted this boolean, rendered as a label.
    Bool(bool),
    /// Follow when the source emitted any of these labels.
    Multi(Vec<String>),
    /// Follow only when no other outgoing edge matched.
    Default,
}

impl Route {
    /// Builds a string route.
    pub fn string(label: impl Into<String>) -> Self {
        Route::String(label.into())
    }

    /// Builds a multi-label route.
    pub fn multi<I, S>(labels: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Route::Multi(labels.into_iter().map(Into::into).collect())
    }

    /// Whether this route matches the emitted labels.
    ///
    /// [`Route::Default`] never matches here — the executor falls back to it
    /// only after every other outgoing edge has failed to match.
    pub fn matches(&self, emitted: &[String]) -> bool {
        match self {
            Route::Always => true,
            Route::Default => false,
            Route::String(label) => emitted.iter().any(|e| e == label),
            Route::Int(n) => {
                let rendered = n.to_string();
                emitted.iter().any(|e| e == &rendered)
            }
            Route::Bool(b) => {
                let rendered = b.to_string();
                emitted.iter().any(|e| e == &rendered)
            }
            Route::Multi(labels) => emitted.iter().any(|e| labels.contains(e)),
        }
    }

    /// Whether this is the fallback route.
    pub fn is_default(&self) -> bool {
        matches!(self, Route::Default)
    }
}

/// A directed connection between two nodes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Edge {
    /// Source node name, or [`START`].
    pub from: String,
    /// Destination node name.
    pub to: String,
    /// The condition under which this edge is followed.
    pub route: Route,
}

impl Edge {
    /// Builds an unconditional edge.
    pub fn new(from: impl Into<String>, to: impl Into<String>) -> Self {
        Self {
            from: from.into(),
            to: to.into(),
            route: Route::Always,
        }
    }

    /// Builds a conditional edge.
    pub fn routed(from: impl Into<String>, to: impl Into<String>, route: Route) -> Self {
        Self {
            from: from.into(),
            to: to.into(),
            route,
        }
    }
}

/// Wires nodes into a sequential chain, starting from [`START`].
///
/// Each node's output becomes the next node's input.
///
/// ```
/// use adk_graph::{chain, START};
///
/// let edges = chain(["a", "b", "c"]);
/// assert_eq!(edges.len(), 3);
/// assert_eq!(edges[0].from, START);
/// assert_eq!(edges[2].to, "c");
/// ```
pub fn chain<I, S>(nodes: I) -> Vec<Edge>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let names: Vec<String> = nodes.into_iter().map(Into::into).collect();
    let mut edges = Vec::new();
    let mut previous = START.to_string();
    for name in names {
        edges.push(Edge::new(previous, &name));
        previous = name;
    }
    edges
}

/// Concatenates edge lists built separately.
///
/// ```
/// use adk_graph::{chain, concat, Edge, Route};
///
/// let edges = concat([
///     chain(["classify"]),
///     vec![Edge::routed("classify", "bug", Route::string("BUG"))],
/// ]);
/// assert_eq!(edges.len(), 2);
/// ```
pub fn concat<I>(groups: I) -> Vec<Edge>
where
    I: IntoIterator<Item = Vec<Edge>>,
{
    groups.into_iter().flatten().collect()
}

/// Fluent builder for a graph's edge set.
#[derive(Debug, Default, Clone)]
pub struct EdgeBuilder {
    edges: Vec<Edge>,
}

impl EdgeBuilder {
    /// An empty builder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds an unconditional edge.
    pub fn add(mut self, from: impl Into<String>, to: impl Into<String>) -> Self {
        self.edges.push(Edge::new(from, to));
        self
    }

    /// Adds an edge from [`START`].
    pub fn start(self, to: impl Into<String>) -> Self {
        self.add(START, to)
    }

    /// Adds a conditional edge.
    pub fn add_route(
        mut self,
        from: impl Into<String>,
        to: impl Into<String>,
        route: Route,
    ) -> Self {
        self.edges.push(Edge::routed(from, to, route));
        self
    }

    /// Adds a fallback edge, followed when nothing else matches.
    pub fn add_default(self, from: impl Into<String>, to: impl Into<String>) -> Self {
        self.add_route(from, to, Route::Default)
    }

    /// Fans out from one node to several, all unconditional.
    pub fn add_fan_out<I, S>(mut self, from: impl Into<String>, targets: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let from = from.into();
        for target in targets {
            self.edges.push(Edge::new(from.clone(), target));
        }
        self
    }

    /// Fans in from several nodes into a join node.
    pub fn add_fan_in<I, S>(mut self, join: impl Into<String>, sources: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let join = join.into();
        for source in sources {
            self.edges.push(Edge::new(source, join.clone()));
        }
        self
    }

    /// Appends a pre-built edge list.
    pub fn extend(mut self, edges: impl IntoIterator<Item = Edge>) -> Self {
        self.edges.extend(edges);
        self
    }

    /// Finishes the builder.
    pub fn build(self) -> Vec<Edge> {
        self.edges
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn always_matches_regardless_of_emitted_routes() {
        assert!(Route::Always.matches(&[]));
        assert!(Route::Always.matches(&["X".into()]));
    }

    #[test]
    fn default_never_matches_directly() {
        // The executor consults Default only as a fallback.
        assert!(!Route::Default.matches(&[]));
        assert!(!Route::Default.matches(&["anything".into()]));
    }

    #[test]
    fn string_route_matches_an_exact_label() {
        let route = Route::string("BUG");
        assert!(route.matches(&["BUG".into()]));
        assert!(!route.matches(&["FEATURE".into()]));
    }

    #[test]
    fn int_and_bool_routes_match_rendered_labels() {
        assert!(Route::Int(3).matches(&["3".into()]));
        assert!(!Route::Int(3).matches(&["4".into()]));
        assert!(Route::Bool(true).matches(&["true".into()]));
    }

    #[test]
    fn multi_route_matches_any_listed_label() {
        let route = Route::multi(["A", "B"]);
        assert!(route.matches(&["B".into()]));
        assert!(!route.matches(&["C".into()]));
    }

    #[test]
    fn chain_wires_sequentially_from_start() {
        let edges = chain(["a", "b"]);
        assert_eq!(edges[0], Edge::new(START, "a"));
        assert_eq!(edges[1], Edge::new("a", "b"));
    }

    #[test]
    fn builder_composes_fan_out_and_fan_in() {
        let edges = EdgeBuilder::new()
            .start("dispatch")
            .add_fan_out("dispatch", ["a", "b"])
            .add_fan_in("join", ["a", "b"])
            .add("join", "report")
            .build();
        assert_eq!(edges.len(), 6);
        assert!(edges.iter().any(|e| e.from == "a" && e.to == "join"));
    }
}
