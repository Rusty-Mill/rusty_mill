import json
import os
import shlex
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

import hints

# Remote-controlled paths: spaces, quotes, substitutions, metacharacters.
HOSTILE = ["a b", "$(id)", "it's", ";rm -rf ~", "`id`", "x&&y", "*", "$HOME", "a\\b", "a|b", "~", "#c", "a'$(b)'c"]


class QuotingTests(unittest.TestCase):
    def test_posix_round_trips_every_hostile_path_as_one_argument(self) -> None:
        for path in HOSTILE:
            with self.subTest(path=path):
                argvs = hints.mod_commands("o/r", "a" * 40, path)
                for argv in argvs:
                    self.assertEqual(shlex.split(" ".join(hints.posix(a) for a in argv)), argv)

    def test_powershell_quotes_as_data(self) -> None:
        self.assertEqual(hints.powershell("it's"), "'it''s'")
        self.assertEqual(hints.powershell("$(id)"), "'$(id)'")
        self.assertEqual(hints.powershell("a b"), "'a b'")
        self.assertEqual(hints.powershell("@splat"), "'@splat'")
        for bare in ("git", "--", "--depth", "-C", "https://github.com/o/r.git", "my_claude_mods/merge-guard"):
            self.assertEqual(hints.powershell(bare), bare)

    def test_powershell_stops_after_every_failing_native_step(self) -> None:
        text = hints.render_powershell(hints.mod_commands("o/r", "a" * 40, "mods/x"))
        guard = "if ($LASTEXITCODE -ne 0) { return }"
        steps = text.removeprefix("& { ").removesuffix(" }").split("; ")
        self.assertEqual(steps[0], "New-Item -ItemType Directory -ErrorAction Stop -Path r | Out-Null")
        for i, step in enumerate(steps[:-1]):
            if step.startswith("git "):
                self.assertEqual(steps[i + 1], guard, f"no exit check after: {step}")
        self.assertEqual(steps.count(guard), 3)
        self.assertEqual(steps[-1], "claude --plugin-dir r/mods/x")

    def test_unquotable_arguments_are_refused(self) -> None:
        for bad in ('say "hi"', "a\nb", "a\x1b[31mb", "a\x00b"):
            with self.subTest(bad=bad), self.assertRaises(ValueError):
                hints.render(hints.mod_commands("o/r", "a" * 40, bad))

    def test_render_has_both_shells_and_the_full_sha(self) -> None:
        sha = "0123456789abcdef0123456789abcdef01234567"
        text = hints.render(hints.mod_commands("o/r", sha, "mods/x y"))
        self.assertIn("# bash / zsh", text)
        self.assertIn("# PowerShell", text)
        self.assertEqual(text.count(sha), 2)
        self.assertIn("'r/mods/x y'", text)


