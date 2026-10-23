"""Offline fixtures for the runner mirror priority adjustment; never touch /etc."""

import importlib.util
from pathlib import Path
import tempfile
import unittest


ACTION = Path(__file__).parents[1] / "actions/setup-build-env"
SPEC = importlib.util.spec_from_file_location("prefer_ubuntu_archive", ACTION / "prefer_ubuntu_archive.py")
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)

# actions/runner-images, ubuntu24/20261012.334, configure-apt-sources.sh.
MIRRORS = (
    "http://azure.archive.ubuntu.com/ubuntu/\tpriority:1\n"
    "https://archive.ubuntu.com/ubuntu/\tpriority:2\n"
    "https://security.ubuntu.com/ubuntu/\tpriority:3\n"
)
PREFERRED = MIRRORS.replace("priority:1", "priority:2", 1).replace(
    "https://archive.ubuntu.com/ubuntu/\tpriority:2",
    "https://archive.ubuntu.com/ubuntu/\tpriority:1",
)
DEB822 = (
    "Types: deb\nURIs: mirror+file:/etc/apt/apt-mirrors.txt\n"
    "Suites: noble noble-updates noble-backports noble-security\n"
    "Components: main restricted universe multiverse\n"
    "Signed-By: /usr/share/keyrings/ubuntu-archive-keyring.gpg\n"
)


class MirrorPriorityTests(unittest.TestCase):
    def test_swaps_numeric_priorities_without_reordering_or_changing_other_lines(self) -> None:
        self.assertEqual(MODULE.preferred_mirrors(MIRRORS), PREFERRED)

    def test_preserves_comments_spacing_line_endings_and_other_mirrors(self) -> None:
        source = "# image mirror list\r\n" + MIRRORS.replace("\n", "\r\n")
        source = source.replace("\tpriority:1", "  priority:1 # Azure fallback")
        source += "https://mirror.example.test/ubuntu/\tpriority:4\r\n"
        expected = source.replace("  priority:1 #", "  priority:2 #").replace(
            "https://archive.ubuntu.com/ubuntu/\tpriority:2",
            "https://archive.ubuntu.com/ubuntu/\tpriority:1",
        )
        self.assertEqual(MODULE.preferred_mirrors(source), expected)

    def test_already_preferred_is_idempotent(self) -> None:
        self.assertEqual(MODULE.preferred_mirrors(PREFERRED), PREFERRED)
        self.assertEqual(MODULE.preferred_mirrors(MODULE.preferred_mirrors(MIRRORS)), PREFERRED)

    def test_missing_duplicate_or_unexpected_target_entries_are_rejected(self) -> None:
        for source in (
            "", MIRRORS.split("\n", 1)[1],
            MIRRORS + MIRRORS.splitlines(keepends=True)[0],
            MIRRORS + MIRRORS.splitlines(keepends=True)[1],
            MIRRORS.replace("priority:1", "priority:3"),
            MIRRORS.replace("priority:2", "priority:1"),
            MIRRORS.replace("priority:2", "priority:20"),
            MIRRORS.replace("priority:2", "priority:2 arch:amd64"),
            MIRRORS.replace("https://archive.", "http://archive."),
            MIRRORS.replace("/ubuntu/\tpriority:2", "/other/\tpriority:2"),
            MIRRORS.replace("https://archive.", "# https://archive."),
        ):
            with self.subTest(source=source):
                self.assertIsNone(MODULE.preferred_mirrors(source))


class MirrorFileTests(unittest.TestCase):
    def test_edits_only_mirror_priorities_and_preserves_sources_byte_for_byte(self) -> None:
        for source in (DEB822, "deb [signed-by=/keys/ubuntu.gpg] mirror+file:/etc/apt/apt-mirrors.txt jammy main\n"):
            with self.subTest(source=source), tempfile.TemporaryDirectory() as tmp:
                mirrors, sources = Path(tmp) / "apt-mirrors.txt", Path(tmp) / "ubuntu.sources"
                mirrors.write_bytes(MIRRORS.encode())
                sources.write_bytes(source.encode())
                self.assertIn("priority 1", MODULE.prefer_archive(mirrors, [sources]))
                self.assertEqual(mirrors.read_bytes(), PREFERRED.encode())
                self.assertEqual(sources.read_bytes(), source.encode())
                self.assertIn("already", MODULE.prefer_archive(mirrors, [sources]))

    def test_absent_mirror_file_is_not_created(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            mirrors = Path(tmp) / "missing"
            self.assertIn("::warning::", MODULE.prefer_archive(mirrors, []))
            self.assertFalse(mirrors.exists())

    def test_absent_unrelated_or_commented_sources_leave_mirrors_unchanged(self) -> None:
        for source in ("", DEB822.replace("mirror+file:/etc/apt/apt-mirrors.txt", "https://archive.ubuntu.com/ubuntu/"),
                       DEB822.replace("URIs:", "# URIs:")):
            with self.subTest(source=source), tempfile.TemporaryDirectory() as tmp:
                mirrors, sources = Path(tmp) / "apt-mirrors.txt", Path(tmp) / "ubuntu.sources"
                mirrors.write_bytes(MIRRORS.encode())
                sources.write_bytes(source.encode())
                self.assertIn("::warning::", MODULE.prefer_archive(mirrors, [Path(tmp) / "absent", sources]))
                self.assertEqual(mirrors.read_bytes(), MIRRORS.encode())
                self.assertEqual(sources.read_bytes(), source.encode())

    def test_unexpected_layout_is_not_rewritten(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            mirrors, sources = Path(tmp) / "apt-mirrors.txt", Path(tmp) / "ubuntu.sources"
            original = MIRRORS.replace("priority:2", "priority:9").encode()
            mirrors.write_bytes(original)
            sources.write_bytes(DEB822.encode())
            self.assertIn("::warning::", MODULE.prefer_archive(mirrors, [sources]))
            self.assertEqual(mirrors.read_bytes(), original)
            self.assertEqual(sources.read_bytes(), DEB822.encode())

    def test_missing_source_files_leave_mirrors_unchanged(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            mirrors = Path(tmp) / "apt-mirrors.txt"
            mirrors.write_bytes(MIRRORS.encode())
            self.assertIn("::warning::", MODULE.prefer_archive(mirrors, [Path(tmp) / "absent"]))
            self.assertEqual(mirrors.read_bytes(), MIRRORS.encode())


class SetupWiringTests(unittest.TestCase):
    def test_runs_only_on_hosted_linux_before_apt_update(self) -> None:
        action = (ACTION / "action.yml").read_text(encoding="utf-8")
        step = action.split("    - name: Prefer official Ubuntu archive", 1)[1].split("\n    # rusty_term", 1)[0]
        self.assertIn("if: runner.os == 'Linux' && runner.environment == 'github-hosted'", step)
        self.assertIn('sudo python3 "${{ github.action_path }}/prefer_ubuntu_archive.py"', step)
        self.assertNotIn("continue-on-error", step)
        self.assertLess(action.index("prefer_ubuntu_archive.py"), action.index("sudo apt-get update"))


if __name__ == "__main__":
    unittest.main()
