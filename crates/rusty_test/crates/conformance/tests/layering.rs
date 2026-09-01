//! Enforces the Rusty Mill layer boundaries against the actual manifests.
//!
//! MSYS2 does not maintain its layering by discipline. Layer 1 is a DLL,
//! layer 2 binaries link against it, and the MINGW subsystems exist
//! precisely because they do *not* link it — the boundary is an ABI, and the
//! linker refuses to let anyone cheat. A Rust workspace has no equivalent
//! forcing function: crates compile together, so a layer boundary is exactly
//! as real as the discipline maintaining it.
//!
//! "Each upper layer depends downward; no lower layer depends upward" is
//! therefore a claim about this repository that nothing checked. In this
//! project specifically, that category of claim has already rotted twice —
//! the behavior matrix and the capability model both asserted things no code
//! enforced. This file is the forcing function the linker would have given
//! us for free.
//!
//! Scope: this checks **who may depend on whom**. It cannot check whether a
//! surface *belongs* in a layer at all — a networking trait added to
//! `contract` would sail through here. That is the complementary rule (name
//! the layer a new surface belongs to; reject "none"), and it needs a
//! reviewer, not a test.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Layer assignment for every workspace member.
///
/// Declared as data rather than inferred from directory names, so adding a
/// crate forces an explicit decision about where it sits — an unassigned
/// member is a hard failure below, not a default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Layer {
    /// Layer 1 API — the portable contract. The bottom of the graph.
    RuntimeApi,
    /// Layer 1 adapters — per-host implementations of the contract.
    RuntimeAdapter,
    /// Cross-cutting verification. Not a layer: it sits beside layer 1 and
    /// is allowed to see the adapters precisely because its job is to
    /// measure them.
    Verification,
    /// Layer 2 — tools built against layer 1. Today these are
    /// proof-of-contract binaries rather than a real userland.
    Userland,
}

fn layer_of(crate_name: &str) -> Option<Layer> {
    match crate_name {
        "contract" => Some(Layer::RuntimeApi),
        "compat" => Some(Layer::RuntimeAdapter),
        "conformance" => Some(Layer::Verification),
        "stat-tool" | "proc-runner" | "pty-shell" => Some(Layer::Userland),
        _ => None,
    }
}

/// Which workspace siblings each layer may depend on at runtime.
fn permitted_dependencies(layer: Layer) -> &'static [&'static str] {
    match layer {
        // The contract depends on nothing in-workspace. If it ever needs a
        // sibling, it has stopped being the bottom of the graph.
        Layer::RuntimeApi => &[],
        Layer::RuntimeAdapter => &["contract"],
        Layer::Verification => &["contract", "compat"],
        // Layer 2 may use the contract and an adapter to satisfy it, and
        // must never reach sideways into another layer-2 crate.
        Layer::Userland => &["contract", "compat"],
    }
}

/// This crate group's prefix within the Rusty Mill monorepo's `members` list.
///
/// Before the monorepo merge these crates were their own Cargo workspace and
/// `members` held exactly the six of them. They are now six members among
/// ~100, so every member path is filtered through this prefix: the layer
/// model below is a claim about *this* group, not about crates that were
/// never part of it. Without the filter, `every_workspace_member_is_assigned_a_layer`
/// would demand a layer for every unrelated crate in the monorepo.
const GROUP_PREFIX: &str = "crates/rusty_test/";

fn workspace_root() -> PathBuf {
    // CARGO_MANIFEST_DIR is crates/rusty_test/crates/conformance; the
    // monorepo workspace root is four up (it was two up when this crate
    // group was its own standalone workspace).
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(4)
        .expect("workspace root is four levels above this crate")
        .to_path_buf()
}

/// Reads this crate group's `members` out of the workspace manifest.
///
/// Panics rather than returning an empty list if the array cannot be found:
/// a check that silently examines zero crates is worse than no check, since
/// it reports success.
fn workspace_members(root: &Path) -> Vec<String> {
    let manifest =
        std::fs::read_to_string(root.join("Cargo.toml")).expect("read workspace manifest");
    let start = manifest
        .find("members = [")
        .expect("workspace manifest has a `members = [` array");
    let rest = &manifest[start..];
    let end = rest.find(']').expect("`members` array is closed");

    let members: Vec<String> = rest[..end]
        .lines()
        .skip(1)
        .filter_map(|line| {
            let trimmed = line.trim().trim_end_matches(',').trim_matches('"');
            (!trimmed.is_empty() && trimmed.starts_with(GROUP_PREFIX)).then(|| trimmed.to_string())
        })
        .collect();

    assert!(
        !members.is_empty(),
        "parsed zero workspace members under {GROUP_PREFIX:?} — the manifest \
         format changed and this check would otherwise pass vacuously"
    );
    members
}

