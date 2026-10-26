"""Unit tests for check_msrv_compile.py against synthetic cargo metadata and a tiny real workspace.

Run from the repo root:
    python3 -m unittest discover -s .github/scripts -p 'test_*.py'
"""
import os
import shutil
import stat
import subprocess
import tempfile
import unittest
from pathlib import Path

from check_msrv_compile import (
    STRIP_COMPILER_FLAGS,
    check_commands,
    describe,
    featured_members,
    format_version,
    msrv_groups,
    parse_version,
    rustc_env,
)


def metadata(
    members: dict[str, str | None],
    outsiders: dict[str, str | None] | None = None,
    features: dict[str, list[str]] | None = None,
) -> dict:
    """`name -> rust-version` for workspace members and for packages outside it; `features` by name."""
    feats = features or {}
    every = {**members, **(outsiders or {})}
    packages = [{"id": n, "name": n, "rust_version": v, "features": dict.fromkeys(feats.get(n, []), [])} for n, v in every.items()]
    return {"workspace_members": list(members), "packages": packages}


class ParseVersionTests(unittest.TestCase):
    def test_omitted_components_are_zero_so_1_85_is_1_85_0(self) -> None:
        self.assertEqual(parse_version("1.85"), parse_version("1.85.0"))
        self.assertEqual(parse_version("2"), (2, 0, 0))

    def test_the_patch_is_kept(self) -> None:
        self.assertEqual(parse_version("1.88.2"), (1, 88, 2))
        self.assertNotEqual(parse_version("1.85.1"), parse_version("1.85"))

    def test_format_round_trips_to_the_full_version(self) -> None:
        self.assertEqual(format_version(parse_version("1.74")), "1.74.0")
        self.assertEqual(format_version(parse_version("1.98.1")), "1.98.1")


class MsrvGroupsTests(unittest.TestCase):
    def test_members_are_grouped_by_full_version_in_ascending_order(self) -> None:
        md = metadata({"b": "1.88", "a": "1.75", "c": "1.88.0", "d": "1.9", "e": "1.88.1"})
        self.assertEqual(
            msrv_groups(md),
            {(1, 9, 0): ["d"], (1, 75, 0): ["a"], (1, 88, 0): ["b", "c"], (1, 88, 1): ["e"]},
        )

    def test_versions_sort_numerically_not_as_text(self) -> None:
        md = metadata({"a": "1.100", "b": "1.75"})
        self.assertEqual(list(msrv_groups(md)), [(1, 75, 0), (1, 100, 0)])

    def test_members_without_a_declaration_and_non_members_are_left_out(self) -> None:
        md = metadata({"a": "1.85", "b": None}, {"dep": "1.60"})
        self.assertEqual(msrv_groups(md), {(1, 85, 0): ["a"]})

    def test_only_limits_the_members_considered(self) -> None:
        md = metadata({"a": "1.85", "b": "1.85", "c": "1.88"})
        self.assertEqual(msrv_groups(md, only={"a", "c"}), {(1, 85, 0): ["a"], (1, 88, 0): ["c"]})
        self.assertEqual(msrv_groups(md, only=set()), {})

    def test_exclude_drops_names(self) -> None:
        md = metadata({"a": "1.85", "b": "1.85"})
        self.assertEqual(msrv_groups(md, exclude=frozenset({"b"})), {(1, 85, 0): ["a"]})

    def test_skip_from_compares_full_versions(self) -> None:
        md = metadata({"a": "1.85", "b": "1.98", "c": "1.98.1", "d": "1.98.2", "e": "1.99"})
        # 1.98 is below the pinned 1.98.1, so it is exact-floor checked; 1.98.1 and above are covered elsewhere.
        self.assertEqual(msrv_groups(md, skip_from=(1, 98, 1)), {(1, 85, 0): ["a"], (1, 98, 0): ["b"]})


class CheckCommandsTests(unittest.TestCase):
    def test_one_locked_invocation_per_member_sharing_the_target_dir(self) -> None:
        commands = check_commands(["a", "b", "c"], "target/msrv/1.75.0")
        self.assertEqual(len(commands), 3)
        for command, name in zip(commands, ["a", "b", "c"]):
            self.assertEqual(command[:2], ["cargo", "check"])
            self.assertIn("--locked", command)
            self.assertEqual(command[command.index("--target-dir") + 1], "target/msrv/1.75.0")
            self.assertEqual(command.count("-p"), 1)
            self.assertEqual(command[command.index("-p") + 1], name)
            self.assertNotIn("--all-features", command)

    def test_a_featured_member_gets_an_all_features_check_right_after_its_default_one(self) -> None:
        commands = check_commands(["a", "b", "c"], "t", all_features=frozenset({"b", "z"}))
        self.assertEqual([describe(c) for c in commands], ["a", "b", "b (--all-features)", "c"])
        self.assertEqual(commands[2], [*commands[1], "--all-features"])


class FeaturedMembersTests(unittest.TestCase):
    def test_members_with_a_feature_beyond_default_are_featured(self) -> None:
        md = metadata(
            {"a": "1.75", "b": "1.75", "c": "1.75", "d": None},
            {"dep": "1.60"},
            features={"a": ["default"], "b": ["default", "std"], "c": ["serde"], "d": ["x"], "dep": ["x"]},
        )
        self.assertEqual(featured_members(md), frozenset({"b", "c", "d"}))


