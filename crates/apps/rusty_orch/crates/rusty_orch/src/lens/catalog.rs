//! The Factory catalog adapter for lens composition (#723): reads the
//! domain map, every file it names, and the target repo's `DOMAIN.md`,
//! once at open, so a bad catalog fails before any call. Composing is then
//! the pure [`orch_lens::compose`] over what was read.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use orch_lens::compose::{self, Composed, DomainMap, Limits, Selection};

/// The domain map's file name, at the catalog root.
pub const MAP: &str = "domains.json";
/// The domain pack's file name, at the target repo root.
pub const DOMAIN_PACK: &str = "DOMAIN.md";

/// A loaded catalog: the map, the files it names, and the domain pack.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Catalog {
    map: DomainMap,
    texts: BTreeMap<String, String>,
    domain_pack: Option<String>,
    limits: Limits,
}

/// One composed lens prompt, with what selected it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Built {
    pub composed: Composed,
    /// The prefix cache key: equal selections give equal bytes.
    pub selection: Selection,
    /// Claim domains the map does not know.
    pub unknown: BTreeSet<String>,
}

impl Catalog {
    /// Read `factory/domains.json` and the files it names, and
    /// `repo/DOMAIN.md`. Either file may be absent: no map selects
    /// nothing, and no pack composes no project section.
    pub fn open(factory: &Path, repo: &Path, limits: Limits) -> Result<Self, String> {
        let map = match optional(&factory.join(MAP))? {
            Some(text) => compose::parse_map(&text)
                .map_err(|e| format!("{}: {e}", factory.join(MAP).display()))?,
            None => DomainMap::new(),
        };
        let texts = read_all(factory, &map)?;
        let domain_pack = optional(&repo.join(DOMAIN_PACK))?;
        Ok(Self {
            map,
            texts,
            domain_pack,
            limits,
        })
    }

    /// Compose `lens_prompt` for a question touching `domains`.
    pub fn build(&self, lens_prompt: &str, domains: &[String]) -> Result<Built, String> {
        let (selection, unknown) = compose::select(&self.map, domains);
        let composed = compose::compose(
            lens_prompt,
            self.domain_pack.as_deref(),
            &selection,
            &self.texts,
            self.limits,
        )
        .map_err(|e| e.to_string())?;
        Ok(Built {
            composed,
            selection,
            unknown,
        })
    }
}

fn read_all(factory: &Path, map: &DomainMap) -> Result<BTreeMap<String, String>, String> {
    let paths: BTreeSet<&String> = map
        .values()
        .flat_map(|p| p.knowledge.iter().chain(&p.standards))
        .collect();
    paths
        .into_iter()
        .map(|rel| {
            let path = factory.join(rel);
            let text = std::fs::read_to_string(&path)
                .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
            Ok((rel.clone(), text))
        })
        .collect()
}

fn optional(path: &Path) -> Result<Option<String>, String> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("cannot read {}: {e}", path.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("orch-catalog-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        dir
    }

    fn write(dir: &Path, rel: &str, text: &str) {
        let path = dir.join(rel);
        std::fs::create_dir_all(path.parent().expect("parent")).expect("dir");
        std::fs::write(path, text).expect("write");
    }

    fn factory(name: &str) -> PathBuf {
        let f = scratch(name);
        write(
            &f,
            MAP,
            r#"{"rust": {"knowledge": ["knowledge/rust.md"], "standards": ["standards/rust.md"]},
                "terminal": {"knowledge": ["knowledge/terminal.md"]}}"#,
        );
        write(&f, "knowledge/rust.md", "Ownership first.");
        write(&f, "standards/rust.md", "MUST: no unwrap.");
        write(&f, "knowledge/terminal.md", "VT is a state machine.");
        f
    }

    fn doms(d: &[&str]) -> Vec<String> {
        d.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn two_domains_build_two_contexts_from_one_catalog() {
        let f = factory("two");
        let repo = scratch("two-repo");
        write(&repo, DOMAIN_PACK, "Dedup 20 TB.");
        let c = Catalog::open(&f, &repo, Limits::default()).expect("opens");
        let rust = c.build("find faults", &doms(&["rust"])).expect("builds");
        let term = c
            .build("find faults", &doms(&["terminal", "gui"]))
            .expect("builds");
        assert_ne!(rust.composed.prompt, term.composed.prompt);
        assert_ne!(rust.selection, term.selection);
        assert!(rust.composed.prompt.contains("# Project\nDedup 20 TB.\n"));
        assert!(term.composed.prompt.contains("VT is a state machine."));
        assert_eq!(term.unknown, BTreeSet::from(["gui".to_owned()]));
    }

    #[test]
    fn an_empty_catalog_and_repo_leave_the_prompt_flat() {
        let c = Catalog::open(&scratch("empty"), &scratch("empty-repo"), Limits::default())
            .expect("opens");
        let b = c.build("find faults", &doms(&["rust"])).expect("builds");
        assert_eq!(b.composed.prompt, "find faults");
    }

    #[test]
    fn a_file_the_map_names_must_exist_at_open() {
        let f = factory("missing");
        std::fs::remove_file(f.join("standards/rust.md")).expect("rm");
        let err = Catalog::open(&f, &scratch("missing-repo"), Limits::default()).unwrap_err();
        assert!(err.contains("standards/rust.md"), "{err}");
    }

    #[test]
    fn a_bad_map_names_its_file() {
        let f = scratch("bad");
        write(&f, MAP, r#"{"rust": {"knowledge": ["../x.md"]}}"#);
        let err = Catalog::open(&f, &scratch("bad-repo"), Limits::default()).unwrap_err();
        assert!(
            err.contains(MAP) && err.contains("inside the catalog"),
            "{err}"
        );
    }
}