/// The `[dependencies]` and `[target.*.dependencies]` tables of one manifest.
///
/// `[dev-dependencies]` are deliberately excluded: test-only wiring may
/// legitimately reach across the graph (rustils does exactly this, keeping
/// its parity and mock backends as dev-dependencies of every backend), and
/// nothing it does ships in a consumer's build.
///
/// Any line inside a dependency table that this parser does not recognize is
/// a hard failure. A dependency it cannot see is a dependency it cannot
/// police, and passing in that case would be the exact failure mode this
/// file exists to prevent.
fn runtime_dependencies(manifest_path: &Path) -> BTreeSet<String> {
    let text = std::fs::read_to_string(manifest_path)
        .unwrap_or_else(|e| panic!("read {}: {e}", manifest_path.display()));

    let mut deps = BTreeSet::new();
    let mut in_runtime_table = false;

    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        if line.starts_with('[') {
            // `[dependencies]` or `[target.'cfg(...)'.dependencies]`, but not
            // `[dev-dependencies]` / `[build-dependencies]`.
            in_runtime_table = line == "[dependencies]"
                || (line.starts_with("[target.") && line.ends_with(".dependencies]"));
            continue;
        }
        if !in_runtime_table {
            continue;
        }

        // Accepted shapes: `name.workspace = true`, `name = "1.0"`,
        // `name = { ... }`. Anything else stops the run.
        let name = line
            .split_once('=')
            .map(|(lhs, _)| lhs.trim().split('.').next().unwrap_or_default().trim())
            .filter(|n| {
                !n.is_empty()
                    && n.chars()
                        .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
            })
            .unwrap_or_else(|| {
                panic!(
                    "{}: unrecognized dependency line {raw:?}. This checker refuses to \
                     skip lines it cannot parse — teach it the new shape rather than \
                     letting an unpoliced dependency through.",
                    manifest_path.display()
                )
            });
        deps.insert(name.to_string());
    }
    deps
}

fn crate_name(manifest_path: &Path) -> String {
    let text = std::fs::read_to_string(manifest_path).expect("read member manifest");
    text.lines()
        .find_map(|line| line.trim().strip_prefix("name = "))
        .map(|n| n.trim().trim_matches('"').to_string())
        .unwrap_or_else(|| panic!("{}: no package name", manifest_path.display()))
}

#[test]
fn every_workspace_member_is_assigned_a_layer() {
    let root = workspace_root();
    let unassigned: Vec<String> = workspace_members(&root)
        .iter()
        .map(|m| crate_name(&root.join(m).join("Cargo.toml")))
        .filter(|name| layer_of(name).is_none())
        .collect();

    assert!(
        unassigned.is_empty(),
        "workspace members with no layer assignment: {unassigned:?}.\n\
         Add them to `layer_of` in this file. A new crate must be placed in \
         the architecture deliberately — there is no default layer, because a \
         default is how a runtime crate quietly acquires a userland dependency."
    );
}

#[test]
fn no_crate_depends_upward_or_sideways() {
    let root = workspace_root();
    let members = workspace_members(&root);

    // Only workspace siblings are policed; third-party crates are a
    // packaging concern, not a layering one.
    let siblings: BTreeSet<String> = members
        .iter()
        .map(|m| crate_name(&root.join(m).join("Cargo.toml")))
        .collect();

    let mut violations = Vec::new();
    for member in &members {
        let manifest = root.join(member).join("Cargo.toml");
        let name = crate_name(&manifest);
        let Some(layer) = layer_of(&name) else {
            continue; // reported by the assignment test
        };
        let permitted = permitted_dependencies(layer);

        for dep in runtime_dependencies(&manifest) {
            if !siblings.contains(&dep) || dep == name {
                continue;
            }
            if !permitted.contains(&dep.as_str()) {
                violations.push(format!(
                    "`{name}` ({layer:?}) depends on `{dep}` ({:?}); permitted: {permitted:?}",
                    layer_of(&dep)
                ));
            }
        }
    }

    assert!(
        violations.is_empty(),
        "layer boundary violations:\n  {}\n\n\
         Each upper layer depends downward; no lower layer depends upward. \
         Nothing in the Rust toolchain enforces this — that is why this test \
         exists.",
        violations.join("\n  ")
    );
}

#[test]
fn the_contract_is_the_bottom_of_the_graph() {
    // Called out separately from the general rule because it is the single
    // property the whole layer model rests on: if the layer-1 API acquires a
    // workspace dependency, every layer above it inherits that dependency and
    // the boundary stops meaning anything.
    let root = workspace_root();
    let deps = runtime_dependencies(&root.join(GROUP_PREFIX).join("crates/contract/Cargo.toml"));
    let siblings: BTreeSet<String> = workspace_members(&root)
        .iter()
        .map(|m| crate_name(&root.join(m).join("Cargo.toml")))
        .collect();

    let sibling_deps: Vec<&String> = deps.iter().filter(|d| siblings.contains(*d)).collect();
    assert!(
        sibling_deps.is_empty(),
        "`contract` gained workspace dependencies {sibling_deps:?}. The layer-1 \
         API must depend on nothing in-workspace."
    );
}

#[test]
fn the_parser_rejects_a_dependency_shape_it_cannot_read() {
    // The checker's own load-bearing property: it must fail on input it does
    // not understand rather than skipping the line and reporting success.
    // Without this test, a manifest-format change would silently disable
    // every assertion above.
    let dir = std::env::temp_dir().join(format!("layering-parser-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let manifest = dir.join("Cargo.toml");
    std::fs::write(&manifest, "[dependencies]\nthis line has no equals sign\n").expect("write");

    let result = std::panic::catch_unwind(|| runtime_dependencies(&manifest));
    std::fs::remove_dir_all(&dir).ok();

    assert!(
        result.is_err(),
        "the parser accepted a dependency line it cannot parse; it would skip \
         unpoliced dependencies and still report success"
    );
}
