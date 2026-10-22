"""Run the non-provider Node policy tests through existing unittest discovery."""
from pathlib import Path
import subprocess
import unittest


class WorkflowPolicyTests(unittest.TestCase):
    def test_node_policy_suite(self) -> None:
        suite = Path(__file__).with_name("test_review_router.cjs")
        result = subprocess.run(["node", "--test", str(suite)], capture_output=True, text=True, check=False)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)


if __name__ == "__main__":
    unittest.main()
