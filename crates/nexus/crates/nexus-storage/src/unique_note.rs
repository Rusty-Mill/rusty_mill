//! Unique-note creation and random-note selection (RFC 0009 port).
//!
//! Ported from `nexus_forge`'s `forge-plugin-unique-note` /
//! `forge-plugin-random-note` crates into the storage engine so every
//! frontend (shell, CLI, TUI, MCP) reaches the same behaviour through
//! `com.nexus.storage::note_create_unique` / `::note_random`.
//!
//! * **Unique note** — Zettelkasten-style: the filename is
//!   `{id}{separator}{title}.md`, where `id` is the current local time
//!   rendered through a chrono `strftime` template (default
//!   `%Y%m%d%H%M%S`). Collisions append `-2`, `-3`, … before the
//!   extension.
//! * **Random note** — a uniform pick over indexed markdown files,
//!   optionally excluding the caller's current note and/or restricted
//!   to a path prefix.
//!
//! Pure helpers are `pub` so the CLI can format ids without an engine.

use chrono::format::{Item, StrftimeItems};
use chrono::Local;
use rand::seq::IndexedRandom;

use crate::{FileFilter, StorageEngine, StorageError};

/// Default chrono `strftime` template for the id prefix.
pub const DEFAULT_ID_FORMAT: &str = "%Y%m%d%H%M%S";
/// Default string between id and title.
pub const DEFAULT_SEPARATOR: &str = " ";
/// Upper bound on `-N` suffix attempts before giving up.
const MAX_ATTEMPTS: u32 = 50;

/// Caller-supplied naming options for [`StorageEngine::create_unique_note`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UniqueNoteOptions {
    /// chrono `strftime` template for the id prefix.
    pub id_format: String,
    /// Inserted between the id and the sanitized title.
    pub separator: String,
    /// Forge-relative folder for the new note; `None` or empty means the
    /// forge root.
    pub folder: Option<String>,
}

impl Default for UniqueNoteOptions {
    fn default() -> Self {
        Self {
            id_format: DEFAULT_ID_FORMAT.to_string(),
            separator: DEFAULT_SEPARATOR.to_string(),
            folder: None,
        }
    }
}

/// `true` iff `template` is a chrono `strftime` string with no invalid
/// specifiers. Checked by walking [`StrftimeItems`] for `Item::Error`
/// rather than formatting and catching a panic.
#[must_use]
pub fn validate_id_format(template: &str) -> bool {
    !StrftimeItems::new(template).any(|item| matches!(item, Item::Error))
}

/// Render the current local time through `template`.
///
/// # Errors
///
/// Returns [`StorageError::ConfigInvalid`] when `template` contains an
/// invalid specifier.
pub fn format_id_now(template: &str) -> Result<String, StorageError> {
    if !validate_id_format(template) {
        return Err(StorageError::ConfigInvalid(format!(
            "invalid chrono strftime id_format: {template:?}"
        )));
    }
    Ok(Local::now().format(template).to_string())
}

/// Strip characters that would break a filename on common platforms,
/// trim whitespace, and drop leading dots so a title can't hide the note.
/// An empty result is allowed and means "id only".
///
/// Removed: `/` `\`, Windows-reserved `< > : " | ? *`, and control
/// characters (including NUL).
#[must_use]
pub fn sanitize_title(raw: &str) -> String {
    let kept: String = raw
        .chars()
        .filter(|ch| !ch.is_control())
        .filter(|ch| !matches!(ch, '/' | '\\' | '<' | '>' | ':' | '"' | '|' | '?' | '*'))
        .collect();
    kept.trim().trim_start_matches('.').trim().to_string()
}

/// Forge-relative path for `(id, title)` on `attempt`. Attempt `0` is the
/// bare filename; attempt `n >= 1` appends `-{n+1}` before `.md`.
#[must_use]
pub fn candidate_path(
    id: &str,
    separator: &str,
    title: &str,
    folder: Option<&str>,
    attempt: u32,
) -> String {
    let sanitized = sanitize_title(title);
    let stem = if sanitized.is_empty() {
        id.to_string()
    } else {
        format!("{id}{separator}{sanitized}")
    };
    let suffix = if attempt == 0 {
        String::new()
    } else {
        format!("-{}", attempt + 1)
    };
    let filename = format!("{stem}{suffix}.md");
    match folder.map(|f| f.trim_matches('/')) {
        Some(f) if !f.is_empty() => format!("{f}/{filename}"),
        _ => filename,
    }
}

