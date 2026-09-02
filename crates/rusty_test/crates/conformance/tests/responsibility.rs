//! Enforces the Layer-1 scope rules from `ARCHITECTURE.md`.
//!
//! Companion to `layering.rs`, and deliberately separate from it because
//! they catch opposite failures. `layering.rs` polices **who may depend on
//! whom** — it would wave through a networking trait added to `contract`.
//! This file polices **what may live in Layer 1 at all**, and would wave
//! through a sneaky upward dependency. Neither subsumes the other.
//!
//! Two rules, both decided in review rather than invented here:
//!
//! 1. Every public Layer-1 API root declares one responsibility from a
//!    **closed vocabulary**. A closed set is the whole point: free text
//!    cannot be checked, so "nobody named anything" would stay a thing a
//!    reviewer has to notice, and noticing is the step that gets skipped.
//! 2. A Layer-1 adapter may depend on a **Layer-0a host binding** only from
//!    an explicit allow-list of `(crate, target, responsibility)`.
//!
//! What neither rule can do is judge whether a declared tag is *honest* —
//! nothing here stops someone tagging a networking trait `filesystem`. That
//! stays a review question, and `ARCHITECTURE.md` says so explicitly. The
//! value is splitting the common failure (omission, now mechanical) from the
//! rare one (misclassification, still human).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// The closed Layer-1 responsibility vocabulary.
///
/// Drawn from what `msys-2.0.dll` was actually responsible for. Adding an
/// entry here is an architecture change, not a convenience: it widens what
/// Layer 1 is permitted to be.
const RESPONSIBILITIES: &[&str] = &[
    "paths",
    "filesystem",
    "process",
    "terminal",
    "environment",
    "locking",
    "standard-directories",
    "capabilities",
    "errors",
];

/// The marker a public Layer-1 item carries in its doc comment.
const MARKER: &str = "/// Layer-1 responsibility:";

/// Layer-0a host bindings a Layer-1 adapter is permitted to link.
///
/// Empty on purpose. `compat` currently satisfies every responsibility from
/// portable crates (cap-std, portable-pty, dirs), so there is no measured
/// adapter gap to justify a target-specific binding. An entry here is a
/// deliberate architecture decision recording that a gap was demonstrated —
/// it is not a list to pad because a crate looks useful.
const ALLOWED_HOST_BINDINGS: &[(&str, &str, &str)] = &[
    // (crate, target predicate, responsibility)
];

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root is two levels above this crate")
        .to_path_buf()
}

/// One public API root and the responsibility it declares, if any.
#[derive(Debug)]
struct Surface {
    name: String,
    declared: Option<String>,
}

/// Scans Rust source for public API roots and the tag each one carries.
///
/// Split out from [`layer1_surfaces`] so its walk-back rule can be tested
/// against synthetic input. Testing it against the real `contract` source
/// cannot work: every property you would want to check there depends on
/// which tags the neighbouring items happen to carry.
fn scan_surfaces(text: &str) -> Vec<Surface> {
    let mut surfaces = Vec::new();
    let lines: Vec<&str> = text.lines().collect();

    for (index, line) in lines.iter().enumerate() {
        let Some(rest) = line
            .strip_prefix("pub trait ")
            .or_else(|| line.strip_prefix("pub struct "))
            .or_else(|| line.strip_prefix("pub enum "))
        else {
            continue;
        };
        let name: String = rest
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();

        // Walk back over attributes and the doc comment looking for the
        // marker. Stop at the first line that is neither, so a tag on an
        // unrelated earlier item cannot be credited to this one.
        let mut declared = None;
        for probe in lines[..index].iter().rev() {
            let trimmed = probe.trim();
            if let Some(tag) = trimmed.strip_prefix(MARKER) {
                declared = Some(tag.trim().to_string());
                break;
            }
            if !trimmed.starts_with("///") && !trimmed.starts_with("#[") {
                break;
            }
        }
        surfaces.push(Surface { name, declared });
    }
    surfaces
}

/// Scans `contract` for public API roots and the tag each one carries.
///
/// Panics if it finds none: a scanner that silently matches nothing would
/// report success while checking nothing, which is the failure mode both
/// gates in this repo exist to prevent.
fn layer1_surfaces(root: &Path) -> Vec<Surface> {
    let text = std::fs::read_to_string(root.join("crates/contract/src/lib.rs"))
        .expect("read the contract crate");
    let surfaces = scan_surfaces(&text);

    assert!(
        !surfaces.is_empty(),
        "found no public API roots in `contract` — the scan pattern no longer \
         matches the source, and every assertion below would pass vacuously"
    );
    surfaces
}

#[test]
fn every_layer1_surface_declares_a_responsibility() {
    let surfaces = layer1_surfaces(&workspace_root());
    let undeclared: Vec<&str> = surfaces
        .iter()
        .filter(|s| s.declared.is_none())
        .map(|s| s.name.as_str())
        .collect();

    assert!(
        undeclared.is_empty(),
        "public Layer-1 API roots with no declared responsibility: {undeclared:?}\n\n\
         Add `{MARKER} <tag>` to the doc comment, choosing from {RESPONSIBILITIES:?}.\n\
         If none of them fits, the surface does not belong in Layer 1 — that is \
         the rule working, not the vocabulary being too small."
    );
}

