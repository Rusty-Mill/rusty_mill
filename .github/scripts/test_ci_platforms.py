"""Checks for per-event platform selection and the merge queue's Linux reuse."""

from pathlib import Path
import io
import json
import re
import subprocess
import tempfile
import unittest
import zipfile

from ci_plan import (
    TESTED_RECORD_ARTIFACT,
    TESTED_RECORD_FILE,
    linux_already_tested,
    platform_outputs,
    tested_record,
)


WORKFLOW = Path(__file__).parents[1] / "workflows" / "ci.yml"


def git(repo: Path, *args: str) -> str:
    return subprocess.run(
        ["git", *args], cwd=repo, check=True, capture_output=True, text=True
    ).stdout.strip()


def record_zip(record: str) -> bytes:
    buffer = io.BytesIO()
    with zipfile.ZipFile(buffer, "w") as archive:
        archive.writestr(TESTED_RECORD_FILE, record + "\n")
    return buffer.getvalue()


class PlatformOutputTests(unittest.TestCase):
    def test_pull_requests_run_linux_only(self) -> None:
        self.assertEqual(
            platform_outputs("pull_request"),
            {
                "linux": "true",
                "windows": "false",
                "macos": "false",
                "rust_os": '["ubuntu-latest"]',
                "rand_os": '["ubuntu-latest"]',
            },
        )

    def test_the_queue_runs_every_platform_unless_linux_was_tested(self) -> None:
        untested = platform_outputs("merge_group")
        self.assertEqual(untested["rust_os"], '["ubuntu-latest","windows-latest"]')
        self.assertEqual(untested["rand_os"], '["ubuntu-latest","macos-latest"]')
        tested = platform_outputs("merge_group", linux_tested=True)
        self.assertEqual(
            (tested["linux"], tested["windows"], tested["macos"]), ("false", "true", "true")
        )
        self.assertEqual(tested["rust_os"], '["windows-latest"]')
        self.assertEqual(tested["rand_os"], '["macos-latest"]')

    def test_manual_and_scheduled_sweeps_run_everything(self) -> None:
        for event in ("workflow_dispatch", "schedule", "push"):
            with self.subTest(event=event):
                # A reuse claim only ever applies to a queue entry.
                outputs = platform_outputs(event, linux_tested=True)
                self.assertEqual(
                    (outputs["linux"], outputs["windows"], outputs["macos"]),
                    ("true", "true", "true"),
                )

    def test_no_event_leaves_a_matrix_empty(self) -> None:
        for event in ("pull_request", "merge_group", "schedule"):
            for tested in (False, True):
                outputs = platform_outputs(event, tested)
                self.assertTrue(json.loads(outputs["rust_os"]))
                self.assertTrue(json.loads(outputs["rand_os"]))


class LinuxReuseTests(unittest.TestCase):
    """A queue commit reuses a PR run only for the same tree on the same parent."""

    def setUp(self) -> None:
        self._dir = tempfile.TemporaryDirectory()
        self.repo = Path(self._dir.name)
        git(self.repo, "init", "-q", "-b", "main")
        git(self.repo, "config", "user.email", "ci@example.invalid")
        git(self.repo, "config", "user.name", "CI test")
        self._commit("base.txt", "main")
        self.base = git(self.repo, "rev-parse", "HEAD")
        git(self.repo, "checkout", "-q", "-b", "pr")
        self._commit("feature.txt", "pr")
        self.pr_head = git(self.repo, "rev-parse", "HEAD")
        git(self.repo, "checkout", "-q", "main")
        # What a pull_request run checks out: the PR merged onto its base.
        git(self.repo, "merge", "-q", "--no-ff", "-m", "pr merge ref", "pr")
        self.pr_record = tested_record(cwd=str(self.repo))
        self.calls: list[list[str]] = []

    def tearDown(self) -> None:
        self._dir.cleanup()

    def _commit(self, name: str, message: str) -> None:
        (self.repo / name).write_text(message + "\n", encoding="utf-8")
        git(self.repo, "add", name)
        git(self.repo, "commit", "-qm", message)

    def _queue_commit_on(self, parent: str) -> None:
        """Check out a queue-style merge commit of the PR head onto ``parent``."""
        git(self.repo, "checkout", "-q", "--detach", parent)
        git(self.repo, "merge", "-q", "--no-ff", "-m", "queue entry", self.pr_head)

    def _gh(self, records: list[str], expired: bool = False):
        def gh(args: list[str]) -> bytes:
            self.calls.append(args)
            path = args[1]
            if "/workflows/ci.yml/runs?" in path:
                return json.dumps({"workflow_runs": [{"id": 7}]}).encode()
            if path.endswith(f"/runs/7/artifacts?name={TESTED_RECORD_ARTIFACT}"):
                artifacts = [{"id": 100 + i, "expired": expired} for i in range(len(records))]
                return json.dumps({"artifacts": artifacts}).encode()
            match = re.search(r"/artifacts/(\d+)/zip$", path)
            if match:
                return record_zip(records[int(match.group(1)) - 100])
            raise AssertionError(f"unexpected gh call {args}")

        return gh

    def test_the_same_tree_on_the_same_parent_is_reused(self) -> None:
        self._queue_commit_on(self.base)
        self.assertEqual(tested_record(cwd=str(self.repo)), self.pr_record)
        self.assertTrue(linux_already_tested("o/r", self._gh([self.pr_record]), cwd=str(self.repo)))
        # The lookup asks only for green pull_request runs of the PR head.
        query = self.calls[0][1]
        self.assertIn("event=pull_request", query)
        self.assertIn("status=success", query)
        self.assertIn(f"head_sha={self.pr_head}", query)

    def test_a_queue_entry_on_a_newer_main_is_not_reused(self) -> None:
        git(self.repo, "checkout", "-q", "--detach", self.base)
        self._commit("other.txt", "another PR landed")
        self._queue_commit_on(git(self.repo, "rev-parse", "HEAD"))
        self.assertFalse(linux_already_tested("o/r", self._gh([self.pr_record]), cwd=str(self.repo)))

    def test_the_same_tree_on_a_different_parent_is_not_reused(self) -> None:
        # main gained the PR's own change, so the trees match but the scope
        # (the diff from the parent) the PR run planned does not.
        git(self.repo, "checkout", "-q", "--detach", self.base)
        self._commit("feature.txt", "pr")
        self._queue_commit_on(git(self.repo, "rev-parse", "HEAD"))
        tree = git(self.repo, "rev-parse", "HEAD^{tree}")
        self.assertEqual(tree, self.pr_record.split()[0])
        self.assertFalse(linux_already_tested("o/r", self._gh([self.pr_record]), cwd=str(self.repo)))

    def test_any_matching_record_among_several_is_enough(self) -> None:
        self._queue_commit_on(self.base)
        gh = self._gh(["0" * 40 + " " + self.base, self.pr_record])
        self.assertTrue(linux_already_tested("o/r", gh, cwd=str(self.repo)))

    def test_expired_records_are_ignored(self) -> None:
        self._queue_commit_on(self.base)
        gh = self._gh([self.pr_record], expired=True)
        self.assertFalse(linux_already_tested("o/r", gh, cwd=str(self.repo)))

    def test_lookup_failures_mean_not_tested(self) -> None:
        self._queue_commit_on(self.base)

        def failing(args: list[str]) -> bytes:
            raise subprocess.CalledProcessError(1, ["gh", *args])

        self.assertFalse(linux_already_tested("o/r", failing, cwd=str(self.repo)))
        self.assertFalse(linux_already_tested("o/r", lambda args: b"not json", cwd=str(self.repo)))

    def test_a_commit_that_is_not_a_merge_is_never_reused(self) -> None:
        git(self.repo, "checkout", "-q", "--detach", self.pr_head)
        self.assertFalse(linux_already_tested("o/r", self._gh([self.pr_record]), cwd=str(self.repo)))
        self.assertEqual(self.calls, [])