@unittest.skipUnless(shutil.which("sh"), "sh not available")
class ExecutionTests(unittest.TestCase):
    """Run the real generated bash/zsh line with stub git and claude binaries."""

    def run_script(self, path: str, fail_on: str = "", existing: bool = False) -> tuple[list[list[str]], bool, int]:
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            work = tmp / "work"
            work.mkdir()
            if existing:
                (work / "r").mkdir()
            bin_ = tmp / "bin"
            bin_.mkdir()
            log = tmp / "calls.jsonl"
            stub = (
                f"#!{sys.executable}\n"
                "import json, os, sys\n"
                f"open({str(log)!r}, 'a').write(json.dumps([sys.argv[0].rsplit('/', 1)[-1]] + sys.argv[1:]) + '\\n')\n"
                "sys.exit(1 if os.environ.get('FAIL_ON') in sys.argv else 0)\n"
            )
            for name in ("git", "claude"):
                (bin_ / name).write_text(stub)
                (bin_ / name).chmod(0o755)
            sentinel = tmp / "SENTINEL"
            script = hints.render_posix(hints.mod_commands("o/r", "a" * 40, path.replace("SENTINEL", str(sentinel))))
            env = {**os.environ, "PATH": f"{bin_}:{os.environ['PATH']}", "FAIL_ON": fail_on or "never"}
            done = subprocess.run(["sh", "-c", script], cwd=work, env=env, capture_output=True)
            calls = [json.loads(line) for line in log.read_text().splitlines()] if log.exists() else []
            return calls, sentinel.exists(), done.returncode

    def test_hostile_paths_reach_the_launcher_as_inert_data(self) -> None:
        for path in [*HOSTILE, "$(touch SENTINEL)", "`touch SENTINEL`", "x; touch SENTINEL", "x' ; touch SENTINEL ; '"]:
            with self.subTest(path=path):
                calls, created, code = self.run_script(path)
                self.assertEqual(code, 0)
                self.assertFalse(created, "command substitution or a metacharacter executed")
                expected = path.replace("SENTINEL", "")  # sentinel path is spliced in by run_script
                launcher = calls[-1]
                self.assertEqual(launcher[:2], ["claude", "--plugin-dir"])
                self.assertTrue(launcher[2].startswith("r/"))
                if "SENTINEL" not in path:
                    self.assertEqual(launcher[2], f"r/{expected}")
                self.assertEqual(len(launcher), 3, "path was split into several arguments")

    def test_a_failing_step_never_reaches_the_launcher(self) -> None:
        for step in ("init", "fetch", "checkout"):
            with self.subTest(step=step):
                calls, _, code = self.run_script("mods/x", fail_on=step)
                self.assertNotEqual(code, 0)
                self.assertIn(step, calls[-1])
                self.assertNotIn("claude", [c[0] for c in calls])

    def test_an_existing_directory_is_never_reused(self) -> None:
        calls, _, code = self.run_script("mods/x", existing=True)
        self.assertNotEqual(code, 0)
        self.assertEqual(calls, [], "nothing may run against a pre-existing checkout")


@unittest.skipUnless(shutil.which("git"), "git not available")
class PinningTests(unittest.TestCase):
    def git(self, cwd: Path, *args: str, extra: dict[str, str] | None = None) -> str:
        env = {
            **os.environ,
            "GIT_AUTHOR_NAME": "t", "GIT_AUTHOR_EMAIL": "t@t", "GIT_COMMITTER_NAME": "t", "GIT_COMMITTER_EMAIL": "t@t",
            "GIT_CONFIG_COUNT": "1", "GIT_CONFIG_KEY_0": "protocol.file.allow", "GIT_CONFIG_VALUE_0": "always",
            **(extra or {}),
        }
        return subprocess.run(["git", *args], cwd=cwd, env=env, check=True, capture_output=True, text=True).stdout.strip()

    def test_the_command_checks_out_the_indexed_commit_even_after_the_branch_moves(self) -> None:
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            remote = tmp / "remote"
            remote.mkdir()
            self.git(remote, "init", "-q")
            (remote / "mod.txt").write_text("reviewed")
            self.git(remote, "add", ".")
            self.git(remote, "commit", "-q", "-m", "A")
            reviewed = self.git(remote, "rev-parse", "HEAD")
            (remote / "mod.txt").write_text("changed after review")
            self.git(remote, "commit", "-q", "-am", "B")
            moved = self.git(remote, "rev-parse", "HEAD")
            self.assertNotEqual(reviewed, moved)

            work = tmp / "work"
            work.mkdir()
            *fetch_steps, _launch = hints.mod_commands("o/r", reviewed, "mods/x", url=remote.as_uri())
            for argv in fetch_steps:
                if argv[0] == "mkdir":
                    (work / argv[1]).mkdir()
                else:
                    self.git(work, *argv[1:])
            self.assertEqual(self.git(work / "r", "rev-parse", "HEAD"), reviewed)
            self.assertEqual((work / "r" / "mod.txt").read_text(), "reviewed")

            # What the old hint did: a plain clone lands on the moved branch tip.
            self.git(work, "clone", "-q", remote.as_uri(), "plain")
            self.assertEqual(self.git(work / "plain", "rev-parse", "HEAD"), moved)


if __name__ == "__main__":
    unittest.main()
