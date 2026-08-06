//! Federated tool names.
//!
//! Two MCP servers behind one endpoint may both export `search`, so names are
//! qualified with the target they came from: `github_search`, `jira_search`.
//!
//! Resolving that back is where it gets sharp. Splitting on the first `_` is
//! wrong the moment a target is called `code_search`, because its `index` tool
//! federates to `code_search_index` and naively splits back to target `code`,
//! tool `search_index`. So resolution matches against the *known target names*,
//! longest first, rather than looking for a separator. With targets `code` and
//! `code_search`, `code_search_index` resolves to the longer one — the only
//! reading that round-trips.
//!
//! Genuinely ambiguous configurations still exist (`code_search`'s `index` and
//! `code`'s `search_index` both federate to `code_search_index`), so
//! [`ToolNamer::collisions`] reports them at startup rather than letting the
//! gateway silently route to whichever target sorted first.

use agentgateway_config::NameMode;

/// Qualifies and resolves federated tool names.
#[derive(Debug, Clone)]
pub struct ToolNamer {
    mode: NameMode,
    /// Target names, longest first, so resolution prefers the longest match.
    targets: Vec<String>,
}

/// What a federated name resolved to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolution<'a> {
    /// The name carried a known target prefix.
    Qualified {
        /// The target that owns the tool.
        target: &'a str,
        /// The tool's name as that target knows it.
        tool: &'a str,
    },
    /// The name carried no target prefix. Under [`NameMode::Passthrough`] this
    /// is expected and the caller resolves it against the tool cache; under
    /// [`NameMode::Prefix`] it means the client asked for something we never
    /// advertised.
    Unqualified(&'a str),
}

impl ToolNamer {
    /// Build a namer for a set of targets.
    pub fn new(mode: NameMode, target_names: impl IntoIterator<Item = String>) -> Self {
        let mut targets: Vec<String> = target_names.into_iter().collect();
        // Longest first: `code_search` must be tried before `code`.
        targets.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));
        ToolNamer { mode, targets }
    }

    /// The name a tool is advertised under.
    pub fn qualify(&self, target: &str, tool: &str) -> String {
        match self.mode {
            NameMode::Prefix => format!("{target}_{tool}"),
            NameMode::Passthrough => tool.to_string(),
        }
    }

    /// Resolve an advertised name back to a target and tool.
    pub fn resolve<'a>(&'a self, federated: &'a str) -> Resolution<'a> {
        if self.mode == NameMode::Passthrough {
            return Resolution::Unqualified(federated);
        }
        for target in &self.targets {
            if let Some(rest) = federated.strip_prefix(target.as_str())
                && let Some(tool) = rest.strip_prefix('_')
                && !tool.is_empty()
            {
                return Resolution::Qualified { target, tool };
            }
        }
        Resolution::Unqualified(federated)
    }

    /// Federated names that more than one `(target, tool)` pair would produce.
    ///
    /// `tools` is the unqualified tool list per target. Under
    /// [`NameMode::Passthrough`] this catches the same tool name exported by
    /// two targets; under [`NameMode::Prefix`] it catches the rarer case of
    /// target names that overlap on an underscore boundary.
    pub fn collisions<'a>(
        &self,
        tools: impl IntoIterator<Item = (&'a str, &'a [String])>,
    ) -> Vec<String> {
        let mut seen: std::collections::BTreeMap<String, Vec<String>> = Default::default();
        for (target, names) in tools {
            for tool in names {
                seen.entry(self.qualify(target, tool))
                    .or_default()
                    .push(format!("{target}/{tool}"));
            }
        }
        seen.into_iter()
            .filter(|(_, sources)| sources.len() > 1)
            .map(|(name, sources)| format!("`{name}` is produced by {}", sources.join(" and ")))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn namer(mode: NameMode, targets: &[&str]) -> ToolNamer {
        ToolNamer::new(mode, targets.iter().map(|t| t.to_string()))
    }

    #[test]
    fn qualified_names_round_trip() {
        let namer = namer(NameMode::Prefix, &["github", "jira"]);
        let federated = namer.qualify("github", "search");
        assert_eq!(federated, "github_search");
        assert_eq!(
            namer.resolve(&federated),
            Resolution::Qualified {
                target: "github",
                tool: "search"
            }
        );
    }

    #[test]
    fn a_target_name_containing_an_underscore_still_round_trips() {
        // Splitting on the first `_` would resolve this to target `code`,
        // tool `search_index`, and the call would go to the wrong server.
        let namer = namer(NameMode::Prefix, &["code_search"]);
        let federated = namer.qualify("code_search", "index");
        assert_eq!(federated, "code_search_index");
        assert_eq!(
            namer.resolve(&federated),
            Resolution::Qualified {
                target: "code_search",
                tool: "index"
            }
        );
    }

    #[test]
    fn the_longest_matching_target_wins() {
        let namer = namer(NameMode::Prefix, &["code", "code_search"]);
        assert_eq!(
            namer.resolve("code_search_index"),
            Resolution::Qualified {
                target: "code_search",
                tool: "index"
            },
            "the longer target is the reading that round-trips"
        );
        assert_eq!(
            namer.resolve("code_index"),
            Resolution::Qualified {
                target: "code",
                tool: "index"
            }
        );
    }

    #[test]
    fn an_unknown_prefix_does_not_resolve() {
        let namer = namer(NameMode::Prefix, &["github"]);
        assert_eq!(
            namer.resolve("gitlab_search"),
            Resolution::Unqualified("gitlab_search"),
            "a name we never advertised must not be routed anywhere"
        );
        assert_eq!(
            namer.resolve("github_"),
            Resolution::Unqualified("github_"),
            "an empty tool name is not a tool"
        );
    }

    #[test]
    fn passthrough_mode_leaves_names_alone() {
        let namer = namer(NameMode::Passthrough, &["github"]);
        assert_eq!(namer.qualify("github", "search"), "search");
        assert_eq!(namer.resolve("search"), Resolution::Unqualified("search"));
    }

    #[test]
    fn passthrough_collisions_are_reported() {
        let namer = namer(NameMode::Passthrough, &["github", "jira"]);
        let github = vec!["search".to_string()];
        let jira = vec!["search".to_string()];
        let collisions =
            namer.collisions([("github", github.as_slice()), ("jira", jira.as_slice())]);
        assert_eq!(collisions.len(), 1, "got: {collisions:?}");
        assert!(collisions[0].contains("github/search"));
        assert!(collisions[0].contains("jira/search"));
    }

    #[test]
    fn prefixing_resolves_what_passthrough_would_collide_on() {
        let namer = namer(NameMode::Prefix, &["github", "jira"]);
        let github = vec!["search".to_string()];
        let jira = vec!["search".to_string()];
        assert!(
            namer
                .collisions([("github", github.as_slice()), ("jira", jira.as_slice())])
                .is_empty(),
            "prefixing exists precisely so this is not a collision"
        );
    }

    #[test]
    fn overlapping_target_names_are_reported_as_a_collision() {
        // `code_search` + `index` and `code` + `search_index` both federate to
        // `code_search_index`. Resolution has to pick one; saying so at
        // startup beats routing to whichever sorted first.
        let namer = namer(NameMode::Prefix, &["code", "code_search"]);
        let code = vec!["search_index".to_string()];
        let code_search = vec!["index".to_string()];
        let collisions = namer.collisions([
            ("code", code.as_slice()),
            ("code_search", code_search.as_slice()),
        ]);
        assert_eq!(collisions.len(), 1, "got: {collisions:?}");
        assert!(collisions[0].contains("code_search_index"));
    }
}
