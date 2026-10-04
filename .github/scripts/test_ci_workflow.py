"""Regression checks for CI event planning and full-sweep scheduling.

These checks intentionally inspect the checked-in workflow text rather than
requiring a GitHub Actions runner. The expressions are GitHub-specific, but
the behavior they encode is simple and important: PR updates cancel obsolete
work, while main keeps an active full sweep and lets Actions replace only
older pending work in that shared group.
"""

from pathlib import Path
import tomllib
import unittest


WORKFLOW = Path(__file__).parents[1] / "workflows" / "ci.yml"
BASELINE_WORKFLOW = Path(__file__).parents[1] / "workflows" / "baseline.yml"
NEXTEST_CONFIG = Path(__file__).parents[2] / ".config" / "nextest.toml"


class CiWorkflowSchedulingTests(unittest.TestCase):
    def setUp(self) -> None:
        self.workflow = WORKFLOW.read_text(encoding="utf-8")

    def test_push_pr_and_manual_full_sweep_events_are_declared(self) -> None:
        self.assertIn("  push:\n    branches: [main]", self.workflow)
        self.assertIn("  pull_request:", self.workflow)
        self.assertIn("  workflow_dispatch:", self.workflow)
        self.assertIn('if [ "${{ github.event_name }}" != "pull_request" ]; then', self.workflow)

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


if __name__ == "__main__":
    unittest.main()
