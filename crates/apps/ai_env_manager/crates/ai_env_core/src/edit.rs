//! The rules for editing one file inside an entry (pack D37, D38 and
//! `docs/projects/ai-env-manager/editor-plan.md`). Pure: no disk access. An
//! adapter reads the file, decides its [`Ownership`] and hashes it; this
//! module decides whether an edit is allowed and what the plan to apply is.
//!
//! The one safety rule, as for links (D6): a file we cannot prove we own is
//! never rewritten. Entry source files are the confirmed exception (D38).

use crate::model::EntryId;
use std::fmt;
use std::path::{Path, PathBuf};

/// Marker file name at the root of a managed copy (pack D6, D20). The
/// filesystem adapter writes and reads it; the editor never opens it.
pub const MARKER_FILE: &str = ".aienv-managed";

/// Largest file the editor opens or writes. A skill file is text; anything
/// bigger is not something to edit in a text box.
pub const MAX_EDIT_BYTES: usize = 1024 * 1024;

/// Why an edit was refused. `Display` is the text front ends show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditRefusal {
    /// The path is not a plain relative path inside the entry.
    BadPath(String),
    /// The marker file records our ownership and is never editable.
    MarkerFile,
    /// The file is at a target path we did not place (foreign, D6).
    Unmanaged,
    /// The text is bigger than [`MAX_EDIT_BYTES`].
    TooLarge(usize),
    /// The content has a NUL byte or is not UTF-8.
    NotText,
    /// The new text equals the current text.
    NoChange,
}

impl fmt::Display for EditRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EditRefusal::BadPath(p) => write!(
                f,
                "{p:?} is not a plain relative path inside the entry (no \"..\", drive, \
                 leading slash or backslash)"
            ),
            EditRefusal::MarkerFile => {
                write!(f, "the {MARKER_FILE} file is ours and is not editable")
            }
            EditRefusal::Unmanaged => {
                write!(
                    f,
                    "this file is not one this app placed, so it is left untouched"
                )
            }
            EditRefusal::TooLarge(n) => {
                write!(
                    f,
                    "{n} bytes is over the {MAX_EDIT_BYTES} byte limit for editing"
                )
            }
            EditRefusal::NotText => write!(f, "the file is not UTF-8 text"),
            EditRefusal::NoChange => write!(f, "nothing changed"),
        }
    }
}

impl std::error::Error for EditRefusal {}

/// A path relative to an entry's folder, validated once so it can be joined
/// onto the folder without escaping it. Always `/`-separated, as the entry
/// walk reports paths.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelPath(String);

impl RelPath {
    /// Accepts non-empty `a/b/c` paths whose parts are plain names: no `.`,
    /// `..`, empty part, `:` (drive or stream), NUL or backslash.
    pub fn parse(s: &str) -> Result<Self, EditRefusal> {
        let plain = |part: &str| {
            !part.is_empty()
                && part != "."
                && part != ".."
                && !part.contains(['\\', ':', '\0'])
                && !part.ends_with(['.', ' '])
                && !windows_reserved(part)
        };
        if s.split('/').all(plain) {
            Ok(Self(s.to_owned()))
        } else {
            Err(EditRefusal::BadPath(s.to_owned()))
        }
    }

    /// The path as `/`-separated text.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// True for the marker file at the entry's root.
    pub fn is_marker(&self) -> bool {
        self.0 == MARKER_FILE
    }

    /// This path under `root` (the entry's folder).
    pub fn under(&self, root: &Path) -> PathBuf {
        self.0
            .split('/')
            .fold(root.to_path_buf(), |p, part| p.join(part))
    }
}

/// A Windows device name (`CON`, `NUL`, `COM1`...), with or without an
/// extension. A file so named cannot be opened normally there, so it is
/// refused on every host to keep one catalog usable on all of them.
fn windows_reserved(part: &str) -> bool {
    let stem = part.split('.').next().unwrap_or(part).to_ascii_uppercase();
    let numbered = |p: &str| {
        stem.strip_prefix(p)
            .is_some_and(|n| matches!(n.as_bytes(), [b'1'..=b'9']))
    };
    matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL") || numbered("COM") || numbered("LPT")
}

