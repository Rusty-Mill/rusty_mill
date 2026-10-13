#!/usr/bin/env python3
"""Build library/dist/index.html: one self-contained, offline library browser.

Entries come from this folder and, optionally, from repos listed in sources.json.
Plain `build.py` is offline and reads only what is already in .cache; `--sync` fetches first.
"""
import argparse
import json
import sys
from pathlib import Path

from fmt import parse, read_mod_source
from sources import Found, SourceError, head_sha, load_sources, scan, sync

ROOT = Path(__file__).parent
SOURCES_FILE = ROOT / "sources.json"
CACHE = ROOT / ".cache"
FLAT = {"prompts": "prompt", "claude-md": "claude-md", "agents-md": "agents-md"}
# Default target tool per kind; a prompt names its own via `for:`.
DEFAULT_FOR = {"skill": "Claude Code", "mod": "Claude Code", "claude-md": "Claude Code", "agents-md": "any agent"}

Item = dict[str, object]


def entry(
    kind: str,
    path: str,
    meta: dict[str, object],
    body: str,
    export: str | None = None,
    repo: str = "",
    group: str = "",
) -> Item:
    """One UI entry. `export` is what Copy/Download hand out; skills export their whole file.

    `repo` is set for entries pulled from another repo, and then `path` is relative to it.
    `group` is the "By source" heading; it defaults to `source`.
    """
    for key in ("name", "description", "source"):
        if not meta.get(key):
            raise ValueError(f"{path}: missing {key}")
    return {
        "kind": kind,
        "name": meta["name"],
        "description": meta["description"],
        "tags": meta.get("tags", []),
        "source": meta["source"],
        "group": group or meta["source"],
        "for": meta.get("for") or DEFAULT_FOR.get(kind, ""),
        "status": meta.get("status", ""),
        "featured": meta.get("featured") == "true",
        "path": path,
        "repo": repo,
        "body": body,
        "export": export if export is not None else body,
    }


def load_mod(d: Path) -> Item:
    manifest = json.loads((d / ".claude-plugin/plugin.json").read_text())
    meta = {**manifest, "tags": manifest.get("keywords", []), "source": manifest.get("repository")}
    return entry("mod", str(d.relative_to(ROOT)), meta, read_mod_source(d))


def collect_local() -> list[Item]:
    items = []
    for folder, kind in FLAT.items():
        for p in sorted((ROOT / folder).glob("*.md")):
            items.append(entry(kind, str(p.relative_to(ROOT)), *parse(p.read_text())))
    for p in sorted((ROOT / "skills").glob("*/SKILL.md")):
        text = p.read_text()
        items.append(entry("skill", str(p.relative_to(ROOT)), *parse(text), export=text))
    for d in sorted((ROOT / "mods").glob("*/")):
        items.append(load_mod(d))
    return items


def from_source(f: Found, repo: str, name: str, sha: str) -> Item:
    # Provenance is computed, never taken from the source's own frontmatter.
    origin = f"github.com/{repo}@{sha[:12]}"
    meta = {**f.meta, "source": f"{origin}:{f.rel}", "tags": [*f.meta.get("tags", []), name]}
    return entry(f.kind, f.rel, meta, f.body, f.export, repo=repo, group=origin)


def collect_sources(sources_file: Path, cache: Path) -> tuple[list[Item], list[str]]:
    """Index every synced source; an unsynced one is a warning, not a silent skip."""
    items: list[Item] = []
    warnings: list[str] = []
    for src in load_sources(sources_file):
        checkout = cache / src.name
        if not checkout.is_dir():
            warnings.append(f"{src.name}: not synced (run build.py --sync)")
            continue
        found, notes = scan(checkout)
        warnings += [f"{src.name}: {n}" for n in notes]
        sha = head_sha(checkout)
        items += [from_source(f, src.repo, src.name, sha) for f in found]
    return items, warnings


def duplicates(items: list[Item]) -> list[str]:
    seen: dict[tuple[object, object], str] = {}
    clashes = []
    for i in items:
        key = (i["kind"], i["name"])
        where = f'{i["repo"] or "local"}:{i["path"]}'
        if key in seen:
            clashes.append(f'{i["kind"]} {i["name"]!r}: {seen[key]} and {where}')
        seen.setdefault(key, where)
    return clashes


def render(items: list[Item]) -> str:
    # `</` is escaped so entry text can never close the inline <script>.
    data = json.dumps(items).replace("</", "<\\/")
    return (ROOT / "template.html").read_text().replace("__DATA__", data)


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--sync", action="store_true", help="fetch sources.json repos first (needs git and network)")
    ap.add_argument("--strict", action="store_true", help="fail on any warning (unsynced source, skipped file)")
    args = ap.parse_args(argv)
    try:
        if args.sync:
            for src in load_sources(SOURCES_FILE):
                print(f"synced {src.name}: {src.repo}@{sync(src, CACHE)[:12]}")
        remote, warnings = collect_sources(SOURCES_FILE, CACHE)
    except SourceError as e:
        print(f"error: {e}", file=sys.stderr)
        return 1
    items = [*collect_local(), *remote]
    for w in warnings:
        print(f"warning: {w}", file=sys.stderr)
    if clashes := duplicates(items):
        print("error: duplicate entries:\n  " + "\n  ".join(clashes), file=sys.stderr)
        return 1
    if args.strict and warnings:
        return 1
    out = ROOT / "dist/index.html"
    out.parent.mkdir(exist_ok=True)
    out.write_text(render(items))
    print(f"{len(items)} entries ({len(remote)} from sources) -> {out}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
