"""Unit tests for check_msrv_compile.py against synthetic cargo metadata and a tiny real workspace.

Run from the repo root:
    python3 -m unittest discover -s .github/scripts -p 'test_*.py'
"""
import json
import os
import shutil
import stat
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

from check_msrv_compile import (
    build_wrapper,
    check_commands,
    describe,
    featured_members,
    format_version,
    members_for_manifests,
    msrv_groups,
    parse_floors,
    parse_version,
    raised_floor_groups,
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


class MembersForManifestsTests(unittest.TestCase):
    ROOT = os.path.abspath("ws")
    MD = {
        "workspace_root": ROOT,
        "workspace_members": ["a", "b"],
        "packages": [
            {"id": "a", "name": "a", "manifest_path": os.path.join(ROOT, "crates", "a", "Cargo.toml")},
            {"id": "b", "name": "b", "manifest_path": os.path.join(ROOT, "crates", "b", "Cargo.toml")},
            {"id": "dep", "name": "dep", "manifest_path": os.path.join(ROOT, "vendor", "dep", "Cargo.toml")},
        ],
    }

    def test_a_changed_manifest_selects_only_its_own_member(self) -> None:
        self.assertEqual(members_for_manifests(self.MD, ["crates/a/Cargo.toml"]), {"a"})

    def test_deleted_non_member_and_root_manifests_select_nothing(self) -> None:
        paths = ["crates/gone/Cargo.toml", "vendor/dep/Cargo.toml", "Cargo.toml"]
        self.assertEqual(members_for_manifests(self.MD, paths), set())

    def test_an_empty_scope_exits_before_installing_anything(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            md = Path(tmp, "metadata.json")
            md.write_text(json.dumps({**self.MD, "packages": [{**p, "rust_version": "1.80", "features": {}} for p in self.MD["packages"]]}))
            script = Path(__file__).with_name("check_msrv_compile.py")
            out = subprocess.run(
                [sys.executable, str(script), str(md), "--manifests", "crates/gone/Cargo.toml", "--target-base", tmp],
                capture_output=True, text=True, check=True,
            ).stdout
            self.assertEqual(os.listdir(tmp), ["metadata.json"])  # no wrapper built, no toolchain fetched
        self.assertIn("Members whose manifest changed: none", out)


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


class RaisedFloorTests(unittest.TestCase):
    MD = metadata({"axum": "1.87", "plain": "1.87", "tokio": "1.87"}, features={"axum": ["http2"], "tokio": ["rt"]})

    def test_pairs_parse_to_full_versions(self) -> None:
        self.assertEqual(parse_floors("a=1.88 b=1.90.1"), {"a": (1, 88, 0), "b": (1, 90, 1)})
        self.assertEqual(parse_floors(""), {})
        for bad in ("a", "a=", "=1.88", "a=x"):
            with self.assertRaises(ValueError):
                parse_floors(bad)

    def test_default_build_stays_at_the_declared_floor_and_all_features_moves_to_the_raised_one(self) -> None:
        groups = msrv_groups(self.MD)
        raised = raised_floor_groups(self.MD, groups, {"axum": (1, 88, 0)})
        self.assertEqual(groups, {(1, 87, 0): ["axum", "plain", "tokio"]})
        self.assertEqual(raised, {(1, 88, 0): ["axum"]})
        # As main() builds them: axum's default check at 1.87, its --all-features check only at 1.88.
        all_features = featured_members(self.MD) - frozenset({"axum"})
        at_187 = [describe(c) for c in check_commands(groups[(1, 87, 0)], "t", all_features)]
        at_188 = [describe(c) for c in check_commands([], "t", all_features, raised[(1, 88, 0)])]
        self.assertEqual(at_187, ["axum", "plain", "tokio", "tokio (--all-features)"])
        self.assertEqual(at_188, ["axum (--all-features)"])

    def test_a_member_out_of_scope_or_at_the_pinned_toolchain_is_not_scheduled(self) -> None:
        scoped = msrv_groups(self.MD, only={"plain"})
        self.assertEqual(raised_floor_groups(self.MD, scoped, {"axum": (1, 88, 0)}), {})
        groups = msrv_groups(self.MD)
        self.assertEqual(raised_floor_groups(self.MD, groups, {"axum": (1, 88, 0)}, skip_from=(1, 88, 0)), {})

    def test_a_stale_override_fails_instead_of_checking_nothing(self) -> None:
        groups = msrv_groups(self.MD)
        for floors in ({"plain": (1, 88, 0)}, {"gone": (1, 88, 0)}, {"axum": (1, 87, 0)}, {"axum": (1, 86, 0)}):
            with self.assertRaises(ValueError):
                raised_floor_groups(self.MD, groups, floors)


class MainTests(unittest.TestCase):
    def test_metadata_is_read_as_utf8_whatever_the_locale(self) -> None:
        # Windows runners default to cp1252; cargo metadata is UTF-8 (a description with a curly quote broke #1081).
        md = metadata({"a": "1.75"})
        md["packages"][0]["description"] = "caf\u00e9 \u201cquoted\u201d"
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp, "m.json")
            path.write_text(json.dumps(md, ensure_ascii=False), encoding="utf-8")
            env = {**os.environ, "LC_ALL": "C", "LANG": "C", "PYTHONUTF8": "0"}
            script = Path(__file__).with_name("check_msrv_compile.py")
            out = subprocess.run([sys.executable, str(script), str(path), "--print-plan"], env=env, capture_output=True, text=True)
        self.assertEqual(out.returncode, 0, out.stderr)
        self.assertIn("1.75.0: 1 crates", out.stdout)


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
    def test_cargo_runs_the_real_rustc_through_the_wrapper(self) -> None:
        env = rustc_env("/x/rustc", Path("/w/msrv_rustc_wrapper"))
        self.assertEqual(env, {"RUSTC": "/x/rustc", "RUSTC_WRAPPER": "/w/msrv_rustc_wrapper"})


@unittest.skipUnless(shutil.which("rustc"), "needs rustc")
class WrapperTests(unittest.TestCase):
    """Run the compiled wrapper against a fake rustc that prints its arguments, one per line."""

    @classmethod
    def setUpClass(cls) -> None:
        cls.tmp = tempfile.TemporaryDirectory()
        cls.dir = Path(cls.tmp.name)
        cls.wrapper = build_wrapper(cls.dir)
        cls.fake = cls.dir / "rustc"
        cls.fake.write_text('#!/bin/sh\nfor a in "$@"; do printf "%s\\n" "$a"; done\n')
        cls.fake.chmod(0o755)

    @classmethod
    def tearDownClass(cls) -> None:
        cls.tmp.cleanup()

    def run_wrapper(self, *args: str) -> list[str]:
        out = subprocess.run([str(self.wrapper), str(self.fake), *args], capture_output=True, text=True, check=True)
        return out.stdout.splitlines()

    def test_the_wrapper_is_an_executable_beside_the_build_output(self) -> None:
        self.assertEqual(self.wrapper.parent, self.dir.resolve())
        self.assertTrue(self.wrapper.stat().st_mode & stat.S_IXUSR)

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

    def test_an_argfile_is_filtered_into_a_copy_and_the_original_left_alone(self) -> None:
        argfile = self.dir / "args"
        original = "--crate-name\nx\n--check-cfg\ncfg(docsrs)\n--warn=unused\nsrc/lib.rs\n"
        argfile.write_text(original)
        got = self.run_wrapper("--edition=2021", f"@{argfile}")
        self.assertEqual(got, ["--edition=2021", f"@{argfile}.msrv"])
        self.assertEqual(Path(f"{argfile}.msrv").read_text().splitlines(), ["--crate-name", "x", "src/lib.rs"])
        self.assertEqual(argfile.read_text(), original)

    def test_the_real_rustcs_exit_code_is_returned(self) -> None:
        failing = self.dir / "failing-rustc"
        failing.write_text("#!/bin/sh\nexit 3\n")
        failing.chmod(0o755)
        self.assertEqual(subprocess.run([str(self.wrapper), str(failing)]).returncode, 3)
        self.assertNotEqual(subprocess.run([str(self.wrapper)], capture_output=True).returncode, 0)


if __name__ == "__main__":
    unittest.main()
