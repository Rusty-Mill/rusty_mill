#!/usr/bin/env python3
"""Compile every workspace crate that declares `rust-version` with a rustc of exactly that version.

`check_rust_versions.py` compares declarations; this builds. Members are grouped by the
full version they declare (`1.74` is `1.74.0`), each version's rustc is installed, and
every member is `cargo check`ed on its own (default features, `--locked`) with cargo from
the pinned toolchain and `RUSTC` set to that rustc. One invocation per member keeps one
member's features from being unified into another's: a no_std default is checked as
declared, not with a sibling's `std`. A member with features beyond `default` is checked a
second time with `--all-features`, since an optional feature can need a newer rustc than its
defaults do; `--all-features-exclude` leaves out members whose declared floor deliberately
covers only their default build, and `--all-features-floor NAME=VERSION` checks such a member's
`--all-features` build at that higher floor instead of skipping it. Cargo itself refuses a package whose `rust-version`
exceeds that rustc, so a dependency above the floor fails too. Cargo from the pinned
toolchain is used on purpose: a cargo older than 1.85 cannot read this workspace's
edition-2024 manifests. Versions at or above `--skip-from` (the pinned toolchain) are
covered by the ordinary jobs and skipped.

A wrapper around rustc removes what a newer cargo adds that an older rustc rejects and
that cannot change whether the code compiles: `--check-cfg` (unstable before rustc 1.80)
and the `--warn=`/`--allow=`/`--deny=`/`--forbid=` flags cargo derives from a dependency's
`[lints]` table (an unknown lint name on the command line is a hard error in old rustc).

Usage: check_msrv_compile.py METADATA.json [--packages "a b"] [--exclude "c d"]
       [--all-features-exclude "e f"] [--all-features-floor "g=1.88"] [--skip-from 1.98.1]
       [--print-plan]
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


def featured_members(metadata: dict) -> frozenset[str]:
    """Workspace members that declare a feature other than `default` (optional dependencies count)."""
    members = set(metadata["workspace_members"])
    return frozenset(
        pkg["name"]
        for pkg in metadata["packages"]
        if pkg["id"] in members and any(f != "default" for f in pkg.get("features", {}))
    )


def parse_floors(text: str) -> dict[str, Version]:
    """Parse space-separated `name=version` pairs."""
    floors = {}
    for pair in text.split():
        name, sep, version = pair.partition("=")
        if not sep or not name or not version:
            raise ValueError(f"--all-features-floor: expected NAME=VERSION, got {pair!r}")
        try:
            floors[name] = parse_version(version)
        except ValueError:
            raise ValueError(f"--all-features-floor: bad version in {pair!r}") from None
    return floors


def raised_floor_groups(
    metadata: dict,
    groups: dict[Version, list[str]],
    floors: dict[str, Version],
    skip_from: Version | None = None,
) -> dict[Version, list[str]]:
    """Group the members whose `--all-features` build is checked at a floor above their declared one.

    Only members already in `groups` (the declared-floor plan, after scope and exclusions) are
    scheduled. A name that is not a featured member, or a floor not above the declared one, is
    an error, so a stale override fails instead of silently checking nothing.
    """
    featured = featured_members(metadata)
    declared = {name: version for version, names in groups.items() for name in names}
    raised: dict[Version, list[str]] = {}
    for name, floor in floors.items():
        if name not in featured:
            raise ValueError(f"--all-features-floor {name}: not a workspace member with features")
        if name not in declared:
            continue
        if floor <= declared[name]:
            raise ValueError(
                f"--all-features-floor {name}={format_version(floor)} is not above its "
                f"rust-version {format_version(declared[name])}"
            )
        if skip_from is not None and floor >= skip_from:
            continue
        raised.setdefault(floor, []).append(name)
    return {version: sorted(names) for version, names in sorted(raised.items())}


def check_commands(
    names: list[str],
    target_dir: str,
    all_features: frozenset[str] = frozenset(),
    all_features_only: list[str] | None = None,
) -> list[list[str]]:
    """One `cargo check` argv per member, all sharing `target_dir`, so features never unify across members.

    A member in `all_features` gets a second argv, with `--all-features`, right after its first.
    Each of `all_features_only` (members whose raised floor this is) gets only the `--all-features` argv.
    """
    commands = []
    for name in names:
        argv = ["cargo", "check", "--locked", "--keep-going", "--target-dir", target_dir, "-p", name]
        commands.append(argv)
        if name in all_features:
            commands.append([*argv, "--all-features"])
    for name in all_features_only or []:
        commands.append(["cargo", "check", "--locked", "--keep-going", "--target-dir", target_dir, "-p", name, "--all-features"])
    return commands


def describe(argv: list[str]) -> str:
    """The member an argv checks, marked when it checks all features."""
    name = argv[argv.index("-p") + 1]
    return f"{name} (--all-features)" if "--all-features" in argv else name


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


def check_version(
    version: Version,
    names: list[str],
    target_base: str,
    all_features: frozenset[str] = frozenset(),
    all_features_only: list[str] | None = None,
) -> list[str]:
    """Install exactly `version`'s rustc and check each member; return the checks that fail."""
    text = format_version(version)
    if run(["rustup", "toolchain", "install", text, "--profile", "minimal", "--no-self-update"]) != 0:
        return [*names, *(f"{n} (--all-features)" for n in all_features_only or [])]
    rustc = subprocess.run(
        ["rustup", "which", "rustc", "--toolchain", text], capture_output=True, text=True, check=True
    ).stdout.strip()
    env = {**os.environ, **rustc_env(rustc, Path(target_base))}
    target_dir = f"{target_base}/{text}"
    try:
        commands = check_commands(names, target_dir, all_features, all_features_only)
        return [describe(argv) for argv in commands if run(argv, env) != 0]
    finally:
        # One version's build output is useless to the next and a runner's disk is finite.
        shutil.rmtree(target_dir, ignore_errors=True)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("metadata", type=Path)
    parser.add_argument("--packages", help="space-separated package names to consider (default: all)")
    parser.add_argument("--exclude", default="", help="space-separated package names to leave out")
    parser.add_argument(
        "--all-features-exclude", default="", help="space-separated package names to check with default features only"
    )
    parser.add_argument(
        "--all-features-floor",
        default="",
        help="space-separated NAME=VERSION: check NAME --all-features at VERSION instead of its rust-version",
    )
    parser.add_argument("--skip-from", default=None, help="skip versions at or above this one (e.g. 1.98.1)")
    parser.add_argument("--target-base", default="target/msrv")
    parser.add_argument("--print-plan", action="store_true", help="print the groups and exit")
    args = parser.parse_args()

    metadata = json.loads(args.metadata.read_text())
    skip_from = parse_version(args.skip_from) if args.skip_from else None
    groups = msrv_groups(
        metadata,
        only=set(args.packages.split()) if args.packages is not None else None,
        exclude=frozenset(args.exclude.split()),
        skip_from=skip_from,
    )
    try:
        floors = parse_floors(args.all_features_floor)
        raised = raised_floor_groups(metadata, groups, floors, skip_from)
    except ValueError as err:
        sys.exit(f"error: {err}")
    # A member with a raised floor is not also checked with --all-features at its declared one.
    all_features = featured_members(metadata) - frozenset(args.all_features_exclude.split()) - frozenset(floors)
    for version in sorted({*groups, *raised}):
        names = groups.get(version, [])
        line = f"{format_version(version)}: {len(names)} crates, {len(all_features.intersection(names))} also with --all-features"
        if version in raised:
            line += f", --all-features only: {' '.join(raised[version])}"
        print(line)
    if args.print_plan:
        return
    failed = {
        v: bad
        for v in sorted({*groups, *raised})
        if (bad := check_version(v, groups.get(v, []), args.target_base, all_features, raised.get(v)))
    }
    if failed:
        print("Did not build at the declared rust-version:", file=sys.stderr)
        for version, names in failed.items():
            print(f"  - {format_version(version)}: {' '.join(names)}", file=sys.stderr)
        sys.exit(1)


if __name__ == "__main__":
    main()
