//! Typed frontmatter properties (RFC 0009 port of `nexus_forge`'s
//! `forge-plugin-properties-panel` / `forge-plugin-properties-view`).
//!
//! Three concerns live here, all reachable through `com.nexus.storage`:
//!
//! * **Schema** — a `key → PropertyType` map. The *inferred* layer comes
//!   from the index's `properties` table (its `property_type` hint and
//!   typed `value_date` / `value_bool` columns); the *override* layer is
//!   `[properties].type_overrides` in `.forge/app.toml`. Overrides win.
//! * **Per-note typed read/write** — parse the note's YAML block with
//!   `serde_norway`, project each key to JSON with its effective type,
//!   and write typed values back by re-serialising the mapping. YAML
//!   comments inside the block are not preserved (accepted trade-off,
//!   surfaced in the panel UI).
//! * **Bulk listing** — a paginated, optionally key/value-filtered table
//!   of every indexed markdown note's properties, with a stable column
//!   set for the properties view.

use std::collections::{BTreeMap, BTreeSet};

use rusqlite::params;
use serde_json::Value as Json;
use serde_norway::Value as Yaml;

use crate::{StorageEngine, StorageError};

/// Declared or inferred type for one frontmatter key. Wire form is the
/// `snake_case` name (`"date_time"`, `"boolean"`, …).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PropertyType {
    /// Free text (the widening fallback).
    Text,
    /// Integer or float.
    Number,
    /// `YYYY-MM-DD`.
    Date,
    /// `YYYY-MM-DDTHH:MM[...]`.
    DateTime,
    /// `true` / `false`.
    Boolean,
    /// Sequence of scalars.
    List,
    /// A `[[wikilink]]` string.
    Link,
    /// The `tags` sequence.
    Tags,
}

impl PropertyType {
    /// Every variant, in display order.
    pub const ALL: [PropertyType; 8] = [
        Self::Text,
        Self::Number,
        Self::Date,
        Self::DateTime,
        Self::Boolean,
        Self::List,
        Self::Link,
        Self::Tags,
    ];

    /// Wire / config name.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Number => "number",
            Self::Date => "date",
            Self::DateTime => "date_time",
            Self::Boolean => "boolean",
            Self::List => "list",
            Self::Link => "link",
            Self::Tags => "tags",
        }
    }

    /// Parse a wire / config name. Accepts a few aliases (`datetime`,
    /// `bool`) so hand-edited `app.toml` stays forgiving.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "text" | "string" => Self::Text,
            "number" => Self::Number,
            "date" => Self::Date,
            "date_time" | "datetime" => Self::DateTime,
            "boolean" | "bool" => Self::Boolean,
            "list" => Self::List,
            "link" => Self::Link,
            "tags" => Self::Tags,
            _ => return None,
        })
    }

    /// Type implied by a live JSON value when neither schema layer knows
    /// the key. `tags` is special-cased by [`infer_for`].
    #[must_use]
    pub fn from_json(value: &Json) -> Self {
        match value {
            Json::Bool(_) => Self::Boolean,
            Json::Number(_) => Self::Number,
            Json::Array(_) => Self::List,
            Json::String(s) if is_date(s) => Self::Date,
            Json::String(s) if is_date_time(s) => Self::DateTime,
            Json::String(s) if is_wikilink(s) => Self::Link,
            _ => Self::Text,
        }
    }
}

/// Inferred + declared layers. `effective(key)` is override-first.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PropertySchema {
    /// Key → type inferred from indexed values.
    pub inferred: BTreeMap<String, PropertyType>,
    /// Key → type declared in `app.toml`; wins over `inferred`.
    pub overrides: BTreeMap<String, PropertyType>,
}

impl PropertySchema {
    /// Override-first lookup; `None` when neither layer knows `key`.
    #[must_use]
    pub fn effective(&self, key: &str) -> Option<PropertyType> {
        self.overrides
            .get(key)
            .or_else(|| self.inferred.get(key))
            .copied()
    }

    /// Effective type, falling back to the shape of `value`.
    #[must_use]
    pub fn infer_for(&self, key: &str, value: &Json) -> PropertyType {
        if let Some(ty) = self.effective(key) {
            return ty;
        }
        if key == "tags" {
            return PropertyType::Tags;
        }
        PropertyType::from_json(value)
    }
}

