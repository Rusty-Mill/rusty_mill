"""The queue extension must not hide unrelated syntax or expression errors."""

import unittest
import argparse
from pathlib import Path
import subprocess
import sys
import tempfile
from unittest.mock import patch

from lint_workflows import UNSUPPORTED_QUEUE, lint, remaining_diagnostics, validated_queue_lines


SOURCE = """jobs:
  test:
    concurrency:
      group: main-test
      queue: max
      cancel-in-progress: false
"""
DIAGNOSTIC = {"line": 5, "kind": "syntax-check", "message": UNSUPPORTED_QUEUE}


class QueueLintTests(unittest.TestCase):
    def test_accepts_only_the_documented_literal_policy(self) -> None:
        self.assertEqual(validated_queue_lines(SOURCE), ({5}, []))
        self.assertEqual(remaining_diagnostics(SOURCE, [DIAGNOSTIC]), [])

    def test_comment_requires_separating_whitespace(self) -> None:
        for suffix in ("#invalid", "\n        invalid", "\n\n        invalid"):
            with self.subTest(suffix=suffix):
                self.assertTrue(remaining_diagnostics(SOURCE.replace("queue: max", "queue: max" + suffix), [DIAGNOSTIC]))
        self.assertEqual(remaining_diagnostics(SOURCE.replace("queue: max", "queue: max # comment"), [DIAGNOSTIC]), [])

    def test_cancel_scalar_cannot_have_a_continuation(self) -> None:
        for suffix in ("#invalid", "\n        invalid"):
            with self.subTest(suffix=suffix):
                self.assertTrue(remaining_diagnostics(SOURCE.replace("cancel-in-progress: false", "cancel-in-progress: false" + suffix), [DIAGNOSTIC]))

    def test_rejects_invalid_or_unverified_queue_values(self) -> None:
        for value in ("maximum", "true", "[max]", "", "${{ inputs.queue }}", "*alias"):
            with self.subTest(value=value):
                self.assertTrue(remaining_diagnostics(SOURCE.replace("queue: max", f"queue: {value}"), [DIAGNOSTIC]))

    def test_rejects_cancellation_true_implicit_or_expression(self) -> None:
        for value in ("true", "${{ github.event_name == 'pull_request' }}", "", "no"):
            with self.subTest(value=value):
                self.assertTrue(remaining_diagnostics(SOURCE.replace("false", value), [DIAGNOSTIC]))
        self.assertTrue(remaining_diagnostics(SOURCE.replace("      cancel-in-progress: false\n", ""), [DIAGNOSTIC]))

    def test_duplicate_keys_are_not_accepted(self) -> None:
        for field in ("queue: max", "cancel-in-progress: false"):
            self.assertTrue(remaining_diagnostics(SOURCE + f"      {field}\n", [DIAGNOSTIC]))

    def test_keeps_all_other_actionlint_errors(self) -> None:
        for error in (
            {"line": 7, "kind": "syntax-check", "message": "unexpected key"},
            {"line": 4, "kind": "expression", "message": "unknown matrix property"},
            {**DIAGNOSTIC, "line": 15},
            {**DIAGNOSTIC, "message": "new unrecognized diagnostic"},
        ):
            with self.subTest(error=error):
                self.assertEqual(len(remaining_diagnostics(SOURCE, [DIAGNOSTIC, error])), 1)

    def test_does_not_accept_queue_at_an_unknown_yaml_location(self) -> None:
        self.assertTrue(remaining_diagnostics(SOURCE.replace("concurrency:", "environment:"), [DIAGNOSTIC]))

    def test_rejects_invalid_policy_even_with_no_actionlint_diagnostic(self) -> None:
        self.assertTrue(remaining_diagnostics(SOURCE.replace("queue: max", "queue: invalid"), []))

    def test_tool_failure_without_diagnostics_fails_closed(self) -> None:
        with patch("lint_workflows.subprocess.run", return_value=subprocess.CompletedProcess([], 1, "[]", "")):
            self.assertFalse(lint("actionlint", Path("workflow.yml")))


def check_real_actionlint(actionlint: str) -> None:
    """Exercise complete YAML through the pinned binary used by workflow CI."""
    source = "name: queue regression\non: push\n" + SOURCE + """    runs-on: ubuntu-latest
    steps:
      - run: echo verified
"""
    cases = {
        "valid": (source, True),
        "separated_comment": (source.replace("queue: max", "queue: max # comment"), True),
        "attached_hash": (source.replace("queue: max", "queue: max#invalid"), False),
        "continuation": (source.replace("queue: max", "queue: max\n        invalid"), False),
        "blank_continuation": (source.replace("queue: max", "queue: max\n\n        invalid"), False),
        "comment_continuation": (source.replace("queue: max", "queue: max\n        # comment\n        invalid"), False),
        "cancel_hash": (source.replace("false", "false#invalid"), False),
        "cancel_continuation": (source.replace("false", "false\n        invalid"), False),
        "unrelated_key": (source.replace("runs-on:", "runs-onn:"), False),
        "unrelated_expression": (source.replace("group: main-test", "group: ${{ matrix.nonexistent }}"), False),
    }
    with tempfile.TemporaryDirectory() as directory:
        for name, (text, expected) in cases.items():
            workflow = Path(directory) / f"{name}.yml"
            workflow.write_text(text, encoding="utf-8")
            actual = lint(actionlint, workflow)
            if actual != expected:
                raise AssertionError(f"real actionlint regression {name}: expected {expected}, got {actual}")
            print(f"real actionlint regression {name}: passed")


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--actionlint")
    args, unittest_args = parser.parse_known_args()
    if args.actionlint:
        check_real_actionlint(args.actionlint)
    unittest.main(argv=[sys.argv[0], *unittest_args])