/// Initial body for a freshly created unique note: an H1 with the raw
/// title (or the id when the title is empty) and a blank line.
fn initial_body(id: &str, title: &str) -> String {
    let heading = title.trim();
    if heading.is_empty() {
        format!("# {id}\n\n")
    } else {
        format!("# {heading}\n\n")
    }
}

impl StorageEngine {
    /// Create a new Zettelkasten-style note named `{id}{sep}{title}.md`
    /// and return its forge-relative path. The file is written through
    /// [`write_file`](Self::write_file), so it is indexed and confined
    /// to the forge root like any other write.
    ///
    /// # Errors
    ///
    /// * [`StorageError::ConfigInvalid`] — bad `id_format`.
    /// * [`StorageError::WriteFailed`] — no free filename after 50
    ///   suffix attempts, or the underlying write failed.
    pub fn create_unique_note(
        &self,
        options: &UniqueNoteOptions,
        title: &str,
    ) -> Result<String, StorageError> {
        let id = format_id_now(&options.id_format)?;
        for attempt in 0..MAX_ATTEMPTS {
            let relpath = candidate_path(
                &id,
                &options.separator,
                title,
                options.folder.as_deref(),
                attempt,
            );
            if self.file_exists(&relpath)? {
                continue;
            }
            self.write_file(&relpath, initial_body(&id, title).as_bytes())?;
            return Ok(relpath);
        }
        Err(StorageError::WriteFailed {
            path: candidate_path(&id, &options.separator, title, options.folder.as_deref(), 0),
            reason: format!("no free filename after {MAX_ATTEMPTS} attempts"),
        })
    }

    /// Pick a uniformly random indexed markdown note, excluding `exclude`
    /// (typically the caller's active note) and, when `prefix` is set,
    /// restricted to paths under it. Returns `None` when no candidate
    /// exists.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError`] on index failure.
    pub fn random_note_path(
        &self,
        exclude: Option<&str>,
        prefix: Option<&str>,
    ) -> Result<Option<String>, StorageError> {
        let filter = FileFilter {
            prefix: prefix.map(str::to_string),
            file_type: Some("markdown".to_string()),
            include_deleted: false,
        };
        let candidates: Vec<String> = self
            .query_files(&filter)?
            .into_iter()
            .map(|record| record.path)
            .filter(|path| Some(path.as_str()) != exclude)
            .collect();
        Ok(candidates.choose(&mut rand::rng()).cloned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_accepts_default_and_rejects_bad_specifier() {
        assert!(validate_id_format(DEFAULT_ID_FORMAT));
        assert!(validate_id_format("%Y-%m-%d"));
        assert!(validate_id_format("literal"));
        assert!(!validate_id_format("%Q"));
        assert!(!validate_id_format("%"));
    }

    #[test]
    fn format_id_now_matches_template_width() {
        let id = format_id_now(DEFAULT_ID_FORMAT).expect("valid template");
        assert_eq!(id.len(), 14);
        assert!(id.chars().all(|c| c.is_ascii_digit()));
        assert!(format_id_now("%Q").is_err());
    }

    #[test]
    fn sanitize_strips_reserved_and_trims() {
        assert_eq!(sanitize_title("  Hello: World?  "), "Hello World");
        assert_eq!(sanitize_title("a/b\\c<d>e\"f|g*h"), "abcdefgh");
        assert_eq!(sanitize_title("...hidden"), "hidden");
        assert_eq!(sanitize_title("tab\there"), "tabhere");
        assert_eq!(sanitize_title("   "), "");
        assert_eq!(sanitize_title("ünïcödé ok"), "ünïcödé ok");
    }

    #[test]
    fn candidate_path_shapes() {
        assert_eq!(candidate_path("123", " ", "Title", None, 0), "123 Title.md");
        assert_eq!(candidate_path("123", "-", "", None, 0), "123.md");
        assert_eq!(candidate_path("123", " ", "T", None, 1), "123 T-2.md");
        assert_eq!(candidate_path("123", " ", "T", None, 4), "123 T-5.md");
        assert_eq!(
            candidate_path("123", " ", "T", Some("/zettel/"), 0),
            "zettel/123 T.md"
        );
        assert_eq!(candidate_path("123", " ", "T", Some(""), 0), "123 T.md");
    }

    #[test]
    fn initial_body_falls_back_to_id() {
        assert_eq!(initial_body("1", "Hi"), "# Hi\n\n");
        assert_eq!(initial_body("1", "   "), "# 1\n\n");
    }
}
