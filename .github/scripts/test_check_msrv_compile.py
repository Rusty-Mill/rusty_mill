"""Unit tests for check_msrv_compile.py against synthetic cargo metadata.

Run from the repo root:
    python3 -m unittest discover -s .github/scripts -p 'test_*.py'
"""
import os
import stat
import subprocess
import tempfile
import unittest
from pathlib import Path

from check_msrv_compile import STRIP_CHECK_CFG, check_command, msrv_groups, parse_floor, rustc_for


def metadata(members: dict[str, str | None], outsiders: dict[str, str | None] | None = None) -> dict:
    """`name -> rust-version` for workspace members, and for packages outside the workspace."""
    packages = [{"id": n, "name": n, "rust_version": v} for n, v in members.items()]
    packages += [{"id": n, "name": n, "rust_version": v} for n, v in (outsiders or {}).items()]
    return {"workspace_members": list(members), "packages": packages}


class MsrvGroupsTests(unittest.TestCase):
    def test_members_are_grouped_by_major_minor_floor_in_ascending_order(self) -> None:
        md = metadata({"b": "1.88", "a": "1.75", "c": "1.88.0", "d": "1.9"})
        self.assertEqual(msrv_groups(md), {(1, 9): ["d"], (1, 75): ["a"], (1, 88): ["b", "c"]})

    def test_floors_sort_numerically_not_as_text(self) -> None:
        md = metadata({"a": "1.100", "b": "1.75"})
        self.assertEqual(list(msrv_groups(md)), [(1, 75), (1, 100)])

    def test_members_without_a_declaration_and_non_members_are_left_out(self) -> None:
        md = metadata({"a": "1.85", "b": None}, {"dep": "1.60"})
        self.assertEqual(msrv_groups(md), {(1, 85): ["a"]})

    def test_only_limits_the_members_considered(self) -> None:
        md = metadata({"a": "1.85", "b": "1.85", "c": "1.88"})
        self.assertEqual(msrv_groups(md, only={"a", "c"}), {(1, 85): ["a"], (1, 88): ["c"]})
        self.assertEqual(msrv_groups(md, only=set()), {})

    def test_exclude_drops_names(self) -> None:
        md = metadata({"a": "1.85", "b": "1.85"})
        self.assertEqual(msrv_groups(md, exclude=frozenset({"b"})), {(1, 85): ["a"]})

    def test_floors_at_or_above_skip_from_are_covered_by_the_ordinary_jobs(self) -> None:
        md = metadata({"a": "1.85", "b": "1.98", "c": "1.98.1", "d": "1.99"})
        self.assertEqual(msrv_groups(md, skip_from=(1, 98)), {(1, 85): ["a"]})


class ParseFloorTests(unittest.TestCase):
    def test_patch_is_dropped_and_a_missing_minor_is_zero(self) -> None:
        self.assertEqual(parse_floor("1.88.2"), (1, 88))
        self.assertEqual(parse_floor("2"), (2, 0))


class CheckCommandTests(unittest.TestCase):
    def test_command_checks_the_group_locked_into_a_per_floor_target_dir(self) -> None:
        argv = check_command((1, 75), ["a", "b"], "target/msrv")
        self.assertEqual(argv[:2], ["cargo", "check"])
        self.assertIn("--locked", argv)
        self.assertIn("--keep-going", argv)
        self.assertEqual(argv[argv.index("--target-dir") + 1], "target/msrv/1.75")
        self.assertEqual(argv[-4:], ["-p", "a", "-p", "b"])


class RustcForTests(unittest.TestCase):
    def test_a_rustc_with_stable_check_cfg_is_used_directly(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            self.assertEqual(rustc_for((1, 80), "/x/rustc", Path(tmp)), {"RUSTC": "/x/rustc"})
            self.assertEqual(rustc_for((1, 88), "/x/rustc", Path(tmp)), {"RUSTC": "/x/rustc"})

    def test_an_older_rustc_goes_through_an_executable_wrapper(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            env = rustc_for((1, 75), "/x/rustc", Path(tmp))
            self.assertEqual(env["RUSTC"], "/x/rustc")
            wrapper = Path(env["RUSTC_WRAPPER"])
            self.assertEqual(wrapper.read_text(), STRIP_CHECK_CFG)
            self.assertTrue(wrapper.stat().st_mode & stat.S_IXUSR)


class StripCheckCfgWrapperTests(unittest.TestCase):
    """Run the real wrapper against a fake rustc that prints its arguments, one per line."""

    def run_wrapper(self, *args: str) -> list[str]:
        with tempfile.TemporaryDirectory() as tmp:
            fake = Path(tmp, "rustc")
            fake.write_text('#!/bin/sh\nfor a in "$@"; do printf "%s\\n" "$a"; done\n')
            fake.chmod(0o755)
            wrapper = Path(tmp, "wrap.sh")
            wrapper.write_text(STRIP_CHECK_CFG)
            wrapper.chmod(0o755)
            out = subprocess.run([str(wrapper), str(fake), *args], capture_output=True, text=True, check=True, env=os.environ)
        return out.stdout.splitlines()

    def test_check_cfg_flag_and_its_value_are_dropped_and_the_rest_kept_in_order(self) -> None:
        got = self.run_wrapper("--crate-name", "a b", "--check-cfg", "cfg(feature, values(\"x\"))", "--edition=2021", "src/lib.rs")
        self.assertEqual(got, ["--crate-name", "a b", "--edition=2021", "src/lib.rs"])

    def test_the_equals_form_is_dropped_too(self) -> None:
        got = self.run_wrapper("--check-cfg=cfg(docsrs)", "-vV")
        self.assertEqual(got, ["-vV"])

    def test_arguments_with_spaces_and_a_trailing_check_cfg_survive(self) -> None:
        got = self.run_wrapper("-C", "link-arg=-Wl,--gc sections", "--check-cfg", "cfg(a)")
        self.assertEqual(got, ["-C", "link-arg=-Wl,--gc sections"])


if __name__ == "__main__":
    unittest.main()