class WorkflowPlatformTests(unittest.TestCase):
    def setUp(self) -> None:
        self.workflow = WORKFLOW.read_text(encoding="utf-8")
        body = self.workflow.split("\njobs:\n", 1)[1]
        parts = re.split(r"^  ([a-z][a-z0-9-]+):\n", body, flags=re.M)
        self.jobs = dict(zip(parts[1::2], parts[2::2]))

    def test_main_has_no_post_merge_run_but_a_daily_sweep(self) -> None:
        triggers = self.workflow.split("\non:\n", 1)[1].split("\npermissions:", 1)[0]
        self.assertNotRegex(triggers, r"(?m)^  push:")
        self.assertRegex(triggers, r'(?m)^    - cron: "\d+ \d+ \* \* \*"$')

    def test_windows_jobs_run_only_where_the_plan_selects_windows(self) -> None:
        for job, text in self.jobs.items():
            if "runs-on: windows-latest" in text:
                with self.subTest(job=job):
                    self.assertIn("needs.plan.outputs.windows == 'true'", text)

    def test_planned_linux_jobs_also_need_the_linux_platform(self) -> None:
        for job, text in self.jobs.items():
            if "runs-on: ubuntu-latest" in text and re.search(r"(?m)^    if: needs\.plan\.outputs\.", text):
                with self.subTest(job=job):
                    self.assertIn("needs.plan.outputs.linux == 'true'", text)
        self.assertIn("    if: needs.plan.outputs.linux == 'true'\n", self.jobs["orch-bbp-acceptance"])

    def test_os_matrices_come_from_the_plan(self) -> None:
        self.assertNotRegex(self.workflow, r"(?m)^        os: \[")
        for job in ("clippy", "ci-smoke", "test"):
            with self.subTest(job=job):
                self.assertIn("os: ${{ fromJSON(needs.plan.outputs.rust_os) }}", self.jobs[job])
        self.assertIn("os: ${{ fromJSON(needs.plan.outputs.rand_os) }}", self.jobs["rusty-rand-platforms"])

    def test_pull_requests_record_their_tree_and_the_queue_reads_it(self) -> None:
        plan = self.jobs["plan"]
        for key in ("linux", "windows", "macos", "rust_os", "rand_os"):
            self.assertIn(f"      {key}: ${{{{ steps.platforms.outputs.{key} }}}}\n", plan)
        self.assertIn("    permissions:\n      contents: read\n      actions: read\n", plan)
        self.assertEqual(plan.count("if: github.event_name == 'pull_request'"), 2)
        self.assertIn("ci_plan.py --tested-record > tested-tree.txt", plan)
        self.assertIn(f"name: {TESTED_RECORD_ARTIFACT}\n", plan)
        self.assertIn(f"path: {TESTED_RECORD_FILE}\n", plan)
        self.assertIn(
            '--emit-platforms "${{ github.event_name }}" --repo "${{ github.repository }}"', plan
        )
        # Only the planner may read Actions data.
        for job, text in self.jobs.items():
            if job != "plan":
                with self.subTest(job=job):
                    self.assertNotIn("actions: read", text)


if __name__ == "__main__":
    unittest.main()
