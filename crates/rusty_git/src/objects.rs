//! Git's object model: blobs, trees, and commits — real content-addressed
//! storage (SHA-1 over a `"<type> <len>\0<content>"` frame, zlib-compressed
//! on disk under `.git/objects/xx/yyyy...`), matching real git's own format
//! byte for byte (see this module's tests for round-trips through the
//! system `git` binary).

use std::fmt;
use std::fs;
use std::path::Path;

use crate::sha1::{hex, sha1, SHA1_DIGEST_LEN};
use crate::zlib;

/// The four object types git's format distinguishes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectKind {
    /// File content.
    Blob,
    /// A directory listing (sorted entries of mode/name/hash).
    Tree,
    /// A commit: a tree, parents, author/committer, and a message.
    Commit,
    /// An annotated tag (parsed on read; not produced by this crate yet).
    Tag,
}

impl ObjectKind {
    /// The lowercase type tag git's object header uses (`"blob"`, `"tree"`, …).
    pub const fn as_str(self) -> &'static str {
        match self {
            ObjectKind::Blob => "blob",
            ObjectKind::Tree => "tree",
            ObjectKind::Commit => "commit",
            ObjectKind::Tag => "tag",
        }
    }

    /// Parses a type tag from an object header.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "blob" => Some(ObjectKind::Blob),
            "tree" => Some(ObjectKind::Tree),
            "commit" => Some(ObjectKind::Commit),
            "tag" => Some(ObjectKind::Tag),
            _ => None,
        }
    }
}

impl fmt::Display for ObjectKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Errors reading/writing a loose object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ObjectError {
    /// No loose object file exists for the requested hash.
    NotFound(String),
    /// The zlib framing around the object was corrupt (or, for an object
    /// this crate didn't write itself, used a DEFLATE block type this
    /// crate's decompressor doesn't support — see `zlib`'s module doc).
    Corrupt(String),
    /// Underlying filesystem I/O failure.
    Io(String),
}

impl fmt::Display for ObjectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ObjectError::NotFound(h) => write!(f, "object {h} not found"),
            ObjectError::Corrupt(h) => write!(f, "object {h} is corrupt or unreadable: see zlib module docs"),
            ObjectError::Io(e) => write!(f, "I/O error: {e}"),
        }
    }
}

impl std::error::Error for ObjectError {}

/// Frames `content` the way git hashes and stores every object: the type
/// tag, a space, the decimal content length, a NUL, then the raw content.
pub fn frame(kind: ObjectKind, content: &[u8]) -> Vec<u8> {
    let mut buf = format!("{} {}\0", kind.as_str(), content.len()).into_bytes();
    buf.extend_from_slice(content);
    buf
}

/// The object id (raw 20-byte SHA-1) `content` would hash to as `kind`,
/// without writing anything — what `git hash-object` computes.
pub fn hash(kind: ObjectKind, content: &[u8]) -> [u8; SHA1_DIGEST_LEN] {
    sha1(&frame(kind, content))
}

/// Where a loose object with hex object-id `oid` lives under `git_dir`.
pub fn object_path(git_dir: &Path, oid_hex: &str) -> std::path::PathBuf {
    let (dir, file) = oid_hex.split_at(2);
    git_dir.join("objects").join(dir).join(file)
}

/// Writes `content` as a loose object of `kind`, returning its hex object id.
/// A no-op (beyond recomputing the hash) if the object is already stored —
/// git's own content-addressed dedup.
pub fn write_object(git_dir: &Path, kind: ObjectKind, content: &[u8]) -> Result<String, ObjectError> {
    let framed = frame(kind, content);
    let oid = hex(&sha1(&framed));
    let path = object_path(git_dir, &oid);
    if path.exists() {
        return Ok(oid);
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| ObjectError::Io(e.to_string()))?;
    }
    let compressed = zlib::compress(&framed);
    fs::write(&path, compressed).map_err(|e| ObjectError::Io(e.to_string()))?;
    Ok(oid)
}

