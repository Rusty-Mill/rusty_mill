//! RFC 0009 — engine-level contract for typed frontmatter properties
//! (ported from `nexus_forge`'s properties panel / properties view).

use nexus_storage::properties::{PropertyFilter, PropertyType};
use nexus_storage::StorageEngine;
use serde_json::json;

fn write(engine: &StorageEngine, path: &str, content: &str) {
    engine
        .write_file(path, content.as_bytes())
        .expect("write_file");
}

fn seed(engine: &StorageEngine) {
    write(
        engine,
        "notes/a.md",
        "---\ntitle: Alpha\npriority: 2\ndue: 2026-09-10\ndone: false\ntags: [x, y]\n---\nA\n",
    );
    write(
        engine,
        "notes/b.md",
        "---\npriority: high\ndue: 2026-09-11\n---\nB\n",
    );
    write(engine, "notes/c.md", "no frontmatter\n");
}

#[test]
fn schema_infers_from_index_and_layers_overrides() {
    let dir = tempfile::tempdir().expect("tempdir");
    let engine = StorageEngine::init(dir.path()).expect("init forge");
    seed(&engine);

    let schema = engine.property_schema().expect("schema");
    assert_eq!(schema.inferred["title"], PropertyType::Text);
    assert_eq!(schema.inferred["due"], PropertyType::Date);
    assert_eq!(schema.inferred["done"], PropertyType::Boolean);
    assert_eq!(schema.inferred["tags"], PropertyType::Tags);
    // `priority` is a number in one note and text in another → widened.
    assert_eq!(schema.inferred["priority"], PropertyType::Text);
    assert!(schema.overrides.is_empty());

    engine
        .set_property_type_override("priority", Some(PropertyType::Number))
        .expect("set override");
    let schema = engine.property_schema().expect("schema");
    assert_eq!(schema.overrides["priority"], PropertyType::Number);
    assert_eq!(schema.effective("priority"), Some(PropertyType::Number));
    assert!(dir.path().join(".forge/app.toml").exists());

    engine
        .set_property_type_override("priority", None)
        .expect("clear override");
    assert!(engine
        .property_schema()
        .expect("schema")
        .overrides
        .is_empty());
}

#[test]
fn note_properties_reads_typed_rows_and_set_round_trips() {
    let dir = tempfile::tempdir().expect("tempdir");
    let engine = StorageEngine::init(dir.path()).expect("init forge");
    seed(&engine);

    let rows = engine.note_properties("notes/a.md").expect("rows");
    let keys: Vec<&str> = rows.iter().map(|r| r.key.as_str()).collect();
    assert_eq!(keys, ["title", "priority", "due", "done", "tags"]);
    let done = rows.iter().find(|r| r.key == "done").expect("done row");
    assert_eq!(
        (done.property_type, &done.value),
        (PropertyType::Boolean, &json!(false))
    );

    engine
        .set_note_property("notes/a.md", "done", Some(&json!(true)), None)
        .expect("set done");
    engine
        .set_note_property("notes/a.md", "tags", Some(&json!("x, z")), None)
        .expect("set tags");
    engine
        .set_note_property("notes/a.md", "priority", None, None)
        .expect("remove priority");

    let text = std::fs::read_to_string(dir.path().join("notes/a.md")).expect("read");
    assert!(text.starts_with("---\n"), "{text}");
    assert!(text.contains("done: true\n"), "{text}");
    assert!(text.contains("tags:\n- x\n- z\n"), "{text}");
    assert!(!text.contains("priority"), "{text}");
    assert!(text.ends_with("---\nA\n"), "{text}");

    // The write went through the index: the schema no longer sees the key
    // on this note and the bulk table reflects the new values.
    let page = engine
        .list_note_properties(&PropertyFilter {
            key: Some("done".to_string()),
            value: Some("true".to_string()),
            ..PropertyFilter::default()
        })
        .expect("list");
    assert_eq!(page.total, 1);
    assert_eq!(page.rows[0].path, "notes/a.md");
    assert_eq!(page.rows[0].properties["tags"], json!(["x", "z"]));

    assert!(engine
        .note_properties("notes/c.md")
        .expect("no block")
        .is_empty());
    engine
        .set_note_property("notes/c.md", "status", Some(&json!("draft")), None)
        .expect("create block");
    let text = std::fs::read_to_string(dir.path().join("notes/c.md")).expect("read");
    assert_eq!(text, "---\nstatus: draft\n---\nno frontmatter\n");
}

#[test]
fn list_paginates_with_stable_columns_and_titles() {
    let dir = tempfile::tempdir().expect("tempdir");
    let engine = StorageEngine::init(dir.path()).expect("init forge");
    seed(&engine);
    engine
        .set_property_type_override("owner", Some(PropertyType::Link))
        .expect("declare a key no note has yet");

    let page = engine
        .list_note_properties(&PropertyFilter::default())
        .expect("list");
    assert_eq!(page.total, 2, "notes without frontmatter are skipped");
    assert_eq!(
        page.columns,
        ["done", "due", "owner", "priority", "tags", "title"]
    );
    assert_eq!(page.rows[0].title, "Alpha");
    assert_eq!(page.rows[1].title, "b", "falls back to the stem");
    assert_eq!(page.rows[0].properties["priority"], json!(2));
    assert_eq!(page.rows[1].properties["priority"], json!("high"));

    let first = engine
        .list_note_properties(&PropertyFilter {
            limit: Some(1),
            ..PropertyFilter::default()
        })
        .expect("page 1");
    let second = engine
        .list_note_properties(&PropertyFilter {
            limit: Some(1),
            offset: Some(1),
            ..PropertyFilter::default()
        })
        .expect("page 2");
    assert_eq!((first.total, first.rows.len()), (2, 1));
    assert_eq!(second.rows[0].path, "notes/b.md");
    assert_eq!(first.columns, second.columns);

    let filtered = engine
        .list_note_properties(&PropertyFilter {
            key: Some("tags".to_string()),
            ..PropertyFilter::default()
        })
        .expect("key filter");
    assert_eq!(filtered.total, 1);
}
