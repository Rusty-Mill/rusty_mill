"""Entries that live in other repos: sync a pinned checkout into a cache, then scan it.

Nothing from a source repo is ever executed: files are only read as text. Symlinks are
never followed, so a hostile repo cannot make the page embed files from outside it. Every
value taken from a source is type-checked here, at the boundary, before it can reach a page.
"""
import json
import os
import re
import shutil
import subprocess
import tempfile
from dataclasses import dataclass
from pathlib import Path

from fmt import parse, read_mod_source, read_text

NAME = re.compile(r"[a-z0-9][a-z0-9_-]{0,39}")
REPO = re.compile(r"[A-Za-z0-9][A-Za-z0-9_.-]*/[A-Za-z0-9][A-Za-z0-9_.-]*")
REF = re.compile(r"[A-Za-z0-9][A-Za-z0-9._/-]{0,99}")
FULL_SHA = re.compile(r"[0-9a-f]{40}")
PRUNE = {".git", "node_modules", "tests", "test", "types", "dist", "target", "__pycache__", ".cache"}
CONTROL = re.compile(r"[\x00-\x08\x0b-\x1f\x7f]")  # tab and newline are checked separately


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
    # Without its own .git, git would walk up and report the surrounding repository's commit.
    if not (checkout / ".git").exists():
        raise SourceError(f"{checkout} is not a git checkout")
    return _git(checkout, "rev-parse", "HEAD")


def record(source: Source, checkout: Path) -> None:
    """Bind a checkout to the repo and ref it was fetched for."""
    _git(checkout, "config", "--local", "library.repo", source.repo)
    _git(checkout, "config", "--local", "library.ref", source.ref)


def _is_own_checkout(path: Path) -> bool:
    """A real directory (not a symlink or junction) that is the root of its own git worktree."""
    git_dir = path / ".git"
    if not path.is_dir() or path.resolve() != path.parent.resolve() / path.name:
        return False
    if not git_dir.is_dir() or git_dir.is_symlink():
        return False
    try:
        return Path(_git(path, "rev-parse", "--show-toplevel")).resolve() == path.resolve()
    except SourceError:
        return False


def _changes(checkout: Path) -> str:
    """`git status` lines for anything that differs from the commit: modified, untracked, ignored."""
    return _git(checkout, "status", "--porcelain", "--untracked-files=all", "--ignored=matching")


def check_cache(source: Source, checkout: Path) -> str | None:
    """Why this cached checkout cannot stand for `source`, or None when it can.

    The scan reads the worktree, so the worktree must be exactly the commit it is attributed
    to: any modified, untracked or ignored file refuses the cache.
    """
    try:
        repo = _git(checkout, "config", "--local", "--get", "library.repo")
        ref = _git(checkout, "config", "--local", "--get", "library.ref")
        sha = head_sha(checkout)
    except SourceError:
        return "cache has no recorded origin (re-sync)"
    if not _is_own_checkout(checkout):
        return "cache is not a plain git checkout of its own (symlinked or foreign); move or delete it and re-sync"
    if (repo, ref) != (source.repo, source.ref):
        return f"cache holds {repo}@{ref} but sources.json says {source.repo}@{source.ref} (re-sync)"
    if FULL_SHA.fullmatch(source.ref) and sha != source.ref:
        return f"cache is at {sha[:12]} but sources.json pins {source.ref[:12]} (re-sync)"
    try:
        changes = _changes(checkout)
    except SourceError:
        return "cache status cannot be read (re-sync)"
    if changes:
        return f"cache has local changes, not the pinned commit: {changes.splitlines()[0]}; move or delete it and re-sync"
    return None


def _fetch(source: Source, dest: Path) -> None:
    _git(dest, "init", "-q")
    _git(dest, "fetch", "-q", "--depth", "1", "--", f"https://github.com/{source.repo}.git", source.ref)
    _git(dest, "-c", "advice.detachedHead=false", "checkout", "-q", "--force", "--detach", "FETCH_HEAD")


