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


UPLOAD_STEP = re.compile(r"^      - uses: actions/upload-artifact@[0-9a-f]{40}(?: #[^\n]*)?\n", re.MULTILINE)


def upload_step_body(source: str) -> str:
    """Text after the upload-artifact step's `uses:` line, whichever commit it pins.

    Matching the action by name and a 40-hex pin, not a literal SHA, keeps this
    test valid across Dependabot's routine pin updates.
    """
    match = UPLOAD_STEP.search(source)
    if match is None:
        raise AssertionError("baseline.yml has no SHA-pinned actions/upload-artifact step")
    return source[match.end():]


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
        upload = upload_step_body(WORKFLOW.read_text(encoding="utf-8"))
        self.assertRegex(upload, re.compile(r"^        if: always\(\)$", re.MULTILINE))
        self.assertIn("path: baseline-${{ matrix.os }}.md", upload)

    def test_upload_step_is_found_after_a_dependabot_style_pin_update(self) -> None:
        source = WORKFLOW.read_text(encoding="utf-8")
        current = UPLOAD_STEP.search(source)
        self.assertIsNotNone(current)
        # A different valid commit and a different version comment.
        bumped = source.replace(current.group(0), "      - uses: actions/upload-artifact@" + "0123456789abcdef" * 2 + "01234567 # v5\n", 1)
        self.assertNotEqual(bumped, source)
        self.assertEqual(upload_step_body(bumped), upload_step_body(source))
        # And with no trailing version comment at all.
        bare = source.replace(current.group(0), "      - uses: actions/upload-artifact@" + "f" * 40 + "\n", 1)
        self.assertEqual(upload_step_body(bare), upload_step_body(source))

    def test_an_unpinned_upload_step_is_not_silently_accepted(self) -> None:
        source = WORKFLOW.read_text(encoding="utf-8")
        current = UPLOAD_STEP.search(source).group(0)
        unpinned = source.replace(current, "      - uses: actions/upload-artifact@v4\n", 1)
        with self.assertRaises(AssertionError):
            upload_step_body(unpinned)


if __name__ == "__main__":
    unittest.main()
