"""Unit tests for affected_crates.py against synthetic cargo metadata.

Run from the repo root:
    python3 -m unittest discover -s .github/scripts -p 'test_*.py'
"""
import tomllib
import unittest
from pathlib import Path

from affected_crates import affected_packages
from ci_components import component_scopes
from ci_plan import is_ci_only_change, is_workspace_wide_change, specialized_job_flags

ROOT = "/repo"


def metadata(crates: dict[str, tuple[str, list[str]]], external: list[str] = ()) -> dict:
    """Build the subset of `cargo metadata` that affected_crates.py reads.

    `crates` maps a package name to (directory relative to the root, list of
    dependency package names). Names double as package ids. `external` lists
    packages that are in the resolve graph but not workspace members.
    """
    packages = [
        {"id": name, "name": name, "manifest_path": f"{ROOT}/{directory}/Cargo.toml"}
        for name, (directory, _) in crates.items()
    ]
    packages += [{"id": name, "name": name, "manifest_path": f"/registry/{name}/Cargo.toml"} for name in external]
    nodes = [
        {"id": name, "deps": [{"name": dep, "pkg": dep} for dep in deps]}
        for name, (_, deps) in crates.items()
    ]
    nodes += [{"id": name, "deps": []} for name in external]
    return {
        "workspace_root": ROOT,
        "workspace_members": list(crates),
        "packages": packages,
        "resolve": {"nodes": nodes},
    }


class OwningCrateTests(unittest.TestCase):
    def test_file_inside_a_crate_selects_that_crate(self) -> None:
        md = metadata({"a": ("crates/a", []), "b": ("crates/b", [])})
        self.assertEqual(affected_packages(md, ["crates/a/src/lib.rs"]), ["a"])

    def test_file_outside_every_crate_selects_nothing(self) -> None:
        md = metadata({"a": ("crates/a", [])})
        self.assertEqual(affected_packages(md, ["README.md", "docs/adr/0001.md"]), [])

    def test_no_changed_files_selects_nothing(self) -> None:
        md = metadata({"a": ("crates/a", [])})
        self.assertEqual(affected_packages(md, []), [])

    def test_nested_crate_wins_over_its_parent(self) -> None:
        md = metadata({"outer": ("crates/outer", []), "inner": ("crates/outer/inner", [])})
        self.assertEqual(affected_packages(md, ["crates/outer/inner/src/lib.rs"]), ["inner"])
        self.assertEqual(affected_packages(md, ["crates/outer/src/lib.rs"]), ["outer"])

    def test_directory_name_prefix_is_not_containment(self) -> None:
        md = metadata({"foo": ("crates/foo", []), "foobar": ("crates/foobar", [])})
        self.assertEqual(affected_packages(md, ["crates/foobar/src/lib.rs"]), ["foobar"])

    def test_manifest_itself_counts_as_inside_the_crate(self) -> None:
        md = metadata({"a": ("crates/a", [])})
        self.assertEqual(affected_packages(md, ["crates/a/Cargo.toml"]), ["a"])


