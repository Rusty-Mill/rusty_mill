//! Builds real, possibly-nested git tree objects from a flat list of staged
//! paths (as the [`crate::index::Index`] stores them) — recursively writing
//! one tree object per directory level, the way real `git write-tree` does.

use std::collections::BTreeMap;
use std::path::Path;

use crate::index::IndexEntry;
use crate::objects::{encode_tree, write_object, ObjectError, ObjectKind, TreeEntry};
use crate::sha1::SHA1_DIGEST_LEN;

enum Node {
    File { mode: u32, hash: [u8; SHA1_DIGEST_LEN] },
    Dir(BTreeMap<String, Node>),
}

/// Builds the tree object(s) for `entries` and writes them all as loose
/// objects under `git_dir`, returning the hex object id of the root tree.
/// An empty `entries` list still writes (and returns) a valid empty tree.
pub fn write_tree(git_dir: &Path, entries: &[IndexEntry]) -> Result<String, ObjectError> {
    let mut root: BTreeMap<String, Node> = BTreeMap::new();

    for entry in entries {
        let parts: Vec<&str> = entry.path.split('/').collect();
        insert(&mut root, &parts, entry.mode, entry.hash);
    }

    write_node(git_dir, &Node::Dir(root))
}

fn insert(dir: &mut BTreeMap<String, Node>, parts: &[&str], mode: u32, hash: [u8; SHA1_DIGEST_LEN]) {
    match parts {
        [] => {}
        [only] => {
            dir.insert(only.to_string(), Node::File { mode, hash });
        }
        [first, rest @ ..] => {
            let child = dir
                .entry(first.to_string())
                .or_insert_with(|| Node::Dir(BTreeMap::new()));
            if let Node::Dir(map) = child {
                insert(map, rest, mode, hash);
            }
            // A path colliding a file with a directory prefix is a caller
            // error (staged two incompatible paths); silently prefers the
            // directory, matching a "last write wins at the leaf" policy
            // rather than panicking on malformed input.
        }
    }
}

fn write_node(git_dir: &Path, node: &Node) -> Result<String, ObjectError> {
    match node {
        Node::File { hash, .. } => Ok(crate::sha1::hex(hash)),
        Node::Dir(children) => {
            let mut tree_entries = Vec::with_capacity(children.len());
            for (name, child) in children {
                match child {
                    Node::File { mode, hash } => {
                        tree_entries.push(TreeEntry {
                            mode: format!("{mode:o}"),
                            name: name.clone(),
                            hash: *hash,
                        });
                    }
                    Node::Dir(_) => {
                        let child_oid = write_node(git_dir, child)?;
                        let mut hash = [0u8; SHA1_DIGEST_LEN];
                        for (i, byte) in hash.iter_mut().enumerate() {
                            *byte = u8::from_str_radix(&child_oid[i * 2..i * 2 + 2], 16).unwrap_or(0);
                        }
                        tree_entries.push(TreeEntry {
                            mode: "40000".to_string(),
                            name: name.clone(),
                            hash,
                        });
                    }
                }
            }
            let content = encode_tree(&tree_entries);
            write_object(git_dir, ObjectKind::Tree, &content)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::MODE_REGULAR;
    use crate::objects::{decode_tree, read_object};
    use crate::sha1::sha1;

    fn entry(path: &str, content: &[u8]) -> IndexEntry {
        IndexEntry {
            mode: MODE_REGULAR,
            size: content.len() as u32,
            hash: sha1(content),
            path: path.to_string(),
        }
    }

    #[test]
    fn empty_entries_write_an_empty_tree() {
        let dir = std::env::temp_dir().join(format!("rusty_git_tree_empty_{}", std::process::id()));
        let git_dir = dir.join(".git");
        std::fs::create_dir_all(&git_dir).unwrap();

        let oid = write_tree(&git_dir, &[]).unwrap();
        // `git hash-object -t tree --stdin < /dev/null` == this value.
        assert_eq!(oid, "4b825dc642cb6eb9a060e54bf8d69288fbee4904");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn flat_entries_build_a_single_level_tree() {
        let dir = std::env::temp_dir().join(format!("rusty_git_tree_flat_{}", std::process::id()));
        let git_dir = dir.join(".git");
        std::fs::create_dir_all(&git_dir).unwrap();

        let entries = vec![entry("a.txt", b"aaa"), entry("b.txt", b"bbb")];
        let oid = write_tree(&git_dir, &entries).unwrap();
        let (kind, content) = read_object(&git_dir, &oid).unwrap();
        assert_eq!(kind, crate::objects::ObjectKind::Tree);
        let decoded = decode_tree(&content).unwrap();
        assert_eq!(decoded.len(), 2);
        assert_eq!(decoded[0].name, "a.txt");
        assert_eq!(decoded[1].name, "b.txt");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn nested_entries_build_a_subtree_referenced_by_the_root() {
        let dir = std::env::temp_dir().join(format!("rusty_git_tree_nested_{}", std::process::id()));
        let git_dir = dir.join(".git");
        std::fs::create_dir_all(&git_dir).unwrap();

        let entries = vec![entry("README.md", b"root file"), entry("src/main.rs", b"fn main() {}")];
        let root_oid = write_tree(&git_dir, &entries).unwrap();

        let (_, root_content) = read_object(&git_dir, &root_oid).unwrap();
        let root_entries = decode_tree(&root_content).unwrap();
        assert_eq!(root_entries.len(), 2);

        let src_entry = root_entries.iter().find(|e| e.name == "src").unwrap();
        assert_eq!(src_entry.mode, "40000");

        let src_oid = crate::sha1::hex(&src_entry.hash);
        let (kind, src_content) = read_object(&git_dir, &src_oid).unwrap();
        assert_eq!(kind, crate::objects::ObjectKind::Tree);
        let src_entries = decode_tree(&src_content).unwrap();
        assert_eq!(src_entries.len(), 1);
        assert_eq!(src_entries[0].name, "main.rs");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
