"""Regression checks for event planning, coverage and component scheduling."""

from pathlib import Path
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import textwrap
import tomllib
import unittest

from ci_plan import changed_paths_from_git, event_plan, is_workspace_wide_change, specialized_job_flags


WORKFLOW = Path(__file__).parents[1] / "workflows" / "ci.yml"
BASELINE_WORKFLOW = Path(__file__).parents[1] / "workflows" / "baseline.yml"
REPO = Path(__file__).parents[2]
NEXTEST_CONFIG = Path(__file__).parents[2] / ".config" / "nextest.toml"
PLANNER = Path(__file__).with_name("ci_plan.py")

PLAN_KEYS = {
    "full",
    "packages",
    "ci_only",
    "dashboard",
    "term_web",
    "key_desktop",
    "tick",
    "fair_play",
    "agui",
    "remind_me",
    "remind_me_legacy_import",
    "win32",
    "multimodal_db",
    "rusty_config_no_std",
    "tls_engine",
    "rleval_viewer",
    "rleval_app",
    "crypto_ct",
    "rand_platforms",
    "shards",
    "components",
}
SPECIALIZED_KEYS = PLAN_KEYS - {"full", "packages", "ci_only", "shards", "components"}


class CiWorkflowSchedulingTests(unittest.TestCase):
    def setUp(self) -> None:
        self.workflow = WORKFLOW.read_text(encoding="utf-8")

    def test_push_pr_and_manual_full_sweep_events_are_declared(self) -> None:
        self.assertIn("  push:\n    branches: [main]", self.workflow)
        self.assertIn("  pull_request:", self.workflow)
        self.assertIn("  workflow_dispatch:", self.workflow)
        self.assertIn('--event "${{ github.event_name }}"', self.workflow)
        self.assertIn('--push-before "${{ github.event.before }}"', self.workflow)
        self.assertIn('changed="$(python3 .github/scripts/ci_plan.py --changed-from "$base")"', self.workflow)
        self.assertIn('Event has no usable scoped base', self.workflow)
        self.assertIn('python3 .github/scripts/ci_plan.py --emit-full', self.workflow)

    def test_rusty_tls_engine_job_cannot_pass_vacuously(self) -> None:
        # The native engine is compiled only under a cfg that no other job sets,
        # so this job is the only thing standing between a regression in it and
        # a green CI. These pin the properties that keep it honest.
        start = self.workflow.index("  rusty-tls-engine:\n")
        end = self.workflow.find("\n  # ----", start)
        job = self.workflow[start : end if end != -1 else None]
        self.assertIn("needs: plan", job)
        self.assertIn("if: needs.plan.outputs.tls_engine == 'true'", job)
        # Both flags: rustdoc does not inherit RUSTFLAGS, so a missing
        # RUSTDOCFLAGS silently skips the module's doctests.
        self.assertIn("RUSTFLAGS: --cfg rusty_tls_handrolled", job)
        self.assertIn("RUSTDOCFLAGS: --cfg rusty_tls_handrolled\n", job)
        self.assertIn("--features handrolled-engine -- -D warnings", job)
        # The OpenSSL leg asserts a non-zero count rather than trusting an exit
        # code, because `--ignored` with nothing ignored runs zero tests.
        self.assertIn("openssl version", job)
        # All four OpenSSL suites run: the TLS 1.3 server, the TLS 1.2 client,
        # the TLS 1.2 server and version negotiation.
        self.assertIn("handrolled_socket_interop", job)
        self.assertIn("handrolled_client12_socket_interop", job)
        self.assertIn("handrolled_server12_socket_interop", job)
        self.assertIn("handrolled_negotiate_socket_interop", job)
        self.assertIn("[1-9][0-9]* passed", job)
        # Suites are discovered from the directory so a new one is guarded
        # automatically, and each must list at least one test.
        self.assertIn("tests/handrolled_*.rs", job)
        self.assertIn("compiled to zero tests", job)
        # BoGo: the run script fails on any FAIL, and the floor catches a shim
        # that skips every test and so "passes".
        self.assertIn("bogo/run.sh", job)
        self.assertIn("-lt 830", job)

    def test_the_fuzz_smoke_job_is_gated_and_cannot_pass_vacuously(self) -> None:
        # Evidence bar item 3: fuzz smoke on every PR that touches the engine.
        start = self.workflow.index("  rusty-tls-fuzz:\n")
        end = self.workflow.find("\n  # ----", start)
        job = self.workflow[start : end if end != -1 else None]
        self.assertIn("if: needs.plan.outputs.tls_engine == 'true'", job)
        # The cfg that compiles the engine in, and a dated (not floating) nightly.
        self.assertIn("RUSTFLAGS: --cfg rusty_tls_handrolled", job)
        self.assertRegex(job, r"toolchain: nightly-\d{4}-\d{2}-\d{2}\n")
        # Through the tested script, with a floor on the target count.
        self.assertRegex(job, r"bash \.github/scripts/tls_fuzz_smoke\.sh \d+ [1-9]\d*")
        # Blocking: no continue-on-error on a finding.
        self.assertNotIn("continue-on-error", job)
        self.assertIn("      - rusty-tls-fuzz\n", self.workflow[self.workflow.index("required-gate:") :])

    def test_the_live_internet_step_can_inform_but_never_gate(self) -> None:
        # A third party's server must not decide whether a PR merges, so the
        # live step is continue-on-error. Its output and exit logic live in
        # live_internet_check.sh (tested offline, including a failing command),
        # so the step must go through that script and not inline its own.
        start = self.workflow.index("  rusty-tls-engine:\n")
        end = self.workflow.find("\n  # ----", start)
        job = self.workflow[start : end if end != -1 else None]
        marker = "      - name: live internet (non-blocking)"
        step = job[job.index(marker) :]
        self.assertIn("continue-on-error: true", step)
        self.assertIn("bash .github/scripts/live_internet_check.sh", step)
        self.assertIn("--test handrolled_live -- --ignored", step)
        # No inline `out=$(cargo ...)`: under `set -e` that loses the output.
        self.assertNotIn("out=$(", step)
        # Last in the job, so nothing after it can be skipped by its failure.
        self.assertNotIn("\n      - name:", step[len(marker) :])

    def test_every_handrolled_suite_is_gated_on_the_cfg_the_job_sets(self) -> None:
        # The guard above only works if each suite really is cfg-gated: an
        # ungated file would run (and count) without the cfg, hiding a typo in
        # the job's flag.
        suites = sorted((WORKFLOW.parents[2] / "crates/libs/net/rusty_tls/tests").glob("handrolled_*.rs"))
        self.assertGreaterEqual(len(suites), 10)
        for suite in suites:
            with self.subTest(suite=suite.name):
                head = suite.read_text(encoding="utf-8")[:6000]
                self.assertIn("rusty_tls_handrolled", head)

    def test_app_jobs_are_planned_not_unconditional(self) -> None:
        self.assertIn("fair_play: ${{ steps.plan.outputs.fair_play }}", self.workflow)
        self.assertIn("tick: ${{ steps.plan.outputs.tick }}", self.workflow)
        self.assertIn("if: needs.plan.outputs.fair_play == 'true'", self.workflow)
        self.assertIn("if: needs.plan.outputs.tick == 'true'", self.workflow)
        self.assertIn("ci_plan.py --packages", self.workflow)
        self.assertIn("win32: ${{ steps.plan.outputs.win32 }}", self.workflow)
        self.assertIn("multimodal_db: ${{ steps.plan.outputs.multimodal_db }}", self.workflow)
        self.assertIn("fetch-depth: 0", self.workflow)

    def test_full_event_entrypoint_emits_every_plan_output(self) -> None:
        outputs = self._planner_map("--emit-full")
        self.assertEqual(set(outputs), PLAN_KEYS)
        self.assertEqual(outputs["full"], "true")
        self.assertEqual(outputs["ci_only"], "false")
        self.assertEqual(outputs["packages"], "")
        self.assertEqual(outputs["shards"], "[1,2,3]")
        self.assertEqual(json.loads(outputs["components"]), [{"component": "workspace", "packages": ""}])
        self.assertTrue(all(outputs[key] == "true" for key in SPECIALIZED_KEYS))

    def test_event_entrypoint_handles_manual_and_unusable_bases(self) -> None:
        for args in (
            ("--event", "workflow_dispatch"),
            ("--event", "push", "--push-before", "0" * 40),
            ("--event", "push"),
            ("--event", "pull_request"),
        ):
            with self.subTest(args=args):
                self.assertEqual(self._planner_map(*args)["mode"], "full")
        scoped = self._planner_map(
            "--event", "push", "--push-before", "1" * 40
        )
        self.assertEqual(scoped, {"mode": "scoped", "base": "1" * 40})

    def test_global_fallback_entrypoint_covers_lock_and_toolchain_inputs(self) -> None:
        for path in (
            "rust-toolchain.toml",
            ".cargo/config.toml",
            ".config/other-build-policy.toml",
        ):
            with self.subTest(path=path):
                self.assertEqual(self._planner("--requires-full", input=f"{path}\n").returncode, 0)
        self.assertEqual(
            self._planner("--requires-full", input="crates/apps/rusty_tick/src/lib.rs\n").returncode,
            1,
        )

    def test_ci_only_entrypoint_uses_smoke_but_mixed_changes_do_not(self) -> None:
        ci_only = self._planner(
            "--is-ci-only", input=".github/workflows/ci.yml\n.config/nextest.toml\n"
        )
        self.assertEqual(ci_only.returncode, 0)
        self.assertNotEqual(
            self._planner(
                "--is-ci-only",
                input=".github/workflows/ci.yml\ncrates/apps/rusty_tick/src/lib.rs\n",
            ).returncode,
            0,
        )
        smoke = self._planner_map("--emit-ci-smoke")
        self.assertEqual(set(smoke), PLAN_KEYS)
        self.assertEqual(smoke["ci_only"], "true")
        self.assertEqual(smoke["full"], "false")
        self.assertEqual(smoke["packages"], "")
        self.assertEqual(json.loads(smoke["components"]), [])
        self.assertTrue(all(smoke[key] == "false" for key in SPECIALIZED_KEYS))
        self.assertIn("CI-only smoke (${{ matrix.os }})", self.workflow)
        self.assertIn('checksums.txt "$base/actionlint_${version}_checksums.txt"', self.workflow)
        self.assertIn("sha256sum --check --status", self.workflow)

    def test_lockfile_is_not_an_unconditional_global_fallback(self) -> None:
        self.assertEqual(self._planner("--requires-full", input="Cargo.lock\n").returncode, 1)
        self.assertIn("lockfile_diff.py", self.workflow)
        self.assertIn("Cargo.lock changed with a manifest", self.workflow)
        self.assertIn("Unable to establish Cargo.lock impact", self.workflow)

    def test_unrelated_base_falls_back_after_fetch_verification(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            repo = Path(directory)
            self._git(repo, "init", "-q")
            self._git(repo, "config", "user.email", "ci@example.invalid")
            self._git(repo, "config", "user.name", "CI test")
            self._write(repo, "main.txt")
            self._git(repo, "add", ".")
            self._git(repo, "commit", "-qm", "main")
            primary_branch = self._git(repo, "branch", "--show-current").strip()
            self._git(repo, "checkout", "--orphan", "unrelated", "-q")
            self._git(repo, "rm", "-rf", ".")
            self._write(repo, "other.txt")
            self._git(repo, "add", ".")
            self._git(repo, "commit", "-qm", "unrelated")
            unrelated = self._git(repo, "rev-parse", "HEAD").strip()
            self._git(repo, "checkout", primary_branch, "-q")
            result = subprocess.run(
                [sys.executable, str(PLANNER), "--verify-base", unrelated],
                cwd=repo,
                capture_output=True,
                text=True,
                check=False,
            )
        self.assertEqual(result.returncode, 1)

    def test_specialized_entrypoint_emits_complete_key_set(self) -> None:
        outputs = self._planner_map(
            "--packages", "rk-app rusty_term rusty-meshed-core rusty_tick"
        )
        self.assertEqual(set(outputs), SPECIALIZED_KEYS)
        self.assertEqual(outputs["key_desktop"], "true")
        self.assertEqual(outputs["term_web"], "true")
        self.assertEqual(outputs["dashboard"], "true")
        self.assertEqual(outputs["tick"], "true")
        self.assertEqual(outputs["fair_play"], "false")

    def test_shallow_clone_becomes_narrowable_after_history_is_complete(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "source"
            origin = root / "origin.git"
            clone = root / "clone"
            source.mkdir()
            self._git(source, "init", "-q")
            self._git(source, "config", "user.email", "ci@example.invalid")
            self._git(source, "config", "user.name", "CI test")
            self._write(source, "crates/apps/rusty_tick/src/lib.rs")
            self._git(source, "add", ".")
            self._git(source, "commit", "-qm", "base")
            base = self._git(source, "rev-parse", "HEAD").strip()
            self._write(source, "crates/apps/rusty_tick/src/next.rs")
            self._git(source, "add", ".")
            self._git(source, "commit", "-qm", "next")
            self._git(source, "clone", "--bare", ".", str(origin))
            subprocess.run(
                ["git", "clone", "--depth", "1", origin.as_uri(), str(clone)],
                check=True,
                capture_output=True,
                text=True,
            )
            self.assertFalse(self._planner_in(clone, "--verify-base", base).returncode == 0)
            self._git(clone, "fetch", "--unshallow", "origin")
            self.assertEqual(self._planner_in(clone, "--verify-base", base).returncode, 0)
            flags = specialized_job_flags(changed_paths_from_git(base, cwd=clone), [])
        self.assertTrue(flags["tick"])

    def test_fair_play_only_change_does_not_select_an_unrelated_app(self) -> None:
        self.assertEqual(
            specialized_job_flags(["crates/apps/rusty_fair_play/web/src/App.tsx"], []),
            {"dashboard": False, "term_web": False, "key_desktop": False, "tick": False, "fair_play": True, "agui": False, "win32": False, "multimodal_db": False, "rusty_config_no_std": False, "tls_engine": False, "rleval_viewer": False, "rleval_app": False, "crypto_ct": False, "rand_platforms": False, "remind_me": False, "remind_me_legacy_import": False},
        )

    def test_rleval_jobs_follow_cargo_impact_and_the_viewer_fixture(self) -> None:
        # A change to a crate the viewer and the app both build on reaches both
        # (affected_crates.py expands it to its reverse dependents first).
        flags = specialized_job_flags([], ["replay-analyzer", "replay-viewer", "rleval-app"])
        self.assertTrue(flags["rleval_viewer"])
        self.assertTrue(flags["rleval_app"])
        # The app alone does not need the headless-GL smoke test...
        flags = specialized_job_flags([], ["rleval-app"])
        self.assertTrue(flags["rleval_app"])
        self.assertFalse(flags["rleval_viewer"])
        # ...and the fixture the smoke test renders is a path-only trigger.
        flags = specialized_job_flags(
            ["crates/apps/rocket_league/rleval/assets/replays/42f2.replay"], []
        )
        self.assertTrue(flags["rleval_viewer"])
        self.assertFalse(flags["rleval_app"])
        # An unrelated change selects neither.
        flags = specialized_job_flags(["crates/apps/rusty_tick/src/lib.rs"], ["rusty_tick"])
        self.assertFalse(flags["rleval_viewer"])
        self.assertFalse(flags["rleval_app"])

    def test_rleval_jobs_are_planned_and_gated(self) -> None:
        for key in ("rleval_viewer", "rleval_app"):
            self.assertIn(f"{key}: ${{{{ steps.plan.outputs.{key} }}}}", self.workflow)
            self.assertIn(f"if: needs.plan.outputs.{key} == 'true'", self.workflow)

    def test_fair_play_reverse_dependencies_select_its_specialized_jobs(self) -> None:
        # affected_crates.py expands a changed domain/shared crate to these
        # application packages before this mapper sees it.
        flags = specialized_job_flags([], ["rusty_fair_play", "rusty_tick"])
        self.assertTrue(flags["fair_play"])
        self.assertTrue(flags["tick"])

    def test_deleted_or_renamed_app_paths_remain_conservative(self) -> None:
        # `git diff --name-only` reports a deletion, and may report both sides
        # of a rename; either Fair Play path must retain its app checks.
        flags = specialized_job_flags(
            [
                "crates/apps/rusty_fair_play/web/src/removed.ts",
                "crates/apps/rusty_tick/web/src/new.ts",
            ],
            [],
        )
        self.assertTrue(flags["fair_play"])
        self.assertTrue(flags["tick"])

    def test_multi_commit_push_and_deletion_use_real_git_history(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            repo = Path(directory)
            self._git(repo, "init", "-q")
            self._git(repo, "config", "user.email", "ci@example.invalid")
            self._git(repo, "config", "user.name", "CI test")
            self._write(repo, "README.md")
            self._write(repo, "crates/apps/rusty_fair_play/web/src/removed.ts")
            self._git(repo, "add", ".")
            self._git(repo, "commit", "-qm", "base")
            base = self._git(repo, "rev-parse", "HEAD").strip()
            self._write(repo, "crates/apps/rusty_tick/web/src/new.ts")
            self._git(repo, "add", ".")
            self._git(repo, "commit", "-qm", "add tick path")
            self._git(repo, "rm", "-q", "crates/apps/rusty_fair_play/web/src/removed.ts")
            self._git(repo, "commit", "-qm", "delete fair play path")
            paths = changed_paths_from_git(base, cwd=repo)
        flags = specialized_job_flags(paths, [])
        self.assertTrue(flags["fair_play"])
        self.assertTrue(flags["tick"])

    def test_rename_preserves_both_app_sides_in_real_git_history(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            repo = Path(directory)
            self._git(repo, "init", "-q")
            self._git(repo, "config", "user.email", "ci@example.invalid")
            self._git(repo, "config", "user.name", "CI test")
            self._write(repo, "crates/apps/rusty_fair_play/web/src/moved.ts")
            self._git(repo, "add", ".")
            self._git(repo, "commit", "-qm", "base")
            base = self._git(repo, "rev-parse", "HEAD").strip()
            target = "crates/apps/rusty_tick/web/src/moved.ts"
            (repo / target).parent.mkdir(parents=True, exist_ok=True)
            self._git(repo, "mv", "crates/apps/rusty_fair_play/web/src/moved.ts", target)
            self._git(repo, "commit", "-qm", "move app path")
            paths = changed_paths_from_git(base, cwd=repo)
        self.assertIn("crates/apps/rusty_fair_play/web/src/moved.ts", paths)
        self.assertIn(target, paths)

    def test_global_workflow_change_requires_full_sweep(self) -> None:
        self.assertFalse(is_workspace_wide_change([".github/workflows/ci.yml"]))
        self.assertFalse(is_workspace_wide_change([".config/nextest.toml"]))
        self.assertTrue(is_workspace_wide_change([".config/other-build-policy.toml"]))
        self.assertTrue(is_workspace_wide_change(["rust-toolchain.toml"]))
        self.assertFalse(is_workspace_wide_change(["crates/apps/rusty_tick/src/lib.rs"]))

    def test_multiple_apps_and_unrelated_app_selection(self) -> None:
        self.assertEqual(
            specialized_job_flags(
                [
                    "crates/apps/rusty_fair_play/src/lib.rs",
                    "crates/apps/rusty_tick/web/src/main.ts",
                ],
                [],
            ),
            {"dashboard": False, "term_web": False, "key_desktop": False, "tick": True, "fair_play": True, "agui": False, "win32": False, "multimodal_db": False, "rusty_config_no_std": False, "tls_engine": False, "rleval_viewer": False, "rleval_app": False, "crypto_ct": False, "rand_platforms": False, "remind_me": False, "remind_me_legacy_import": False},
        )
        self.assertEqual(
            specialized_job_flags(["crates/apps/rusty_tick/src/lib.rs"], ["rusty_tick"]),
            {"dashboard": False, "term_web": False, "key_desktop": False, "tick": True, "fair_play": False, "agui": False, "win32": False, "multimodal_db": False, "rusty_config_no_std": False, "tls_engine": False, "rleval_viewer": False, "rleval_app": False, "crypto_ct": False, "rand_platforms": False, "remind_me": False, "remind_me_legacy_import": False},
        )

    def test_agui_package_change_selects_tick_web_too(self) -> None:
        flags = specialized_job_flags(["crates/libs/protocol/rusty_agui/packages/agui-react/src/hooks.tsx"], [])
        self.assertTrue(flags["agui"])
        self.assertTrue(flags["tick"])

    def test_shared_and_specialized_package_rules(self) -> None:
        flags = specialized_job_flags(
            [], ["rusty_win32", "rusty_multimodal_db", "rusty_config", "rusty_tls", "remind_me_hub"]
        )
        self.assertTrue(flags["win32"])
        self.assertTrue(flags["multimodal_db"])
        self.assertTrue(flags["rusty_config_no_std"])
        self.assertTrue(flags["tls_engine"])
        self.assertTrue(flags["remind_me"])

    def test_legacy_import_routes_sources_contracts_and_dependencies(self) -> None:
        hub = "crates/apps/rusty_remind_me/crates/remind_me_hub/"
        for path in ("src/import/postgres.rs", "src/record.rs", "src/store/multimodal/rows.rs",
                     "tests/fixtures/postgres_hub.sql", "tests/suite/recorded.rs",
                     "Cargo.toml", "Containerfile", "setup.sh"):
            with self.subTest(path=path):
                self.assertTrue(specialized_job_flags([hub + path], [])["remind_me_legacy_import"])
        self.assertTrue(specialized_job_flags(["Cargo.lock"], [])["remind_me_legacy_import"])
        self.assertTrue(specialized_job_flags([], ["remind_me_hub"])["remind_me_legacy_import"])
        flags = specialized_job_flags(
            ["crates/apps/rusty_remind_me/crates/remind_me_core/src/lib.rs"], ["remind_me_core"]
        )
        self.assertTrue(flags["remind_me"])
        self.assertFalse(flags["remind_me_legacy_import"])

    def test_node_wire_contract_selects_import_even_without_hub_cargo_impact(self) -> None:
        core = "crates/apps/rusty_remind_me/crates/remind_me_core/src/"
        for path in ("sync/record.rs", "sync/push.rs", "db/engine/rows.rs", "models.rs"):
            with self.subTest(path=path):
                flags = specialized_job_flags([core + path], ["remind_me_core"])
                self.assertTrue(flags["remind_me_legacy_import"])

    def test_engine_checks_do_not_need_a_postgres_service(self) -> None:
        engine = self.workflow.split("  remind-me-hub:\n")[1].split("  remind-me-legacy-import:\n")[0]
        self.assertNotIn("services:", engine)
        self.assertIn("--no-default-features", engine)
        legacy = self.workflow.split("  remind-me-legacy-import:\n")[1].split("  remind-me-windows:\n")[0]
        self.assertIn("if: needs.plan.outputs.remind_me_legacy_import == 'true'", legacy)
        self.assertIn('REMIND_ME_HUB_REQUIRE_POSTGRES: "1"', legacy)
        self.assertIn("--features postgres-import --test hub_postgres_copy_test", legacy)

    def test_pr_runs_cancel_but_main_and_manual_runs_are_isolated(self) -> None:
        self.assertIn(
            "group: ci-${{ github.event_name == 'pull_request' && format('pr-{0}', github.event.pull_request.number) || format('run-{0}', github.run_id) }}",
            self.workflow,
        )
        self.assertIn(
            "cancel-in-progress: ${{ github.event_name == 'pull_request' }}",
            self.workflow,
        )

    def test_all_scoped_jobs_retain_pending_main_work(self) -> None:
        jobs = re.split(r"^  ([a-z][a-z0-9-]+):\n", self.workflow.split("\njobs:\n", 1)[1], flags=re.M)
        groups = []
        for job, body in zip(jobs[1::2], jobs[2::2]):
            if job in {"fmt", "plan-tests", "workflow-lint", "dependency-policy", "cargo-deny", "cargo-shear", "plan", "required-gate"}:
                self.assertNotIn("    concurrency:", body)
                continue  # These start for every SHA, without a coalescing lock.
            with self.subTest(job=job):
                self.assertIn("      queue: max\n      cancel-in-progress: false", body)
                group = re.search(r"^      group: (.+)$", body, re.M).group(1)
                self.assertIn("github.ref == 'refs/heads/main'", group)
                self.assertIn("format('run-{0}', github.run_id)", group)
                self.assertIn(f"-{job}", group)
                groups.append(group)
        self.assertEqual(len(groups), len(set(groups)))

    def test_required_gate_needs_every_other_job(self) -> None:
        body = self.workflow.split("\njobs:\n", 1)[1]
        jobs = re.findall(r"^  ([a-z][a-z0-9-]+):\n", body, flags=re.M)
        gate = body.split("  required-gate:\n", 1)[1]
        needs = set(re.findall(r"^      - ([a-z][a-z0-9-]+)$", gate.split("runs-on:")[0], flags=re.M))
        self.assertEqual(needs, set(jobs) - {"required-gate"})
        self.assertIn("if: always()", gate)
        self.assertIn("*\" failure \"*|*\" cancelled \"*) exit 1", gate)

    def test_merge_group_entries_are_scoped_against_their_base(self) -> None:
        self.assertEqual(event_plan("merge_group", merge_base="abc123"), ("scoped", "abc123"))
        # No usable base: fall back to the full sweep rather than guess.
        self.assertEqual(event_plan("merge_group"), ("full", ""))
        # Unchanged behaviour for the other events.
        self.assertEqual(event_plan("pull_request", pr_base="b"), ("scoped", "b"))
        self.assertEqual(event_plan("workflow_dispatch", merge_base="abc123"), ("full", ""))
        self.assertEqual(event_plan("schedule"), ("full", ""))

    def test_planner_cli_accepts_a_merge_group_base(self) -> None:
        out = subprocess.run(
            [sys.executable, str(PLANNER), "--event", "merge_group", "--merge-base", "abc123"],
            capture_output=True, text=True, check=True,
        ).stdout.split()
        self.assertEqual(out, ["mode=scoped", "base=abc123"])

    def test_ci_runs_on_the_merge_queue_and_scopes_it(self) -> None:
        # Without this trigger the queue waits forever for required-gate.
        self.assertRegex(self.workflow, r"(?m)^  merge_group:\n    types: \[checks_requested\]\n")
        self.assertIn('--merge-base "${{ github.event.merge_group.base_sha }}"', self.workflow)
        # ci.yml is the only workflow that feeds the required check.
        for path in sorted(REPO.glob(".github/workflows/*.yml")):
            if path != WORKFLOW:
                with self.subTest(workflow=path.name):
                    self.assertNotIn("merge_group", path.read_text(encoding="utf-8"))

    def test_merge_group_runs_never_share_a_concurrency_key_with_pull_requests(self) -> None:
        group = re.search(r"(?m)^concurrency:\n(?:  #.*\n)*  group: (.+)\n  cancel-in-progress: (.+)\n", self.workflow)
        self.assertIsNotNone(group)
        key, cancel = group.groups()
        # The per-PR key (and cancellation) applies to pull_request only; every
        # other event, merge_group included, gets its own run-id key.
        self.assertIn("github.event_name == 'pull_request' && format('pr-{0}'", key)
        self.assertIn("format('run-{0}', github.run_id)", key)
        self.assertEqual(cancel, "${{ github.event_name == 'pull_request' }}")
        self.assertNotIn("merge_group", key)
    def job_text(self, job: str) -> str:
        body = self.workflow.split("\njobs:\n", 1)[1]
        chunk = body.split(f"\n  {job}:\n", 1)[1]
        return re.split(r"\n  [a-z][a-z0-9-]+:\n", chunk, maxsplit=1)[0]

    def test_heavy_matrix_jobs_wait_for_the_format_check(self) -> None:
        # A formatting typo should cost ~30 s, not start the 45-minute shards.
        for job in ("clippy", "test"):
            with self.subTest(job=job):
                self.assertIn("    needs: [plan, fmt]\n", self.job_text(job))
        self.assertNotIn("    needs: fmt\n", self.workflow)

    def test_workflows_default_to_read_only_token(self) -> None:
        for path in (WORKFLOW, BASELINE_WORKFLOW):
            with self.subTest(workflow=path.name):
                self.assertRegex(path.read_text(encoding="utf-8"), r"(?m)^permissions:\n  contents: read$")

    def test_scheduled_sweep_is_full_and_msrv_ignores_toolchain_file(self) -> None:
        self.assertRegex(self.workflow, r"(?m)^  schedule:\n    - cron: ")
        self.assertEqual(self._planner_map("--event", "schedule")["mode"], "full")
        msrv = self.workflow.split("  multimodal-db-msrv:\n")[1].split("  rusty-config-no-std-check:\n")[0]
        self.assertIn('RUSTUP_TOOLCHAIN: "1.89.0"', msrv)

    def test_every_workflow_pin_matches_rust_toolchain_file(self) -> None:
        channel = tomllib.loads((REPO / "rust-toolchain.toml").read_text())["toolchain"]["channel"]
        for path in (WORKFLOW, BASELINE_WORKFLOW, WORKFLOW.with_name("remind-me-release.yml")):
            with self.subTest(workflow=path.name):
                pin = re.search(r'(?m)^  RUST_TOOLCHAIN: "([^"]+)"', path.read_text(encoding="utf-8"))
                self.assertEqual(pin and pin.group(1), channel)

    def test_every_action_is_pinned_to_a_commit_sha(self) -> None:
        for path in sorted(REPO.glob(".github/workflows/*.yml")) + sorted(REPO.glob(".github/actions/*/action.yml")):
            for number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
                match = re.match(r"\s*-?\s*uses: (\S+)", line)
                if not match or match[1].startswith("./"):
                    continue
                with self.subTest(file=path.name, line=number):
                    self.assertRegex(match[1], r"@[0-9a-f]{40}$")

    def test_cargo_deny_runs_every_event_and_blocks(self) -> None:
        deny = self.workflow.split("  cargo-deny:\n")[1].split("  cargo-shear:\n")[0]
        self.assertNotIn("    if:", deny)
        self.assertNotIn("    concurrency:", deny)
        self.assertNotIn("continue-on-error", deny)
        self.assertIn("cargo deny --workspace check", deny)
        self.assertTrue((REPO / "deny.toml").is_file())

    def test_cargo_shear_runs_every_event_and_is_non_blocking(self) -> None:
        shear = self.workflow.split("  cargo-shear:\n")[1].split("  plan:\n")[0]
        self.assertNotIn("    if:", shear)
        self.assertNotIn("    concurrency:", shear)
        self.assertIn("continue-on-error: true", shear)
        self.assertIn("run: cargo shear", shear)
        root = tomllib.loads((REPO / "Cargo.toml").read_text(encoding="utf-8"))
        self.assertTrue(root["workspace"]["metadata"]["cargo-shear"]["ignored"])

    def test_dependabot_tracks_github_actions(self) -> None:
        text = (REPO / ".github" / "dependabot.yml").read_text(encoding="utf-8")
        self.assertIn("package-ecosystem: github-actions", text)

    def test_every_pr_check_runs_through_ci_yml_and_so_through_the_gate(self) -> None:
        # A PR-triggered workflow outside ci.yml is a check `required-gate`
        # cannot see: it can fail while the sole required check stays green.
        outside = [
            path.name
            for path in sorted(WORKFLOW.parent.glob("*.yml"))
            if path != WORKFLOW and re.search(r"(?m)^  pull_request(_target)?:", path.read_text(encoding="utf-8"))
        ]
        self.assertEqual(outside, [], "PR checks outside ci.yml bypass required-gate")

    def test_plugin_version_check_is_a_gated_planner_job(self) -> None:
        job = self.workflow.split("  remind-me-plugin-version:\n")[1].split("  remind-me-hub:\n")[0]
        self.assertIn("if: needs.plan.outputs.remind_me == 'true'", job)
        self.assertIn("crates/apps/rusty_remind_me/scripts/check_plugin_version.sh", job)
        gate = self.workflow.split("  required-gate:\n")[1]
        self.assertIn("      - remind-me-plugin-version\n", gate.split("runs-on:")[0])

    def test_plugin_version_job_runs_for_product_changes_and_not_for_unrelated_ones(self) -> None:
        def flag(paths: list[str]) -> bool:
            return specialized_job_flags(paths, [])["remind_me"]

        self.assertTrue(flag(["crates/apps/rusty_remind_me/.claude-plugin/plugin.json"]))
        self.assertTrue(flag(["crates/apps/rusty_remind_me/crates/remind_me_core/Cargo.toml"]))
        self.assertFalse(flag(["crates/libs/net/rusty_http/src/lib.rs", "README.md"]))
        # Sweeps that run everything (manual, scheduled, no usable base) include it.
        self.assertEqual(self._planner_map("--emit-full")["remind_me"], "true")

    @unittest.skipUnless(shutil.which("bash") and shutil.which("jq"), "bash and jq are needed to run the check")
    def test_plugin_version_drift_fails_the_check_and_agreement_passes(self) -> None:
        product = Path(__file__).parents[2] / "crates" / "apps" / "rusty_remind_me"
        members = "remind_me_core remind_me_mcp remind_me_api remind_me_cli remind_me_remote remind_me_hub".split()

        def run(edit=None) -> subprocess.CompletedProcess[str]:
            with tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                shutil.copytree(product / "scripts", root / "scripts")
                (root / ".claude-plugin").mkdir()
                shutil.copy(product / ".claude-plugin" / "plugin.json", root / ".claude-plugin" / "plugin.json")
                for member in members:
                    (root / "crates" / member).mkdir(parents=True)
                    shutil.copy(product / "crates" / member / "Cargo.toml", root / "crates" / member / "Cargo.toml")
                if edit:
                    edit(root)
                return subprocess.run(
                    ["bash", str(root / "scripts" / "check_plugin_version.sh")],
                    capture_output=True, text=True, check=False,
                )

        self.assertEqual(run().returncode, 0, "the repo as committed must agree")

        def bump_plugin(root: Path) -> None:
            manifest = root / ".claude-plugin" / "plugin.json"
            manifest.write_text(re.sub(r'("version"\s*:\s*")[^"]+', r"\g<1>99.0.0", manifest.read_text()))

        drifted = run(bump_plugin)
        self.assertEqual(drifted.returncode, 1, drifted.stderr)
        self.assertIn("Plugin version drift", drifted.stderr)

        def bump_one_crate(root: Path) -> None:
            manifest = root / "crates" / "remind_me_hub" / "Cargo.toml"
            manifest.write_text(re.sub(r'(?m)^version = "[^"]+"', 'version = "99.0.0"', manifest.read_text(), count=1))

        self.assertEqual(run(bump_one_crate).returncode, 1, "crates disagreeing also fails")

    @unittest.skipUnless(shutil.which("bash"), "bash is needed to run the gate step")
    def test_required_gate_fails_when_any_needed_job_fails_and_passes_when_skipped(self) -> None:
        gate = self.workflow.split("  required-gate:\n")[1]
        script = textwrap.dedent(gate.split("        run: |\n", 1)[1])

        def run(results: str) -> int:
            return subprocess.run(
                ["bash", "--noprofile", "--norc", "-e", "-o", "pipefail", "-c", script],
                env=dict(os.environ, RESULTS=results), capture_output=True, text=True, check=False,
            ).returncode

        self.assertEqual(run("success skipped success"), 0, "jobs the planner skipped must not fail the gate")
        self.assertEqual(run("success failure skipped"), 1, "a failed job (e.g. plugin version drift) fails the gate")
        self.assertEqual(run("success cancelled"), 1)

    def test_generic_matrix_uses_component_scope_and_unique_artifacts(self) -> None:
        for job, end in (("clippy", "ci-smoke"), ("test", "data-mesh-monitor")):
            body = self.workflow.split(f"  {job}:\n")[1].split(f"  {end}:\n")[0]
            self.assertIn("scope: ${{ fromJson(needs.plan.outputs.components) }}", body)
            self.assertIn("packages: ${{ matrix.scope.packages }}", body)
            self.assertIn(f"-{job}-${{{{ matrix.scope.component }}}}-${{{{ matrix.os }}}}", body)
        self.assertIn("name: nextest-${{ matrix.scope.component }}-${{ matrix.os }}-shard-${{ matrix.shard }}", self.workflow)
        self.assertIn("--partition count:{0}/3", self.workflow)
        self.assertIn("--test-threads 2", self.workflow)
        self.assertIn("--no-fail-fast", self.workflow)
        self.assertIn("matrix.scope.packages, 'mill-term'", self.workflow)
        self.assertIn("matrix.scope.packages, 'rusty_lines'", self.workflow)

    def test_queue_lint_extension_preserves_the_pinned_tool(self) -> None:
        self.assertIn("version=1.7.12", self.workflow)
        self.assertIn("lint_workflows.py --actionlint ./actionlint .github/workflows/*.yml", self.workflow)
        self.assertIn("test_lint_workflows.py --actionlint ./actionlint", self.workflow)

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

    @staticmethod
    def _write(repo: Path, relative_path: str) -> None:
        target = repo / relative_path
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text("fixture\n", encoding="utf-8")

    @staticmethod
    def _git(repo: Path, *args: str) -> str:
        return subprocess.run(
            ["git", *args], cwd=repo, check=True, capture_output=True, text=True
        ).stdout

    @staticmethod
    def _planner(*args: str, input: str = "") -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [sys.executable, str(PLANNER), *args],
            input=input,
            capture_output=True,
            text=True,
            check=False,
        )

    @staticmethod
    def _planner_in(repo: Path, *args: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [sys.executable, str(PLANNER), *args],
            cwd=repo,
            capture_output=True,
            text=True,
            check=False,
        )

    @classmethod
    def _planner_map(cls, *args: str, input: str = "") -> dict[str, str]:
        result = cls._planner(*args, input=input)
        if result.returncode:
            raise AssertionError(result.stderr)
        return dict(line.split("=", 1) for line in result.stdout.splitlines())

    def test_rand_platform_job_follows_rusty_rand(self) -> None:
        self.assertTrue(specialized_job_flags([], ["rusty_rand"])["rand_platforms"])
        self.assertFalse(specialized_job_flags([], ["rusty_config", "rusty_pk"])["rand_platforms"])
        self.assertIn("rand_platforms: ${{ steps.plan.outputs.rand_platforms }}", self.workflow)
        self.assertIn("if: needs.plan.outputs.rand_platforms == 'true'", self.workflow)

    def test_rand_platforms_selected_for_mixed_ci_and_docs_changes(self) -> None:
        for path in (
            ".github/workflows/ci.yml",
            ".github/scripts/ci_plan.py",
            ".github/scripts/affected_crates.py",
            ".github/scripts/test_ci_workflow.py",
        ):
            with self.subTest(path=path):
                changed = f"{path}\ndocs/research/CRYPTO-REPLACEMENT-PLAN.md\n"
                self.assertNotEqual(self._planner("--is-ci-only", input=changed).returncode, 0)
                outputs = self._planner_map("--packages", "", input=changed)
                self.assertEqual(outputs["rand_platforms"], "true")
        self.assertEqual(self._planner_map(input="docs/README.md\n")["rand_platforms"], "false")

    def test_rand_platforms_survive_ci_only_shortcut(self) -> None:
        for path in (
            ".github/workflows/ci.yml",
            ".github/scripts/ci_plan.py",
            ".github/scripts/affected_crates.py",
            ".github/scripts/test_ci_workflow.py",
            ".github/workflows/unrelated.yml",
        ):
            with self.subTest(path=path):
                changed = f"{path}\n.config/nextest.toml\n"
                self.assertEqual(self._planner("--is-ci-only", input=changed).returncode, 0)
                outputs = self._planner_map("--emit-ci-smoke", input=changed)
                self.assertEqual(set(outputs), PLAN_KEYS)
                self.assertEqual(outputs["ci_only"], "true")
                self.assertEqual(outputs["full"], "false")
                self.assertEqual(outputs["rand_platforms"], "false" if "unrelated" in path else "true")
                self.assertTrue(all(outputs[key] == "false" for key in SPECIALIZED_KEYS - {"rand_platforms"}))
        self.assertIn(
            "printf '%s\\n' \"$changed\" | python3 .github/scripts/ci_plan.py --emit-ci-smoke >> \"$GITHUB_OUTPUT\"",
            self.workflow,
        )

    def test_rand_platform_commands_and_required_gate_are_preserved(self) -> None:
        job = self.workflow.split("  rusty-rand-platforms:\n", 1)[1].split("\n  # Every other Rust job", 1)[0]
        self.assertIn("os: [ubuntu-latest, macos-latest]", job)
        self.assertIn("run: cargo test -p rusty_rand", job)
        for target in ("aarch64-unknown-linux-gnu", "riscv64gc-unknown-linux-gnu"):
            self.assertIn(f"run: cargo check -p rusty_rand --target {target} --all-targets", job)
        gate = self.workflow.split("  required-gate:\n", 1)[1]
        self.assertIn("      - rusty-rand-platforms\n", gate)

    def test_crypto_constant_time_job_follows_its_crates(self) -> None:
        for package in ("rusty_pk", "rusty_sha2", "rusty_aead", "rusty_ct_check", "rusty_crypto_key"):
            with self.subTest(package=package):
                self.assertTrue(specialized_job_flags([], [package])["crypto_ct"])
        self.assertFalse(specialized_job_flags([], ["rusty_config", "rusty_tls"])["crypto_ct"])
        self.assertIn("crypto_ct: ${{ steps.plan.outputs.crypto_ct }}", self.workflow)
        self.assertIn("if: needs.plan.outputs.crypto_ct == 'true'", self.workflow)
        for script in (
            "crates/foundation/rusty_ct_check/scripts/valgrind_selftest.sh",
            "crates/foundation/rusty_sha2/scripts/ct_check.sh",
            "crates/foundation/rusty_aead/scripts/ct_check.sh",
            "crates/foundation/rusty_pk/scripts/ct_check.sh",
        ):
            with self.subTest(script=script):
                self.assertIn(f"run: sh {script}", self.workflow)
                self.assertTrue((WORKFLOW.parents[2] / script).is_file())


if __name__ == "__main__":
    unittest.main()