class TlsAdrImpactTests(unittest.TestCase):
    # Exact changed paths from PR #868, including its newly added ADR.
    PR_868_PATHS = [
        "changes/2026-10-22-rusty-tls-adr-0005-accepted.md",
        "crates/libs/net/rusty_tls/docs/adr/0002-handrolled-engine-behind-a-permanently-non-default-seam.md",
        "crates/libs/net/rusty_tls/docs/adr/0005-first-party-primitives-for-the-client-engine.md",
        "docs/research/CRYPTO-REPLACEMENT-PLAN.md",
        "docs/research/TLS-NATIVE-CRYPTO-SEAM.md",
    ]

    def setUp(self) -> None:
        self.metadata = metadata({
            "rusty_tls": ("crates/libs/net/rusty_tls", []),
            "rusty_request": ("crates/libs/net/rusty_request", ["rusty_tls"]),
            "rusty_tick": ("crates/apps/rusty_tick", ["rusty_request"]),
            "unrelated": ("crates/libs/other", []),
        })
        self.tls_dependents = ["rusty_request", "rusty_tick", "rusty_tls"]

    def test_pr_868_prose_does_not_seed_cargo_impact(self) -> None:
        for paths in ([path] for path in self.PR_868_PATHS):
            with self.subTest(paths=paths):
                self.assertEqual(affected_packages(self.metadata, paths), [])
        self.assertEqual(affected_packages(self.metadata, iter(self.PR_868_PATHS)), [])

    def test_pr_868_plans_no_cargo_lanes_or_specialized_jobs(self) -> None:
        packages = affected_packages(self.metadata, self.PR_868_PATHS)
        self.assertEqual(component_scopes(self.metadata, packages), [])
        self.assertFalse(any(specialized_job_flags(self.PR_868_PATHS, packages).values()))
        self.assertFalse(is_workspace_wide_change(self.PR_868_PATHS))
        self.assertFalse(is_ci_only_change(self.PR_868_PATHS))

    def test_mixed_tls_source_manifest_and_fixture_changes_keep_dependents(self) -> None:
        for path in (
            "src/lib.rs", "Cargo.toml", "tests/data/rsa2048_pkcs8.hex",
            "tests/data/fixture.md",
        ):
            with self.subTest(path=path):
                paths = [*self.PR_868_PATHS, f"crates/libs/net/rusty_tls/{path}"]
                packages = affected_packages(self.metadata, paths)
                self.assertEqual(packages, self.tls_dependents)
                self.assertTrue(specialized_job_flags(paths, packages)["tls_engine"])
                self.assertEqual(
                    sorted(" ".join(scope["packages"] for scope in component_scopes(self.metadata, packages)).split()),
                    self.tls_dependents,
                )

    def test_only_direct_markdown_in_the_audited_directory_is_excluded(self) -> None:
        for path in (
            "README.md", "docs/README.md", "docs/adr-other/decision.md",
            "docs/adr.md", "docs/adr/nested/decision.md", "docs/adr/fixture.json",
            "docs/adr/example.rs", "docs/adr/../fixture.md",
        ):
            with self.subTest(path=path):
                self.assertEqual(
                    affected_packages(self.metadata, [f"crates/libs/net/rusty_tls/{path}"]),
                    self.tls_dependents,
                )
        self.assertEqual(
            affected_packages(self.metadata, ["crates/libs/other/docs/adr/decision.md"]),
            ["unrelated"],
        )

    def test_mixed_unrelated_code_is_not_dropped_with_the_adrs(self) -> None:
        self.assertEqual(
            affected_packages(self.metadata, [*self.PR_868_PATHS, "crates/libs/other/src/lib.rs"]),
            ["unrelated"],
        )


