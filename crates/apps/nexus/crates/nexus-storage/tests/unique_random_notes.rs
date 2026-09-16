//! RFC 0009 — engine-level contract for the unique-note and random-note
//! features ported from `nexus_forge`.

use std::collections::HashSet;

use nexus_storage::unique_note::UniqueNoteOptions;
use nexus_storage::{StorageEngine, StorageError};

fn fixed_options(folder: Option<&str>) -> UniqueNoteOptions {
    UniqueNoteOptions {
        // A constant template makes every call collide, which is exactly
        // what the suffix test wants.
        id_format: "20260908".to_string(),
        separator: " ".to_string(),
        folder: folder.map(str::to_string),
    }
}

#[test]
fn unique_note_is_created_indexed_and_suffixed_on_collision() {
    let dir = tempfile::tempdir().expect("tempdir");
    let engine = StorageEngine::init(dir.path()).expect("init forge");

    let first = engine
        .create_unique_note(&fixed_options(Some("zettel")), "My: Idea?")
        .expect("first create");
    assert_eq!(first, "zettel/20260908 My Idea.md");
    assert!(dir.path().join(&first).exists());
    assert!(engine.file_exists(&first).expect("exists"));

    let body = std::fs::read_to_string(dir.path().join(&first)).expect("read body");
    assert_eq!(body, "# My: Idea?\n\n");

    let second = engine
        .create_unique_note(&fixed_options(Some("zettel")), "My: Idea?")
        .expect("second create");
    assert_eq!(second, "zettel/20260908 My Idea-2.md");

    let third = engine
        .create_unique_note(&fixed_options(Some("zettel")), "My: Idea?")
        .expect("third create");
    assert_eq!(third, "zettel/20260908 My Idea-3.md");
}

#[test]
fn unique_note_with_empty_title_uses_id_only_and_root_folder() {
    let dir = tempfile::tempdir().expect("tempdir");
    let engine = StorageEngine::init(dir.path()).expect("init forge");

    let path = engine
        .create_unique_note(&fixed_options(None), "   ")
        .expect("create");
    assert_eq!(path, "20260908.md");
    let body = std::fs::read_to_string(dir.path().join(&path)).expect("read body");
    assert_eq!(body, "# 20260908\n\n");
}

#[test]
fn unique_note_rejects_invalid_id_format() {
    let dir = tempfile::tempdir().expect("tempdir");
    let engine = StorageEngine::init(dir.path()).expect("init forge");

    let options = UniqueNoteOptions {
        id_format: "%Q".to_string(),
        ..UniqueNoteOptions::default()
    };
    let err = engine
        .create_unique_note(&options, "x")
        .expect_err("invalid template must fail");
    assert!(matches!(err, StorageError::ConfigInvalid(_)), "{err}");
}

#[test]
fn unique_note_default_options_produce_timestamp_id() {
    let dir = tempfile::tempdir().expect("tempdir");
    let engine = StorageEngine::init(dir.path()).expect("init forge");

    let path = engine
        .create_unique_note(&UniqueNoteOptions::default(), "Hello")
        .expect("create");
    let (id, rest) = path
        .split_once(' ')
        .expect("id and title separated by space");
    assert_eq!(id.len(), 14);
    assert!(id.chars().all(|c| c.is_ascii_digit()));
    assert_eq!(rest, "Hello.md");
}

#[test]
fn random_note_excludes_active_and_covers_all_candidates() {
    let dir = tempfile::tempdir().expect("tempdir");
    let engine = StorageEngine::init(dir.path()).expect("init forge");

    assert_eq!(engine.random_note_path(None, None).expect("empty"), None);

    for name in ["a", "b", "c"] {
        engine
            .write_file(&format!("notes/{name}.md"), b"# n\n")
            .expect("write");
    }
    engine
        .write_file("notes/image.png", b"\x89PNG\r\n")
        .expect("write attachment");

    let mut seen = HashSet::new();
    for _ in 0..200 {
        let picked = engine
            .random_note_path(Some("notes/a.md"), None)
            .expect("random")
            .expect("has candidates");
        assert_ne!(picked, "notes/a.md");
        assert!(
            picked.ends_with(".md"),
            "attachments are never drawn: {picked}"
        );
        seen.insert(picked);
    }
    assert_eq!(
        seen.len(),
        2,
        "both non-excluded notes should be drawn: {seen:?}"
    );

    assert_eq!(
        engine
            .random_note_path(Some("notes/a.md"), Some("notes/a"))
            .expect("prefix draw"),
        None,
        "prefix that only matches the excluded note yields nothing"
    );
}
