//! Lens composition (#723): one lens's prompt per question, built from a
//! catalog the shell has already read.
//!
//! A question's claim domains pick knowledge and standards files through
//! the domain map; the target repo's `DOMAIN.md` is the project section.
//! The result replaces the flat lens prompt inside
//! [`crate::context::prefix`], so the order is: lens, `# Project`,
//! `# Knowledge`, `# Standards`, then the reply protocol.
//!
//! With no domain pack and nothing selected the output is the lens prompt,
//! byte for byte, so composition off and composition with an empty catalog
//! build the same prefix. Files are deduped and sorted: the order of a
//! claim's domains never changes the bytes, so a [`Selection`] is a stable
//! cache key for the prefix.
//!
//! Rules stay in code (ADR-0021); nothing composed here can loosen them.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path};

use rusty_json::Value;

/// The files one domain loads, as catalog-relative paths.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Packs {
    pub knowledge: Vec<String>,
    pub standards: Vec<String>,
}

/// Domain token to the files it loads (Factory `domains.toml`).
pub type DomainMap = BTreeMap<String, Packs>;

/// What a question's domains select. Equal selections compose equal bytes.
#[derive(Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Selection {
    pub knowledge: BTreeSet<String>,
    pub standards: BTreeSet<String>,
}

/// Size limits on composed sections, in bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// A file larger than this is left out.
    pub file_bytes: usize,
    /// Everything composed beyond the lens prompt must fit in this.
    pub total_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            file_bytes: 8 * 1024,
            total_bytes: 32 * 1024,
        }
    }
}

/// A composed lens prompt and what was left out to make it fit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Composed {
    pub prompt: String,
    /// Paths (or `DOMAIN.md`) left out, in the order they were dropped.
    pub dropped: Vec<String>,
}

/// Why a prompt could not be composed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ComposeError {
    /// The map selects a file the shell did not supply.
    Missing(String),
}

impl std::fmt::Display for ComposeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Missing(path) => write!(f, "catalog file not loaded: {path}"),
        }
    }
}

impl std::error::Error for ComposeError {}

/// Parse the domain map (Factory `domains.json`):
/// `{"rust": {"knowledge": ["knowledge/rust.md"], "standards": [...]}}`.
/// Paths must be relative and stay inside the catalog.
pub fn parse_map(text: &str) -> Result<DomainMap, String> {
    let v = Value::parse(text).map_err(|e| format!("domain map is not JSON: {e}"))?;
    let top = v.as_object().ok_or("domain map: object required")?;
    top.iter()
        .map(|(domain, packs)| {
            let packs = parse_packs(packs).map_err(|e| format!("{domain}.{e}"))?;
            Ok((domain.clone(), packs))
        })
        .collect()
}

fn parse_packs(v: &Value) -> Result<Packs, String> {
    let m = v.as_object().ok_or("object required")?;
    if let Some(k) = m
        .keys()
        .find(|k| !["knowledge", "standards"].contains(&k.as_str()))
    {
        return Err(format!("unknown field {k:?}"));
    }
    Ok(Packs {
        knowledge: paths(m.get("knowledge"), "knowledge")?,
        standards: paths(m.get("standards"), "standards")?,
    })
}

fn paths(v: Option<&Value>, what: &str) -> Result<Vec<String>, String> {
    let Some(v) = v else {
        return Ok(vec![]);
    };
    let items = v
        .as_array()
        .ok_or_else(|| format!("{what}: array required"))?;
    items
        .iter()
        .map(|p| {
            let p = p
                .as_str()
                .ok_or_else(|| format!("{what}: strings required"))?;
            inside(p)
                .then(|| p.to_owned())
                .ok_or_else(|| format!("{what}: {p:?} must be a relative path inside the catalog"))
        })
        .collect()
}

/// Relative, non-empty, and no `..`, root or prefix component.
fn inside(p: &str) -> bool {
    !p.is_empty()
        && Path::new(p)
            .components()
            .all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
}

/// A composed prefix's key: the lens id and a claim's domains, sorted and
/// deduped, so domain order never picks a different prefix.
pub type PrefixKey = (String, Vec<String>);

/// The [`PrefixKey`] for `lens` on a claim in `domains`.
pub fn prefix_key(lens: &str, domains: &[String]) -> PrefixKey {
    let set: BTreeSet<&String> = domains.iter().collect();
    (lens.to_owned(), set.into_iter().cloned().collect())
}

/// The name reported when the domain pack is dropped.
pub const DOMAIN_PACK: &str = "DOMAIN.md";

/// The union of what `domains` load, and the tokens the map does not know.
pub fn select(map: &DomainMap, domains: &[String]) -> (Selection, BTreeSet<String>) {
    let mut sel = Selection::default();
    let mut unknown = BTreeSet::new();
    for d in domains {
        let Some(packs) = map.get(d) else {
            unknown.insert(d.clone());
            continue;
        };
        sel.knowledge.extend(packs.knowledge.iter().cloned());
        sel.standards.extend(packs.standards.iter().cloned());
    }
    (sel, unknown)
}