/// One projected property of a note.
#[derive(Debug, Clone, PartialEq)]
pub struct PropertyRow {
    /// Frontmatter key.
    pub key: String,
    /// Effective type.
    pub property_type: PropertyType,
    /// JSON projection of the YAML value.
    pub value: Json,
}

/// Pagination + filter for [`StorageEngine::list_note_properties`].
#[derive(Debug, Clone, Default)]
pub struct PropertyFilter {
    /// Only notes carrying this key.
    pub key: Option<String>,
    /// With `key`: only notes whose value for it renders to this string.
    pub value: Option<String>,
    /// Page size; `None` means every row.
    pub limit: Option<u32>,
    /// Rows to skip.
    pub offset: Option<u32>,
}

/// One row of the bulk table.
#[derive(Debug, Clone, PartialEq)]
pub struct PropertyListRow {
    /// Forge-relative path.
    pub path: String,
    /// `title` property when present, else the filename stem.
    pub title: String,
    /// Sparse key → JSON value.
    pub properties: BTreeMap<String, Json>,
}

/// A page of the bulk table. `columns` is the full key set regardless
/// of which rows are in the window, so a table header stays stable.
#[derive(Debug, Clone, PartialEq)]
pub struct PropertyPage {
    /// Every known key, sorted.
    pub columns: Vec<String>,
    /// The requested window.
    pub rows: Vec<PropertyListRow>,
    /// Matching notes before pagination.
    pub total: u32,
}

fn is_date(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 10
        && b[4] == b'-'
        && b[7] == b'-'
        && b.iter()
            .enumerate()
            .all(|(i, c)| matches!(i, 4 | 7) || c.is_ascii_digit())
}

fn is_date_time(s: &str) -> bool {
    s.len() >= 16 && is_date(&s[..10]) && matches!(s.as_bytes()[10], b'T' | b' ')
}

fn is_wikilink(s: &str) -> bool {
    s.starts_with("[[") && s.ends_with("]]")
}

/// Merge `(key, hint)` observations into an inferred schema: the
/// "widest" type wins when a key is seen with conflicting hints, and
/// `Text` absorbs any disagreement.
pub fn infer_from_observations(
    observations: impl IntoIterator<Item = (String, PropertyType)>,
) -> BTreeMap<String, PropertyType> {
    let mut out: BTreeMap<String, PropertyType> = BTreeMap::new();
    for (key, ty) in observations {
        match out.get(&key) {
            None => {
                out.insert(key, ty);
            }
            Some(existing) if *existing == ty => {}
            Some(_) => {
                out.insert(key, PropertyType::Text);
            }
        }
    }
    out
}

// ── frontmatter block ──────────────────────────────────────────────────────

/// `(block_end, yaml_body)` for a document that opens with a `---`
/// fence, where `block_end` is the byte just past the closing fence
/// line. `None` when the document has no well-formed frontmatter.
fn locate_block(content: &str) -> Option<(usize, &str)> {
    let open_len = if content.starts_with("---\r\n") {
        5
    } else if content.starts_with("---\n") {
        4
    } else {
        return None;
    };
    let rest = &content[open_len..];
    let mut from = 0;
    loop {
        let rel = rest[from..].find("\n---")?;
        let close = from + rel;
        let after = close + 4;
        match rest.as_bytes().get(after) {
            None => return Some((open_len + after, &rest[..close])),
            Some(b'\n') => return Some((open_len + after + 1, &rest[..close])),
            Some(b'\r') if rest.as_bytes().get(after + 1) == Some(&b'\n') => {
                return Some((open_len + after + 2, &rest[..close]));
            }
            _ => from = close + 1,
        }
    }
}

fn parse_mapping(yaml_body: &str) -> Result<serde_norway::Mapping, StorageError> {
    if yaml_body.trim().is_empty() {
        return Ok(serde_norway::Mapping::new());
    }
    match serde_norway::from_str::<Yaml>(yaml_body) {
        Ok(Yaml::Mapping(m)) => Ok(m),
        Ok(_) => Ok(serde_norway::Mapping::new()),
        Err(e) => Err(StorageError::ParseError {
            file: "<frontmatter>".to_string(),
            error: e.to_string(),
        }),
    }
}

