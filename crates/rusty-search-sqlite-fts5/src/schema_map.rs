use std::collections::HashMap;

use rusty_search_core::{FieldType as CoreFieldType, Schema as CoreSchema, SearchError};
use rusty_sqlite::rusqlite::Connection;
use rusty_sqlite::{Fts5TableBuilder, Fts5Tokenizer};

/// Per-field metadata kept alongside the physical SQL tables, so query and
/// row-conversion logic know a field's core type, whether it round-trips
/// back out in search hits, and whether it has an FTS5 column to search
/// against without re-deriving any of that from `sqlite_master` later.
#[derive(Debug, Clone, Copy)]
pub struct FieldMeta {
    pub field_type: CoreFieldType,
    pub stored: bool,
    /// Whether this field has a column in the FTS5 index (only ever true
    /// for `Text` fields created with `indexed: true`).
    pub fts_indexed: bool,
}

/// Quotes a SQL identifier (table/column name) so schema field names that
/// collide with SQL keywords, or contain unusual characters, are always
/// safe to splice into generated SQL text.
pub fn quote_ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

fn column_affinity(field_type: CoreFieldType) -> &'static str {
    match field_type {
        CoreFieldType::Text | CoreFieldType::Keyword | CoreFieldType::Date => "TEXT",
        CoreFieldType::I64 | CoreFieldType::Bool => "INTEGER",
        CoreFieldType::F64 => "REAL",
    }
}

/// Creates the `content` table (one real, typed column per schema field,
/// plus `rowid`/`_id`) and, if the schema has any indexed `Text` fields, the
/// `idx_fts` FTS5 virtual table shadowing them - mirroring
/// `rusty-search-tantivy::schema_map::build_tantivy_schema`'s role of
/// translating a backend-agnostic [`CoreSchema`] into the concrete engine's
/// own schema, once per index.
pub fn create_tables(
    conn: &Connection,
    schema: &CoreSchema,
) -> Result<HashMap<String, FieldMeta>, SearchError> {
    let mut fields = HashMap::new();
    let mut content_columns = Vec::new();
    let mut fts_columns = Vec::new();

    for def in &schema.fields {
        let fts_indexed = def.field_type == CoreFieldType::Text && def.options.indexed;
        fields.insert(
            def.name.clone(),
            FieldMeta {
                field_type: def.field_type,
                stored: def.options.stored,
                fts_indexed,
            },
        );

        content_columns.push(format!(
            "{} {}",
            quote_ident(&def.name),
            column_affinity(def.field_type)
        ));
        if fts_indexed {
            fts_columns.push(quote_ident(&def.name));
        }
    }

    let create_content = format!(
        "CREATE TABLE content (rowid INTEGER PRIMARY KEY, _id TEXT UNIQUE NOT NULL{}{})",
        if content_columns.is_empty() { "" } else { ", " },
        content_columns.join(", ")
    );
    conn.execute(&create_content, []).map_err(backend_err)?;

    if !fts_columns.is_empty() {
        let mut builder = Fts5TableBuilder::new("idx_fts").tokenizer(Fts5Tokenizer::Unicode61);
        for def in &schema.fields {
            if fields[&def.name].fts_indexed {
                builder = builder.column(def.name.as_str());
            }
        }
        builder.create(conn).map_err(rusty_sqlite_err)?;
    }

    for def in &schema.fields {
        if def.field_type != CoreFieldType::Text && def.options.indexed {
            let index_sql = format!(
                "CREATE INDEX {} ON content({})",
                quote_ident(&format!("idx_{}", def.name)),
                quote_ident(&def.name)
            );
            conn.execute(&index_sql, []).map_err(backend_err)?;
        }
    }

    Ok(fields)
}

fn backend_err(e: rusty_sqlite::rusqlite::Error) -> SearchError {
    SearchError::Backend(anyhow::Error::new(e))
}

fn rusty_sqlite_err(e: rusty_sqlite::Error) -> SearchError {
    SearchError::Backend(anyhow::Error::new(e))
}
