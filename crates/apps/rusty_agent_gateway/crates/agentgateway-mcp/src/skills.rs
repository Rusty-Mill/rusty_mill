//! Skill packs: knowledge a lens may read as MCP resources (LEL #1381).
//!
//! A pack is a named set of text files (726-sandbox-design §8). The gateway
//! serves each file as the resource `skill://<pack>/<path>`, and a token sees
//! only the packs its lens policy lists in `[skills].packs`. Each pack carries
//! a digest of its contents, so a session can report exactly which knowledge
//! it was given.
//!
//! This module is the pure core: it takes files already read, checks them
//! once, and answers listings and reads from an immutable snapshot. Where the
//! files come from is the caller's business.
//!
//! # Fails closed
//!
//! A bad pack name, a path that could leave its pack, a file that is not
//! UTF-8, an oversize file or a duplicate refuses the whole catalog at load,
//! so a broken pack stops startup instead of half-serving. A read of a pack
//! the caller may not see is answered exactly like a read of a file that does
//! not exist.

use std::collections::BTreeMap;

/// The URI scheme of every skill resource.
pub const SKILL_SCHEME: &str = "skill://";

/// The largest file a pack may hold. Knowledge is read into a model's
/// context; a file past this is a mistake, not a pack.
pub const MAX_SKILL_FILE_BYTES: usize = 64 * 1024;

/// A skill catalog that can't be loaded.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("skill `{pack}/{path}`: {reason}")]
pub struct SkillError {
    /// The pack the file was given for.
    pub pack: String,
    /// The file's path inside the pack.
    pub path: String,
    /// What is wrong with it.
    pub reason: String,
}

/// One file as a listing reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillEntry<'a> {
    /// `skill://<pack>/<path>`.
    pub uri: String,
    /// The pack it belongs to.
    pub pack: &'a str,
    /// Its path inside the pack.
    pub path: &'a str,
    /// The pack's digest (see [`SkillCatalog::digest`]).
    pub digest: &'a str,
    /// Its length in bytes.
    pub size: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Pack {
    digest: String,
    files: BTreeMap<String, String>,
}

/// Every skill pack the gateway serves, checked and fixed at load.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SkillCatalog {
    packs: BTreeMap<String, Pack>,
}

impl SkillCatalog {
    /// A catalog from `(pack, path, bytes)` triples. The order of the
    /// triples never changes the result.
    ///
    /// # Errors
    /// The first file whose pack name, path or contents is refused, or
    /// that appears twice.
    pub fn from_files(
        files: impl IntoIterator<Item = (String, String, Vec<u8>)>,
    ) -> Result<Self, SkillError> {
        let mut contents: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
        for (pack, path, bytes) in files {
            let text = check(&pack, &path, bytes)?;
            let files = contents.entry(pack.clone()).or_default();
            if files.insert(path.clone(), text).is_some() {
                return Err(fail(&pack, &path, "listed twice"));
            }
        }
        let packs = contents
            .into_iter()
            .map(|(name, files)| {
                let digest = pack_digest(&files);
                (name, Pack { digest, files })
            })
            .collect();
        Ok(SkillCatalog { packs })
    }

    /// How many packs are loaded.
    pub fn len(&self) -> usize {
        self.packs.len()
    }

    /// Whether none are.
    pub fn is_empty(&self) -> bool {
        self.packs.is_empty()
    }

    /// The digest of `pack`: the sha256, lowercase hex, of one line per file
    /// in path order, each `<path> <sha256 of the file>`. It changes when
    /// any file is added, removed, renamed or edited, and only then.
    pub fn digest(&self, pack: &str) -> Option<&str> {
        self.packs.get(pack).map(|p| p.digest.as_str())
    }

    /// The names in `allowed` that are not loaded packs, for the caller to
    /// report. A missing pack serves nothing; it is not an error.
    pub fn missing<'a>(&self, allowed: &'a [String]) -> Vec<&'a str> {
        allowed
            .iter()
            .filter(|name| !self.packs.contains_key(name.as_str()))
            .map(String::as_str)
            .collect()
    }

    /// Every file of the packs in `allowed`, by pack then path.
    pub fn list(&self, allowed: &[String]) -> Vec<SkillEntry<'_>> {
        self.packs
            .iter()
            .filter(|(name, _)| allowed.iter().any(|a| a == *name))
            .flat_map(|(name, pack)| {
                pack.files.iter().map(move |(path, text)| SkillEntry {
                    uri: format!("{SKILL_SCHEME}{name}/{path}"),
                    pack: name,
                    path,
                    digest: &pack.digest,
                    size: text.len(),
                })
            })
            .collect()
    }

    /// The text at `uri`, if it names a file of a pack in `allowed`.
    pub fn read(&self, uri: &str, allowed: &[String]) -> Option<&str> {
        let (pack, path) = uri.strip_prefix(SKILL_SCHEME)?.split_once('/')?;
        if !allowed.iter().any(|a| a == pack) {
            return None;
        }
        self.packs.get(pack)?.files.get(path).map(String::as_str)
    }
}

fn fail(pack: &str, path: &str, reason: impl Into<String>) -> SkillError {
    SkillError {
        pack: pack.to_owned(),
        path: path.to_owned(),
        reason: reason.into(),
    }
}

/// A name a URI and a directory can both hold: `[A-Za-z0-9._-]+`, not `.`
/// or `..`.
fn is_segment(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

fn check(pack: &str, path: &str, bytes: Vec<u8>) -> Result<String, SkillError> {
    if !is_segment(pack) {
        return Err(fail(pack, path, "a pack name must be [A-Za-z0-9._-]+"));
    }
    if !path.split('/').all(is_segment) {
        return Err(fail(
            pack,
            path,
            "a path must be relative, `/`-separated [A-Za-z0-9._-]+ segments, no `.` or `..`",
        ));
    }
    if bytes.len() > MAX_SKILL_FILE_BYTES {
        return Err(fail(
            pack,
            path,
            format!(
                "{} bytes, over the {MAX_SKILL_FILE_BYTES}-byte limit",
                bytes.len()
            ),
        ));
    }
    String::from_utf8(bytes).map_err(|_| fail(pack, path, "not UTF-8"))
}

fn sha256_hex(bytes: &[u8]) -> String {
    rusty_hex::encode(&rusty_rsa::sha256(bytes))
}

fn pack_digest(files: &BTreeMap<String, String>) -> String {
    let lines: String = files
        .iter()
        .map(|(path, text)| format!("{path} {}\n", sha256_hex(text.as_bytes())))
        .collect();
    sha256_hex(lines.as_bytes())
}

#[cfg(test)]
mod tests;