fn yaml_to_json(value: &Yaml) -> Json {
    serde_json::to_value(value).unwrap_or(Json::Null)
}

/// Coerce a JSON value into the YAML node appropriate for `ty`, so the
/// on-disk shape matches what the index classifier expects.
fn coerce(ty: PropertyType, value: &Json) -> Yaml {
    let as_text = |v: &Json| match v {
        Json::String(s) => s.clone(),
        Json::Null => String::new(),
        other => other.to_string(),
    };
    match ty {
        PropertyType::Text | PropertyType::Link | PropertyType::Date | PropertyType::DateTime => {
            Yaml::String(as_text(value))
        }
        PropertyType::Number => match value {
            Json::Number(n) => serde_norway::from_str(&n.to_string()).unwrap_or(Yaml::Null),
            Json::String(s) => s
                .trim()
                .parse::<f64>()
                .ok()
                .and_then(|f| serde_norway::from_str(&f.to_string()).ok())
                .unwrap_or(Yaml::Null),
            _ => Yaml::Null,
        },
        PropertyType::Boolean => match value {
            Json::Bool(b) => Yaml::Bool(*b),
            Json::String(s) => Yaml::Bool(matches!(s.trim(), "true" | "yes" | "1")),
            _ => Yaml::Bool(false),
        },
        PropertyType::List | PropertyType::Tags => {
            let items: Vec<Yaml> = match value {
                Json::Array(xs) => xs.iter().map(|x| Yaml::String(as_text(x))).collect(),
                Json::String(s) => s
                    .split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(|s| Yaml::String(s.to_string()))
                    .collect(),
                Json::Null => Vec::new(),
                other => vec![Yaml::String(other.to_string())],
            };
            Yaml::Sequence(items)
        }
    }
}

/// Pure core of [`StorageEngine::set_note_property`]: returns the new
/// document text with `key` set to `value` (typed per `ty`), or removed
/// when `value` is `None`. Creates a frontmatter block when absent.
///
/// # Errors
///
/// [`StorageError::ParseError`] when the existing block is not valid YAML.
pub fn apply_typed_property(
    content: &str,
    key: &str,
    value: Option<&Json>,
    ty: PropertyType,
) -> Result<String, StorageError> {
    let (block_end, body) = locate_block(content)
        .map(|(end, body)| (end, body.to_string()))
        .unwrap_or((0, String::new()));
    let mut mapping = parse_mapping(&body)?;
    let yaml_key = Yaml::String(key.to_string());
    match value {
        Some(v) => {
            mapping.insert(yaml_key, coerce(ty, v));
        }
        None => {
            mapping.remove(&yaml_key);
        }
    }
    let rest = &content[block_end..];
    if mapping.is_empty() {
        return Ok(rest.to_string());
    }
    let serialised =
        serde_norway::to_string(&Yaml::Mapping(mapping)).map_err(|e| StorageError::ParseError {
            file: "<frontmatter>".to_string(),
            error: format!("serialise frontmatter: {e}"),
        })?;
    let yaml = serialised.strip_prefix("---\n").unwrap_or(&serialised);
    Ok(format!("---\n{yaml}---\n{rest}"))
}

/// Pure core of [`StorageEngine::note_properties`]: every top-level
/// frontmatter key of `content` projected through `schema`.
///
/// # Errors
///
/// [`StorageError::ParseError`] when the block is not valid YAML.
pub fn typed_properties(
    content: &str,
    schema: &PropertySchema,
) -> Result<Vec<PropertyRow>, StorageError> {
    let Some((_, body)) = locate_block(content) else {
        return Ok(Vec::new());
    };
    let mapping = parse_mapping(body)?;
    Ok(mapping
        .iter()
        .map(|(k, v)| {
            let key = match k {
                Yaml::String(s) => s.clone(),
                other => yaml_to_json(other).to_string(),
            };
            let value = yaml_to_json(v);
            let property_type = schema.infer_for(&key, &value);
            PropertyRow {
                key,
                property_type,
                value,
            }
        })
        .collect())
}

fn render_for_filter(value: &Json) -> String {
    match value {
        Json::String(s) => s.clone(),
        Json::Array(xs) => xs
            .iter()
            .map(render_for_filter)
            .collect::<Vec<_>>()
            .join(", "),
        Json::Null => String::new(),
        other => other.to_string(),
    }
}

