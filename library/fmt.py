"""Parse and read the file formats a library entry can take."""
import re
from pathlib import Path

MAX_FILE = 64 * 1024  # one file shown in the UI; the rest is cut with a marker
MAX_MOD = 256 * 1024  # all files of one mod


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


def read_text(path: Path, limit: int = MAX_FILE) -> str:
    """Read at most `limit` bytes as text, saying so when the file was cut."""
    with path.open("rb") as f:
        data = f.read(limit + 1)
    text = data[:limit].decode("utf-8", errors="replace")
    return text + "\n… [truncated]" if len(data) > limit else text


def read_mod_source(d: Path) -> str:
    """A mod's README, then every hook file except tests, each under a `// name` header."""
    files = [d / "README.md", *sorted((d / "hooks").glob("*"))]
    parts: list[str] = []
    used = 0
    for p in files:
        if ".test." in p.name or p.is_symlink() or not p.is_file():
            continue
        if used >= MAX_MOD:
            parts.append("// … remaining files omitted (size cap)")
            break
        text = read_text(p).strip()
        used += len(text)
        parts.append(f"// {p.relative_to(d).as_posix()}\n{text}")
    return "\n\n".join(parts)
