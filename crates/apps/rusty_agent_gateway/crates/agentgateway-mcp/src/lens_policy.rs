//! Lens policies: which tools a jailed agent's token may see and call.
//!
//! One TOML file per lens (726-sandbox-design §5, LEL D102, D223). orch picks
//! the lens, mints a token naming it (`lel.lens`) and the sha256 of the file's
//! bytes (`lel.policy`); the gateway enforces. The two share only the file and
//! the claims, so neither imports the other.
//!
//! ```toml
//! schema = 1
//! lens = "builder-rust"
//! role = "builder"            # or "critic"
//!
//! [tools]                     # federated names, `*` globs
//! allow = ["sandbox_*", "board_card_get"]
//! deny  = ["github_merge_*"]
//! ```
//!
//! `[skills]` names the skill packs (#1381) the lens may read:
//! `packs = ["rust"]`, nothing else.
//!
//! `[workspace]` and `[grants]` belong to the design too. They must be
//! tables when present, but they configure the sandbox and the token, not
//! this gate, so their contents are not read here.
//!
//! # Fails closed
//!
//! A token with no lens, an unknown lens, or a digest that is not the loaded
//! file's sees **no tool** and can call none. A route with lens policies serves
//! no prompts or resources: the policy names only tools. A tool must match an
//! `allow` and no `deny`; an empty `allow` allows nothing.

use std::collections::BTreeMap;

use rusty_codec::toml::TomlValue;

use crate::glob::glob;
use crate::rules::Subject;

/// The only schema this build reads.
const SCHEMA: i64 = 1;

/// Top-level keys a policy may carry; anything else is a typo, refused.
/// The keys `[tools]` may hold. `workspace` and `grants` stay opaque here:
/// other readers own them.
const TOOL_KEYS: &[&str] = &["allow", "deny"];

/// The keys `[skills]` may hold.
const SKILL_KEYS: &[&str] = &["packs"];

const KEYS: &[&str] = &[
    "schema",
    "lens",
    "role",
    "tools",
    "workspace",
    "skills",
    "grants",
];

/// A lens policy that can't be loaded.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("lens policy `{file}`: {reason}")]
pub struct LensPolicyError {
    /// The file it came from.
    pub file: String,
    /// What is wrong with it.
    pub reason: String,
}

/// One lens's tool rules, and the digest a token must name to use them.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Policy {
    digest: String,
    allow: Vec<String>,
    deny: Vec<String>,
    packs: Vec<String>,
}

impl Policy {
    fn permits(&self, tool: &str) -> bool {
        self.allow.iter().any(|p| glob(p, tool)) && !self.deny.iter().any(|p| glob(p, tool))
    }
}

/// Every lens policy of a route, keyed by lens id.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LensPolicies {
    by_lens: BTreeMap<String, Policy>,
}

impl LensPolicies {
    /// Policies from `(file name, bytes)` pairs, such as a directory's
    /// `*.toml` files. Each file's `lens` must equal its name without
    /// `.toml`, and no lens may appear twice.
    ///
    /// # Errors
    /// The first file that does not parse or validate.
    pub fn from_files(
        files: impl IntoIterator<Item = (String, Vec<u8>)>,
    ) -> Result<Self, LensPolicyError> {
        let mut by_lens = BTreeMap::new();
        for (file, bytes) in files {
            let (lens, policy) = parse(&file, &bytes)?;
            if by_lens.insert(lens.clone(), policy).is_some() {
                return Err(fail(&file, format!("lens `{lens}` is defined twice")));
            }
        }
        Ok(LensPolicies { by_lens })
    }

    /// How many lenses are loaded.
    pub fn len(&self) -> usize {
        self.by_lens.len()
    }

    /// Whether none are.
    pub fn is_empty(&self) -> bool {
        self.by_lens.is_empty()
    }

    /// Whether the holder of `claims` may see or use `subject`, named
    /// `federated` to the client.
    pub fn permits(
        &self,
        subject: Subject<'_>,
        federated: &str,
        claims: Option<&serde_json::Value>,
    ) -> bool {
        let Subject::Tool(_) = subject else {
            return false;
        };
        self.policy_for(claims)
            .is_some_and(|policy| policy.permits(federated))
    }

    /// The skill packs the holder of `claims` may read: none unless its
    /// lens is known and its digest matches, exactly as for tools.
    pub fn skill_packs(&self, claims: Option<&serde_json::Value>) -> &[String] {
        self.policy_for(claims)
            .map_or(&[], |policy| policy.packs.as_slice())
    }

