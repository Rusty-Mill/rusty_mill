//! RFC 0009 — engine-level contract for note merge and create-from-title
//! (ported from `nexus_forge`'s note-composer plugin).

use nexus_storage::note_composer::MERGE_SEPARATOR;
use nexus_storage::{DeleteDestination, FileFilter, StorageEngine, StorageError};

fn write(engine: &StorageEngine, path: &str, content: &str) {
    engine
        .write_file(path, content.as_bytes())
        .expect("write_file");
}

fn read(dir: &std::path::Path, path: &str) -> String {
    std::fs::read_to_string(dir.join(path)).expect("read")
}

#[test]
fn merge_appends_redirects_links_and_trashes_source() {
    let dir = tempfile::tempdir().expect("tempdir");
    let engine = StorageEngine::init(dir.path()).expect("init forge");

    write(&engine, "notes/target.md", "# Target\n\nkeep\n");
    write(&engine, "notes/source.md", "# Source\n\nabsorb me\n");
    write(
        &engine,
        "notes/ref.md",
        "See [[source]] and [[source|alias]].\n",
    );
    write(&engine, "notes/other.md", "Unrelated [[target]].\n");

    let outcome = engine
        .merge_notes(
            "notes/source.md",
            "notes/target.md",
            true,
            DeleteDestination::ForgeTrash,
        )
        .expect("merge");

    assert_eq!(
        read(dir.path(), "notes/target.md"),
        format!("# Target\n\nkeep{MERGE_SEPARATOR}# Source\n\nabsorb me\n")
    );
    assert_eq!(
        read(dir.path(), "notes/ref.md"),
        "See [[target]] and [[target|alias]].\n"
    );
    assert_eq!(
        read(dir.path(), "notes/other.md"),
        "Unrelated [[target]].\n"
    );
    assert_eq!((outcome.files_rewritten, outcome.links_updated), (1, 2));

    assert!(!dir.path().join("notes/source.md").exists());
    let trash_id = outcome.trash_id.expect("forge trash returns a bucket id");
    assert!(dir
        .path()
        .join(".trash")
        .join(&trash_id)
        .join("notes/source.md")
        .exists());
    // Trash soft-deletes the index row, so the source drops out of the
    // visible listing (`include_deleted: false`).
    let visible: Vec<String> = engine
        .query_files(&FileFilter {
            prefix: Some("notes/".to_string()),
            file_type: None,
            include_deleted: false,
        })
        .expect("query_files")
        .into_iter()
        .map(|r| r.path)
        .collect();
    assert!(
        !visible.contains(&"notes/source.md".to_string()),
        "{visible:?}"
    );
    assert!(visible.contains(&"notes/target.md".to_string()));
}

#[test]
fn merge_without_link_update_leaves_references_alone() {
    let dir = tempfile::tempdir().expect("tempdir");
    let engine = StorageEngine::init(dir.path()).expect("init forge");

    write(&engine, "a.md", "A\n");
    write(&engine, "b.md", "B\n");
    write(&engine, "ref.md", "[[a]]\n");

    let outcome = engine
        .merge_notes("a.md", "b.md", false, DeleteDestination::Permanent)
        .expect("merge");
    assert_eq!((outcome.files_rewritten, outcome.links_updated), (0, 0));
    assert_eq!(outcome.trash_id, None);
    assert_eq!(read(dir.path(), "ref.md"), "[[a]]\n");
    assert!(!dir.path().join("a.md").exists());
    assert!(!dir.path().join(".trash").exists());
}

#[test]
fn merge_rejects_self_and_missing_notes() {
    let dir = tempfile::tempdir().expect("tempdir");
    let engine = StorageEngine::init(dir.path()).expect("init forge");
    write(&engine, "a.md", "A\n");

    let err = engine
        .merge_notes("a.md", "a.md", true, DeleteDestination::ForgeTrash)
        .expect_err("self merge");
    assert!(matches!(err, StorageError::ConfigInvalid(_)), "{err}");

    let err = engine
        .merge_notes("missing.md", "a.md", true, DeleteDestination::ForgeTrash)
        .expect_err("missing source");
    assert!(matches!(err, StorageError::FileNotFound(_)), "{err}");
    assert_eq!(
        read(dir.path(), "a.md"),
        "A\n",
        "target untouched on failure"
    );
}

#[test]
fn create_from_title_writes_body_and_refuses_overwrite() {
    let dir = tempfile::tempdir().expect("tempdir");
    let engine = StorageEngine::init(dir.path()).expect("init forge");

    let path = engine
        .create_note_from_title("Split: Part?", Some("inbox"), "extracted text\n")
        .expect("create");
    assert_eq!(path, "inbox/Split Part.md");
    assert_eq!(read(dir.path(), &path), "extracted text\n");
    assert!(engine.file_exists(&path).expect("indexed"));

    let err = engine
        .create_note_from_title("Split: Part?", Some("inbox"), "other\n")
        .expect_err("second create must fail");
    assert!(
        matches!(&err, StorageError::WriteFailed { reason, .. } if reason == "already exists"),
        "{err}"
    );
    assert_eq!(
        read(dir.path(), &path),
        "extracted text\n",
        "not overwritten"
    );

    let err = engine
        .create_note_from_title("???", None, "")
        .expect_err("empty stem");
    assert!(matches!(err, StorageError::ConfigInvalid(_)), "{err}");
}
