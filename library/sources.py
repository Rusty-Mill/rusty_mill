"""Entries that live in other repos: sync a pinned checkout into a cache, then scan it.

Nothing from a source repo is ever executed: files are only read as text. Symlinks are
never followed, so a hostile repo cannot make the page embed files from outside it.
"""
import json
import os
import re
import subprocess
from dataclasses import dataclass
from pathlib import Path

from fmt import parse, read_mod_source, read_text

NAME = re.compile(r"[a-z0-9][a-z0-9_-]{0,39}")
REPO = re.compile(r"[A-Za-z0-9][A-Za-z0-9_.-]*/[A-Za-z0-9][A-Za-z0-9_.-]*")
REF = re.compile(r"[A-Za-z0-9][A-Za-z0-9._/-]{0,99}")
PRUNE = {".git", "node_modules", "tests", "test", "types", "dist", "target", "__pycache__", ".cache"}
# Only these frontmatter keys are taken from a source repo: `featured` and `status`
# would let a source promote itself or grey out other entries.
TRUSTED_KEYS = ("name", "description", "tags", "for")


class SourceError(Exception):
    """A sources.json problem, or a git failure while syncing or reading a source."""


@dataclass(frozen=True)
class Source:
    name: str
    repo: str  # owner/name on github.com
    ref: str  # branch, tag or commit SHA; a full SHA pins the build


@dataclass(frozen=True)
class Found:
    kind: str  # "skill" | "mod" | "prompt"
    rel: str  # path inside the source repo
    meta: dict[str, object]
    body: str
    export: str


def load_sources(path: Path) -> list[Source]:
    """Read and validate sources.json; a missing file means no sources."""
    if not path.exists():
        return []
    sources: list[Source] = []
    for raw in json.loads(path.read_text()):
        src = Source(raw.get("name", ""), raw.get("repo", ""), raw.get("ref", ""))
        for label, value, rx in (("name", src.name, NAME), ("repo", src.repo, REPO), ("ref", src.ref, REF)):
            if not rx.fullmatch(value) or ".." in value:
                raise SourceError(f"{path}: bad {label} {value!r}")
        sources.append(src)
    names = [s.name for s in sources]
    if len(names) != len(set(names)):
        raise SourceError(f"{path}: duplicate source name")
    return sources


def _git(cwd: Path, *args: str) -> str:
    env = {**os.environ, "GIT_TERMINAL_PROMPT": "0", "GIT_LFS_SKIP_SMUDGE": "1"}
    cmd = ["git", "-c", "protocol.allow=never", "-c", "protocol.https.allow=always", *args]
    try:
        done = subprocess.run(cmd, cwd=cwd, env=env, capture_output=True, text=True, timeout=300)
    except (OSError, subprocess.TimeoutExpired) as e:
        raise SourceError(f"git {args[0]} failed: {e}") from e
    if done.returncode:
        raise SourceError(f"git {args[0]} failed: {done.stderr.strip()}")
    return done.stdout.strip()


def head_sha(checkout: Path) -> str:
    return _git(checkout, "rev-parse", "HEAD")


def sync(source: Source, cache: Path) -> str:
    """Fetch `source.ref` (shallow, https only) into cache/<name>; returns the commit SHA."""
    dest = cache / source.name
    dest.mkdir(parents=True, exist_ok=True)
    _git(dest, "init", "-q")
    _git(dest, "fetch", "-q", "--depth", "1", "--", f"https://github.com/{source.repo}.git", source.ref)
    _git(dest, "-c", "advice.detachedHead=false", "checkout", "-q", "--force", "--detach", "FETCH_HEAD")
    return head_sha(dest)


def _regular(path: Path, root: Path) -> bool:
    """A real file inside `root`: not a symlink, and not reached through one."""
    return path.is_file() and not path.is_symlink() and path.resolve().is_relative_to(root.resolve())


def _trusted(meta: dict[str, object]) -> dict[str, object]:
    out = {k: meta[k] for k in TRUSTED_KEYS if k in meta}
    if isinstance(out.get("tags"), str):
        out["tags"] = [out["tags"]]
    return out


def _doc(root: Path, path: Path, kind: str, found: list[Found], warnings: list[str]) -> None:
    rel = path.relative_to(root).as_posix()
    try:
        text = read_text(path)
        meta, body = parse(text)
    except (OSError, ValueError) as e:
        warnings.append(f"{rel}: skipped ({e})")
        return
    meta = _trusted(meta)
    if not (meta.get("name") and meta.get("description")):
        warnings.append(f"{rel}: skipped (needs name and description)")
        return
    found.append(Found(kind, rel, meta, body, text if kind == "skill" else body))


def _mod(root: Path, d: Path, found: list[Found], warnings: list[str]) -> None:
    rel = d.relative_to(root).as_posix() or "."
    try:
        manifest = json.loads(read_text(d / ".claude-plugin" / "plugin.json"))
        meta = _trusted({"name": manifest["name"], "description": manifest["description"], "tags": manifest.get("keywords", [])})
    except (OSError, ValueError, KeyError, TypeError) as e:
        warnings.append(f"{rel}: skipped (bad plugin.json: {e})")
        return
    src = read_mod_source(d)
    found.append(Found("mod", rel, meta, src, src))


def scan(root: Path) -> tuple[list[Found], list[str]]:
    """Find skills (SKILL.md), mods (plugin folders) and root `prompts/*.md` under `root`."""
    found: list[Found] = []
    warnings: list[str] = []
    for dirpath, dirnames, filenames in os.walk(root):
        d = Path(dirpath)
        dirnames[:] = sorted(n for n in dirnames if n not in PRUNE and not (d / n).is_symlink())
        if _regular(d / ".claude-plugin" / "plugin.json", root):
            _mod(root, d, found, warnings)
            dirnames[:] = []  # a plugin's own folders are part of it, not separate entries
        elif "SKILL.md" in filenames and _regular(d / "SKILL.md", root):
            _doc(root, d / "SKILL.md", "skill", found, warnings)
            dirnames[:] = []
        elif d == root / "prompts":
            for name in sorted(filenames):
                if name.endswith(".md") and _regular(d / name, root):
                    _doc(root, d / name, "prompt", found, warnings)
    return found, warnings
