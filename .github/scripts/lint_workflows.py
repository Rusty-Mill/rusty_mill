"""Run pinned actionlint with strict support for GitHub's concurrency queue.

actionlint 1.7.12 rejects the documented queue key (upstream issue #657).
Validate our deliberately small subset, literal queue: max with explicit
cancel-in-progress: false, and accept ONLY that exact unsupported-key diagnostic
at a validated line. All other actionlint errors remain errors; original YAML
is passed to actionlint unchanged. Remove this adapter when upstream supports it.
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path
import re
import subprocess


UNSUPPORTED_QUEUE = (
    'unexpected key "queue" for "concurrency" section. '
    'expected one of "cancel-in-progress", "group"'
)


def validated_queue_lines(source: str) -> tuple[set[int], list[str]]:
    """Validate literal queue blocks at workflow/job level; fail closed otherwise."""
    lines = source.splitlines()
    validated: set[int] = set()
    errors: list[str] = []
    for index, line in enumerate(lines):
        match = re.fullmatch(r"( {0}| {4})concurrency:\s*(?:#.*)?", line)
        if not match:
            continue
        indent = len(match[1])
        block: list[tuple[int, str]] = []
        for offset in range(index + 1, len(lines)):
            child = lines[offset]
            if not child.strip() or child.lstrip().startswith("#"):
                continue
            if len(child) - len(child.lstrip()) <= indent:
                break
            block.append((offset + 1, child))
        queue = [(number, child) for number, child in block
                 if re.match(r"^ {" + str(indent + 2) + r"}queue:", child)]
        if not queue:
            continue
        cancel = [(number, child) for number, child in block
                  if re.match(r"^ {" + str(indent + 2) + r"}cancel-in-progress:", child)]
        def literal_scalar(fields: list[tuple[int, str]], key: str, value: str) -> bool:
            if len(fields) != 1:
                return False
            number, scalar = fields[0]
            # A '#' without preceding whitespace belongs to the plain scalar.
            if not re.fullmatch(
                r" {" + str(indent + 2) + "}" + key + ": " + value + r"(?:[ \t]+#.*)?[ \t]*",
                scalar,
            ):
                return False
            # YAML folds indented continuation lines into a plain scalar's
            # value, including across blank/comment lines. Validate its end,
            # not merely the first line that actionlint points at.
            following = next((child for offset, child in block if offset > number), None)
            return following is None or len(following) - len(following.lstrip()) <= indent + 2

        valid_queue = literal_scalar(queue, "queue", "max")
        valid_cancel = literal_scalar(cancel, "cancel-in-progress", "false")
        if not valid_queue or not valid_cancel:
            errors.append(f"line {index + 1}: queue requires literal max and explicit cancel-in-progress: false")
            continue
        validated.add(queue[0][0])
    return validated, errors


def remaining_diagnostics(source: str, diagnostics: list[dict]) -> list[str]:
    """Retain every diagnostic except a validated unsupported queue key."""
    queue_lines, errors = validated_queue_lines(source)
    for diagnostic in diagnostics:
        if (diagnostic.get("kind") == "syntax-check"
                and diagnostic.get("message") == UNSUPPORTED_QUEUE
                and diagnostic.get("line") in queue_lines):
            continue
        errors.append(f"line {diagnostic.get('line')}: {diagnostic.get('message')}")
    return errors


def lint(actionlint: str, workflow: Path) -> bool:
    """Check unchanged workflow YAML, including the strict queue extension."""
    result = subprocess.run(
        [actionlint, "-format", "{{json .}}", str(workflow)],
        capture_output=True, text=True, check=False,
    )
    if result.stderr:
        print(result.stderr, end="")
    # Tool crashes, malformed output and undocumented exit codes are errors,
    # never evidence that a workflow passed validation.
    if result.returncode not in (0, 1) or (result.returncode and not result.stdout.strip()):
        print(f"{workflow}: actionlint failed with exit {result.returncode}")
        return False
    diagnostics = json.loads(result.stdout) if result.stdout.strip() else []
    if result.returncode and not diagnostics:
        print(f"{workflow}: actionlint failed without a diagnostic")
        return False
    errors = remaining_diagnostics(workflow.read_text(encoding="utf-8"), diagnostics)
    for error in errors:
        print(f"{workflow}: {error}")
    return not errors


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--actionlint", required=True)
    parser.add_argument("workflows", nargs="+", type=Path)
    args = parser.parse_args()
    passed = [lint(args.actionlint, workflow) for workflow in args.workflows]
    raise SystemExit(0 if all(passed) else 1)


if __name__ == "__main__":
    main()