@unittest.skipUnless(shutil.which("cargo"), "needs cargo")
class MemberFeatureIsolationTests(unittest.TestCase):
    """`a` builds only without `std` (its default); `b` enables `std` on `a`. Grouped, `b` hides the break."""

    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory()
        root = Path(self.tmp.name)
        (root / "Cargo.toml").write_text('[workspace]\nmembers = ["a", "b"]\nresolver = "2"\n')
        (root / "a/src").mkdir(parents=True)
        (root / "a/Cargo.toml").write_text(
            '[package]\nname = "a"\nversion = "0.1.0"\nedition = "2021"\n[features]\ndefault = []\nstd = []\n'
        )
        (root / "a/src/lib.rs").write_text(
            '#![cfg_attr(not(feature = "std"), no_std)]\n'
            '#[cfg(not(feature = "std"))]\npub fn broken() -> usize { let v: Vec<u8> = Vec::new(); v.len() }\n'
        )
        (root / "b/src").mkdir(parents=True)
        (root / "b/Cargo.toml").write_text(
            '[package]\nname = "b"\nversion = "0.1.0"\nedition = "2021"\n'
            '[dependencies]\na = { path = "../a", features = ["std"] }\n'
        )
        (root / "b/src/lib.rs").write_text("")
        self.root = root
        subprocess.run(["cargo", "generate-lockfile"], cwd=root, check=True, capture_output=True)

    def tearDown(self) -> None:
        self.tmp.cleanup()

    def returncode(self, argv: list[str]) -> int:
        return subprocess.run(argv, cwd=self.root, capture_output=True, env=os.environ).returncode

    def test_a_group_hides_the_default_configuration_but_per_member_checks_expose_it(self) -> None:
        target = str(self.root / "target")
        grouped = ["cargo", "check", "--locked", "--target-dir", target, "-p", "a", "-p", "b"]
        self.assertEqual(self.returncode(grouped), 0)
        per_member = check_commands(["a", "b"], target)
        self.assertNotEqual(self.returncode(per_member[0]), 0)
        self.assertEqual(self.returncode(per_member[1]), 0)

    def test_the_all_features_check_exposes_what_the_default_one_hides(self) -> None:
        # `a`'s `std` feature now breaks it; its default build is fine.
        (self.root / "a/src/lib.rs").write_text('#[cfg(feature = "std")]\ncompile_error!("std needs a newer rustc");\n')
        commands = check_commands(["a"], str(self.root / "target"), all_features=frozenset({"a"}))
        self.assertEqual([self.returncode(c) for c in commands][0], 0)
        self.assertNotEqual(self.returncode(commands[1]), 0)


class RustcEnvTests(unittest.TestCase):
    def test_cargo_runs_the_real_rustc_through_an_executable_wrapper(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            env = rustc_env("/x/rustc", Path(tmp))
            self.assertEqual(env["RUSTC"], "/x/rustc")
            wrapper = Path(env["RUSTC_WRAPPER"])
            self.assertEqual(wrapper.read_text(), STRIP_COMPILER_FLAGS)
            self.assertTrue(wrapper.stat().st_mode & stat.S_IXUSR)


class StripCompilerFlagsWrapperTests(unittest.TestCase):
    """Run the real wrapper against a fake rustc that prints its arguments, one per line."""

    def run_wrapper(self, *args: str) -> list[str]:
        with tempfile.TemporaryDirectory() as tmp:
            fake = Path(tmp, "rustc")
            fake.write_text('#!/bin/sh\nfor a in "$@"; do printf "%s\\n" "$a"; done\n')
            fake.chmod(0o755)
            wrapper = Path(tmp, "wrap.sh")
            wrapper.write_text(STRIP_COMPILER_FLAGS)
            wrapper.chmod(0o755)
            out = subprocess.run([str(wrapper), str(fake), *args], capture_output=True, text=True, check=True)
        return out.stdout.splitlines()

    def test_check_cfg_flag_and_its_value_are_dropped_and_the_rest_kept_in_order(self) -> None:
        got = self.run_wrapper("--crate-name", "a b", "--check-cfg", 'cfg(feature, values("x"))', "--edition=2021", "src/lib.rs")
        self.assertEqual(got, ["--crate-name", "a b", "--edition=2021", "src/lib.rs"])

    def test_the_equals_form_is_dropped_too(self) -> None:
        self.assertEqual(self.run_wrapper("--check-cfg=cfg(docsrs)", "-vV"), ["-vV"])

    def test_lint_level_flags_from_a_dependencys_lints_table_are_dropped(self) -> None:
        got = self.run_wrapper(
            "--warn=deprecated_safe", "--allow=clippy::pedantic", "--deny=unsafe_code", "--forbid=missing_docs",
            "--edition=2018", "src/lib.rs",
        )
        self.assertEqual(got, ["--edition=2018", "src/lib.rs"])

    def test_flags_that_change_what_is_built_survive(self) -> None:
        args = ["--cap-lints", "allow", "-C", "link-arg=-Wl,--gc sections", "--cfg", 'feature="std"', "-Wunused", "--crate-type", "lib"]
        self.assertEqual(self.run_wrapper(*args), args)

    def test_a_trailing_check_cfg_with_no_value_does_not_break_the_command(self) -> None:
        self.assertEqual(self.run_wrapper("-C", "x y", "--check-cfg"), ["-C", "x y"])


if __name__ == "__main__":
    unittest.main()
