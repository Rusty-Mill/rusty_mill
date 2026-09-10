//! Note composer: merge two notes, or create a note from a title
//! (RFC 0009 port of `nexus_forge`'s `forge-plugin-note-composer`).
//!
//! Lives on the storage engine so the shell, CLI, TUI, and MCP reach one
//! implementation through `com.nexus.storage::note_merge` /
//! `::note_create_from_title`.
//!
//! * **Merge** appends the source body to the target (separated by
//!   [`MERGE_SEPARATOR`]), rewrites every inbound link that pointed at
//!   the source so it points at the target (same machinery as
//!   `rename_entry_with_links`), then deletes the source to the
//!   requested destination.
//! * **Create from title** sanitises a human title into `{stem}.md`
//!   under an optional folder and writes the given body. Refuses to
//!   overwrite: the caller is expected to prompt for another title.
//!
//! "Extract selection to a new note" is create-from-title with the
//! selection as `content`; replacing the selection in the still-open
//! source buffer is the editor's job, since the shell owns that buffer.

use crate::unique_note::sanitize_title;
use crate::{DeleteDestination, StorageEngine, StorageError};

/// Inserted between the target body and the appended source body.
pub const MERGE_SEPARATOR: &str = "\n\n---\n\n";

/// Target body, then [`MERGE_SEPARATOR`], then source body. Trailing
/// newlines on the target are collapsed so the separator always sits
/// after exactly one blank line; the result ends with a single `\n`.
#[must_use]
pub fn merge_content(target: &str, source: &str) -> String {
    let head = target.trim_end_matches('\n');
    let tail = source.trim_end_matches('\n');
    format!("{head}{MERGE_SEPARATOR}{tail}\n")
}

/// Forge-relative `.md` path for a note titled `title`, under `folder`
/// when given.
///
/// # Errors
///
/// Returns [`StorageError::ConfigInvalid`] when the title sanitises to
/// an empty stem (e.g. `"???"`).
pub fn path_from_title(title: &str, folder: Option<&str>) -> Result<String, StorageError> {
    let stem = sanitize_title(title);
    if stem.is_empty() {
        return Err(StorageError::ConfigInvalid(format!(
            "title sanitises to an empty filename: {title:?}"
        )));
    }
    let filename = format!("{stem}.md");
    Ok(match folder.map(|f| f.trim_matches('/')) {
        Some(f) if !f.is_empty() => format!("{f}/{filename}"),
        _ => filename,
    })
}

/// What [`StorageEngine::merge_notes`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergeOutcome {
    /// Referencing files whose links were rewritten from source to target.
    pub files_rewritten: usize,
    /// Individual link occurrences updated across those files.
    pub links_updated: usize,
    /// Forge-trash bucket id for the deleted source; `None` for
    /// permanent or OS-trash deletes.
    pub trash_id: Option<String>,
}

impl StorageEngine {
    /// Merge `source` into `target`: append, redirect inbound links,
    /// delete the source. Both paths are forge-relative markdown notes.
    ///
    /// The write to `target` happens first so a failure while rewriting
    /// links or deleting leaves the merged content on disk and the
    /// source untouched, which the watcher-reconcile contract already
    /// tolerates (mirrors `rename_entry_with_links`).
    ///
    /// # Errors
    ///
    /// * [`StorageError::ConfigInvalid`] — `source == target`.
    /// * [`StorageError::FileNotFound`] — either note is missing.
    /// * [`StorageError::CorruptFile`] — either note is not UTF-8.
    /// * Any [`StorageError`] from the underlying write, rewrite, or delete.
    pub fn merge_notes(
        &self,
        source: &str,
        target: &str,
        update_links: bool,
        destination: DeleteDestination,
    ) -> Result<MergeOutcome, StorageError> {
        if source == target {
            return Err(StorageError::ConfigInvalid(
                "cannot merge a note into itself".to_string(),
            ));
        }
        let source_text = self.read_utf8(source)?;
        let target_text = self.read_utf8(target)?;

        let merged = merge_content(&target_text, &source_text);
        self.write_file(target, merged.as_bytes())?;

        let (files_rewritten, links_updated) = if update_links {
            self.rewrite_inbound_links(source, target)?
        } else {
            (0, 0)
        };

        let trash_id = self.delete_entry_to(source, destination)?;
        Ok(MergeOutcome {
            files_rewritten,
            links_updated,
            trash_id,
        })
    }

    /// Create `{folder}/{sanitised title}.md` containing `content` and
    /// return its forge-relative path.
    ///
    /// # Errors
    ///
    /// * [`StorageError::ConfigInvalid`] — title sanitises to nothing.
    /// * [`StorageError::WriteFailed`] with reason `"already exists"` —
    ///   a note at that path is already present.
    pub fn create_note_from_title(
        &self,
        title: &str,
        folder: Option<&str>,
        content: &str,
    ) -> Result<String, StorageError> {
        let relpath = path_from_title(title, folder)?;
        if self.file_exists(&relpath)? {
            return Err(StorageError::WriteFailed {
                path: relpath,
                reason: "already exists".to_string(),
            });
        }
        self.write_file(&relpath, content.as_bytes())?;
        Ok(relpath)
    }

    fn read_utf8(&self, relpath: &str) -> Result<String, StorageError> {
        let bytes = self.read_file(relpath)?;
        String::from_utf8(bytes).map_err(|e| StorageError::CorruptFile {
            path: relpath.to_string(),
            reason: format!("not valid UTF-8: {e}"),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_content_normalises_trailing_newlines() {
        assert_eq!(merge_content("# A\n\n\n", "# B\n"), "# A\n\n---\n\n# B\n");
        assert_eq!(merge_content("", "x"), "\n\n---\n\nx\n");
    }

    #[test]
    fn path_from_title_sanitises_and_places_under_folder() {
        assert_eq!(path_from_title("My: Idea?", None).unwrap(), "My Idea.md");
        assert_eq!(
            path_from_title("Idea", Some("/inbox/")).unwrap(),
            "inbox/Idea.md"
        );
        assert!(matches!(
            path_from_title("???", None),
            Err(StorageError::ConfigInvalid(_))
        ));
    }
}
