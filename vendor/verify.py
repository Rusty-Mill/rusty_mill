#!/usr/bin/env python3
"""Reproduce pinned crypto source patches from authenticated crate archives.

Run from any directory: python3 vendor/verify.py. Requires network and git.
Downloads only the URLs/checksums in sources.json and writes only to a temporary
folder. Compare every vendored byte, including licenses and standalone locks.
"""
import hashlib
import io
import json
from pathlib import Path
import shutil
import subprocess
import tarfile
import tempfile
import urllib.request

ROOT = Path(__file__).resolve().parent


def files(root: Path) -> dict[str, bytes]:
    return {
        p.relative_to(root).as_posix(): p.read_bytes()
        for p in root.rglob("*")
        if p.is_file() and "target" not in p.relative_to(root).parts
    }


def main() -> None:
    sources = json.loads((ROOT / "sources.json").read_text())
    with tempfile.TemporaryDirectory(prefix="rusty-crypto-provenance-") as temporary:
        stage = Path(temporary)
        for name, source in sources.items():
            with urllib.request.urlopen(source["archive"], timeout=60) as response:
                data = response.read()
            if hashlib.sha256(data).hexdigest() != source["sha256"]:
                raise SystemExit(f"{name}: upstream archive checksum mismatch")
            with tarfile.open(fileobj=io.BytesIO(data), mode="r:gz") as archive:
                archive.extractall(stage, filter="data")
            shutil.move(stage / f"{name}-{source['version']}", stage / name)
            patch = ROOT / "patches" / f"{name}.patch"
            subprocess.run(["git", "apply", "--check", str(patch)], cwd=stage, check=True)
            subprocess.run(["git", "apply", str(patch)], cwd=stage, check=True)
            expected, actual = files(stage / name), files(ROOT / name)
            changed = sorted(k for k in expected.keys() | actual.keys() if expected.get(k) != actual.get(k))
            if changed:
                raise SystemExit(f"{name}: unreproducible files: {changed}")
            print(f"{name} {source['version']}: verified archive + patch, {len(actual)} files")


if __name__ == "__main__":
    main()