/// Compose `lens` with the domain pack and the selected files from `texts`
/// (path to contents). Oversized files are dropped first; then, until the
/// rest fits `total_bytes`, knowledge files go (last first), then the
/// domain pack, then standards (last first).
pub fn compose(
    lens: &str,
    domain_pack: Option<&str>,
    sel: &Selection,
    texts: &BTreeMap<String, String>,
    limits: Limits,
) -> Result<Composed, ComposeError> {
    let mut dropped = Vec::new();
    let mut knowledge = load(&sel.knowledge, texts, limits, &mut dropped)?;
    let mut standards = load(&sel.standards, texts, limits, &mut dropped)?;
    let mut pack = domain_pack.filter(|p| !p.trim().is_empty());
    while size(lens, pack, &knowledge, &standards) > limits.total_bytes {
        if let Some((path, _)) = knowledge.pop() {
            dropped.push(path.to_owned());
        } else if pack.take().is_some() {
            dropped.push(DOMAIN_PACK.to_owned());
        } else if let Some((path, _)) = standards.pop() {
            dropped.push(path.to_owned());
        }
    }
    Ok(Composed {
        prompt: render(lens, pack, &knowledge, &standards),
        dropped,
    })
}

type File<'a> = (&'a str, &'a str);

fn load<'a>(
    paths: &'a BTreeSet<String>,
    texts: &'a BTreeMap<String, String>,
    limits: Limits,
    dropped: &mut Vec<String>,
) -> Result<Vec<File<'a>>, ComposeError> {
    let mut out = Vec::new();
    for path in paths {
        let text = texts
            .get(path)
            .ok_or_else(|| ComposeError::Missing(path.clone()))?;
        if text.len() > limits.file_bytes {
            dropped.push(path.clone());
            continue;
        }
        out.push((path.as_str(), text.as_str()));
    }
    Ok(out)
}

/// Bytes composed beyond the lens prompt.
fn size(lens: &str, pack: Option<&str>, knowledge: &[File<'_>], standards: &[File<'_>]) -> usize {
    render(lens, pack, knowledge, standards).len() - lens.len()
}