/// Who owns the file an edit would rewrite, as the adapter observed it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ownership {
    /// A file in the entry's own source folder (where links come from).
    Source,
    /// A file in a marked copy at a tool's target. Editing it moves the copy
    /// to the "edited" state, which already blocks disable and replace.
    MarkedCopy,
    /// A file at a target path with no marker of ours.
    Unmanaged,
}

/// Whether `path` with this `ownership` may be edited at all.
pub fn check_editable(path: &RelPath, ownership: Ownership) -> Result<(), EditRefusal> {
    if path.is_marker() {
        return Err(EditRefusal::MarkerFile);
    }
    match ownership {
        Ownership::Unmanaged => Err(EditRefusal::Unmanaged),
        Ownership::Source | Ownership::MarkedCopy => Ok(()),
    }
}

/// `bytes` as editable text: UTF-8, no NUL, within [`MAX_EDIT_BYTES`].
pub fn check_text(bytes: &[u8]) -> Result<&str, EditRefusal> {
    if bytes.len() > MAX_EDIT_BYTES {
        return Err(EditRefusal::TooLarge(bytes.len()));
    }
    match std::str::from_utf8(bytes) {
        Ok(text) if !text.contains('\0') => Ok(text),
        _ => Err(EditRefusal::NotText),
    }
}

/// An edit that passed every rule and is ready to show for consent. It holds
/// what the adapter needs to apply it only if the file is still as read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditPlan {
    entry: EntryId,
    path: RelPath,
    ownership: Ownership,
    base_hash: String,
    new_text: String,
}

impl EditPlan {
    /// The entry the file belongs to.
    pub fn entry(&self) -> &EntryId {
        &self.entry
    }

    /// The file, relative to the entry's folder.
    pub fn path(&self) -> &RelPath {
        &self.path
    }

    /// Who owns the file being rewritten.
    pub fn ownership(&self) -> Ownership {
        self.ownership
    }

    /// Content hash of the file as read; applying refuses if it differs now.
    pub fn base_hash(&self) -> &str {
        &self.base_hash
    }

    /// The text to write.
    pub fn new_text(&self) -> &str {
        &self.new_text
    }

    /// True when applying leaves a marked copy in the "edited" state.
    pub fn marks_edited(&self) -> bool {
        self.ownership == Ownership::MarkedCopy
    }
}