class ReverseDependencyTests(unittest.TestCase):
    def test_direct_dependents_are_included(self) -> None:
        md = metadata({"a": ("crates/a", []), "b": ("crates/b", ["a"]), "c": ("crates/c", [])})
        self.assertEqual(affected_packages(md, ["crates/a/src/lib.rs"]), ["a", "b"])

    def test_dependents_are_transitive(self) -> None:
        md = metadata({"a": ("crates/a", []), "b": ("crates/b", ["a"]), "c": ("crates/c", ["b"])})
        self.assertEqual(affected_packages(md, ["crates/a/src/lib.rs"]), ["a", "b", "c"])

    def test_shared_server_selects_each_dependent_application(self) -> None:
        md = metadata(
            {
                "rusty_serve": ("crates/libs/net/rusty_serve", []),
                "rusty_fair_play": ("crates/apps/rusty_fair_play", ["rusty_serve"]),
                "rusty_tick": ("crates/apps/rusty_tick", ["rusty_serve"]),
            }
        )
        self.assertEqual(
            affected_packages(md, ["crates/libs/net/rusty_serve/src/lib.rs"]),
            ["rusty_fair_play", "rusty_serve", "rusty_tick"],
        )

    def test_changing_a_leaf_does_not_pull_in_its_dependencies(self) -> None:
        md = metadata({"a": ("crates/a", []), "b": ("crates/b", ["a"])})
        self.assertEqual(affected_packages(md, ["crates/b/src/lib.rs"]), ["b"])

    def test_external_dependencies_are_ignored(self) -> None:
        md = metadata({"a": ("crates/a", ["serde"]), "b": ("crates/b", ["serde"])}, external=["serde"])
        self.assertEqual(affected_packages(md, ["crates/a/src/lib.rs"]), ["a"])

    def test_local_patched_package_selects_its_workspace_consumers(self) -> None:
        # A [patch]ed vendor crate is in the resolve graph but not a member.
        md = metadata(
            {"app": ("crates/app", ["auth"]), "auth": ("crates/auth", ["jwt"]), "cli": ("crates/cli", ["app"]),
             "other": ("crates/other", [])}
        )
        md["packages"].append({"id": "jwt", "name": "jwt", "manifest_path": f"{ROOT}/vendor/jwt/Cargo.toml"})
        md["resolve"]["nodes"].append({"id": "jwt", "deps": []})
        self.assertEqual(affected_packages(md, ["vendor/jwt/Cargo.toml"]), ["app", "auth", "cli"])
        self.assertEqual(affected_packages(md, ["vendor/jwt/src/lib.rs"]), ["app", "auth", "cli"])
        # Reached only through a registry crate.
        md["resolve"]["nodes"][-1:] = [{"id": "jwt", "deps": []}, {"id": "oauth", "deps": [{"name": "jwt", "pkg": "jwt"}]}]
        md["resolve"]["nodes"][3]["deps"] = [{"name": "oauth", "pkg": "oauth"}]
        md["packages"].append({"id": "oauth", "name": "oauth", "manifest_path": "/registry/oauth/Cargo.toml"})
        self.assertEqual(affected_packages(md, ["vendor/jwt/Cargo.toml"]), ["app", "auth", "cli", "other"])

    def test_dependency_cycles_terminate(self) -> None:
        # A dev-dependency back-edge makes the resolve graph cyclic.
        md = metadata({"a": ("crates/a", ["b"]), "b": ("crates/b", ["a"])})
        self.assertEqual(affected_packages(md, ["crates/a/src/lib.rs"]), ["a", "b"])

    def test_result_is_sorted_and_deduplicated(self) -> None:
        md = metadata({"z": ("crates/z", []), "m": ("crates/m", ["z"]), "a": ("crates/a", ["z"])})
        changed = ["crates/z/src/lib.rs", "crates/z/src/other.rs", "crates/a/src/lib.rs"]
        self.assertEqual(affected_packages(md, changed), ["a", "m", "z"])

    def test_workspace_member_missing_from_resolve_graph_is_tolerated(self) -> None:
        md = metadata({"a": ("crates/a", []), "b": ("crates/b", ["a"])})
        md["resolve"]["nodes"] = [n for n in md["resolve"]["nodes"] if n["id"] != "b"]
        self.assertEqual(affected_packages(md, ["crates/a/src/lib.rs"]), ["a"])


if __name__ == "__main__":
    unittest.main()


class RealWorkspaceEdgeTests(unittest.TestCase):
    """Edges that exist only so CI re-runs a crate's tests, pinned to the real manifests."""

    REPO = Path(__file__).resolve().parents[2]

    def test_a_harness_only_change_selects_rsi_cli(self) -> None:
        # rsi-cli never calls rsi-harness: its end-to-end tests compile the
        # harness source with rustc. The dev-dependency is the only thing that
        # makes the planner re-run them when the harness changes, and
        # cargo-shear would remove it (see rsi-cli's cargo-shear ignore).
        cli_dir = "crates/apps/rusty_rsi/crates/rsi-cli"
        manifest = tomllib.loads((self.REPO / cli_dir / "Cargo.toml").read_text(encoding="utf-8"))
        edge = manifest.get("dev-dependencies", {}).get("rsi-harness")
        self.assertIsNotNone(edge, "rsi-cli must keep its rsi-harness dev-dependency")
        self.assertIn("rsi-harness", manifest["package"]["metadata"]["cargo-shear"]["ignored"])
        harness_dir = (self.REPO / cli_dir / edge["path"]).resolve().relative_to(self.REPO).as_posix()
        md = metadata({"rsi-harness": (harness_dir, []), "rsi-cli": (cli_dir, ["rsi-harness"])})
        self.assertEqual(
            affected_packages(md, [f"{harness_dir}/src/lib.rs"]),
            ["rsi-cli", "rsi-harness"],
        )