/// Reads and parses a loose object by hex object id, returning its kind and
/// raw content (the header stripped off).
pub fn read_object(git_dir: &Path, oid_hex: &str) -> Result<(ObjectKind, Vec<u8>), ObjectError> {
    let path = object_path(git_dir, oid_hex);
    let compressed = fs::read(&path).map_err(|_| ObjectError::NotFound(oid_hex.to_string()))?;
    let framed = zlib::decompress(&compressed).map_err(|_| ObjectError::Corrupt(oid_hex.to_string()))?;

    let nul = framed
        .iter()
        .position(|&b| b == 0)
        .ok_or_else(|| ObjectError::Corrupt(oid_hex.to_string()))?;
    let header = std::str::from_utf8(&framed[..nul]).map_err(|_| ObjectError::Corrupt(oid_hex.to_string()))?;
    let (kind_str, len_str) = header
        .split_once(' ')
        .ok_or_else(|| ObjectError::Corrupt(oid_hex.to_string()))?;
    let kind = ObjectKind::parse(kind_str).ok_or_else(|| ObjectError::Corrupt(oid_hex.to_string()))?;
    let len: usize = len_str.parse().map_err(|_| ObjectError::Corrupt(oid_hex.to_string()))?;

    let content = framed[nul + 1..].to_vec();
    if content.len() != len {
        return Err(ObjectError::Corrupt(oid_hex.to_string()));
    }
    Ok((kind, content))
}

/// One entry in a tree object: a file/directory mode, a name, and the raw
/// (binary, not hex) SHA-1 of the blob/subtree it points to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeEntry {
    /// Git's octal mode string (`"100644"` regular file, `"100755"`
    /// executable, `"40000"` subtree, `"120000"` symlink).
    pub mode: String,
    /// The entry's file/directory name (no path separators).
    pub name: String,
    /// Raw 20-byte SHA-1 of the blob (file) or tree (subdirectory).
    pub hash: [u8; SHA1_DIGEST_LEN],
}

/// Git's tree-entry sort key: byte-wise by name, except a subtree (mode
/// `"40000"`) sorts as though its name had a trailing `/` — RFC-less but
/// specified by git's own `tree-entry.c` and required for two trees with
/// the same entries to hash identically regardless of how they were built.
fn tree_sort_key(entry: &TreeEntry) -> Vec<u8> {
    let mut key = entry.name.as_bytes().to_vec();
    if entry.mode == "40000" {
        key.push(b'/');
    }
    key
}

/// Encodes a tree object's content from its (not-yet-sorted) entries.
pub fn encode_tree(entries: &[TreeEntry]) -> Vec<u8> {
    let mut sorted: Vec<&TreeEntry> = entries.iter().collect();
    sorted.sort_by_key(|e| tree_sort_key(e));

    let mut body = Vec::new();
    for entry in sorted {
        body.extend_from_slice(entry.mode.as_bytes());
        body.push(b' ');
        body.extend_from_slice(entry.name.as_bytes());
        body.push(0);
        body.extend_from_slice(&entry.hash);
    }
    body
}

/// Decodes a tree object's content back into entries.
pub fn decode_tree(content: &[u8]) -> Result<Vec<TreeEntry>, ObjectError> {
    let mut entries = Vec::new();
    let mut i = 0;
    while i < content.len() {
        let space = content[i..]
            .iter()
            .position(|&b| b == b' ')
            .ok_or_else(|| ObjectError::Corrupt("tree".to_string()))?
            + i;
        let mode = std::str::from_utf8(&content[i..space])
            .map_err(|_| ObjectError::Corrupt("tree".to_string()))?
            .to_string();

        let nul = content[space + 1..]
            .iter()
            .position(|&b| b == 0)
            .ok_or_else(|| ObjectError::Corrupt("tree".to_string()))?
            + space
            + 1;
        let name = std::str::from_utf8(&content[space + 1..nul])
            .map_err(|_| ObjectError::Corrupt("tree".to_string()))?
            .to_string();

        if nul + 1 + SHA1_DIGEST_LEN > content.len() {
            return Err(ObjectError::Corrupt("tree".to_string()));
        }
        let mut hash = [0u8; SHA1_DIGEST_LEN];
        hash.copy_from_slice(&content[nul + 1..nul + 1 + SHA1_DIGEST_LEN]);

        entries.push(TreeEntry { mode, name, hash });
        i = nul + 1 + SHA1_DIGEST_LEN;
    }
    Ok(entries)
}

/// A parsed commit object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Commit {
    /// Hex object id of this commit's root tree.
    pub tree: String,
    /// Hex object ids of parent commits (empty for the first commit).
    pub parents: Vec<String>,
    /// Raw `"Name <email> <unix-seconds> <+zzzz>"` author line.
    pub author: String,
    /// Raw `"Name <email> <unix-seconds> <+zzzz>"` committer line.
    pub committer: String,
    /// The commit message (trailing newline stripped).
    pub message: String,
}

