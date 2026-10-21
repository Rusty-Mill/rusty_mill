#!/usr/bin/env python3
"""Compile every workspace crate that declares `rust-version` with a rustc of exactly that version.

`check_rust_versions.py` compares declarations; this builds. Members are grouped by the
full version they declare (`1.74` is `1.74.0`), each version's rustc is installed, and
every member is `cargo check`ed on its own (default features, `--locked`) with cargo from
the pinned toolchain and `RUSTC` set to that rustc. One invocation per member keeps one
member's features from being unified into another's: a no_std default is checked as
declared, not with a sibling's `std`. Cargo itself refuses a package whose `rust-version`
exceeds that rustc, so a dependency above the floor fails too. Cargo from the pinned
toolchain is used on purpose: a cargo older than 1.85 cannot read this workspace's
edition-2024 manifests. Versions at or above `--skip-from` (the pinned toolchain) are
covered by the ordinary jobs and skipped.

A wrapper around rustc removes what a newer cargo adds that an older rustc rejects and
that cannot change whether the code compiles: `--check-cfg` (unstable before rustc 1.80)
and the `--warn=`/`--allow=`/`--deny=`/`--forbid=` flags cargo derives from a dependency's
`[lints]` table (an unknown lint name on the command line is a hard error in old rustc).

Usage: check_msrv_compile.py METADATA.json [--packages "a b"] [--exclude "c d"]
       [--skip-from 1.98.1] [--print-plan]
"""
import argparse
import json
import os
import shutil
import stat
import subprocess
import sys
from pathlib import Path

Version = tuple[int, int, int]

STRIP_COMPILER_FLAGS = """#!/bin/sh
real="$1"; shift
n=$#; skip=0
while [ "$n" -gt 0 ]; do
  a="$1"; shift; n=$((n - 1))
  if [ "$skip" = 1 ]; then skip=0; continue; fi
  case "$a" in
    --check-cfg) skip=1; continue ;;
    --check-cfg=*|--warn=*|--allow=*|--deny=*|--forbid=*) continue ;;
  esac
  set -- "$@" "$a"
done
exec "$real" "$@"
"""


def parse_version(text: str) -> Version:
    """Parse `major[.minor[.patch]]`; omitted components are zero, so `1.85` equals `1.85.0`."""
    parts = [int(p) for p in text.split(".")]
    major, minor, patch = (parts + [0, 0, 0])[:3]
    return (major, minor, patch)


def format_version(version: Version) -> str:
    return ".".join(str(part) for part in version)


def msrv_groups(
    metadata: dict,
    only: set[str] | None = None,
    exclude: frozenset[str] = frozenset(),
    skip_from: Version | None = None,
) -> dict[Version, list[str]]:
    """Group workspace members that declare a `rust-version` by their full declared version.

    `only` limits the members considered (None means all), `exclude` drops names, and
    versions at or above `skip_from` are left out. Groups come back in ascending order
    with sorted names.
    """
    members = set(metadata["workspace_members"])
    groups: dict[Version, list[str]] = {}
    for pkg in metadata["packages"]:
        declared = pkg.get("rust_version")
        if pkg["id"] not in members or not declared:
            continue
        if (only is not None and pkg["name"] not in only) or pkg["name"] in exclude:
            continue
        version = parse_version(declared)
        if skip_from is not None and version >= skip_from:
            continue
        groups.setdefault(version, []).append(pkg["name"])
    return {version: sorted(names) for version, names in sorted(groups.items())}


def check_commands(names: list[str], target_dir: str) -> list[list[str]]:
    """One `cargo check` argv per member, all sharing `target_dir`, so features never unify across members."""
    return [["cargo", "check", "--locked", "--keep-going", "--target-dir", target_dir, "-p", name] for name in names]


def run(argv: list[str], env: dict[str, str] | None = None) -> int:
    print("+", " ".join(argv), flush=True)
    return subprocess.run(argv, env=env, check=False).returncode


def rustc_env(rustc: str, work_dir: Path) -> dict[str, str]:
    """Environment that makes cargo run `rustc` through the flag-stripping wrapper."""
    work_dir.mkdir(parents=True, exist_ok=True)
    wrapper = work_dir / "rustc-wrapper.sh"
    wrapper.write_text(STRIP_COMPILER_FLAGS)
    wrapper.chmod(wrapper.stat().st_mode | stat.S_IXUSR)
    # cargo runs `$RUSTC_WRAPPER $RUSTC <args>`; the wrapper takes the real rustc as its first argument.
    return {"RUSTC": rustc, "RUSTC_WRAPPER": str(wrapper.resolve())}


def check_version(version: Version, names: list[str], target_base: str) -> list[str]:
    """Install exactly `version`'s rustc and check each member; return the members that fail."""
    text = format_version(version)
    if run(["rustup", "toolchain", "install", text, "--profile", "minimal", "--no-self-update"]) != 0:
        return list(names)
    rustc = subprocess.run(
        ["rustup", "which", "rustc", "--toolchain", text], capture_output=True, text=True, check=True
    ).stdout.strip()
    env = {**os.environ, **rustc_env(rustc, Path(target_base))}
    target_dir = f"{target_base}/{text}"
    try:
        return [name for name, argv in zip(names, check_commands(names, target_dir)) if run(argv, env) != 0]
    finally:
        # One version's build output is useless to the next and a runner's disk is finite.
        shutil.rmtree(target_dir, ignore_errors=True)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("metadata", type=Path)
    parser.add_argument("--packages", help="space-separated package names to consider (default: all)")
    parser.add_argument("--exclude", default="", help="space-separated package names to leave out")
    parser.add_argument("--skip-from", default=None, help="skip versions at or above this one (e.g. 1.98.1)")
    parser.add_argument("--target-base", default="target/msrv")
    parser.add_argument("--print-plan", action="store_true", help="print the groups and exit")
    args = parser.parse_args()

    groups = msrv_groups(
        json.loads(args.metadata.read_text()),
        only=set(args.packages.split()) if args.packages is not None else None,
        exclude=frozenset(args.exclude.split()),
        skip_from=parse_version(args.skip_from) if args.skip_from else None,
    )
    for version, names in groups.items():
        print(f"{format_version(version)}: {len(names)} crates")
    if args.print_plan:
        return
    failed = {v: bad for v, names in groups.items() if (bad := check_version(v, names, args.target_base))}
    if failed:
        print("Did not build at the declared rust-version:", file=sys.stderr)
        for version, names in failed.items():
            print(f"  - {format_version(version)}: {' '.join(names)}", file=sys.stderr)
        sys.exit(1)


if __name__ == "__main__":
    main()