#[test]
fn no_surface_declares_a_responsibility_outside_the_vocabulary() {
    let surfaces = layer1_surfaces(&workspace_root());
    let unknown: Vec<String> = surfaces
        .iter()
        .filter_map(|s| s.declared.as_ref().map(|d| (s, d)))
        .filter(|(_, declared)| !RESPONSIBILITIES.contains(&declared.as_str()))
        .map(|(s, declared)| format!("{} -> `{declared}`", s.name))
        .collect();

    assert!(
        unknown.is_empty(),
        "responsibilities outside the closed vocabulary: {unknown:?}\n\n\
         Permitted: {RESPONSIBILITIES:?}. Widening this list is an architecture \
         change — it enlarges what Layer 1 is allowed to be — so it belongs in \
         a reviewed edit to ARCHITECTURE.md, not in the tag of a new surface."
    );
}

#[test]
fn no_layer1_adapter_links_an_unlisted_host_binding() {
    // Target-gated dependencies are how a per-OS binding enters an adapter.
    // Portable dependencies in the plain `[dependencies]` table are not
    // Layer-0a bindings and are out of scope here.
    let manifest = workspace_root().join("crates/compat/Cargo.toml");
    let text = std::fs::read_to_string(&manifest).expect("read the compat manifest");

    let allowed: BTreeSet<&str> = ALLOWED_HOST_BINDINGS.iter().map(|(c, _, _)| *c).collect();
    let mut unlisted = Vec::new();
    let mut in_target_table = false;

    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') {
            in_target_table = line.starts_with("[target.") && line.ends_with(".dependencies]");
            continue;
        }
        if !in_target_table {
            continue;
        }
        let name = line
            .split_once('=')
            .map(|(lhs, _)| lhs.trim().split('.').next().unwrap_or_default().trim())
            .unwrap_or_else(|| panic!("unrecognized dependency line {raw:?} in {manifest:?}"));
        if !allowed.contains(name) {
            unlisted.push(name.to_string());
        }
    }

    assert!(
        unlisted.is_empty(),
        "`compat` links target-specific host bindings that are not allow-listed: \
         {unlisted:?}\n\n\
         A Layer-0a binding is permitted only when it is target-gated, private to \
         the adapter, tied to one Layer-1 responsibility, and justified by a \
         demonstrated gap in the portable stack. Record it in \
         ALLOWED_HOST_BINDINGS with its target and responsibility, and say why in \
         ARCHITECTURE.md."
    );
}

#[test]
fn the_allow_list_only_names_known_responsibilities() {
    // The allow-list is data someone edits under time pressure. If an entry
    // could name a responsibility outside the vocabulary, the allow-list
    // would become a side door around the closed set.
    let bad: Vec<&str> = ALLOWED_HOST_BINDINGS
        .iter()
        .filter(|(_, _, responsibility)| !RESPONSIBILITIES.contains(responsibility))
        .map(|(krate, _, _)| *krate)
        .collect();

    assert!(
        bad.is_empty(),
        "allow-list entries naming a responsibility outside the vocabulary: {bad:?}"
    );
}

#[test]
fn a_tag_does_not_carry_across_intervening_code() {
    // The subtle failure available to this scanner: crediting one item's tag
    // to a later *untagged* item, which would silently exempt every new
    // surface and quietly disable the omission test above.
    //
    // Checked against synthetic source rather than the real `contract`,
    // because the real source cannot discriminate. `ProcessOutput` follows
    // `ProcessSpec` across an `impl` block, but both are tagged `process` —
    // so a scanner that wrongly carried the tag forward would produce the
    // identical answer and the assertion would pass. The tags below are
    // deliberately distinct so a wrongly-carried tag is observable.
    const SOURCE: &str = "\
/// Layer-1 responsibility: filesystem
pub struct Tagged {}

impl Tagged {
    pub fn helper(&self) {}
}

pub struct Untagged {}

/// Layer-1 responsibility: locking
pub trait AlsoTagged {}
";

    let surfaces = scan_surfaces(SOURCE);
    let names: Vec<&str> = surfaces.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["Tagged", "Untagged", "AlsoTagged"]);

    assert_eq!(surfaces[0].declared.as_deref(), Some("filesystem"));
    assert_eq!(
        surfaces[1].declared, None,
        "`Untagged` inherited a tag across an intervening `impl` block; every \
         new surface would be silently exempt from the omission check"
    );
    assert_eq!(
        surfaces[2].declared.as_deref(),
        Some("locking"),
        "a tagged item after untagged code must still read its own tag"
    );
}

#[test]
fn the_scanner_finds_every_shape_of_public_root() {
    // If a `pub` form stopped matching, the surfaces carrying it would vanish
    // from every check above while the suite stayed green.
    let surfaces = scan_surfaces(
        "/// Layer-1 responsibility: errors\npub enum E {}\n\
         /// Layer-1 responsibility: process\npub struct S {}\n\
         /// Layer-1 responsibility: locking\npub trait T {}\n",
    );
    let names: Vec<&str> = surfaces.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(
        names,
        ["E", "S", "T"],
        "a public root form stopped matching"
    );
}
