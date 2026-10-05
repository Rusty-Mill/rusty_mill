"""Small, runner-independent helpers for CI's specialized-job selection.

The workflow already delegates Cargo dependency impact to affected_crates.py.
This module covers path-only front ends and package-specific checks while
making the selection contract unit-testable.
"""

from __future__ import annotations

import argparse
import subprocess
import sys
from collections.abc import Iterable


PATH_JOB_PREFIXES = {
    "dashboard": "crates/apps/rusty_meshed/data-mesh-monitor/",
    "term_web": "crates/libs/ui/rusty_term/web/",
    "key_desktop": "crates/apps/rusty_key/desktop/",
    "tick": "crates/apps/rusty_tick/",
    "fair_play": "crates/apps/rusty_fair_play/",
}

PACKAGE_JOB_PREFIXES = {
    "dashboard": ("rusty-meshed-",),
    "key_desktop": ("rk-",),
}

PACKAGE_JOB_NAMES = {
    "term_web": frozenset({"rusty_term"}),
    "tick": frozenset({"rusty_tick"}),
    "fair_play": frozenset({"rusty_fair_play"}),
    "win32": frozenset({"rusty_win32"}),
    "multimodal_db": frozenset({"rusty_multimodal_db"}),
    "rusty_config_no_std": frozenset({"rusty_config"}),
}

SPECIALIZED_JOB_NAMES = tuple(
    dict.fromkeys((*PATH_JOB_PREFIXES, *PACKAGE_JOB_NAMES, "remind_me"))
)


def full_plan_outputs() -> dict[str, str]:
    """Return the complete output set for every conservative full fallback."""
    return {
        "full": "true",
        "packages": "",
        "ci_only": "false",
        **{job: "true" for job in SPECIALIZED_JOB_NAMES},
        "shards": "[1,2,3]",
    }


def ci_smoke_plan_outputs() -> dict[str, str]:
    """Return the complete output set for CI-only smoke validation."""
    return {
        "full": "false",
        "packages": "",
        "ci_only": "true",
        **{job: "false" for job in SPECIALIZED_JOB_NAMES},
        "shards": "[1]",
    }


def event_plan(event: str, pr_base: str = "", push_before: str = "") -> tuple[str, str]:
    """Return ``(mode, base)`` before Git history is fetched and verified."""
    if event == "workflow_dispatch":
        return "full", ""
    if event == "pull_request":
        return ("scoped", pr_base) if pr_base else ("full", "")
    if event == "push":
        if not push_before or push_before == "0" * 40:
            return "full", ""
        return "scoped", push_before
    return "full", ""


def changed_paths_from_git(base: str, head: str = "HEAD", cwd: str | None = None) -> list[str]:
    """Return every path changed across an event's base-to-head range.

    Disabling rename detection preserves both sides of a rename so moving an
    app file cannot accidentally skip checks owned by its former location.
    """
    result = subprocess.run(
        ["git", "diff", "--name-only", "--no-renames", base, head],
        check=True,
        capture_output=True,
        text=True,
        cwd=cwd,
    )
    return [line for line in result.stdout.splitlines() if line]


def base_is_ancestor(base: str, head: str = "HEAD", cwd: str | None = None) -> bool:
    """Whether a fetched event base actually belongs to the checked-out history."""
    return (
        subprocess.run(
            ["git", "merge-base", "--is-ancestor", base, head],
            check=False,
            capture_output=True,
            cwd=cwd,
        ).returncode
        == 0
    )


def is_workspace_wide_change(changed_paths: Iterable[str]) -> bool:
    """Whether a path can alter CI behavior for every workspace package."""
    return any(
        path == "rust-toolchain"
        or path == "rust-toolchain.toml"
        or path.startswith(".cargo/")
        or (path.startswith(".config/") and path != ".config/nextest.toml")
        for path in changed_paths
    )


def is_ci_only_change(changed_paths: Iterable[str]) -> bool:
    """Whether every changed file is CI implementation/configuration itself."""
    paths = tuple(changed_paths)
    return bool(paths) and all(
        path == ".config/nextest.toml"
        or path.startswith(".github/workflows/")
        or path.startswith(".github/actions/")
        or path.startswith(".github/scripts/")
        for path in paths
    )


def specialized_job_flags(
    changed_paths: Iterable[str], packages: Iterable[str]
) -> dict[str, bool]:
    """Return all specialized CI jobs required by paths and Cargo impact.

    ``packages`` is the transitive reverse-dependent set selected by
    affected_crates.py.  This means a shared crate such as rusty_serve enables
    every app that actually depends on it, without guessing a one-to-one owner.
    """
    paths = tuple(changed_paths)
    package_set = set(packages)
    flags = {
        job: any(path.startswith(prefix) for path in paths)
        for job, prefix in PATH_JOB_PREFIXES.items()
    }
    for job, matching_packages in PACKAGE_JOB_NAMES.items():
        flags[job] = flags.get(job, False) or bool(package_set & matching_packages)
    for job, prefixes in PACKAGE_JOB_PREFIXES.items():
        flags[job] = flags.get(job, False) or any(
            package.startswith(prefix) for package in package_set for prefix in prefixes
        )
    flags["remind_me"] = any(
        path.startswith("crates/apps/rusty_remind_me/") for path in paths
    ) or any(
        package == "rusty-remind-me" or package.startswith("remind_me_")
        for package in package_set
    )
    return flags


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--packages",
        default="",
        help="Space-separated affected workspace package names.",
    )
    parser.add_argument("--emit-full", action="store_true")
    parser.add_argument("--emit-ci-smoke", action="store_true")
    parser.add_argument("--event")
    parser.add_argument("--pr-base", default="")
    parser.add_argument("--push-before", default="")
    parser.add_argument("--verify-base")
    parser.add_argument(
        "--changed-from",
        help="Git commit from which to select changed paths through HEAD.",
    )
    parser.add_argument(
        "--requires-full",
        action="store_true",
        help="Exit successfully only if stdin contains a workspace-wide path.",
    )
    parser.add_argument("--is-ci-only", action="store_true")
    args = parser.parse_args()
    if args.emit_full:
        for key, value in full_plan_outputs().items():
            print(f"{key}={value}")
        return
    if args.emit_ci_smoke:
        for key, value in ci_smoke_plan_outputs().items():
            print(f"{key}={value}")
        return
    if args.event:
        mode, base = event_plan(args.event, args.pr_base, args.push_before)
        print(f"mode={mode}")
        print(f"base={base}")
        return
    if args.verify_base:
        raise SystemExit(0 if base_is_ancestor(args.verify_base) else 1)
    if args.changed_from:
        print("\n".join(changed_paths_from_git(args.changed_from)))
        return
    paths = [line.strip() for line in sys.stdin if line.strip()]
    if args.requires_full:
        raise SystemExit(0 if is_workspace_wide_change(paths) else 1)
    if args.is_ci_only:
        raise SystemExit(0 if is_ci_only_change(paths) else 1)
    flags = specialized_job_flags(paths, args.packages.split())
    for job, enabled in flags.items():
        print(f"{job}={'true' if enabled else 'false'}")


if __name__ == "__main__":
    main()
