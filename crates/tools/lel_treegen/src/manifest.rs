//! The manifest: canonical JSON Lines naming the tree exactly, and its digest.
//!
//! Line 1 is the header, then one line per entry in plan order, then one line
//! per expected group. The bytes are canonical (fixed field order, no
//! whitespace), so `sha256(manifest)` with the seed, scale and profile digest
//! names the tree. The manifest is the answer key: it stays in a harness-only
//! directory, never under the tree or any sandbox or builder mount.

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::plan::{Entry, Group, Plan};
use crate::profile::{Profile, ProfileError, hex};

/// Manifest schema and generator version.
pub const SCHEMA: &str = "lel-treegen/1";

#[derive(Serialize)]
struct Header<'a> {
    record: &'static str,
    schema: &'static str,
    seed: u64,
    scale: f64,
    profile_source: &'a str,
    profile_sha256: String,
    entries: usize,
    groups: usize,
    bytes: u64,
}

#[derive(Serialize)]
struct EntryLine<'a> {
    record: &'static str,
    #[serde(flatten)]
    entry: &'a Entry,
}

#[derive(Serialize)]
struct GroupLine<'a> {
    record: &'static str,
    #[serde(flatten)]
    group: &'a Group,
}

/// The canonical manifest bytes.
///
/// # Errors
/// Only if serialization fails, which a plan from [`crate::plan`] never does.
pub fn render(plan: &Plan, profile: &Profile) -> Result<Vec<u8>, ProfileError> {
    let header = Header {
        record: "header",
        schema: SCHEMA,
        seed: plan.seed,
        scale: plan.scale,
        profile_source: &profile.source,
        profile_sha256: profile.digest()?,
        entries: plan.entries.len(),
        groups: plan.groups.len(),
        bytes: plan.bytes(),
    };
    let mut out = Vec::new();
    line(&mut out, &header)?;
    for entry in &plan.entries {
        line(
            &mut out,
            &EntryLine {
                record: "entry",
                entry,
            },
        )?;
    }
    for group in &plan.groups {
        line(
            &mut out,
            &GroupLine {
                record: "group",
                group,
            },
        )?;
    }
    Ok(out)
}

fn line(out: &mut Vec<u8>, value: &impl Serialize) -> Result<(), ProfileError> {
    serde_json::to_writer(&mut *out, value).map_err(|e| ProfileError(e.to_string()))?;
    out.push(b'\n');
    Ok(())
}

/// Lowercase hex sha256 of manifest bytes.
#[must_use]
pub fn digest(manifest: &[u8]) -> String {
    hex(&Sha256::digest(manifest))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::plan;

    fn manifest(seed: u64, scale: f64) -> Vec<u8> {
        let profile = Profile::default();
        render(&plan(seed, scale, &profile).expect("plan"), &profile).expect("render")
    }

    #[test]
    fn same_seed_gives_the_same_digest() {
        assert_eq!(digest(&manifest(11, 0.001)), digest(&manifest(11, 0.001)));
        assert_ne!(digest(&manifest(11, 0.001)), digest(&manifest(12, 0.001)));
    }

    /// A seed must name the same tree forever: any change to the PRNG, the
    /// planner, the default profile or the manifest format breaks this pin and
    /// needs a new `SCHEMA` version.
    #[test]
    fn golden_digest_is_pinned() {
        assert_eq!(
            digest(&manifest(1, 0.0001)),
            "316f774fe70905f6047c40dfe8c82de26605441225e853ae7764796ae0eebdfe"
        );
    }

    #[test]
    fn lines_are_header_entries_then_groups() {
        let m = String::from_utf8(manifest(13, 0.001)).expect("utf-8");
        let records: Vec<String> = m
            .lines()
            .map(|l| {
                serde_json::from_str::<serde_json::Value>(l).expect("json line")["record"]
                    .to_string()
            })
            .collect();
        assert_eq!(records[0], "\"header\"");
        let first_group = records
            .iter()
            .position(|r| r == "\"group\"")
            .expect("some group");
        assert!(records[1..first_group].iter().all(|r| r == "\"entry\""));
        assert!(records[first_group..].iter().all(|r| r == "\"group\""));
        let header: serde_json::Value =
            serde_json::from_str(m.lines().next().expect("header")).expect("json");
        assert_eq!(header["schema"], SCHEMA);
        assert_eq!(header["entries"].as_u64(), Some(first_group as u64 - 1));
    }

    #[test]
    fn profile_changes_the_digest() {
        let base = Profile::default();
        let other = Profile {
            nas_share: 0.5,
            ..Profile::default()
        };
        let a = render(&plan(1, 0.001, &base).expect("plan"), &base).expect("render");
        let b = render(&plan(1, 0.001, &other).expect("plan"), &other).expect("render");
        assert_ne!(digest(&a), digest(&b));
    }
}
