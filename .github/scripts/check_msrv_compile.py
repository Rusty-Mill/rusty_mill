#!/usr/bin/env python3
"""Compile every workspace crate that declares `rust-version` with a rustc of that version.

`check_rust_versions.py` compares declarations; this builds. Members are grouped by the
major.minor floor they declare, each floor's rustc is installed, and the group is
`cargo check`ed (default features, `--locked`) with cargo from the pinned toolchain and
`RUSTC` set to the floor's rustc. Cargo itself refuses a package whose `rust-version`
exceeds that rustc, so a dependency above the floor fails here too. Cargo from the
pinned toolchain is used on purpose: a cargo older than 1.85 cannot read this
workspace's edition-2024 manifests. Floors at or above `--skip-from` (the pinned
toolchain) are covered by the ordinary jobs and are skipped.

Usage: check_msrv_compile.py METADATA.json [--packages "a b"] [--exclude "c d"]
       [--skip-from 1.98] [--print-plan]
"""
import argparse
import json
import os
import stat
import subprocess
import sys
from pathlib import Path

# Current cargo passes flags an older rustc rejects, though they only affect lints:
# `--check-cfg` (accepted without `-Z unstable-options` from 1.80) and each dependency's
# `[lints]` table as `--warn=NAME` etc. (a lint newer than the rustc is error E0602; a
# cargo of the floor's vintage would not pass these). A wrapper drops them before
# running the real rustc. Lints are enforced at the pinned toolchain by the other jobs.
STRIP_LINT_FLAGS = """#!/bin/sh
real="$1"; shift
n=$#; skip=0
while [ "$n" -gt 0 ]; do
  a="$1"; shift; n=$((n - 1))
  if [ "$skip" = 1 ]; then skip=0; continue; fi
  case "$a" in
    --check-cfg) skip=1; continue ;;
    --check-cfg=*) continue ;;
    --warn=*|--allow=*|--deny=*|--forbid=*|--force-warn=*) continue ;;
  esac
  set -- "$@" "$a"
done
exec "$real" "$@"
"""


def parse_floor(text: str) -> tuple[int, int]:
    """Parse `major.minor[.patch]` as (major, minor)."""
    parts = [int(p) for p in text.split(".")]
    return (parts[0], parts[1] if len(parts) > 1 else 0)


def msrv_groups(
    metadata: dict,
    only: set[str] | None = None,
    exclude: frozenset[str] = frozenset(),
    skip_from: tuple[int, int] | None = None,
) -> dict[tuple[int, int], list[str]]:
    """Group workspace members that declare a `rust-version` by their (major, minor) floor.

    `only` limits the members considered (None means all), `exclude` drops names, and
    floors at or above `skip_from` are left out. Groups come back in ascending floor
    order with sorted names.
    """
    members = set(metadata["workspace_members"])
    groups: dict[tuple[int, int], list[str]] = {}
    for pkg in metadata["packages"]:
        declared = pkg.get("rust_version")
        if pkg["id"] not in members or not declared:
            continue
        if (only is not None and pkg["name"] not in only) or pkg["name"] in exclude:
            continue
        floor = parse_floor(declared)
        if skip_from is not None and floor >= skip_from:
            continue
        groups.setdefault(floor, []).append(pkg["name"])
    return {floor: sorted(names) for floor, names in sorted(groups.items())}


def check_command(floor: tuple[int, int], names: list[str], target_base: str) -> list[str]:
    """The `cargo check` argv for one floor's group; its target dir is separate per floor."""
    argv = ["cargo", "check", "--locked", "--keep-going", "--target-dir", f"{target_base}/{floor[0]}.{floor[1]}"]
    for name in names:
        argv += ["-p", name]
    return argv


def run(argv: list[str], env: dict[str, str] | None = None) -> int:
    print("+", " ".join(argv), flush=True)
    return subprocess.run(argv, env=env, check=False).returncode


def check_floor(floor: tuple[int, int], names: list[str], target_base: str) -> bool:
    """Install the floor's rustc and check its group; True when the group builds."""
    version = f"{floor[0]}.{floor[1]}"
    if run(["rustup", "toolchain", "install", version, "--profile", "minimal", "--no-self-update"]) != 0:
        return False
    rustc = subprocess.run(
        ["rustup", "which", "rustc", "--toolchain", version], capture_output=True, text=True, check=True
    ).stdout.strip()
    rustc_env = rustc_for(floor, rustc, Path(target_base))
    return run(check_command(floor, names, target_base), {**os.environ, **rustc_env}) == 0


def rustc_for(floor: tuple[int, int], rustc: str, work_dir: Path) -> dict[str, str]:
    """Environment that makes cargo use `rustc` through the lint-flag-stripping wrapper."""
    work_dir.mkdir(parents=True, exist_ok=True)
    wrapper = work_dir / f"rustc-{floor[0]}.{floor[1]}.sh"
    wrapper.write_text(STRIP_LINT_FLAGS)
    wrapper.chmod(wrapper.stat().st_mode | stat.S_IXUSR)
    # cargo runs `$RUSTC_WRAPPER rustc ...`; the wrapper takes the real path as its first argument.
    return {"RUSTC": rustc, "RUSTC_WRAPPER": str(wrapper.resolve())}


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("metadata", type=Path)
    parser.add_argument("--packages", help="space-separated package names to consider (default: all)")
    parser.add_argument("--exclude", default="", help="space-separated package names to leave out")
    parser.add_argument("--skip-from", default=None, help="skip floors at or above this major.minor")
    parser.add_argument("--target-base", default="target/msrv")
    parser.add_argument("--print-plan", action="store_true", help="print the groups and exit")
    args = parser.parse_args()

    groups = msrv_groups(
        json.loads(args.metadata.read_text()),
        only=set(args.packages.split()) if args.packages is not None else None,
        exclude=frozenset(args.exclude.split()),
        skip_from=parse_floor(args.skip_from) if args.skip_from else None,
    )
    for floor, names in groups.items():
        print(f"{floor[0]}.{floor[1]}: {len(names)} crates")
    if args.print_plan:
        return
    failed = [floor for floor, names in groups.items() if not check_floor(floor, names, args.target_base)]
    if failed:
        print("Did not build at the declared rust-version: " + ", ".join(f"{f[0]}.{f[1]}" for f in failed), file=sys.stderr)
        sys.exit(1)


if __name__ == "__main__":
    main()
