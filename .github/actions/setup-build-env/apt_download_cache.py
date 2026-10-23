"""Cache downloaded archives only; authenticate restored bytes before root APT use."""

import argparse
from datetime import datetime, timezone
import hashlib
import os
from pathlib import Path
import platform
import re
import shlex
import shutil
import stat
import subprocess
import tempfile
import time
from urllib.parse import unquote


ArchiveIndex = dict[str, set[tuple[int, str]]]


def packages(path: Path) -> list[str]:
    """Read the shared native package list without shell evaluation."""
    names = path.read_text(encoding="utf-8").split()
    if not names or any(not re.fullmatch(r"[a-z0-9][a-z0-9+.-]*", name) for name in names):
        raise ValueError("Invalid native package list")
    return names


def cache_key(release: str, architecture: str, names: list[str], day: str) -> tuple[str, str]:
    """Partition archives by Ubuntu/architecture/list, with daily immutable generations."""
    if any(not re.fullmatch(r"[a-zA-Z0-9.]+", value) for value in (release, architecture, day)):
        raise ValueError("Invalid cache key component")
    digest = hashlib.sha256("\n".join(sorted(names)).encode()).hexdigest()
    prefix = f"apt-debs-v1-main-ubuntu-{release}-{architecture}-{digest}-"
    return prefix + day, prefix


def can_cache(env: dict[str, str]) -> bool:
    """Only hosted Actions consumers; never pull_request_target or a developer host."""
    return (env.get("GITHUB_ACTIONS") == "true" and env.get("RUNNER_ENVIRONMENT") == "github-hosted"
            and env.get("GITHUB_EVENT_NAME") in {"schedule", "workflow_dispatch", "pull_request", "merge_group"})


def trusted_index(names: set[str]) -> ArchiveIndex:
    """Get exact version/architecture SHA256 and size from APT's signed indexes.

    Use each record's own trusted IndexFile, not merely another origin carrying
    the same package version. The dpkg status file is not an authenticated source.
    """
    if not names:
        return {}
    import apt_pkg  # System python3-apt; no pip package or additional installation.
    apt_pkg.init()
    cache = apt_pkg.Cache()
    records = apt_pkg.PackageRecords(cache)
    sources = apt_pkg.SourceList()
    sources.read_main_list()
    wanted = {unquote(name).split("_", 1)[0] for name in names}
    result: ArchiveIndex = {}
    for package in cache.packages:
        if package.name not in wanted:
            continue
        for version in package.version_list:
            for location in version.file_list:
                index = sources.find_index(location[0])
                if index is None or not index.is_trusted or not records.lookup(location):
                    continue
                if not all(key in records for key in ("Package", "Version", "Architecture", "Size", "SHA256")):
                    continue
                name = f"{records['Package']}_{records['Version']}_{records['Architecture']}.deb"
                digest = records["SHA256"].lower()
                if re.fullmatch(r"[0-9a-f]{64}", digest):
                    result.setdefault(name, set()).add((int(records["Size"]), digest))
    return result


def copy_verified(source: Path, destination: Path, index: ArchiveIndex) -> int:
    """Copy regular .deb files through a private temporary file; reject stale/bad bytes.

    The destination during seeding is root-owned and never restored from cache.
    Hash the copied bytes before publishing them, avoiding source races/symlinks.
    """
    count = 0
    if not source.is_dir() or source.is_symlink():
        return count
    for archive in source.glob("*.deb"):
        expected = index.get(unquote(archive.name), set())
        if not expected or archive.is_symlink() or not archive.is_file():
            continue
        descriptor, name = tempfile.mkstemp(dir=destination)
        temporary = Path(name)
        try:
            with os.fdopen(descriptor, "wb") as copied, archive.open("rb") as original:
                details = os.fstat(original.fileno())
                sizes = {size for size, _ in expected}
                if not stat.S_ISREG(details.st_mode) or details.st_size not in sizes:
                    continue
                digest = hashlib.sha256()
                total = 0
                for block in iter(lambda: original.read(1024 * 1024), b""):
                    total += len(block)
                    if total > max(sizes):
                        break  # A restored file grew after fstat: never copy it unbounded.
                    copied.write(block)
                    digest.update(block)
            if total > max(sizes) or (temporary.stat().st_size, digest.hexdigest()) not in expected:
                continue
            temporary.chmod(0o644)
            os.replace(temporary, destination / archive.name)
            count += 1
        finally:
            temporary.unlink(missing_ok=True)
    return count


def apt_options(archives: Path) -> list[str]:
    """Keep downloads outside image docker-clean hooks and retain them after install."""
    return ["-o", f"Dir::Cache::archives={archives}", "-o", "APT::Keep-Downloaded-Packages=true",
            "-o", "Binary::apt-get::APT::Keep-Downloaded-Packages=true"]


