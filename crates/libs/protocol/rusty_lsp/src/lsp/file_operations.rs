//! `workspace/willCreateFiles`/`didCreateFiles`,
//! `willRenameFiles`/`didRenameFiles`, and `willDeleteFiles`/`didDeleteFiles`
//! types — hooks for a server to react to (or veto/rewrite) file-system
//! operations the client performs, e.g. auto-updating imports on rename.

use super::base::Uri;
use serde::{Deserialize, Serialize};

/// One file being created, in a [`CreateFilesParams`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileCreate {
    /// The new file's URI.
    pub uri: Uri,
}

/// Parameters of `workspace/willCreateFiles` / `workspace/didCreateFiles`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateFilesParams {
    /// The files being created.
    pub files: Vec<FileCreate>,
}

/// One file being renamed, in a [`RenameFilesParams`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileRename {
    /// The file's URI before the rename.
    pub old_uri: Uri,
    /// The file's URI after the rename.
    pub new_uri: Uri,
}

/// Parameters of `workspace/willRenameFiles` / `workspace/didRenameFiles`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenameFilesParams {
    /// The files being renamed.
    pub files: Vec<FileRename>,
}

/// One file being deleted, in a [`DeleteFilesParams`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileDelete {
    /// The deleted file's URI.
    pub uri: Uri,
}

/// Parameters of `workspace/willDeleteFiles` / `workspace/didDeleteFiles`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeleteFilesParams {
    /// The files being deleted.
    pub files: Vec<FileDelete>,
}

/// A glob pattern restricting which files a
/// [`FileOperationFilter`] applies to.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileOperationPattern {
    /// The glob pattern, e.g. `"**/*.rs"`.
    pub glob: String,
    /// Restrict matches to files, folders, or both (unset). One of
    /// `"file"`/`"folder"` per the spec; kept as a plain string since it's
    /// a small, closed set not worth a dedicated enum.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub matches: Option<String>,
    /// Whether the glob match is case-insensitive.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ignore_case: Option<bool>,
}

/// One filter within a [`FileOperationRegistrationOptions`], restricting a
/// file-operation hook to files matching `pattern`, optionally scoped to a
/// URI `scheme` (e.g. `"file"`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileOperationFilter {
    /// Restrict to this URI scheme, if given.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scheme: Option<String>,
    /// The glob pattern files must match.
    pub pattern: FileOperationPattern,
}

/// Options describing a server's interest in one file-operation hook (e.g.
/// `willRenameFiles`), advertised under
/// [`FileOperationsServerCapabilities`].
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileOperationRegistrationOptions {
    /// Only files matching one of these filters trigger the hook.
    pub filters: Vec<FileOperationFilter>,
}

/// The file-operation hooks a server is interested in, advertised under
/// [`crate::lsp::ServerCapabilities`]`.workspace.file_operations`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileOperationsServerCapabilities {
    /// Interest in `workspace/didCreateFiles`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub did_create: Option<FileOperationRegistrationOptions>,
    /// Interest in `workspace/willCreateFiles`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub will_create: Option<FileOperationRegistrationOptions>,
    /// Interest in `workspace/didRenameFiles`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub did_rename: Option<FileOperationRegistrationOptions>,
    /// Interest in `workspace/willRenameFiles`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub will_rename: Option<FileOperationRegistrationOptions>,
    /// Interest in `workspace/didDeleteFiles`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub did_delete: Option<FileOperationRegistrationOptions>,
    /// Interest in `workspace/willDeleteFiles`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub will_delete: Option<FileOperationRegistrationOptions>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn file_rename_uses_camel_case() {
        let rename = FileRename {
            old_uri: "file:///a".into(),
            new_uri: "file:///b".into(),
        };
        let value = serde_json::to_value(&rename).unwrap();
        assert_eq!(value, json!({"oldUri": "file:///a", "newUri": "file:///b"}));
    }

    #[test]
    fn file_operation_registration_options_round_trips() {
        let options = FileOperationRegistrationOptions {
            filters: vec![FileOperationFilter {
                scheme: Some("file".to_owned()),
                pattern: FileOperationPattern {
                    glob: "**/*.rs".to_owned(),
                    matches: Some("file".to_owned()),
                    ignore_case: None,
                },
            }],
        };
        let value = serde_json::to_value(&options).unwrap();
        assert_eq!(value["filters"][0]["pattern"]["glob"], json!("**/*.rs"));
        assert!(value["filters"][0]["pattern"].get("ignoreCase").is_none());
    }
}