impl StorageEngine {
    /// Inferred + declared property schema for the whole forge.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError`] on index or config failure.
    pub fn property_schema(&self) -> Result<PropertySchema, StorageError> {
        let conn = self.pool.get().map_err(|e| {
            StorageError::Database(rusqlite::Error::InvalidParameterName(e.to_string()))
        })?;
        let mut stmt = conn.prepare(
            "SELECT p.key, p.property_type, p.value, p.value_date IS NOT NULL,
                    p.value_bool IS NOT NULL
             FROM properties p JOIN files f ON f.id = p.file_id
             WHERE f.is_deleted = 0;",
        )?;
        let observations = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, Option<String>>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, bool>(3)?,
                    r.get::<_, bool>(4)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        let inferred = infer_from_observations(observations.into_iter().map(
            |(key, hint, value, has_date, has_bool)| {
                let ty = if key == "tags" {
                    PropertyType::Tags
                } else if has_bool {
                    PropertyType::Boolean
                } else if has_date {
                    PropertyType::Date
                } else {
                    match hint.as_deref() {
                        Some("number") => PropertyType::Number,
                        Some("list") => PropertyType::List,
                        _ => serde_json::from_str::<Json>(&value)
                            .map_or(PropertyType::Text, |v| PropertyType::from_json(&v)),
                    }
                };
                (key, ty)
            },
        ));
        let config = crate::config::load_app_config(self.forge.root())?;
        let overrides = config
            .properties
            .type_overrides
            .iter()
            .filter_map(|(k, v)| PropertyType::parse(v).map(|ty| (k.clone(), ty)))
            .collect();
        Ok(PropertySchema {
            inferred,
            overrides,
        })
    }

    /// Declare (or, with `None`, forget) the type of `key` in
    /// `.forge/app.toml`'s `[properties].type_overrides`.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError`] on config load/save failure.
    pub fn set_property_type_override(
        &self,
        key: &str,
        ty: Option<PropertyType>,
    ) -> Result<(), StorageError> {
        let mut config = crate::config::load_app_config(self.forge.root())?;
        match ty {
            Some(ty) => {
                config
                    .properties
                    .type_overrides
                    .insert(key.to_string(), ty.as_str().to_string());
            }
            None => {
                config.properties.type_overrides.remove(key);
            }
        }
        crate::config::save_app_config(self.forge.root(), &config)
    }

    /// Typed view of one note's frontmatter, straight from disk.
    ///
    /// # Errors
    ///
    /// [`StorageError::FileNotFound`], non-UTF-8 as
    /// [`StorageError::CorruptFile`], or [`StorageError::ParseError`]
    /// for invalid YAML.
    pub fn note_properties(&self, relpath: &str) -> Result<Vec<PropertyRow>, StorageError> {
        let content = self.read_note_text(relpath)?;
        let schema = self.property_schema()?;
        typed_properties(&content, &schema)
    }

    /// Set `key` to `value` (or remove it when `None`) in `relpath`'s
    /// frontmatter, typed per `ty` or, when `None`, the effective schema
    /// type / the value's own shape. Written through `write_file` so the
    /// index and watcher see the change.
    ///
    /// # Errors
    ///
    /// As [`note_properties`](Self::note_properties), plus write failures.
    pub fn set_note_property(
        &self,
        relpath: &str,
        key: &str,
        value: Option<&Json>,
        ty: Option<PropertyType>,
    ) -> Result<(), StorageError> {
        let content = self.read_note_text(relpath)?;
        let ty = match (ty, value) {
            (Some(ty), _) => ty,
            (None, Some(v)) => self.property_schema()?.infer_for(key, v),
            (None, None) => PropertyType::Text,
        };
        let next = apply_typed_property(&content, key, value, ty)?;
        if next != content {
            self.write_file(relpath, next.as_bytes())?;
        }
        Ok(())
    }

