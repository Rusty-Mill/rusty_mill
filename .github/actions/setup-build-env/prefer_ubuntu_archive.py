"""Prefer the hosted runner's existing official Ubuntu mirror, if recognized."""

from pathlib import Path
import re


MIRROR_URI = "mirror+file:/etc/apt/apt-mirrors.txt"
AZURE = "http://azure.archive.ubuntu.com/ubuntu/"
ARCHIVE = "https://archive.ubuntu.com/ubuntu/"


def preferred_mirrors(source: str) -> str | None:
    """Swap only Azure 1/archive 2; reject ambiguous or changed target entries."""
    lines = source.splitlines(keepends=True)
    targets: dict[str, list[int]] = {AZURE: [], ARCHIVE: []}
    for index, line in enumerate(lines):
        fields = line.split()
        if fields and fields[0] in targets:
            targets[fields[0]].append(index)
    if any(len(indices) != 1 for indices in targets.values()):
        return None
    azure, archive = targets[AZURE][0], targets[ARCHIVE][0]
    matches = [
        re.fullmatch(re.escape(uri) + r"([ \t]+priority:)([12])([ \t]*(?:#.*)?)(\r?\n)?", lines[index])
        for uri, index in ((AZURE, azure), (ARCHIVE, archive))
    ]
    if any(match is None for match in matches):
        return None
    priorities = tuple(match[2] for match in matches)
    if priorities == ("2", "1"):
        return source  # Already preferred: keep repeated setup idempotent.
    if priorities != ("1", "2"):
        return None
    for index, match, priority in zip((azure, archive), matches, ("2", "1")):
        line = lines[index]
        lines[index] = line[:match.start(2)] + priority + line[match.end(2):]
    return "".join(lines)


def prefer_archive(mirrors: Path, sources: list[Path]) -> str:
    """Edit only a recognized mirror list already used by Ubuntu's sources."""
    if not mirrors.is_file() or mirrors.is_symlink():
        return "::warning::Ubuntu mirror list absent or unexpected; leaving APT configuration unchanged."
    # Ubuntu 24.04 uses deb822; 22.04 uses the one-line format. Never edit either.
    reference = re.compile(
        r"(?m)^(?:URIs:[ \t]+|deb(?:-src)?[ \t]+(?:\[[^\]\n]+\][ \t]+)?)"
        + re.escape(MIRROR_URI) + r"(?:[ \t]|$)"
    )
    if not any(path.is_file() and reference.search(path.read_text(encoding="utf-8")) for path in sources):
        return "::warning::Ubuntu sources do not use the expected mirror list; leaving APT configuration unchanged."
    original = mirrors.read_bytes().decode("utf-8")
    preferred = preferred_mirrors(original)
    if preferred is None:
        return "::warning::Ubuntu mirror priorities unrecognized; leaving APT configuration unchanged."
    if preferred == original:
        return "Official Ubuntu archive already has priority 1."
    mirrors.write_bytes(preferred.encode("utf-8"))
    return "Preferred existing HTTPS Ubuntu archive (priority 1); Azure retained at priority 2."


if __name__ == "__main__":
    print(prefer_archive(Path("/etc/apt/apt-mirrors.txt"), [
        Path("/etc/apt/sources.list.d/ubuntu.sources"), Path("/etc/apt/sources.list"),
    ]))