    /// The policy the token selects, if its lens is known and its digest is
    /// that policy's.
    fn policy_for(&self, claims: Option<&serde_json::Value>) -> Option<&Policy> {
        let claims = claims?;
        let lens = claims.get("lel.lens")?.as_str()?;
        let digest = claims.get("lel.policy")?.as_str()?;
        self.by_lens.get(lens).filter(|p| p.digest == digest)
    }
}

/// The sha256 of a policy file's bytes, lowercase hex: what orch puts in the
/// token's `lel.policy` claim.
pub fn policy_digest(bytes: &[u8]) -> String {
    rusty_hex::encode(&rusty_rsa::sha256(bytes))
}

fn fail(file: &str, reason: impl Into<String>) -> LensPolicyError {
    LensPolicyError {
        file: file.to_owned(),
        reason: reason.into(),
    }
}

fn parse(file: &str, bytes: &[u8]) -> Result<(String, Policy), LensPolicyError> {
    let text = std::str::from_utf8(bytes).map_err(|_| fail(file, "not UTF-8"))?;
    let doc = TomlValue::parse_str(text).map_err(|e| fail(file, e))?;
    let table = doc.as_table().ok_or_else(|| fail(file, "not a table"))?;
    if let Some(key) = table.keys().find(|k| !KEYS.contains(&k.as_str())) {
        return Err(fail(file, format!("unknown key `{key}`")));
    }

    if doc.get("schema").and_then(TomlValue::as_i64) != Some(SCHEMA) {
        return Err(fail(file, format!("`schema` must be {SCHEMA}")));
    }
    let lens = doc
        .get("lens")
        .and_then(TomlValue::as_str)
        .filter(|l| !l.is_empty())
        .ok_or_else(|| fail(file, "`lens` must be a non-empty string"))?;
    if file.strip_suffix(".toml") != Some(lens) {
        return Err(fail(
            file,
            format!("`lens = \"{lens}\"` must match the file name"),
        ));
    }
    match doc.get("role").and_then(TomlValue::as_str) {
        Some("builder" | "critic") => {}
        _ => return Err(fail(file, "`role` must be \"builder\" or \"critic\"")),
    }
    for section in ["workspace", "skills", "grants"] {
        if doc.get(section).is_some_and(|v| v.as_table().is_none()) {
            return Err(fail(file, format!("`{section}` must be a table")));
        }
    }

    let tools = doc.get("tools");
    if tools.is_some_and(|v| v.as_table().is_none()) {
        return Err(fail(file, "`tools` must be a table"));
    }
    // Fail closed: a misspelt `deny` would otherwise leave `allow` with no
    // exclusions, and a misspelt `packs` would silently serve no skills.
    let skills = doc.get("skills");
    check_keys(file, "tools", tools, TOOL_KEYS)?;
    check_keys(file, "skills", skills, SKILL_KEYS)?;

    let policy = Policy {
        digest: policy_digest(bytes),
        allow: strings(file, "tools", tools, "allow")?,
        deny: strings(file, "tools", tools, "deny")?,
        packs: strings(file, "skills", skills, "packs")?,
    };
    Ok((lens.to_owned(), policy))
}

/// Refuses any key of `section` outside `allowed`.
fn check_keys(
    file: &str,
    name: &str,
    section: Option<&TomlValue>,
    allowed: &[&str],
) -> Result<(), LensPolicyError> {
    let unknown = section
        .and_then(TomlValue::as_table)
        .and_then(|t| t.keys().find(|k| !allowed.contains(&k.as_str())));
    match unknown {
        Some(key) => Err(fail(file, format!("unknown key `{name}.{key}`"))),
        None => Ok(()),
    }
}

/// `section.key` as a list of non-empty strings; absent is empty.
fn strings(
    file: &str,
    name: &str,
    section: Option<&TomlValue>,
    key: &str,
) -> Result<Vec<String>, LensPolicyError> {
    let Some(value) = section.and_then(|t| t.get(key)) else {
        return Ok(Vec::new());
    };
    let wrong = || fail(file, format!("`{name}.{key}` must be a list of strings"));
    let items = value.as_array().ok_or_else(wrong)?;
    items
        .iter()
        .map(|item| {
            item.as_str()
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .ok_or_else(wrong)
        })
        .collect()
}

#[cfg(test)]
mod tests;
