#![allow(clippy::unwrap_used)]

use super::*;

fn file(pack: &str, path: &str, text: &str) -> (String, String, Vec<u8>) {
    (pack.to_owned(), path.to_owned(), text.as_bytes().to_vec())
}

fn catalog() -> SkillCatalog {
    SkillCatalog::from_files([
        file("rust", "knowledge/rust-expert.md", "# Rust"),
        file("rust", "standards/rust.md", "# Rust standards"),
        file("terminal", "knowledge/terminal-emulator-expert.md", "# VT"),
    ])
    .unwrap()
}

fn packs(names: &[&str]) -> Vec<String> {
    names.iter().map(|n| (*n).to_owned()).collect()
}

fn uris(c: &SkillCatalog, allowed: &[&str]) -> Vec<String> {
    c.list(&packs(allowed)).into_iter().map(|e| e.uri).collect()
}

#[test]
fn a_lens_lists_only_its_packs() {
    let c = catalog();
    assert_eq!(
        uris(&c, &["rust"]),
        [
            "skill://rust/knowledge/rust-expert.md",
            "skill://rust/standards/rust.md"
        ]
    );
    assert_eq!(
        uris(&c, &["terminal", "rust"]).len(),
        3,
        "order of the allowed list is irrelevant"
    );
    assert!(uris(&c, &[]).is_empty(), "no packs, no skills");
}

#[test]
fn a_listing_carries_the_packs_digest_and_size() {
    let c = catalog();
    let allowed = packs(&["rust"]);
    let entries = c.list(&allowed);
    let first = entries.first().unwrap();
    assert_eq!(first.pack, "rust");
    assert_eq!(first.path, "knowledge/rust-expert.md");
    assert_eq!(first.size, "# Rust".len());
    assert_eq!(Some(first.digest), c.digest("rust"));
    assert_eq!(first.digest.len(), 64);
}

#[test]
fn a_read_outside_the_lens_is_not_found() {
    let c = catalog();
    let rust = packs(&["rust"]);
    assert_eq!(
        c.read("skill://rust/knowledge/rust-expert.md", &rust),
        Some("# Rust")
    );
    assert_eq!(
        c.read(
            "skill://terminal/knowledge/terminal-emulator-expert.md",
            &rust
        ),
        None
    );
    assert_eq!(c.read("skill://rust/knowledge/nope.md", &rust), None);
    assert_eq!(c.read("skill://rust", &rust), None);
    assert_eq!(c.read("file:///etc/passwd", &rust), None);
    assert_eq!(c.read("skill://rust/../terminal/x", &rust), None);
}

#[test]
fn the_digest_follows_the_contents_and_nothing_else() {
    let a = catalog();
    let reordered = SkillCatalog::from_files([
        file("terminal", "knowledge/terminal-emulator-expert.md", "# VT"),
        file("rust", "standards/rust.md", "# Rust standards"),
        file("rust", "knowledge/rust-expert.md", "# Rust"),
    ])
    .unwrap();
    assert_eq!(a, reordered, "load order changes nothing");

    let edited = SkillCatalog::from_files([
        file("rust", "knowledge/rust-expert.md", "# Rust!"),
        file("rust", "standards/rust.md", "# Rust standards"),
    ])
    .unwrap();
    assert_ne!(a.digest("rust"), edited.digest("rust"), "an edit");

    let renamed = SkillCatalog::from_files([
        file("rust", "knowledge/rust.md", "# Rust"),
        file("rust", "standards/rust.md", "# Rust standards"),
    ])
    .unwrap();
    assert_ne!(a.digest("rust"), renamed.digest("rust"), "a rename");

    let dropped =
        SkillCatalog::from_files([file("rust", "knowledge/rust-expert.md", "# Rust")]).unwrap();
    assert_ne!(a.digest("rust"), dropped.digest("rust"), "a removal");
    assert_eq!(a.digest("nope"), None);
}

#[test]
fn packs_the_lens_names_but_the_catalog_lacks_are_reported() {
    let c = catalog();
    let allowed = packs(&["rust", "python", "go"]);
    assert_eq!(c.missing(&allowed), ["python", "go"]);
    assert_eq!(uris(&c, &["python"]), Vec::<String>::new());
}

#[test]
fn a_bad_file_refuses_the_whole_catalog() {
    let refused = |pack: &str, path: &str, bytes: Vec<u8>| {
        SkillCatalog::from_files([
            file("rust", "ok.md", "fine"),
            (pack.to_owned(), path.to_owned(), bytes),
        ])
        .unwrap_err()
    };
    for path in [
        "",
        "/etc/passwd",
        "../x.md",
        "a/../b.md",
        "./a.md",
        "a//b.md",
        "a b.md",
        "a\\b.md",
    ] {
        let err = refused("rust", path, b"x".to_vec());
        assert!(err.reason.contains("path"), "{path:?}: {err}");
    }
    for pack in ["", ".", "..", "a/b", "a b"] {
        let err = refused(pack, "x.md", b"x".to_vec());
        assert!(err.reason.contains("pack name"), "{pack:?}: {err}");
    }
    assert_eq!(
        refused("rust", "bin.md", vec![0xff, 0xfe]).reason,
        "not UTF-8"
    );
    assert_eq!(
        refused("rust", "ok.md", b"again".to_vec()).reason,
        "listed twice"
    );

    let big = vec![b'a'; MAX_SKILL_FILE_BYTES + 1];
    assert!(refused("rust", "big.md", big).reason.contains("limit"));
    let at_limit = vec![b'a'; MAX_SKILL_FILE_BYTES];
    assert!(SkillCatalog::from_files([("rust".into(), "max.md".into(), at_limit)]).is_ok());
}

#[test]
fn an_empty_catalog_serves_nothing() {
    let c = SkillCatalog::from_files([]).unwrap();
    assert!(c.is_empty());
    assert!(uris(&c, &["rust"]).is_empty());
    assert_eq!(c.len(), 0);
    assert_eq!(catalog().len(), 2);
}
