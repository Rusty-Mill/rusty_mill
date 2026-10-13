#!/usr/bin/env python3
"""Build library/dist/index.html: one self-contained, offline library browser."""
import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).parent
KINDS = {"prompts": "prompt", "skills": "skill", "mods": "mod"}


def parse(text: str) -> tuple[dict[str, object], str]:
    """Split `---` frontmatter (flat `key: value`, `[a, b]` lists) from the body."""
    m = re.match(r"---\n(.*?)\n---\n(.*)", text, re.S)
    if not m:
        raise ValueError("missing frontmatter")
    meta: dict[str, object] = {}
    for line in m[1].splitlines():
        key, _, val = line.partition(":")
        val = val.strip()
        meta[key.strip()] = (
            [t.strip() for t in val[1:-1].split(",") if t.strip()]
            if val.startswith("[")
            else val
        )
    return meta, m[2].strip()


def entry(kind: str, path: Path, meta: dict[str, object], body: str) -> dict[str, object]:
    for key in ("name", "description"):
        if not meta.get(key):
            raise ValueError(f"{path}: missing {key}")
    return {
        "kind": kind,
        "name": meta["name"],
        "description": meta["description"],
        "tags": meta.get("tags", []),
        "path": str(path.relative_to(ROOT)),
        "body": body,
    }


def load_mod(d: Path) -> dict[str, object]:
    manifest = json.loads((d / ".claude-plugin/plugin.json").read_text())
    src = "\n\n".join(
        f"// {p.relative_to(d)}\n{p.read_text().strip()}"
        for p in sorted((d / "hooks").glob("*"))
    )
    meta = {**manifest, "tags": manifest.get("keywords", [])}
    return entry("mod", d, meta, src)


def collect() -> list[dict[str, object]]:
    items = []
    for p in sorted((ROOT / "prompts").glob("*.md")):
        items.append(entry("prompt", p, *parse(p.read_text())))
    for p in sorted((ROOT / "skills").glob("*/SKILL.md")):
        items.append(entry("skill", p, *parse(p.read_text())))
    for d in sorted((ROOT / "mods").glob("*/")):
        items.append(load_mod(d))
    return items


def main() -> int:
    items = collect()
    names = [(i["kind"], i["name"]) for i in items]
    if len(names) != len(set(names)):
        sys.exit("duplicate kind/name")
    # `</` is escaped so entry text can never close the inline <script>.
    data = json.dumps(items).replace("</", "<\\/")
    out = ROOT / "dist/index.html"
    out.parent.mkdir(exist_ok=True)
    out.write_text((ROOT / "template.html").read_text().replace("__DATA__", data))
    print(f"{len(items)} entries -> {out}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
