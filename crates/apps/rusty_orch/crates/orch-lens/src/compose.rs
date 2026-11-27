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

/// Domain token to the files it loads (Factory `domains.json`).
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
    /// A file larger than this, `DOMAIN.md` included, is left out.
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

/// A catalog file as the shell read it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Text {
    /// The file's contents.
    Loaded(String),
    /// Larger than the shell's read cap, so not read; dropped if selected.
    OverCap,
}

impl Text {
    /// The contents, if read and no larger than `cap` bytes.
    fn within(&self, cap: usize) -> Option<&str> {
        match self {
            Self::Loaded(t) if t.len() <= cap => Some(t),
            _ => None,
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

/// A claim's domains, sorted and deduped: the key a lens's composed
/// prefix is found by, so domain order never picks a different prefix.
pub fn domain_set(domains: &[String]) -> Vec<String> {
    let set: BTreeSet<&String> = domains.iter().collect();
    set.into_iter().cloned().collect()
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
/// (path to contents). Oversized files ([`Text::OverCap`], or larger than
/// `file_bytes`), the domain pack included, are dropped first; then, until the
/// rest fits `total_bytes`, knowledge files go (last first), then the
/// domain pack, then standards (last first).
pub fn compose(
    lens: &str,
    domain_pack: Option<&Text>,
    sel: &Selection,
    texts: &BTreeMap<String, Text>,
    limits: Limits,
) -> Result<Composed, ComposeError> {
    let mut d = Draft::new(lens, domain_pack, sel, texts, limits)?;
    d.fit(limits.total_bytes);
    Ok(d.composed())
}

/// A composition that fits `total_bytes`, not yet rendered: what is in,
/// in drop order, and the prompt's byte length at each step of leaving one
/// more out, so a caller can pick the step that fits a provider's window
/// and render only that one ([`Plan::render`]). Linear in what is in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    lens: String,
    pack: Option<String>,
    knowledge: Vec<(String, String)>,
    standards: Vec<(String, String)>,
    /// Left out by the limits, before any window.
    dropped: Vec<String>,
    /// `sizes[k]`: the prompt's bytes with the first `k` items of the drop
    /// order left out. Never empty; the last is the bare lens prompt's.
    sizes: Vec<usize>,
}

impl Plan {
    /// Plan `lens` with the domain pack and the selected files, as
    /// [`compose`] would compose them.
    pub fn new(
        lens: &str,
        domain_pack: Option<&Text>,
        sel: &Selection,
        texts: &BTreeMap<String, Text>,
        limits: Limits,
    ) -> Result<Self, ComposeError> {
        let mut d = Draft::new(lens, domain_pack, sel, texts, limits)?;
        d.fit(limits.total_bytes);
        let owned = |files: &[File<'_>]| {
            files
                .iter()
                .map(|(p, t)| ((*p).to_owned(), (*t).to_owned()))
                .collect()
        };
        let mut plan = Self {
            lens: lens.to_owned(),
            pack: d.pack.map(str::to_owned),
            knowledge: owned(&d.knowledge.files),
            standards: owned(&d.standards.files),
            dropped: d.dropped.clone(),
            sizes: vec![lens.len() + d.size()],
        };
        while d.drop_one() {
            plan.sizes.push(lens.len() + d.size());
        }
        Ok(plan)
    }

    /// How many items can still be left out.
    pub fn steps(&self) -> usize {
        self.sizes.len() - 1
    }

    /// The prompt's bytes with `k` items left out (`k` past
    /// [`Plan::steps`] counts as all of them).
    pub fn bytes(&self, k: usize) -> usize {
        self.sizes
            .get(k)
            .or(self.sizes.last())
            .copied()
            .unwrap_or(self.lens.len())
    }

    /// What the limits left out before any window, in drop order.
    pub fn dropped(&self) -> &[String] {
        &self.dropped
    }

    /// The first `k` items of the drop order: knowledge (last first), the
    /// domain pack, standards (last first).
    pub fn left_out(&self, k: usize) -> Vec<String> {
        let rev = |files: &[(String, String)]| {
            files.iter().rev().map(|f| f.0.clone()).collect::<Vec<_>>()
        };
        rev(&self.knowledge)
            .into_iter()
            .chain(self.pack.as_ref().map(|_| DOMAIN_PACK.to_owned()))
            .chain(rev(&self.standards))
            .take(k)
            .collect()
    }

    /// The prompt with `k` items left out; `bytes(k)` long.
    pub fn render(&self, k: usize) -> String {
        #[cfg(test)]
        RENDERS.with(|n| n.set(n.get() + 1));
        let nk = self.knowledge.len();
        let pack = self.pack.as_deref().filter(|_| k <= nk);
        let past = k.saturating_sub(nk + usize::from(self.pack.is_some()));
        render(
            &self.lens,
            pack,
            &first(&self.knowledge, nk.saturating_sub(k)),
            &first(&self.standards, self.standards.len().saturating_sub(past)),
        )
    }
}

/// The first `keep` of `files`, borrowed.
fn first(files: &[(String, String)], keep: usize) -> Vec<File<'_>> {
    files
        .iter()
        .take(keep)
        .map(|(p, t)| (p.as_str(), t.as_str()))
        .collect()
}

#[cfg(test)]
thread_local! {
    /// Prompts rendered by [`Plan::render`] on this thread.
    pub(crate) static RENDERS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// A composition in progress: what is still in, and what was left out.
struct Draft<'a> {
    lens: &'a str,
    pack: Option<&'a str>,
    knowledge: Part<'a>,
    standards: Part<'a>,
    dropped: Vec<String>,
}

impl<'a> Draft<'a> {
    fn new(
        lens: &'a str,
        domain_pack: Option<&'a Text>,
        sel: &'a Selection,
        texts: &'a BTreeMap<String, Text>,
        limits: Limits,
    ) -> Result<Self, ComposeError> {
        let mut dropped = Vec::new();
        let mut pack = None;
        if let Some(text) = domain_pack {
            match text.within(limits.file_bytes) {
                Some(p) => pack = Some(p).filter(|p| !p.trim().is_empty()),
                None => dropped.push(DOMAIN_PACK.to_owned()),
            }
        }
        let knowledge = Part::load("Knowledge", &sel.knowledge, texts, limits, &mut dropped)?;
        let standards = Part::load("Standards", &sel.standards, texts, limits, &mut dropped)?;
        Ok(Self {
            lens,
            pack,
            knowledge,
            standards,
            dropped,
        })
    }

    /// Bytes composed beyond the lens prompt, counted without rendering.
    fn size(&self) -> usize {
        let body = self.pack.map_or(0, |p| header("Project") + body(p))
            + self.knowledge.bytes()
            + self.standards.bytes();
        let lead = usize::from(!self.lens.is_empty() && !self.lens.ends_with('\n'));
        if body == 0 {
            0
        } else {
            lead + body
        }
    }

    fn fit(&mut self, total_bytes: usize) {
        while self.size() > total_bytes && self.drop_one() {}
    }

    /// Leave out the next item in drop order: knowledge (last first), the
    /// domain pack, standards (last first). False when nothing is left.
    fn drop_one(&mut self) -> bool {
        let gone = match self.knowledge.pop() {
            Some(path) => path.to_owned(),
            None => match self.pack.take() {
                Some(_) => DOMAIN_PACK.to_owned(),
                None => match self.standards.pop() {
                    Some(path) => path.to_owned(),
                    None => return false,
                },
            },
        };
        self.dropped.push(gone);
        true
    }

    fn composed(&self) -> Composed {
        Composed {
            prompt: render(
                self.lens,
                self.pack,
                &self.knowledge.files,
                &self.standards.files,
            ),
            dropped: self.dropped.clone(),
        }
    }
}

type File<'a> = (&'a str, &'a str);

/// One section's files and their rendered size, kept as files are
/// dropped, so the budget is counted without rendering.
struct Part<'a> {
    title: &'static str,
    files: Vec<File<'a>>,
    file_bytes: usize,
}

impl<'a> Part<'a> {
    fn load(
        title: &'static str,
        paths: &'a BTreeSet<String>,
        texts: &'a BTreeMap<String, Text>,
        limits: Limits,
        dropped: &mut Vec<String>,
    ) -> Result<Self, ComposeError> {
        let mut files = Vec::new();
        for path in paths {
            let text = texts
                .get(path)
                .ok_or_else(|| ComposeError::Missing(path.clone()))?;
            match text.within(limits.file_bytes) {
                Some(text) => files.push((path.as_str(), text)),
                None => dropped.push(path.clone()),
            }
        }
        let file_bytes = files.iter().map(|f| file(*f)).sum();
        Ok(Self {
            title,
            files,
            file_bytes,
        })
    }