/// The plan to replace a file's `current` text with `new_text`. Writes
/// nothing; refuses what [`check_editable`] and [`check_text`] refuse, and an
/// edit that changes nothing.
pub fn plan_edit(
    entry: EntryId,
    path: RelPath,
    ownership: Ownership,
    base_hash: String,
    current: &str,
    new_text: String,
) -> Result<EditPlan, EditRefusal> {
    check_editable(&path, ownership)?;
    check_text(new_text.as_bytes())?;
    if new_text == current {
        return Err(EditRefusal::NoChange);
    }
    Ok(EditPlan {
        entry,
        path,
        ownership,
        base_hash,
        new_text,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(s: &str) -> RelPath {
        RelPath::parse(s).unwrap()
    }

    fn id() -> EntryId {
        "my-skill".parse().unwrap()
    }

    fn plan(own: Ownership, p: &str, current: &str, new: &str) -> Result<EditPlan, EditRefusal> {
        plan_edit(id(), path(p), own, "h".into(), current, new.into())
    }

    #[test]
    fn relpath_accepts_plain_nested_paths() {
        assert_eq!(path("SKILL.md").as_str(), "SKILL.md");
        assert_eq!(path("refs/a/b.txt").as_str(), "refs/a/b.txt");
    }

    #[test]
    fn relpath_rejects_escapes_and_oddities() {
        for bad in [
            "",
            "/",
            "/etc/passwd",
            "a//b",
            "a/",
            "./a",
            "a/./b",
            "..",
            "a/../b",
            "../x",
            "a\\b",
            "C:/x",
            "a:b",
            "a\0b",
            "CON",
            "nul.txt",
            "dir/Com1.md",
            "LPT9",
            "a.",
            "a ",
        ] {
            assert!(
                matches!(RelPath::parse(bad), Err(EditRefusal::BadPath(_))),
                "{bad:?} should be refused"
            );
        }
    }

    #[test]
    fn relpath_accepts_names_that_only_look_reserved() {
        for ok in ["CONSOLE.md", "com10", "COM0", "lpt", "a.b", ".hidden"] {
            assert!(RelPath::parse(ok).is_ok(), "{ok:?} should be accepted");
        }
    }

    #[test]
    fn relpath_joins_under_the_entry_folder() {
        let root = Path::new("skills").join("my-skill");
        assert_eq!(
            path("refs/a.md").under(&root),
            root.join("refs").join("a.md")
        );
    }

    #[test]
    fn marker_is_never_editable_but_a_nested_file_of_that_name_is() {
        assert!(path(MARKER_FILE).is_marker());
        assert_eq!(
            check_editable(&path(MARKER_FILE), Ownership::MarkedCopy),
            Err(EditRefusal::MarkerFile)
        );
        assert!(!path(&format!("sub/{MARKER_FILE}")).is_marker());
    }

    #[test]
    fn unmanaged_target_files_are_refused_but_source_and_marked_are_editable() {
        let p = path("SKILL.md");
        assert_eq!(
            check_editable(&p, Ownership::Unmanaged),
            Err(EditRefusal::Unmanaged)
        );
        assert_eq!(check_editable(&p, Ownership::Source), Ok(()));
        assert_eq!(check_editable(&p, Ownership::MarkedCopy), Ok(()));
    }

    #[test]
    fn text_limits_are_enforced_at_the_boundary() {
        assert_eq!(check_text(b"hello"), Ok("hello"));
        assert_eq!(
            check_text(&vec![b'a'; MAX_EDIT_BYTES]).map(str::len),
            Ok(MAX_EDIT_BYTES)
        );
        assert_eq!(
            check_text(&vec![b'a'; MAX_EDIT_BYTES + 1]),
            Err(EditRefusal::TooLarge(MAX_EDIT_BYTES + 1))
        );
        assert_eq!(check_text(b"a\0b"), Err(EditRefusal::NotText));
        assert_eq!(check_text(&[0xff, 0xfe]), Err(EditRefusal::NotText));
    }

    #[test]
    fn a_valid_edit_becomes_a_plan() {
        let p = plan(Ownership::Source, "SKILL.md", "old", "new").unwrap();
        assert_eq!(p.new_text(), "new");
        assert_eq!(p.base_hash(), "h");
        assert!(!p.marks_edited());
    }

    #[test]
    fn editing_a_marked_copy_marks_it_edited() {
        assert!(plan(Ownership::MarkedCopy, "SKILL.md", "old", "new")
            .unwrap()
            .marks_edited());
    }

    #[test]
    fn plan_edit_refuses_each_rule() {
        assert_eq!(
            plan(Ownership::Unmanaged, "SKILL.md", "a", "b"),
            Err(EditRefusal::Unmanaged)
        );
        assert_eq!(
            plan(Ownership::Source, MARKER_FILE, "a", "b"),
            Err(EditRefusal::MarkerFile)
        );
        assert_eq!(
            plan(Ownership::Source, "SKILL.md", "a", "a"),
            Err(EditRefusal::NoChange)
        );
        assert_eq!(
            plan(Ownership::Source, "SKILL.md", "a", "b\0"),
            Err(EditRefusal::NotText)
        );
        let big = "x".repeat(MAX_EDIT_BYTES + 1);
        assert_eq!(
            plan(Ownership::Source, "SKILL.md", "a", &big),
            Err(EditRefusal::TooLarge(MAX_EDIT_BYTES + 1))
        );
    }
}
