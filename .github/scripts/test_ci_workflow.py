"""Regression checks for CI event planning and full-sweep scheduling.

These checks intentionally inspect the checked-in workflow text rather than
requiring a GitHub Actions runner. The expressions are GitHub-specific, but
the behavior they encode is simple and important: PR updates cancel obsolete
work, while main keeps an active full sweep and lets Actions replace only
older pending work in that shared group.
"""

from pathlib import Path
import subprocess
import sys
import tempfile
import tomllib
import unittest

from ci_plan import changed_paths_from_git, is_workspace_wide_change, specialized_job_flags


WORKFLOW = Path(__file__).parents[1] / "workflows" / "ci.yml"
BASELINE_WORKFLOW = Path(__file__).parents[1] / "workflows" / "baseline.yml"
NEXTEST_CONFIG = Path(__file__).parents[2] / ".config" / "nextest.toml"
PLANNER = Path(__file__).with_name("ci_plan.py")

PLAN_KEYS = {
    "full",
    "packages",
    "dashboard",
    "term_web",
    "key_desktop",
    "tick",
    "fair_play",
    "remind_me",
    "win32",
    "multimodal_db",
    "rusty_config_no_std",
    "shards",
}
SPECIALIZED_KEYS = PLAN_KEYS - {"full", "packages", "shards"}


