#!/usr/bin/env python3
"""Per-PR change fragments: validate them and render the combined log.

Each PR adds one file ``changes/<YYYY-MM-DD>-<slug>.md`` (see changes/README.md),
so two PRs never edit the same lines and never conflict. Stdlib only.

usage: changes.py check [DIR]       validate every fragment; exit 1 on the first bad one
       changes.py notes [DIR]       print the release-notes view, newest first
       changes.py changelog [DIR]   print the changelog view, grouped by category
"""
from __future__ import annotations

import re
import sys
from dataclasses import dataclass
from datetime import date as calendar_date
from pathlib import Path

CATEGORIES = ("Added", "Changed", "Deprecated", "Removed", "Fixed", "Security")
NAME = re.compile(r"^(\d{4}-\d{2}-\d{2})-[a-z0-9][a-z0-9-]*\.md$")
KEYS = {"category", "changelog"}


@dataclass(frozen=True)
class Fragment:
    name: str
    date: str
    category: str
    changelog: str
    notes: str


def parse(name: str, text: str) -> Fragment:
    """Parse one fragment; raise ValueError naming the file on any format error."""
    match = NAME.match(name)
    if not match:
        raise ValueError(f"{name}: file name must be YYYY-MM-DD-slug.md (lowercase, digits, hyphens)")
    date = match[1]
    try:
        calendar_date.fromisoformat(date)
    except ValueError:
        raise ValueError(f"{name}: {date} is not a calendar date") from None
    if not text.startswith("---\n"):
        raise ValueError(f"{name}: must start with a '---' front-matter block")
    front, sep, body = text[4:].partition("\n---\n")
    if not sep:
        raise ValueError(f"{name}: front matter is not closed by a '---' line")
    meta: dict[str, str] = {}
    for line in front.splitlines():
        key, colon, value = line.partition(": ")
        if not colon or not value.strip():
            raise ValueError(f"{name}: front-matter line {line!r} is not 'key: value'")
        if key in meta:
            raise ValueError(f"{name}: front-matter key {key!r} appears more than once")
        meta[key] = value.strip()
    if set(meta) != KEYS:
        raise ValueError(f"{name}: front matter must have exactly {sorted(KEYS)}, got {sorted(meta)}")
    if meta["category"] not in CATEGORIES:
        raise ValueError(f"{name}: category must be one of {', '.join(CATEGORIES)}")
    notes = body.strip()
    if not notes.startswith(f"## {date} - "):
        raise ValueError(f"{name}: the notes must start with '## {date} - <title>'")
    return Fragment(name, date, meta["category"], meta["changelog"], notes)


def load(directory: Path) -> list[Fragment]:
    """All fragments in ``directory``, newest first (README.md is not a fragment).

    The directory must exist: a missing one is an error, not an empty log. An empty one is valid.
    """
    if not directory.is_dir():
        raise ValueError(f"{directory}: not a directory")
    paths = sorted((p for p in directory.glob("*.md") if p.name != "README.md"), reverse=True)
    return [parse(p.name, p.read_text(encoding="utf-8")) for p in paths]


def render_notes(fragments: list[Fragment]) -> str:
    return "\n\n".join(f.notes for f in fragments) + "\n"


def render_changelog(fragments: list[Fragment]) -> str:
    sections = []
    for category in CATEGORIES:
        lines = [f"- {f.changelog}" for f in fragments if f.category == category]
        if lines:
            sections.append(f"### {category}\n" + "\n".join(lines))
    return "\n\n".join(sections) + "\n"


def main(argv: list[str]) -> int:
    command = argv[1] if len(argv) > 1 else ""
    directory = Path(argv[2]) if len(argv) > 2 else Path("changes")
    views = {"notes": render_notes, "changelog": render_changelog}
    if command not in {"check", *views}:
        print(__doc__, file=sys.stderr)
        return 2
    try:
        fragments = load(directory)
    except ValueError as error:
        print(error, file=sys.stderr)
        return 1
    if command == "check":
        print(f"{len(fragments)} fragment(s) ok")
        return 0
    print(views[command](fragments), end="")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