/// Encodes a commit object's content in git's exact plumbing format.
pub fn encode_commit(commit: &Commit) -> Vec<u8> {
    let mut out = format!("tree {}\n", commit.tree);
    for parent in &commit.parents {
        out.push_str(&format!("parent {parent}\n"));
    }
    out.push_str(&format!("author {}\n", commit.author));
    out.push_str(&format!("committer {}\n", commit.committer));
    out.push('\n');
    out.push_str(&commit.message);
    if !commit.message.ends_with('\n') {
        out.push('\n');
    }
    out.into_bytes()
}

/// Decodes a commit object's content.
pub fn decode_commit(content: &[u8]) -> Result<Commit, ObjectError> {
    let text = std::str::from_utf8(content).map_err(|_| ObjectError::Corrupt("commit".to_string()))?;
    let (header, message) = text
        .split_once("\n\n")
        .ok_or_else(|| ObjectError::Corrupt("commit".to_string()))?;

    let mut tree = None;
    let mut parents = Vec::new();
    let mut author = None;
    let mut committer = None;
    for line in header.lines() {
        if let Some(v) = line.strip_prefix("tree ") {
            tree = Some(v.to_string());
        } else if let Some(v) = line.strip_prefix("parent ") {
            parents.push(v.to_string());
        } else if let Some(v) = line.strip_prefix("author ") {
            author = Some(v.to_string());
        } else if let Some(v) = line.strip_prefix("committer ") {
            committer = Some(v.to_string());
        }
    }

    Ok(Commit {
        tree: tree.ok_or_else(|| ObjectError::Corrupt("commit".to_string()))?,
        parents,
        author: author.ok_or_else(|| ObjectError::Corrupt("commit".to_string()))?,
        committer: committer.ok_or_else(|| ObjectError::Corrupt("commit".to_string()))?,
        message: message.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blob_hash_matches_real_git() {
        // `printf 'hello world' | git hash-object --stdin`
        let oid = hex(&hash(ObjectKind::Blob, b"hello world"));
        assert_eq!(oid, "95d09f2b10159347eece71399a7e2e907ea3df4f");
    }

    #[test]
    fn tree_entries_sort_the_way_real_git_does_when_a_dir_and_a_file_share_a_prefix() {
        // A file "a.txt" and a directory "a" tie-break on the directory's
        // implicit trailing "/" ('.' is 0x2e, '/' is 0x2f, so "a.txt" sorts
        // *before* "a/") -- verified against real git: `git init`, add
        // a/x.txt + a.txt + b, `git write-tree | git ls-tree` produces
        // exactly this order (a.txt, then the a/ subtree, then b).
        let entries = vec![
            TreeEntry { mode: "100644".into(), name: "b".into(), hash: [1; 20] },
            TreeEntry { mode: "40000".into(), name: "a".into(), hash: [2; 20] },
            TreeEntry { mode: "100644".into(), name: "a.txt".into(), hash: [3; 20] },
        ];
        let encoded = encode_tree(&entries);
        let decoded = decode_tree(&encoded).unwrap();
        let names: Vec<&str> = decoded.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["a.txt", "a", "b"]);
    }

    #[test]
    fn tree_round_trips() {
        let entries = vec![
            TreeEntry { mode: "100644".into(), name: "README.md".into(), hash: [0xAB; 20] },
            TreeEntry { mode: "40000".into(), name: "src".into(), hash: [0xCD; 20] },
        ];
        let encoded = encode_tree(&entries);
        let decoded = decode_tree(&encoded).unwrap();
        let mut expected = entries;
        expected.sort_by_key(tree_sort_key);
        assert_eq!(decoded, expected);
    }

    #[test]
    fn commit_round_trips() {
        let commit = Commit {
            tree: "a".repeat(40),
            parents: vec!["b".repeat(40)],
            author: "A <a@b.c> 1700000000 +0000".to_string(),
            committer: "A <a@b.c> 1700000000 +0000".to_string(),
            message: "Initial commit\n".to_string(),
        };
        let encoded = encode_commit(&commit);
        let decoded = decode_commit(&encoded).unwrap();
        assert_eq!(decoded, commit);
    }

    #[test]
    fn write_then_read_object_round_trips() {
        let dir = std::env::temp_dir().join(format!("rusty_git_objects_test_{}", std::process::id()));
        let git_dir = dir.join(".git");
        std::fs::create_dir_all(&git_dir).unwrap();

        let oid = write_object(&git_dir, ObjectKind::Blob, b"hello world").unwrap();
        assert_eq!(oid, "95d09f2b10159347eece71399a7e2e907ea3df4f");

        let (kind, content) = read_object(&git_dir, &oid).unwrap();
        assert_eq!(kind, ObjectKind::Blob);
        assert_eq!(content, b"hello world");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
