#!/usr/bin/env python3
"""Classify a root `Cargo.toml` change as a safe pure-addition to
`[workspace] members`, or not.

`plan`'s blanket rule -- any root `Cargo.toml` edit triggers the full
workspace sweep -- exists because a `Cargo.toml` change can alter what
every crate resolves or builds against (`[workspace.dependencies]` version
bumps, `[profile.*]` tweaks, a removed/renamed member). But the single most
common `Cargo.toml` edit is the opposite of that: a PR that adds a handful
of brand-new, independent workspace members and touches nothing else in the
file (see PR #170's `rusty_hister` bootstrap, eight new members with no
existing line changed). That case has no blast radius on any existing
crate -- the new members' own files already flow through `affected_crates.py`
normally -- so it doesn't need a ~90-crate full sweep to validate safely.

A change is "safe" only when, comparing old (base) to new (HEAD) content:
  - `[workspace.members]` gained zero or more entries and lost none.
  - Every other `[workspace]` key (`exclude`, `resolver`, `dependencies`,
    `package`, ...) is unchanged.
  - Every top-level table other than `[workspace]` (`[profile.*]`,
    `[patch.*]`, ...) is unchanged.

Anything else -- a dependency version bump, a removed or reordered-with-
modification member, a `[profile]` edit, an unparsable file -- is "unsafe"
and the caller falls back to the full sweep, since those genuinely can
change what every workspace crate resolves or builds against.

Reads both `Cargo.toml` revisions from file paths (matching
`lockfile_diff.py`'s convention: the caller extracts the base revision via
`git show` into a temp file, the new revision is just the working tree's
`Cargo.toml`), so this module's own logic can be unit-tested against
in-memory text without a real git checkout.
"""
import sys
import tomllib
from pathlib import Path


def is_safe_pure_addition(old_text: str, new_text: str) -> bool:
    try:
        old = tomllib.loads(old_text)
        new = tomllib.loads(new_text)
    except tomllib.TOMLDecodeError:
        return False

    old_rest = {k: v for k, v in old.items() if k != "workspace"}
    new_rest = {k: v for k, v in new.items() if k != "workspace"}
    if old_rest != new_rest:
        return False

    old_ws = dict(old.get("workspace", {}))
    new_ws = dict(new.get("workspace", {}))
    old_members = old_ws.pop("members", [])
    new_members = new_ws.pop("members", [])

    if old_ws != new_ws:
        return False

    new_members_set = set(new_members)
    if any(member not in new_members_set for member in old_members):
        return False  # a member was removed or renamed -- not a pure addition

    return True


def main() -> None:
    old_path, new_path = sys.argv[1], sys.argv[2]
    old_text = Path(old_path).read_text()
    new_text = Path(new_path).read_text()
    print("safe" if is_safe_pure_addition(old_text, new_text) else "unsafe")


if __name__ == "__main__":
    main()