    /// Paginated, optionally filtered table of every indexed markdown
    /// note's properties.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError`] on index or config failure.
    pub fn list_note_properties(
        &self,
        filter: &PropertyFilter,
    ) -> Result<PropertyPage, StorageError> {
        let schema = self.property_schema()?;
        let conn = self.pool.get().map_err(|e| {
            StorageError::Database(rusqlite::Error::InvalidParameterName(e.to_string()))
        })?;

        let mut columns: BTreeSet<String> = schema.inferred.keys().cloned().collect();
        columns.extend(schema.overrides.keys().cloned());

        // Candidate files, then their properties, then filter + window.
        let mut stmt = conn.prepare(
            "SELECT f.id, f.path FROM files f
             WHERE f.is_deleted = 0 AND f.file_type = 'markdown'
             ORDER BY f.path;",
        )?;
        let files = stmt
            .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?
            .collect::<Result<Vec<_>, _>>()?;

        let mut props_stmt =
            conn.prepare("SELECT key, value FROM properties WHERE file_id = ?1 ORDER BY key;")?;

        let mut all_rows: Vec<PropertyListRow> = Vec::new();
        for (file_id, path) in files {
            let raw = props_stmt
                .query_map(params![file_id], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            if raw.is_empty() {
                continue;
            }
            let mut properties: BTreeMap<String, Json> = BTreeMap::new();
            for (key, value_json) in raw {
                let value =
                    serde_json::from_str::<Json>(&value_json).unwrap_or(Json::String(value_json));
                let value = match schema.infer_for(&key, &value) {
                    PropertyType::List | PropertyType::Tags if !value.is_array() => {
                        Json::Array(vec![value])
                    }
                    _ => value,
                };
                properties.insert(key, value);
            }
            if let Some(key) = filter.key.as_deref() {
                let Some(v) = properties.get(key) else {
                    continue;
                };
                if let Some(want) = filter.value.as_deref() {
                    if render_for_filter(v) != want {
                        continue;
                    }
                }
            }
            let title = match properties.get("title") {
                Some(Json::String(s)) if !s.is_empty() => s.clone(),
                _ => stem_of(&path),
            };
            all_rows.push(PropertyListRow {
                path,
                title,
                properties,
            });
        }

        let total = u32::try_from(all_rows.len()).unwrap_or(u32::MAX);
        let offset = filter.offset.unwrap_or(0) as usize;
        let limit = filter.limit.map_or(usize::MAX, |l| l as usize);
        let rows = all_rows.into_iter().skip(offset).take(limit).collect();
        Ok(PropertyPage {
            columns: columns.into_iter().collect(),
            rows,
            total,
        })
    }

    fn read_note_text(&self, relpath: &str) -> Result<String, StorageError> {
        let bytes = self.read_file(relpath)?;
        String::from_utf8(bytes).map_err(|e| StorageError::CorruptFile {
            path: relpath.to_string(),
            reason: format!("not valid UTF-8: {e}"),
        })
    }
}

fn stem_of(path: &str) -> String {
    let base = path.rsplit('/').next().unwrap_or(path);
    base.strip_suffix(".md").unwrap_or(base).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn type_names_round_trip_and_alias() {
        for ty in PropertyType::ALL {
            assert_eq!(PropertyType::parse(ty.as_str()), Some(ty));
        }
        assert_eq!(PropertyType::parse("bool"), Some(PropertyType::Boolean));
        assert_eq!(
            PropertyType::parse("DateTime"),
            Some(PropertyType::DateTime)
        );
        assert_eq!(PropertyType::parse("nope"), None);
    }

    #[test]
    fn from_json_classifies_shapes() {
        assert_eq!(PropertyType::from_json(&json!(true)), PropertyType::Boolean);
        assert_eq!(PropertyType::from_json(&json!(3)), PropertyType::Number);
        assert_eq!(PropertyType::from_json(&json!(["a"])), PropertyType::List);
        assert_eq!(
            PropertyType::from_json(&json!("2026-09-08")),
            PropertyType::Date
        );
        assert_eq!(
            PropertyType::from_json(&json!("2026-09-08T10:00")),
            PropertyType::DateTime
        );
        assert_eq!(
            PropertyType::from_json(&json!("[[Other]]")),
            PropertyType::Link
        );
        assert_eq!(PropertyType::from_json(&json!("hello")), PropertyType::Text);
    }

    #[test]
    fn infer_widens_conflicts_to_text() {
        let got = infer_from_observations(vec![
            ("due".to_string(), PropertyType::Date),
            ("priority".to_string(), PropertyType::Number),
            ("priority".to_string(), PropertyType::Text),
        ]);
        assert_eq!(got["due"], PropertyType::Date);
        assert_eq!(got["priority"], PropertyType::Text);
    }

    #[test]
    fn schema_prefers_override_then_inferred_then_value() {
        let mut schema = PropertySchema::default();
        schema.inferred.insert("due".into(), PropertyType::Text);
        schema.overrides.insert("due".into(), PropertyType::Date);
        assert_eq!(schema.effective("due"), Some(PropertyType::Date));
        assert_eq!(schema.infer_for("tags", &json!(["x"])), PropertyType::Tags);
        assert_eq!(schema.infer_for("n", &json!(1)), PropertyType::Number);
    }

    #[test]
    fn locate_block_handles_crlf_and_missing() {
        assert_eq!(locate_block("---\na: 1\n---\nbody"), Some((13, "a: 1")));
        assert_eq!(
            locate_block("---\r\na: 1\r\n---\r\nbody"),
            Some((16, "a: 1\r"))
        );
        assert_eq!(locate_block("---\na: 1\n---"), Some((12, "a: 1")));
        assert_eq!(locate_block("no block"), None);
        assert_eq!(locate_block("---\nunterminated\n"), None);
    }

    #[test]
    fn typed_properties_projects_values() {
        let doc =
            "---\ntitle: Hi\ncount: 3\ndone: true\ntags: [a, b]\ndue: 2026-09-08\n---\nbody\n";
        let rows = typed_properties(doc, &PropertySchema::default()).unwrap();
        let by_key: BTreeMap<_, _> = rows
            .into_iter()
            .map(|r| (r.key.clone(), (r.property_type, r.value)))
            .collect();
        assert_eq!(by_key["title"], (PropertyType::Text, json!("Hi")));
        assert_eq!(by_key["count"], (PropertyType::Number, json!(3)));
        assert_eq!(by_key["done"], (PropertyType::Boolean, json!(true)));
        assert_eq!(by_key["tags"], (PropertyType::Tags, json!(["a", "b"])));
        assert_eq!(by_key["due"], (PropertyType::Date, json!("2026-09-08")));
        assert!(typed_properties("plain\n", &PropertySchema::default())
            .unwrap()
            .is_empty());
    }

    #[test]
    fn apply_creates_block_replaces_key_and_removes() {
        let created =
            apply_typed_property("body\n", "title", Some(&json!("Hi")), PropertyType::Text)
                .unwrap();
        assert_eq!(created, "---\ntitle: Hi\n---\nbody\n");

        let replaced =
            apply_typed_property(&created, "title", Some(&json!("New")), PropertyType::Text)
                .unwrap();
        assert_eq!(replaced, "---\ntitle: New\n---\nbody\n");

        let with_list =
            apply_typed_property(&replaced, "tags", Some(&json!("x, y")), PropertyType::Tags)
                .unwrap();
        assert!(with_list.contains("tags:\n- x\n- y\n"), "{with_list}");
        assert!(with_list.ends_with("---\nbody\n"));

        let removed = apply_typed_property(&with_list, "tags", None, PropertyType::Tags).unwrap();
        assert_eq!(removed, "---\ntitle: New\n---\nbody\n");

        let emptied = apply_typed_property(&removed, "title", None, PropertyType::Text).unwrap();
        assert_eq!(emptied, "body\n");
    }

    #[test]
    fn apply_coerces_numbers_and_booleans_from_strings() {
        let doc = apply_typed_property("", "n", Some(&json!("42")), PropertyType::Number).unwrap();
        assert_eq!(doc, "---\nn: 42\n---\n");
        let doc =
            apply_typed_property(&doc, "ok", Some(&json!("yes")), PropertyType::Boolean).unwrap();
        assert!(doc.contains("ok: true\n"));
        let doc = apply_typed_property(&doc, "n", Some(&json!(2.5)), PropertyType::Number).unwrap();
        assert!(doc.contains("n: 2.5\n"));
    }

    #[test]
    fn apply_rejects_invalid_yaml() {
        let err = apply_typed_property(
            "---\n: : :\n  - [\n---\n",
            "k",
            Some(&json!("v")),
            PropertyType::Text,
        )
        .unwrap_err();
        assert!(matches!(err, StorageError::ParseError { .. }), "{err}");
    }
}