def _require_replaceable(dest: Path) -> None:
    """Refuse to replace anything but an untouched checkout this tool made itself."""
    if not os.path.lexists(dest):
        return
    try:
        recorded = bool(_git(dest, "config", "--local", "--get", "library.repo")) if _is_own_checkout(dest) else False
        dirty = _changes(dest) if recorded else ""
    except SourceError:
        recorded, dirty = False, ""
    if not recorded:
        raise SourceError(f"{dest} exists but was not made by --sync (symlink, plain or foreign directory); move or delete it and re-run")
    if dirty:
        raise SourceError(f"{dest} has local changes; move or delete it and re-run")


def sync(source: Source, cache: Path) -> str:
    """Fetch `source.ref` (shallow, https only) into cache/<name>; returns the commit SHA.

    The fetch happens in a fresh directory this call created, so nothing already on disk is
    ever initialised, checked out over or cleaned. An existing cache/<name> is replaced only
    if it is an untouched checkout made by a previous sync; anything else is left alone and
    reported.
    """
    cache.mkdir(parents=True, exist_ok=True)
    dest = cache / source.name
    stage = Path(tempfile.mkdtemp(prefix=f".{source.name}-", dir=cache))
    old = cache / f".old{stage.name}"
    try:
        _fetch(source, stage)
        record(source, stage)
        sha = head_sha(stage)
        _require_replaceable(dest)
        if os.path.lexists(dest):
            dest.rename(old)
        stage.rename(dest)
    except BaseException:
        if old.exists() and not os.path.lexists(dest):
            old.rename(dest)  # put the previous cache back
        shutil.rmtree(stage, ignore_errors=True)
        raise
    shutil.rmtree(old, ignore_errors=True)
    return sha


def _regular(path: Path, root: Path) -> bool:
    """A real file inside `root`: not a symlink, and not reached through one."""
    return path.is_file() and not path.is_symlink() and path.resolve().is_relative_to(root.resolve())


def _text(value: object, label: str, limit: int, multiline: bool = False) -> str:
    """A non-empty string without control characters, or ValueError."""
    if not isinstance(value, str) or not value.strip():
        raise ValueError(f"{label} must be a non-empty string")
    if len(value) > limit:
        raise ValueError(f"{label} is longer than {limit} characters")
    if CONTROL.search(value) or (not multiline and ("\n" in value or "\t" in value)):
        raise ValueError(f"{label} has control characters")
    return value.strip()


def _tags(value: object) -> list[str]:
    if value is None:
        return []
    if isinstance(value, str):
        value = [value]
    if not isinstance(value, list) or len(value) > 20:
        raise ValueError("tags must be a list of at most 20 strings")
    return [_text(t, "tag", 40) for t in value]


def _trusted(raw: dict[str, object]) -> dict[str, object]:
    """Only these keys are taken from a source, each type-checked.

    `featured` and `status` are never taken: they would let a source promote itself or
    grey out other entries.
    """
    meta: dict[str, object] = {
        "name": _text(raw.get("name"), "name", 80),
        "description": _text(raw.get("description"), "description", 1000, multiline=True),
        "tags": _tags(raw.get("tags")),
    }
    if "for" in raw:
        meta["for"] = _text(raw["for"], "for", 40)
    return meta


def _doc(root: Path, path: Path, kind: str, found: list[Found], warnings: list[str]) -> None:
    rel = path.relative_to(root).as_posix()
    try:
        _text(rel, "path", 300)
        text = read_text(path)
        meta, body = parse(text)
        meta = _trusted(meta)
    except (OSError, ValueError) as e:
        warnings.append(f"{rel}: skipped ({e})")
        return
    found.append(Found(kind, rel, meta, body, text if kind == "skill" else body))


def _mod(root: Path, d: Path, found: list[Found], warnings: list[str]) -> None:
    rel = d.relative_to(root).as_posix() or "."
    try:
        _text(rel, "path", 300)
        manifest = json.loads(read_text(d / ".claude-plugin" / "plugin.json"))
        if not isinstance(manifest, dict):
            raise ValueError("plugin.json must be an object")
        meta = _trusted({**manifest, "tags": manifest.get("keywords")})
    except (OSError, ValueError) as e:
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