    /// Rendered bytes: the heading and every file, or nothing.
    fn bytes(&self) -> usize {
        if self.files.is_empty() {
            0
        } else {
            header(self.title) + self.file_bytes
        }
    }

    fn pop(&mut self) -> Option<&'a str> {
        let f = self.files.pop()?;
        self.file_bytes -= file(f);
        Some(f.0)
    }
}

/// Bytes of `\n# {title}\n`, as [`section`] writes it after the lead.
fn header(title: &str) -> usize {
    title.len() + 4
}

/// Bytes of a body, as [`push_body`] writes it.
fn body(text: &str) -> usize {
    text.trim_end().len() + 1
}

/// Bytes of one file: `## {path}\n` and its body.
fn file((path, text): File<'_>) -> usize {
    path.len() + 4 + body(text)
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

    fn texts() -> BTreeMap<String, Text> {
        [
            ("knowledge/rust.md", "Ownership first.\n"),
            ("standards/rust.md", "MUST: no unwrap in library code.\n"),
            ("knowledge/terminal.md", "VT parsing is a state machine.\n"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), Text::Loaded(v.to_owned())))
        .collect()
    }

    fn doms(d: &[&str]) -> Vec<String> {
        d.iter().map(|s| (*s).to_owned()).collect()
    }

    fn build(domains: &[&str], pack: Option<&str>, limits: Limits) -> Composed {
        let (sel, _) = select(&map(), &doms(domains));
        let pack = pack.map(|p| Text::Loaded(p.to_owned()));
        compose("find faults", pack.as_ref(), &sel, &texts(), limits).expect("composes")
    }

    #[test]
    fn an_over_cap_file_is_dropped_only_when_selected() {
        let mut t = texts();
        t.insert("standards/rust.md".into(), Text::OverCap);
        let (rust, _) = select(&map(), &doms(&["rust"]));
        let c = compose("l", Some(&Text::OverCap), &rust, &t, Limits::default()).expect("composes");
        assert_eq!(
            c.dropped,
            vec![DOMAIN_PACK.to_owned(), "standards/rust.md".to_owned()]
        );
        assert!(c.prompt.contains("Ownership first.") && !c.prompt.contains("# Standards"));
        let (term, _) = select(&map(), &doms(&["terminal"]));
        let c = compose("l", None, &term, &t, Limits::default()).expect("composes");
        assert!(c.dropped.is_empty());
        assert!(c.prompt.contains("VT parsing"));
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
    fn an_oversized_domain_pack_is_dropped_even_when_it_fits_the_total() {
        let limits = Limits {
            file_bytes: 12,
            total_bytes: 1024,
        };
        let c = build(&[], Some("Dedup 20 TB, 3 hosts."), limits);
        assert_eq!(c.dropped, vec![DOMAIN_PACK.to_owned()]);
        assert_eq!(c.prompt, "find faults");
        let kept = build(&[], Some("Dedup 20 TB."), limits);
        assert_eq!(kept.prompt, "find faults\n\n# Project\nDedup 20 TB.\n");
    }

    #[test]
    fn exact_budget_boundaries_count_heading_bytes() {
        let tail = |c: &Composed| c.prompt.len() - "find faults".len();
        let at = |total_bytes| {
            build(
                &["rust"],
                None,
                Limits {
                    file_bytes: 1024,
                    total_bytes,
                },
            )
        };
        let full = "\n\n# Knowledge\n## knowledge/rust.md\nOwnership first.\n\
                    \n# Standards\n## standards/rust.md\nMUST: no unwrap in library code.\n";
        assert_eq!(at(full.len()).prompt, format!("find faults{full}"));
        assert!(at(full.len()).dropped.is_empty());
        let only_std = "\n\n# Standards\n## standards/rust.md\nMUST: no unwrap in library code.\n";
        let one_short = at(full.len() - 1);
        assert_eq!(one_short.dropped, vec!["knowledge/rust.md"]);
        assert_eq!(one_short.prompt, format!("find faults{only_std}"));
        assert_eq!(at(only_std.len()).prompt, format!("find faults{only_std}"));
        assert_eq!(at(only_std.len() - 1).prompt, "find faults");
        for total in 0..=full.len() + 2 {
            assert!(tail(&at(total)) <= total, "{total}");
        }
    }

    fn plan(domains: &[&str], pack: Option<&str>, limits: Limits) -> Plan {
        let (sel, _) = select(&map(), &doms(domains));
        let pack = pack.map(|p| Text::Loaded(p.to_owned()));
        Plan::new("find faults", pack.as_ref(), &sel, &texts(), limits).expect("plans")
    }

    #[test]
    fn a_plan_steps_down_in_drop_order_to_the_bare_lens() {
        let p = plan(&["rust", "terminal"], Some("brief"), Limits::default());
        let full = build(&["rust", "terminal"], Some("brief"), Limits::default());
        assert_eq!(p.render(0), full.prompt);
        assert_eq!(p.steps(), 4);
        assert_eq!(
            p.left_out(p.steps()),
            vec![
                "knowledge/terminal.md",
                "knowledge/rust.md",
                DOMAIN_PACK,
                "standards/rust.md"
            ]
        );
        for k in 0..=p.steps() {
            assert_eq!(p.render(k).len(), p.bytes(k), "step {k}");
        }
        assert!((0..p.steps()).all(|k| p.bytes(k) > p.bytes(k + 1)));
        assert_eq!(p.render(p.steps()), "find faults");
        assert_eq!(p.bytes(99), "find faults".len());
        let flat = plan(&[], None, Limits::default());
        assert_eq!((flat.steps(), flat.render(0).as_str()), (0, "find faults"));
    }

    #[test]
    fn a_plan_keeps_what_the_limits_dropped_apart_from_its_steps() {
        let limits = Limits {
            file_bytes: 20,
            ..Limits::default()
        };
        let p = plan(&["rust"], Some("brief"), limits);
        assert_eq!(p.dropped(), ["standards/rust.md".to_owned()]);
        assert_eq!(p.left_out(9), vec!["knowledge/rust.md", DOMAIN_PACK]);
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