def planned_downloads(output: str) -> set[str]:
    """Read print-uris filenames for retention checks, never as integrity evidence."""
    return {shlex.split(line)[1] for line in output.splitlines() if line.startswith("'")}


def retained(archives: Path, expected: set[str]) -> bool:
    """A successful install is cacheable only if its planned downloads survived."""
    return all((archives / name).is_file() and not (archives / name).is_symlink()
               for name in expected if Path(name).name == name) and all(Path(name).name == name for name in expected)


def emit(output: Path, values: dict[str, str]) -> None:
    """Append only controlled scalar values to GitHub step outputs."""
    with output.open("a", encoding="utf-8") as handle:
        for key, value in values.items():
            if "\n" in value or "\r" in value:
                raise ValueError("Multiline output rejected")
            handle.write(f"{key}={value}\n")


def install(names: list[str], staging: Path | None, output: Path) -> None:
    """Always update/install; only verified cache bytes enter the private APT directory."""
    started = time.monotonic()
    subprocess.run(["apt-get", "update"], check=True)
    if staging is None:
        subprocess.run(["apt-get", "install", "-y", *names], check=True)
        return
    archives = Path(tempfile.mkdtemp(prefix="rusty-mill-apt-", dir="/var/tmp"))
    archives.chmod(0o755)
    partial = archives / "partial"
    partial.mkdir(mode=0o700)
    shutil.chown(partial, user="_apt", group="root")
    try:
        index = trusted_index({path.name for path in staging.glob("*.deb")})
    except (ImportError, OSError, SystemError, ValueError) as error:
        print(f"::warning::APT cache verification unavailable ({type(error).__name__}); fetching normally.")
        index = {}
    try:
        reused = copy_verified(staging, archives, index)
    except OSError:
        print("::warning::Cannot read restored archives; fetching missing packages normally.")
        reused = 0
    command = ["apt-get", *apt_options(archives)]
    plan = subprocess.run([*command, "--print-uris", "install", "-y", *names],
                          check=True, capture_output=True, text=True)
    expected = planned_downloads(plan.stdout)
    # This is never gated on cache-hit. APT resolves current versions normally.
    subprocess.run([*command, "install", "-y", *names], check=True)
    kept = retained(archives, expected)
    if not kept:
        print("::warning::APT removed planned downloads; skip saving instead of publishing an empty cache.")
    print(f"APT download cache: verified-reused={reused}, planned-downloads={len(expected)}, "
          f"retention={kept}, install-seconds={time.monotonic() - started:.1f}")
    emit(output, {"archives": str(archives), "retained": str(kept).lower()})


def export(archives: Path, staging: Path, output: Path) -> None:
    """Unprivileged export: publish only currently authenticated archives, never locks/state."""
    if staging.is_symlink() or not staging.is_dir():
        print("::warning::Unexpected cache staging path; skip saving.")
        return
    try:
        index = trusted_index({path.name for path in archives.glob("*.deb")})
    except (ImportError, OSError, SystemError, ValueError) as error:
        print(f"::warning::Cannot verify archives for save ({type(error).__name__}); skip saving.")
        return
    for previous in staging.glob("*.deb"):
        if previous.is_dir() and not previous.is_symlink():
            print("::warning::Unexpected directory in cache staging; skip saving.")
            return
        previous.unlink()
    count = copy_verified(archives, staging, index)
    print(f"APT download cache: verified archives retained for save={count}")
    emit(output, {"cacheable": str(count > 0).lower()})


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", choices=("key", "install", "export"))
    parser.add_argument("--packages", type=Path, default=Path(__file__).with_name("native-packages.txt"))
    parser.add_argument("--output", type=Path, default=Path(os.environ.get("GITHUB_OUTPUT", "/dev/stdout")))
    parser.add_argument("--staging", default="")
    parser.add_argument("--archives", type=Path)
    args = parser.parse_args()
    if args.mode == "key":
        release = platform.freedesktop_os_release()
        if not can_cache(os.environ) or release.get("ID") != "ubuntu":
            emit(args.output, {"enabled": "false"})
            return
        staging = Path(os.environ["RUNNER_TEMP"]) / "rusty-mill-apt-debs"
        staging.mkdir(mode=0o700, exist_ok=True)
        if staging.is_symlink():
            raise ValueError("Symlinked cache staging rejected")
        architecture = subprocess.check_output(["dpkg", "--print-architecture"], text=True).strip()
        key, prefix = cache_key(release["VERSION_ID"], architecture, packages(args.packages),
                                datetime.now(timezone.utc).strftime("%Y%m%d"))
        emit(args.output, {"enabled": "true", "key": key, "prefix": prefix, "path": str(staging)})
    elif args.mode == "install":
        install(packages(args.packages), Path(args.staging) if args.staging else None, args.output)
    else:
        export(args.archives, Path(args.staging), args.output)


if __name__ == "__main__":
    main()
