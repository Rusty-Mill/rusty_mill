"""Exercise the baseline report handoff with successful and failing fixtures."""

import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import textwrap
import unittest


WORKFLOW = Path(__file__).parents[1] / "workflows" / "baseline.yml"
BASH = shutil.which("bash")
if os.name == "nt":
    # Prefer Git Bash to a WSL launcher, which does not inherit these env vars.
    candidate = Path(os.environ.get("ProgramFiles", "C:/Program Files")) / "Git/bin/bash.exe"
    if candidate.is_file():
        BASH = str(candidate)


class BaselineWorkflowTests(unittest.TestCase):
    @unittest.skipUnless(BASH, "bash is needed to execute the Actions run block")
    def test_measure_preserves_report_and_actual_exit_status(self) -> None:
        source = WORKFLOW.read_text(encoding="utf-8")
        measure = source.split("      - name: Measure\n", 1)[1].split("      - uses:", 1)[0]
        script = textwrap.dedent(measure.split("        run: |\n", 1)[1])
        # Run the actual workflow shell block under Actions' error settings.
        # Only Cargo is replaced; no real products are built or benchmarked.
        fixture = "cargo() { printf '%s\\n' 'partial measurement table'; return \"$FIXTURE_STATUS\"; }\n"
        for status in (0, 1, 17):
            with self.subTest(status=status), tempfile.TemporaryDirectory() as directory:
                env = dict(os.environ, REPORT="report.md", GITHUB_STEP_SUMMARY="summary.md",
                           RUNNER_TEMP=".", PRODUCTS="", FIXTURE_STATUS=str(status))
                result = subprocess.run(
                    [BASH, "--noprofile", "--norc", "-e", "-o", "pipefail", "-c", fixture + script],
                    cwd=directory, env=env, capture_output=True, text=True, check=False,
                )
                self.assertEqual(result.returncode, status, result.stderr)
                self.assertEqual((Path(directory) / "report.md").read_text(), "partial measurement table\n")
                self.assertEqual((Path(directory) / "summary.md").read_text(), "partial measurement table\n")

    def test_report_artifact_is_uploaded_after_measurement_failure(self) -> None:
        source = WORKFLOW.read_text(encoding="utf-8")
        upload = source.split("      - uses: actions/upload-artifact@ea165f8d65b6e75b540449e92b4886f43607fa02 # v4\n", 1)[1]
        self.assertRegex(upload, re.compile(r"^        if: always\(\)$", re.MULTILINE))
        self.assertIn("path: baseline-${{ matrix.os }}.md", upload)


if __name__ == "__main__":
    unittest.main()