class CiWorkflowSchedulingTests(unittest.TestCase):
    def setUp(self) -> None:
        self.workflow = WORKFLOW.read_text(encoding="utf-8")

    def test_push_pr_and_manual_full_sweep_events_are_declared(self) -> None:
        self.assertIn("  push:\n    branches: [main]", self.workflow)
        self.assertIn("  pull_request:", self.workflow)
        self.assertIn("  workflow_dispatch:", self.workflow)
        self.assertIn('--event "${{ github.event_name }}"', self.workflow)
        self.assertIn('--push-before "${{ github.event.before }}"', self.workflow)
        self.assertIn('changed="$(python3 .github/scripts/ci_plan.py --changed-from "$base")"', self.workflow)
        self.assertIn('Event has no usable scoped base', self.workflow)
        self.assertIn('python3 .github/scripts/ci_plan.py --emit-full', self.workflow)

    def test_app_jobs_are_planned_not_unconditional(self) -> None:
        self.assertIn("fair_play: ${{ steps.plan.outputs.fair_play }}", self.workflow)
        self.assertIn("tick: ${{ steps.plan.outputs.tick }}", self.workflow)
        self.assertIn("if: needs.plan.outputs.fair_play == 'true'", self.workflow)
        self.assertIn("if: needs.plan.outputs.tick == 'true'", self.workflow)
        self.assertIn("ci_plan.py --packages", self.workflow)
        self.assertIn("win32: ${{ steps.plan.outputs.win32 }}", self.workflow)
        self.assertIn("multimodal_db: ${{ steps.plan.outputs.multimodal_db }}", self.workflow)
        self.assertIn("fetch-depth: 0", self.workflow)

    def test_full_event_entrypoint_emits_every_plan_output(self) -> None:
        outputs = self._planner_map("--emit-full")
        self.assertEqual(set(outputs), PLAN_KEYS)
        self.assertEqual(outputs["full"], "true")
        self.assertEqual(outputs["packages"], "")
        self.assertEqual(outputs["shards"], "[1,2,3]")
        self.assertTrue(all(outputs[key] == "true" for key in SPECIALIZED_KEYS))

    def test_event_entrypoint_handles_manual_and_unusable_bases(self) -> None:
        for args in (
            ("--event", "workflow_dispatch"),
            ("--event", "push", "--push-before", "0" * 40),
            ("--event", "push"),
            ("--event", "pull_request"),
        ):
            with self.subTest(args=args):
                self.assertEqual(self._planner_map(*args)["mode"], "full")
        scoped = self._planner_map(
            "--event", "push", "--push-before", "1" * 40
        )
        self.assertEqual(scoped, {"mode": "scoped", "base": "1" * 40})

    def test_global_fallback_entrypoint_covers_lock_and_toolchain_inputs(self) -> None:
        for path in (
            ".github/workflows/ci.yml",
            "rust-toolchain.toml",
            ".cargo/config.toml",
        ):
            with self.subTest(path=path):
                self.assertEqual(self._planner("--requires-full", input=f"{path}\n").returncode, 0)
        self.assertEqual(
            self._planner("--requires-full", input="crates/apps/rusty_tick/src/lib.rs\n").returncode,
            1,
        )

    def test_lockfile_is_not_an_unconditional_global_fallback(self) -> None:
        self.assertEqual(self._planner("--requires-full", input="Cargo.lock\n").returncode, 1)
        self.assertIn("lockfile_diff.py", self.workflow)
        self.assertIn("Cargo.lock changed with a manifest", self.workflow)
        self.assertIn("Unable to establish Cargo.lock impact", self.workflow)

    def test_unrelated_base_falls_back_after_fetch_verification(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            repo = Path(directory)
            self._git(repo, "init", "-q")
            self._git(repo, "config", "user.email", "ci@example.invalid")
            self._git(repo, "config", "user.name", "CI test")
            self._write(repo, "main.txt")
            self._git(repo, "add", ".")
            self._git(repo, "commit", "-qm", "main")
            primary_branch = self._git(repo, "branch", "--show-current").strip()
            self._git(repo, "checkout", "--orphan", "unrelated", "-q")
            self._git(repo, "rm", "-rf", ".")
            self._write(repo, "other.txt")
            self._git(repo, "add", ".")
            self._git(repo, "commit", "-qm", "unrelated")
            unrelated = self._git(repo, "rev-parse", "HEAD").strip()
            self._git(repo, "checkout", primary_branch, "-q")
            result = subprocess.run(
                [sys.executable, str(PLANNER), "--verify-base", unrelated],
                cwd=repo,
                capture_output=True,
                text=True,
                check=False,
            )
        self.assertEqual(result.returncode, 1)

    def test_specialized_entrypoint_emits_complete_key_set(self) -> None:
        outputs = self._planner_map(
            "--packages", "rk-app rusty_term rusty-meshed-core rusty_tick"
        )
        self.assertEqual(set(outputs), SPECIALIZED_KEYS)
        self.assertEqual(outputs["key_desktop"], "true")
        self.assertEqual(outputs["term_web"], "true")
        self.assertEqual(outputs["dashboard"], "true")
        self.assertEqual(outputs["tick"], "true")
        self.assertEqual(outputs["fair_play"], "false")

    def test_shallow_clone_becomes_narrowable_after_history_is_complete(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "source"
            origin = root / "origin.git"
            clone = root / "clone"
            source.mkdir()
            self._git(source, "init", "-q")
            self._git(source, "config", "user.email", "ci@example.invalid")
            self._git(source, "config", "user.name", "CI test")
            self._write(source, "crates/apps/rusty_tick/src/lib.rs")
            self._git(source, "add", ".")
            self._git(source, "commit", "-qm", "base")
            base = self._git(source, "rev-parse", "HEAD").strip()
            self._write(source, "crates/apps/rusty_tick/src/next.rs")
            self._git(source, "add", ".")
            self._git(source, "commit", "-qm", "next")
            self._git(source, "clone", "--bare", ".", str(origin))
            subprocess.run(
                ["git", "clone", "--depth", "1", origin.as_uri(), str(clone)],
                check=True,
                capture_output=True,
                text=True,
            )
            self.assertFalse(self._planner_in(clone, "--verify-base", base).returncode == 0)
            self._git(clone, "fetch", "--unshallow", "origin")
            self.assertEqual(self._planner_in(clone, "--verify-base", base).returncode, 0)
            flags = specialized_job_flags(changed_paths_from_git(base, cwd=clone), [])
        self.assertTrue(flags["tick"])

    def test_fair_play_only_change_does_not_select_an_unrelated_app(self) -> None:
        self.assertEqual(
            specialized_job_flags(["crates/apps/rusty_fair_play/web/src/App.tsx"], []),
            {"dashboard": False, "term_web": False, "key_desktop": False, "tick": False, "fair_play": True, "win32": False, "multimodal_db": False, "rusty_config_no_std": False, "remind_me": False},
        )

    def test_fair_play_reverse_dependencies_select_its_specialized_jobs(self) -> None:
        # affected_crates.py expands a changed domain/shared crate to these
        # application packages before this mapper sees it.
        flags = specialized_job_flags([], ["rusty_fair_play", "rusty_tick"])
        self.assertTrue(flags["fair_play"])
        self.assertTrue(flags["tick"])

    def test_deleted_or_renamed_app_paths_remain_conservative(self) -> None:
        # `git diff --name-only` reports a deletion, and may report both sides
        # of a rename; either Fair Play path must retain its app checks.
        flags = specialized_job_flags(
            [
                "crates/apps/rusty_fair_play/web/src/removed.ts",
                "crates/apps/rusty_tick/web/src/new.ts",
            ],
            [],
        )
        self.assertTrue(flags["fair_play"])
        self.assertTrue(flags["tick"])

    def test_multi_commit_push_and_deletion_use_real_git_history(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            repo = Path(directory)
            self._git(repo, "init", "-q")
            self._git(repo, "config", "user.email", "ci@example.invalid")
            self._git(repo, "config", "user.name", "CI test")
            self._write(repo, "README.md")
            self._write(repo, "crates/apps/rusty_fair_play/web/src/removed.ts")
            self._git(repo, "add", ".")
            self._git(repo, "commit", "-qm", "base")
            base = self._git(repo, "rev-parse", "HEAD").strip()
            self._write(repo, "crates/apps/rusty_tick/web/src/new.ts")
            self._git(repo, "add", ".")
            self._git(repo, "commit", "-qm", "add tick path")
            self._git(repo, "rm", "-q", "crates/apps/rusty_fair_play/web/src/removed.ts")
            self._git(repo, "commit", "-qm", "delete fair play path")
            paths = changed_paths_from_git(base, cwd=repo)
        flags = specialized_job_flags(paths, [])
        self.assertTrue(flags["fair_play"])
        self.assertTrue(flags["tick"])

    def test_rename_preserves_both_app_sides_in_real_git_history(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            repo = Path(directory)
            self._git(repo, "init", "-q")
            self._git(repo, "config", "user.email", "ci@example.invalid")
            self._git(repo, "config", "user.name", "CI test")
            self._write(repo, "crates/apps/rusty_fair_play/web/src/moved.ts")
            self._git(repo, "add", ".")
            self._git(repo, "commit", "-qm", "base")
            base = self._git(repo, "rev-parse", "HEAD").strip()
            target = "crates/apps/rusty_tick/web/src/moved.ts"
            (repo / target).parent.mkdir(parents=True, exist_ok=True)
            self._git(repo, "mv", "crates/apps/rusty_fair_play/web/src/moved.ts", target)
            self._git(repo, "commit", "-qm", "move app path")
            paths = changed_paths_from_git(base, cwd=repo)
        self.assertIn("crates/apps/rusty_fair_play/web/src/moved.ts", paths)
        self.assertIn(target, paths)

    def test_global_workflow_change_requires_full_sweep(self) -> None:
        self.assertTrue(is_workspace_wide_change([".github/workflows/ci.yml"]))
        self.assertFalse(is_workspace_wide_change(["crates/apps/rusty_tick/src/lib.rs"]))

    def test_multiple_apps_and_unrelated_app_selection(self) -> None:
        self.assertEqual(
            specialized_job_flags(
                [
                    "crates/apps/rusty_fair_play/src/lib.rs",
                    "crates/apps/rusty_tick/web/src/main.ts",
                ],
                [],
            ),
            {"dashboard": False, "term_web": False, "key_desktop": False, "tick": True, "fair_play": True, "win32": False, "multimodal_db": False, "rusty_config_no_std": False, "remind_me": False},
        )
        self.assertEqual(
            specialized_job_flags(["crates/apps/rusty_tick/src/lib.rs"], ["rusty_tick"]),
            {"dashboard": False, "term_web": False, "key_desktop": False, "tick": True, "fair_play": False, "win32": False, "multimodal_db": False, "rusty_config_no_std": False, "remind_me": False},
        )

    def test_shared_and_specialized_package_rules(self) -> None:
        flags = specialized_job_flags(
            [], ["rusty_win32", "rusty_multimodal_db", "rusty_config", "remind_me_hub"]
        )
        self.assertTrue(flags["win32"])
        self.assertTrue(flags["multimodal_db"])
        self.assertTrue(flags["rusty_config_no_std"])
        self.assertTrue(flags["remind_me"])

    def test_pr_runs_cancel_but_main_and_manual_runs_are_isolated(self) -> None:
        self.assertIn(
            "group: ci-${{ github.event_name == 'pull_request' && format('pr-{0}', github.event.pull_request.number) || github.event_name == 'workflow_dispatch' && format('manual-{0}', github.run_id) || 'main' }}",
            self.workflow,
        )
        self.assertIn(
            "cancel-in-progress: ${{ github.event_name == 'pull_request' }}",
            self.workflow,
        )

    def test_nextest_reports_use_the_configured_junit_profile(self) -> None:
        with NEXTEST_CONFIG.open("rb") as nextest_file:
            nextest_config = tomllib.load(nextest_file)
        self.assertEqual(nextest_config["profile"]["default"]["junit"]["path"], "junit.xml")
        self.assertNotIn("--junit-path", self.workflow)
        self.assertIn("          tool: nextest\n", self.workflow)
        self.assertIn("target/nextest/default/junit.xml", self.workflow)
        self.assertIn("tee \"test-results/nextest-", self.workflow)
        self.assertIn("Upload nextest timing and retry reports", self.workflow)
        self.assertIn("if: ${{ always() && steps.scope.outputs.args != '' }}", self.workflow)

    def test_baseline_installs_its_linux_dbus_build_prerequisite(self) -> None:
        baseline = BASELINE_WORKFLOW.read_text(encoding="utf-8")
        self.assertIn("if: runner.os == 'Linux'", baseline)
        self.assertIn("libdbus-1-dev", baseline)

    @staticmethod
    def _write(repo: Path, relative_path: str) -> None:
        target = repo / relative_path
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text("fixture\n", encoding="utf-8")

    @staticmethod
    def _git(repo: Path, *args: str) -> str:
        return subprocess.run(
            ["git", *args], cwd=repo, check=True, capture_output=True, text=True
        ).stdout

    @staticmethod
    def _planner(*args: str, input: str = "") -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [sys.executable, str(PLANNER), *args],
            input=input,
            capture_output=True,
            text=True,
            check=False,
        )

    @staticmethod
    def _planner_in(repo: Path, *args: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [sys.executable, str(PLANNER), *args],
            cwd=repo,
            capture_output=True,
            text=True,
            check=False,
        )

    @classmethod
    def _planner_map(cls, *args: str) -> dict[str, str]:
        result = cls._planner(*args)
        if result.returncode:
            raise AssertionError(result.stderr)
        return dict(line.split("=", 1) for line in result.stdout.splitlines())


if __name__ == "__main__":
    unittest.main()