fn render(
    lens: &str,
    pack: Option<&str>,
    knowledge: &[File<'_>],
    standards: &[File<'_>],
) -> String {
    let mut out = lens.to_owned();
    if let Some(p) = pack {
        section(&mut out, "Project");
        push_body(&mut out, p);
    }
    files(&mut out, "Knowledge", knowledge);
    files(&mut out, "Standards", standards);
    out
}

fn files(out: &mut String, title: &str, files: &[File<'_>]) {
    if files.is_empty() {
        return;
    }
    section(out, title);
    for (path, text) in files {
        out.push_str(&format!("## {path}\n"));
        push_body(out, text);
    }
}

fn section(out: &mut String, title: &str) {
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(&format!("\n# {title}\n"));
}

fn push_body(out: &mut String, text: &str) {
    out.push_str(text.trim_end());
    out.push('\n');
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map() -> DomainMap {
        let mut m = DomainMap::new();
        m.insert(
            "rust".into(),
            Packs {
                knowledge: vec!["knowledge/rust.md".into()],
                standards: vec!["standards/rust.md".into()],
            },
        );
        m.insert(
            "terminal".into(),
            Packs {
                knowledge: vec!["knowledge/terminal.md".into()],
                standards: vec![],
            },
        );
        m
    }

    fn texts() -> BTreeMap<String, String> {
        [
            ("knowledge/rust.md", "Ownership first.\n"),
            ("standards/rust.md", "MUST: no unwrap in library code.\n"),
            ("knowledge/terminal.md", "VT parsing is a state machine.\n"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect()
    }

    fn doms(d: &[&str]) -> Vec<String> {
        d.iter().map(|s| (*s).to_owned()).collect()
    }

    fn build(domains: &[&str], pack: Option<&str>, limits: Limits) -> Composed {
        let (sel, _) = select(&map(), &doms(domains));
        compose("find faults", pack, &sel, &texts(), limits).expect("composes")
    }

    #[test]
    fn the_map_parses() {
        let m = parse_map(
            r#"{"rust": {"knowledge": ["knowledge/rust.md"], "standards": ["standards/rust.md"]},
                "terminal": {"knowledge": ["knowledge/terminal.md"]}}"#,
        )
        .expect("parses");
        assert_eq!(m, map());
    }

    #[test]
    fn a_bad_map_is_refused_with_the_field_named() {
        for (text, err) in [
            ("[]", "domain map: object required"),
            (r#"{"rust": []}"#, "rust.object required"),
            (
                r#"{"rust": {"skills": []}}"#,
                "rust.unknown field \"skills\"",
            ),
            (
                r#"{"rust": {"knowledge": "k.md"}}"#,
                "rust.knowledge: array required",
            ),
            (
                r#"{"rust": {"knowledge": [1]}}"#,
                "rust.knowledge: strings required",
            ),
        ] {
            assert_eq!(parse_map(text).unwrap_err(), err, "{text}");
        }
        for bad in ["../secrets.md", "/etc/passwd", ""] {
            let text = format!(r#"{{"rust": {{"standards": ["{bad}"]}}}}"#);
            assert!(
                parse_map(&text).unwrap_err().contains("inside the catalog"),
                "{bad}"
            );
        }
    }

    #[test]
    fn nothing_selected_and_no_pack_is_the_flat_lens_prompt() {
        let c = build(&[], None, Limits::default());
        assert_eq!(c.prompt, "find faults");
        assert!(c.dropped.is_empty());
        assert_eq!(
            build(&["dedup"], Some("  \n"), Limits::default()).prompt,
            "find faults"
        );
    }

    #[test]
    fn different_domains_compose_different_prompts() {
        let rust = build(&["rust"], None, Limits::default()).prompt;
        let term = build(&["terminal"], None, Limits::default()).prompt;
        assert_ne!(rust, term);
        assert!(rust.contains("## knowledge/rust.md\nOwnership first.\n"));
        assert!(rust.contains("# Standards\n## standards/rust.md\n"));
        assert!(term.contains("VT parsing") && !term.contains("# Standards"));
    }

    #[test]
    fn domain_order_and_repeats_do_not_change_the_bytes() {
        let a = build(&["rust", "terminal"], Some("brief"), Limits::default());
        let b = build(
            &["terminal", "rust", "rust"],
            Some("brief"),
            Limits::default(),
        );
        assert_eq!(a, b);
        let (sa, _) = select(&map(), &doms(&["rust", "terminal"]));
        let (sb, _) = select(&map(), &doms(&["terminal", "rust"]));
        assert_eq!(sa, sb);
    }

    #[test]
    fn sections_come_in_order() {
        let p = build(&["rust"], Some("Dedup 20 TB."), Limits::default()).prompt;
        let at = |s: &str| p.find(s).expect(s);
        assert!(at("find faults") < at("# Project"));
        assert!(at("# Project") < at("# Knowledge"));
        assert!(at("# Knowledge") < at("# Standards"));
    }

    #[test]
    fn unknown_domains_are_reported_not_errors() {
        let (sel, unknown) = select(&map(), &doms(&["rust", "gui"]));
        assert_eq!(unknown, BTreeSet::from(["gui".to_owned()]));
        assert_eq!(sel.knowledge.len(), 1);
    }

    #[test]
    fn a_missing_file_is_an_error() {
        let (sel, _) = select(&map(), &doms(&["rust"]));
        let mut t = texts();
        t.remove("standards/rust.md");
        let err = compose("l", None, &sel, &t, Limits::default()).unwrap_err();
        assert_eq!(err, ComposeError::Missing("standards/rust.md".into()));
    }

    #[test]
    fn an_oversized_file_is_dropped() {
        let limits = Limits {
            file_bytes: 20,
            ..Limits::default()
        };
        let c = build(&["rust"], None, limits);
        assert_eq!(c.dropped, vec!["standards/rust.md".to_owned()]);
        assert!(c.prompt.contains("Ownership") && !c.prompt.contains("MUST"));
    }

    #[test]
    fn over_budget_drops_knowledge_then_pack_then_standards() {
        let full = build(&["rust", "terminal"], Some("brief"), Limits::default());
        let extra = full.prompt.len() - "find faults".len();
        let tight = |total_bytes| {
            build(
                &["rust", "terminal"],
                Some("brief"),
                Limits {
                    file_bytes: 1024,
                    total_bytes,
                },
            )
        };
        assert!(tight(extra).dropped.is_empty());
        assert_eq!(tight(extra - 1).dropped, vec!["knowledge/terminal.md"]);
        let only_std = Selection {
            standards: BTreeSet::from(["standards/rust.md".to_owned()]),
            ..Selection::default()
        };
        let std_len = compose("find faults", None, &only_std, &texts(), Limits::default())
            .expect("composes")
            .prompt
            .len()
            - "find faults".len();
        assert_eq!(
            tight(std_len).dropped,
            vec!["knowledge/terminal.md", "knowledge/rust.md", DOMAIN_PACK]
        );
        let none = tight(0);
        assert_eq!(none.prompt, "find faults");
        assert_eq!(
            none.dropped.last().map(String::as_str),
            Some("standards/rust.md")
        );
    }
}
