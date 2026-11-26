//! The Factory catalog adapter for lens composition (#723): reads the
//! domain map, every file it names, and the target repo's `DOMAIN.md`,
//! once at open, so a bad catalog fails before any call. Composing is then
//! the pure [`orch_lens::compose`] over what was read.
//!
//! Every read is confined through [`rusty_confined_fs::open_for_read`]:
//! the path is opened beneath its root (the Factory directory, or the
//! target repo for `DOMAIN.md`) without following a symlink anywhere below
//! it, and the opened handle must be a regular file, so no check can be
//! raced between test and use. Each file is read only up to its byte cap;
//! a larger file is refused, not truncated. A missing root reads as if
//! every file under it were missing.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Read as _;
use std::path::Path;

use orch_lens::compose::{self, Composed, DomainMap, Limits, Selection};

/// The domain map's file name, at the catalog root.
pub const MAP: &str = "domains.json";
/// The domain pack's file name, at the target repo root.
pub const DOMAIN_PACK: &str = "DOMAIN.md";
/// The most bytes read from the domain map.
pub const MAP_BYTES: usize = 64 * 1024;

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
        let map = match read(factory, MAP, MAP_BYTES, Need::Optional)? {
            Some(text) => compose::parse_map(&text)
                .map_err(|e| format!("{}: {e}", factory.join(MAP).display()))?,
            None => DomainMap::new(),
        };
        let texts = read_all(factory, &map, limits.file_bytes)?;
        let domain_pack = read(repo, DOMAIN_PACK, limits.file_bytes, Need::Optional)?;
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

fn read_all(
    factory: &Path,
    map: &DomainMap,
    cap: usize,
) -> Result<BTreeMap<String, String>, String> {
    let paths: BTreeSet<&String> = map
        .values()
        .flat_map(|p| p.knowledge.iter().chain(&p.standards))
        .collect();
    let mut texts = BTreeMap::new();
    for rel in paths {
        if let Some(text) = read(factory, rel, cap, Need::Required)? {
            texts.insert(rel.clone(), text);
        }
    }
    Ok(texts)
}

/// Whether a missing file is an error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Need {
    Optional,
    Required,
}

/// Read `root/rel` if it is a regular file reached without a symlink
/// below `root`, holding at most `cap` bytes of UTF-8. A missing file, or a
/// missing root, is `None` when `need` allows it.
fn read(root: &Path, rel: &str, cap: usize, need: Need) -> Result<Option<String>, String> {
    let shown = root.join(rel);
    let shown = shown.display();
    let file = match rusty_confined_fs::open_for_read(root, Path::new(rel)) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound && need == Need::Optional => {
            return Ok(None)
        }
        Err(e) => return Err(format!("cannot open {shown}: {e}")),
    };
    let mut bytes = Vec::new();
    file.take(cap as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("cannot read {shown}: {e}"))?;
    if bytes.len() > cap {
        return Err(format!("{shown}: over the {cap}-byte cap"));
    }
    String::from_utf8(bytes)
        .map(Some)
        .map_err(|_| format!("{shown}: not UTF-8"))
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
    fn an_oversized_input_is_refused_before_loading() {
        let f = factory("big");
        let limits = Limits {
            file_bytes: 10,
            ..Limits::default()
        };
        let err = Catalog::open(&f, &scratch("big-repo"), limits).unwrap_err();
        assert!(err.contains("over the 10-byte cap"), "{err}");
        let repo = scratch("big-pack");
        write(&repo, DOMAIN_PACK, "A domain digest well over ten bytes.");
        let err = Catalog::open(&scratch("big-empty"), &repo, limits).unwrap_err();
        assert!(err.contains(DOMAIN_PACK) && err.contains("cap"), "{err}");
    }

    #[test]
    fn a_directory_is_not_a_regular_file() {
        let f = factory("dir");
        std::fs::remove_file(f.join("standards/rust.md")).expect("rm");
        std::fs::create_dir_all(f.join("standards/rust.md")).expect("dir");
        let err = Catalog::open(&f, &scratch("dir-repo"), Limits::default()).unwrap_err();
        assert!(err.contains("not a regular file"), "{err}");
        let repo = scratch("dir-pack");
        std::fs::create_dir_all(repo.join(DOMAIN_PACK)).expect("dir");
        let err = Catalog::open(&scratch("dir-empty"), &repo, Limits::default()).unwrap_err();
        assert!(err.contains("not a regular file"), "{err}");
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_are_refused_for_every_input() {
        use std::os::unix::fs::symlink;
        let outside = scratch("link-outside");
        write(&outside, "secret.md", "not for lenses");
        write(&outside, "rust.md", "outside");
        write(&outside, MAP, "{}");
        let refused = |err: &str, rel: &str| {
            assert!(err.contains(rel) && err.contains("cannot open"), "{err}");
        };
        // A named file that is a symlink out of the catalog.
        let f = factory("link-file");
        std::fs::remove_file(f.join("standards/rust.md")).expect("rm");
        symlink(outside.join("secret.md"), f.join("standards/rust.md")).expect("link");
        let err = Catalog::open(&f, &scratch("link-repo"), Limits::default()).unwrap_err();
        refused(&err, "standards/rust.md");
        // A symlinked parent directory: the file under it is regular.
        let f = factory("link-dir");
        std::fs::remove_dir_all(f.join("standards")).expect("rm");
        symlink(&outside, f.join("standards")).expect("link");
        let err = Catalog::open(&f, &scratch("link-dir-repo"), Limits::default()).unwrap_err();
        refused(&err, "standards/rust.md");
        // The map and the domain pack.
        let f = scratch("link-map");
        symlink(outside.join(MAP), f.join(MAP)).expect("link");
        let err = Catalog::open(&f, &scratch("link-map-repo"), Limits::default()).unwrap_err();
        refused(&err, MAP);
        let repo = scratch("link-pack");
        symlink(outside.join("secret.md"), repo.join(DOMAIN_PACK)).expect("link");
        let err = Catalog::open(&scratch("link-pack-f"), &repo, Limits::default()).unwrap_err();
        refused(&err, DOMAIN_PACK);
    }

    #[test]
    fn a_missing_root_reads_as_absent_files() {
        let gone = scratch("gone").join("nowhere");
        let repo = scratch("gone-repo");
        write(&repo, DOMAIN_PACK, "Dedup 20 TB.");
        let c = Catalog::open(&gone, &repo, Limits::default()).expect("no factory is no map");
        let b = c.build("find faults", &doms(&["rust"])).expect("builds");
        assert_eq!(
            b.composed.prompt,
            "find faults\n\n# Project\nDedup 20 TB.\n"
        );
        let c = Catalog::open(&factory("gone-f"), &gone, Limits::default())
            .expect("no repo is no pack");
        let b = c.build("find faults", &doms(&["rust"])).expect("builds");
        assert!(!b.composed.prompt.contains("# Project"));
        assert!(b.composed.prompt.contains("MUST: no unwrap."));
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
