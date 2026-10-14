"""Install commands for mods from other repos, quoted for the shell they are pasted into.

A mod's path comes from a repo we do not control, and following the hint launches its
hooks unsandboxed. So the command is built here, where it can be tested by running it,
every argument is quoted as data, it fetches exactly the commit the page indexed into a
fresh directory (never a leftover checkout), and it stops at the first failing step.
"""
import re
import shlex

# Words that need no quoting in either shell. A leading `-` is allowed only for the flags
# written in this file: the one remote-controlled argument is always `<dir>/<path>`.
BARE = re.compile(r"--|-{0,2}[A-Za-z0-9][A-Za-z0-9._/:=+-]*")
# Control characters, and the double quote Windows PowerShell 5.1 mangles in native arguments.
UNQUOTABLE = re.compile(r'[\x00-\x1f\x7f"]')


def posix(arg: str) -> str:
    return shlex.quote(arg)


def powershell(arg: str) -> str:
    """Single quotes: nothing inside expands; an embedded single quote is doubled."""
    return arg if BARE.fullmatch(arg) else "'" + arg.replace("'", "''") + "'"


def mod_commands(repo: str, sha: str, path: str, url: str | None = None) -> list[list[str]]:
    """Fetch exactly `sha` (not the default branch) into a new `<name>/` and launch the mod.

    `mkdir` fails if `<name>/` exists, so a stale or modified checkout can never be launched.
    """
    url = url or f"https://github.com/{repo}.git"
    name = repo.split("/")[1]
    return [
        ["mkdir", name],
        ["git", "-C", name, "init", "-q"],
        ["git", "-C", name, "fetch", "-q", "--depth", "1", "--", url, sha],
        ["git", "-C", name, "-c", "advice.detachedHead=false", "checkout", "-q", "--detach", "FETCH_HEAD"],
        ["claude", "--plugin-dir", f"{name}/{path}"],
    ]


def render_posix(commands: list[list[str]]) -> str:
    """One line for bash/zsh; `&&` stops at the first failure."""
    return " && ".join(" ".join(posix(a) for a in c) for c in commands)


def render_powershell(commands: list[list[str]]) -> str:
    """One pasteable statement: Windows PowerShell 5.1 has no `&&` and ignores native exit codes.

    Each native step is followed by an exit-code check that leaves the block; `mkdir` becomes
    a terminating `New-Item`, which also stops the block when the directory exists.
    """
    steps = []
    for i, c in enumerate(commands):
        if c[0] == "mkdir":
            steps.append(f"New-Item -ItemType Directory -ErrorAction Stop -Path {powershell(c[1])} | Out-Null")
            continue
        steps.append(" ".join(powershell(a) for a in c))
        if i < len(commands) - 1:
            steps.append("if ($LASTEXITCODE -ne 0) { return }")
    return "& { " + "; ".join(steps) + " }"


def render(commands: list[list[str]]) -> str:
    """Both shells, labelled. Raises ValueError for an argument that cannot be quoted safely."""
    for arg in (a for c in commands for a in c):
        if UNQUOTABLE.search(arg):
            raise ValueError(f"cannot quote {arg!r} safely for every shell")
    return f"# bash / zsh\n{render_posix(commands)}\n\n# PowerShell\n{render_powershell(commands)}"
