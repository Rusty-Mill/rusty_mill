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
    "win32",
    "multimodal_db",
    "rusty_config_no_std",
    "term_bench",
    "tls_engine",
    "rleval_viewer",
    "rleval_app",
    "rleval_service",
    "rleval_image",
    "crypto_ct",
    "rand_platforms",
    "msrv_manifests",
    "shards",
    "components",
}
SPECIALIZED_KEYS = PLAN_KEYS - {"full", "packages", "ci_only", "msrv_manifests", "shards", "components"}


class CiWorkflowSchedulingTests(unittest.TestCase):
    def setUp(self) -> None:
        self.workflow = WORKFLOW.read_text(encoding="utf-8")

    def test_win32_cross_check_needs_no_native_compiler_and_stays_gated(self) -> None:
        start = self.workflow.index("  win32-cross-compile-check:\n")
        end = self.workflow.index("\n  rusty-rand-platforms:", start)
        job = self.workflow[start:end]
        self.assertIn("needs: plan", job)
        self.assertIn("if: needs.plan.outputs.win32 == 'true' && needs.plan.outputs.linux == 'true'", job)
        self.assertIn("runs-on: ubuntu-latest", job)
        self.assertIn("targets: x86_64-pc-windows-gnu", job)
        self.assertIn("components: clippy", job)
        self.assertIn("run: cargo check -p rusty_win32 --target x86_64-pc-windows-gnu --all-targets --features std\n", job)
        self.assertIn("run: cargo clippy -p rusty_win32 --target x86_64-pc-windows-gnu --all-targets --features std -- -D warnings\n", job)
        self.assertNotIn("apt-get", job)
        self.assertNotIn("continue-on-error", job)
        gate = self.workflow[self.workflow.index("  required-gate:\n"):]
        self.assertIn("      - win32-cross-compile-check\n", gate)

    def test_win32_check_runs_for_ci_edits_and_affected_package_only(self) -> None:
        for path in (".github/workflows/ci.yml", ".github/scripts/ci_plan.py",
                     ".github/scripts/test_ci_workflow.py"):
            with self.subTest(path=path):
                self.assertTrue(specialized_job_flags([path], [])["win32"])
                smoke = self._planner_map("--emit-ci-smoke", input=path + "\n")
                self.assertEqual(smoke["win32"], "true")
        self.assertTrue(specialized_job_flags([], ["rusty_win32"])["win32"])
        self.assertFalse(specialized_job_flags([], ["rusty_tick"])["win32"])
        self.assertFalse(specialized_job_flags(["changes/2026-10-27-other.md"], [])["win32"])

    def test_build_environment_setup_has_a_blocking_step_timeout(self) -> None:
        steps = re.findall(
            r"(?m)^      - uses: \./\.github/actions/setup-build-env\n"
            r"(?:(?!      - |  \S)[^\n]*\n)*",
            self.workflow,
        )
        self.assertEqual(len(steps), 4)
        for step in steps:
            with self.subTest(step=step):
                self.assertIn("\n        timeout-minutes: 15\n", step)
                self.assertNotIn("continue-on-error", step)

    def test_push_pr_and_manual_full_sweep_events_are_declared(self) -> None:
        # No post-merge push run: the merge queue tested the tree that lands.
        self.assertNotIn("  push:\n    branches: [main]", self.workflow)
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
        self.assertRegex(job, r"FUZZ_TOOLCHAIN: nightly-\d{4}-\d{2}-\d{2}\n")
        self.assertIn("toolchain: ${{ env.FUZZ_TOOLCHAIN }}", job)  # installed...
        # ...and the script selects that same toolchain, not a floating `nightly`.
        self.assertNotIn("+nightly\n", job)
        # Through the tested script, with a floor on the target count.
        self.assertRegex(job, r"bash \.github/scripts/tls_fuzz_smoke\.sh \d+ [1-9]\d*")
        # The prebuilt cargo-fuzz is musl and defaults to building for musl;
        # the runner has no musl std, so the host triple must be explicit.
        self.assertIn("FUZZ_TARGET_TRIPLE: x86_64-unknown-linux-gnu", job)
        # Blocking: no continue-on-error on a finding.
        self.assertNotIn("continue-on-error", job)
        self.assertIn("      - rusty-tls-fuzz\n", self.workflow[self.workflow.index("required-gate:") :])

    def test_the_conhost_fuzz_job_runs_only_on_sweeps_and_cannot_pass_vacuously(self) -> None:
        # Design PR 13 (D2): scheduled and manual runs only, never a PR or the queue.
        start = self.workflow.index("  rusty-conhost-fuzz:\n")
        end = self.workflow.find("\n  # ----", start)
        job = self.workflow[start : end if end != -1 else None]
        self.assertIn(
            "if: github.event_name == 'schedule' || github.event_name == 'workflow_dispatch'\n", job
        )
        self.assertRegex(job, r"FUZZ_TOOLCHAIN: nightly-\d{4}-\d{2}-\d{2}\n")
        self.assertIn("toolchain: ${{ env.FUZZ_TOOLCHAIN }}", job)
        self.assertIn("tool: cargo-fuzz@0.13.2", job)
        # Both targets, through the tested script.
        self.assertIn("FUZZ_ROOT: crates/libs/ui/rusty_conhost", job)
        self.assertRegex(job, r"bash \.github/scripts/tls_fuzz_smoke\.sh 30 2\n")
        self.assertIn("FUZZ_TARGET_TRIPLE: x86_64-unknown-linux-gnu", job)
        self.assertNotIn("continue-on-error", job)
        self.assertIn("      - rusty-conhost-fuzz\n", self.workflow[self.workflow.index("required-gate:") :])

    # Both engine jobs, `ring` and native (ADR-0002 Amendment 2), carry the same
    # limbo and live steps; each is checked with the same rules.
    ENGINE_JOBS = (("rusty-tls-engine", "ring"), ("rusty-tls-engine-native", "native"))

    def engine_job(self, name: str) -> str:
        start = self.workflow.index(f"  {name}:\n")
        end = self.workflow.find("\n  # ", start + 1)
        return self.workflow[start : end if end != -1 else None]

    def test_the_limbo_differential_step_runs_through_its_script_before_the_live_step(self) -> None:
        for name, _ in self.ENGINE_JOBS:
            with self.subTest(job=name):
                job = self.engine_job(name)
                marker = "      - name: x509-limbo certificate differential"
                step = job[job.index(marker) :]
                step = step[: step.index("\n      - name:", len(marker))]
                self.assertIn("bash .github/scripts/limbo_differential.sh", step)
                self.assertIn("--test handrolled_limbo -- --ignored", step)
                # Blocking when the corpus matches (the script itself skips otherwise).
                self.assertNotIn("continue-on-error", step)
                # Before the live step, which must stay last in the job.
                self.assertLess(
                    job.index(marker), job.index("      - name: live internet (non-blocking)")
                )

    def test_the_live_internet_step_can_inform_but_never_gate(self) -> None:
        # A third party's server must not decide whether a PR merges, so the
        # live step is continue-on-error. Its output and exit logic live in
        # live_internet_check.sh (tested offline, including a failing command),
        # so the step must go through that script and not inline its own.
        for name, backend in self.ENGINE_JOBS:
            with self.subTest(job=name):
                job = self.engine_job(name)
                marker = "      - name: live internet (non-blocking)"
                step = job[job.index(marker) :]
                self.assertIn("continue-on-error: true", step)
                self.assertIn("bash .github/scripts/live_internet_check.sh", step)
                self.assertIn("--test handrolled_live -- --ignored", step)
                self.assertIn(f"LIVE_BACKEND: {backend}", step)
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

    def test_ci_change_with_no_affected_crate_still_runs_smoke(self) -> None:
        # Every PR adds a change fragment, so a real CI PR is never CI-only.
        # Without this fallback it selected no crate and so built nothing at
        # all: no smoke, no test, no clippy (#928, #1010, #1073, #1081).
        for extra in ("changes/2026-10-26-ci-fix.md", "docs/agents/review-router.md"):
            with self.subTest(extra=extra):
                changed = f".github/actions/setup-build-env/action.yml\n{extra}\n"
                self.assertNotEqual(self._planner("--is-ci-only", input=changed).returncode, 0)
                self.assertEqual(
                    self._planner("--needs-ci-smoke", "--packages", "", input=changed).returncode, 0
                )
        # A crate lane already runs setup-build-env, and prose alone needs no smoke.
        self.assertEqual(
            self._planner(
                "--needs-ci-smoke", "--packages", "rusty_tick",
                input=".github/workflows/ci.yml\ncrates/apps/rusty_tick/src/lib.rs\n",
            ).returncode,
            1,
        )
        self.assertEqual(
            self._planner("--needs-ci-smoke", "--packages", "", input="docs/README.md\n").returncode, 1
        )
        self.assertIn(
            'printf \'%s\\n\' "$changed" | python3 .github/scripts/ci_plan.py --needs-ci-smoke --packages "$packages"',
            self.workflow,
        )

    def test_lockfile_is_not_an_unconditional_global_fallback(self) -> None:
        self.assertEqual(self._planner("--requires-full", input="Cargo.lock\n").returncode, 1)
        self.assertIn("lockfile_diff.py", self.workflow)
        self.assertNotIn("Cargo.lock changed with a manifest", self.workflow)
        self.assertIn("Unable to establish Cargo.lock impact", self.workflow)

    def test_lanes_spanning_most_components_fall_back_to_the_full_sweep(self) -> None:
        self.assertIn('--packages "$packages" --full-if-most)" || components_status=$?', self.workflow)
        self.assertIn('emit_full "Affected crates span most of the workspace\'s components"', self.workflow)

    def test_member_dependency_edits_are_scoped(self) -> None:
        # The edited member and its reverse dependencies are path-selected; the
        # daily full sweep covers feature unification in unrelated crates.
        self.assertNotIn("cargo_toml_diff.py --members", self.workflow)
        self.assertNotIn("Member dependency declarations changed", self.workflow)
        # A lock change beside a manifest edit adds its consumers to the path selection.
        self.assertIn('packages="$(printf \'%s\\n\' "$packages" "$lockfile_packages"', self.workflow)

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
        self.assertEqual(set(outputs), SPECIALIZED_KEYS | {"msrv_manifests"})
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
            {"dashboard": False, "term_web": False, "term_bench": False, "key_desktop": False, "tick": False, "fair_play": True, "agui": False, "win32": False, "multimodal_db": False, "rusty_config_no_std": False, "tls_engine": False, "rleval_viewer": False, "rleval_app": False, "rleval_service": False, "rleval_image": False, "crypto_ct": False, "rand_platforms": False},
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

    def test_rleval_service_jobs_follow_their_paths_only(self) -> None:
        # A test-only change runs pytest but cannot change the image.
        flags = specialized_job_flags(
            ["crates/apps/rocket_league/rleval/service/tests/test_api.py"], []
        )
        self.assertTrue(flags["rleval_service"])
        self.assertFalse(flags["rleval_image"])
        # What the image installs runs both: pytest on the runner's Python with the dev extra
        # is not the image's Python with the production extras.
        for path in (
            "crates/apps/rocket_league/rleval/service/app/models.py",
            "crates/apps/rocket_league/rleval/service/migrations/env.py",
            "crates/apps/rocket_league/rleval/service/pyproject.toml",
            "crates/apps/rocket_league/rleval/service/alembic.ini",
        ):
            flags = specialized_job_flags([path], [])
            self.assertTrue(flags["rleval_service"], path)
            self.assertTrue(flags["rleval_image"], path)
        for path in (
            "crates/apps/rocket_league/rleval/Dockerfile",
            "crates/apps/rocket_league/rleval/Dockerfile.dockerignore",
            "crates/apps/rocket_league/rleval/docker-compose.yml",
        ):
            flags = specialized_job_flags([path], [])
            self.assertTrue(flags["rleval_image"], path)
            self.assertFalse(flags["rleval_service"], path)
        # Docs and the Rust crates beside it select neither.
        flags = specialized_job_flags(
            ["crates/apps/rocket_league/rleval/docs/episodes-design.md"], ["rleval-app"]
        )
        self.assertFalse(flags["rleval_service"])
        self.assertFalse(flags["rleval_image"])

    def test_rleval_jobs_are_planned_and_gated(self) -> None:
        for key in ("rleval_viewer", "rleval_app", "rleval_service", "rleval_image"):
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
            {"dashboard": False, "term_web": False, "term_bench": False, "key_desktop": False, "tick": True, "fair_play": True, "agui": False, "win32": False, "multimodal_db": False, "rusty_config_no_std": False, "tls_engine": False, "rleval_viewer": False, "rleval_app": False, "rleval_service": False, "rleval_image": False, "crypto_ct": False, "rand_platforms": False},
        )
        self.assertEqual(
            specialized_job_flags(["crates/apps/rusty_tick/src/lib.rs"], ["rusty_tick"]),
            {"dashboard": False, "term_web": False, "term_bench": False, "key_desktop": False, "tick": True, "fair_play": False, "agui": False, "win32": False, "multimodal_db": False, "rusty_config_no_std": False, "tls_engine": False, "rleval_viewer": False, "rleval_app": False, "rleval_service": False, "rleval_image": False, "crypto_ct": False, "rand_platforms": False},
        )

    def test_agui_package_change_selects_tick_web_too(self) -> None:
        flags = specialized_job_flags(["crates/libs/protocol/rusty_agui/packages/agui-react/src/hooks.tsx"], [])
        self.assertTrue(flags["agui"])
        self.assertTrue(flags["tick"])

    def test_term_bench_follows_the_engine_and_its_driver(self) -> None:
        for package in ("rusty_term_core", "rusty_term"):
            self.assertTrue(specialized_job_flags([], [package])["term_bench"], package)
        self.assertFalse(specialized_job_flags([], ["rusty_tick"])["term_bench"])

    def test_shared_and_specialized_package_rules(self) -> None:
        flags = specialized_job_flags(
            [], ["rusty_win32", "rusty_multimodal_db", "rusty_config", "rusty_tls"]
        )
        self.assertTrue(flags["win32"])
        self.assertTrue(flags["multimodal_db"])
        self.assertTrue(flags["rusty_config_no_std"])
        self.assertTrue(flags["tls_engine"])

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
            if job in {"fmt", "plan-tests", "library-tests", "claude-hooks", "workflow-lint", "dependency-policy", "cargo-deny", "cargo-shear", "plan", "required-gate"}:
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
        # ci.yml is the only workflow the queue triggers, so the only one that
        # feeds the required check (the reaper reads merge_group runs, it is not one).
        for path in sorted(REPO.glob(".github/workflows/*.yml")):
            if path != WORKFLOW:
                with self.subTest(workflow=path.name):
                    self.assertNotRegex(path.read_text(encoding="utf-8"), r"(?m)^  merge_group:")

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

    def test_orch_bbp_acceptance_is_required_and_cannot_be_planned_out(self) -> None:
        job = self.job_text("orch-bbp-acceptance")
        self.assertIn("runs-on: ubuntu-latest", job)
        self.assertIn("needs: [plan, fmt]", job)
        self.assertNotIn("continue-on-error", job)
        # Only the platform gate: no scope flag may plan it out.
        self.assertEqual(re.findall(r"(?m)^    if: (.+)$", job), ["needs.plan.outputs.linux == 'true'"])
        self.assertIn("cargo build --locked -p rusty_orch -p rusty_bbp_host -p rusty_mill_board", job)
        self.assertIn("orch_bbp_acceptance.py --output acceptance-results", job)
        self.assertIn('export BBP_TEST_BIN="$PWD/target/debug/bbp"', job)
        self.assertIn('export ORCH_TEST_BIN="$PWD/target/debug/rusty_orch"', job)
        self.assertIn('export MILLBOARD_TEST_BIN="$PWD/target/debug/rusty_mill_board"', job)
        self.assertEqual(job.count("if: always()"), 2)
        self.assertIn("path: acceptance-results/", job)
        gate = self.workflow.split("  required-gate:\n", 1)[1]
        self.assertIn("      - orch-bbp-acceptance\n", gate)

    def test_workflows_default_to_read_only_token(self) -> None:
        for path in (WORKFLOW, BASELINE_WORKFLOW):
            with self.subTest(workflow=path.name):
                self.assertRegex(path.read_text(encoding="utf-8"), r"(?m)^permissions:\n  contents: read$")

    def test_scheduled_sweep_is_full_and_msrv_ignores_toolchain_file(self) -> None:
        self.assertRegex(self.workflow, r"(?m)^  schedule:\n    - cron: ")
        self.assertEqual(self._planner_map("--event", "schedule")["mode"], "full")
        msrv = self.workflow.split("  multimodal-db-msrv:\n")[1].split("  rusty-config-no-std-check:\n")[0]
        self.assertIn('RUSTUP_TOOLCHAIN: "1.89.0"', msrv)

    def test_msrv_compile_job_builds_at_declared_floors_and_gates(self) -> None:
        job = self.workflow.split("  msrv-compile:\n")[1].split("  rusty-config-no-std-check:\n")[0]
        self.assertIn("needs: plan", job)
        self.assertIn("(needs.plan.outputs.full == 'true' || needs.plan.outputs.msrv_manifests != '')", job)
        self.assertIn('scope=(--manifests "$PLAN_MANIFESTS")', job)
        self.assertIn("msrv_manifests: ${{ steps.plan.outputs.msrv_manifests }}", self.workflow)
        self.assertIn("check_msrv_compile.py", job)
        # The pinned toolchain's own versions are covered elsewhere (compared as full versions); builds stay
        # out of the cached target dir; metadata is locked and Cargo.lock must come out unchanged.
        self.assertIn('--skip-from "$RUST_TOOLCHAIN"', job)
        self.assertIn("cargo metadata --locked --format-version=1", job)
        self.assertIn('test "$(sha256sum Cargo.lock)" = "$lock_before"', job)
        self.assertIn('--target-base "$RUNNER_TEMP/msrv"', job)
        # Every crate with features is also checked with all of them; only documented exceptions opt out.
        self.assertIn('--all-features-exclude "platform-linux platform-windows rusty_json rusty_tls rusty_rdp rusty_tonic" \\\n', job)
        # rusty_axum's default build stays at its declared 1.87; its http2 (all-features) build is held to 1.88.
        self.assertIn('--all-features-floor "rusty_axum=1.88" \\\n', job)
        # Windows joins on the scheduled and manual sweeps only, minus the crates no Windows leg builds (#845).
        self.assertIn("runs-on: ${{ matrix.os }}", job)
        self.assertIn(
            "(github.event_name == 'schedule' || github.event_name == 'workflow_dispatch') "
            "&& needs.plan.outputs.windows == 'true' && '[\"ubuntu-latest\",\"windows-latest\"]' || '[\"ubuntu-latest\"]'",
            job,
        )
        self.assertIn("'rusty_journal rusty_fedora_agent'", job)
        self.assertIn("shell: bash", job)
        self.assertIn("      - msrv-compile\n", self.workflow.split("  required-gate:\n", 1)[1])

    def test_msrv_scope_is_the_crates_whose_own_manifest_changed(self) -> None:
        changed = (
            "crates/libs/net/rusty_axum/Cargo.toml\n"
            "crates/libs/net/rusty_axum/src/lib.rs\n"
            "crates/libs/a/Cargo.toml\n"
            "Cargo.toml\n"
            "Cargo.lock\n"
            "crates/libs/b/NotCargo.toml\n"
        )
        outputs = self._planner_map("--packages", "rusty_axum a dependent", input=changed)
        self.assertEqual(outputs["msrv_manifests"], "crates/libs/a/Cargo.toml crates/libs/net/rusty_axum/Cargo.toml")
        # Code-only, lockfile-only and root-manifest changes leave it to the scheduled sweep.
        code_only = self._planner_map("--packages", "rusty_axum", input="Cargo.toml\nCargo.lock\nsrc/lib.rs\n")
        self.assertEqual(code_only["msrv_manifests"], "")
        self.assertEqual(self._planner_map("--emit-full")["msrv_manifests"], "")

    def test_rusty_axum_floors_match_the_manifests(self) -> None:
        # The default build is held to rusty_axum's own rust-version; the all-features floor is exactly
        # what its `http2` dependencies declare, so neither check can drift above or below its manifest.
        def declared(crate: str) -> str:
            return tomllib.loads((REPO / "crates/libs/net" / crate / "Cargo.toml").read_text())["package"]["rust-version"]

        floor = re.search(r'--all-features-floor "rusty_axum=([0-9.]+)"', self.workflow)
        self.assertIsNotNone(floor)
        self.assertEqual(declared("rusty_axum"), "1.87")
        self.assertEqual(declared("rusty_axum_tokio"), "1.87")
        self.assertEqual({declared("rusty_hyper"), declared("rusty_h2")}, {floor.group(1)})

    def test_every_workflow_pin_matches_rust_toolchain_file(self) -> None:
        channel = tomllib.loads((REPO / "rust-toolchain.toml").read_text())["toolchain"]["channel"]
        for path in (WORKFLOW, BASELINE_WORKFLOW):
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

    def test_a_failed_nightly_sweep_files_an_issue(self) -> None:
        alert = (WORKFLOW.parent / "nightly-alert.yml").read_text(encoding="utf-8")
        self.assertIn("  workflow_run:\n    workflows: [CI]\n    types: [completed]\n", alert)
        self.assertIn("github.event.workflow_run.event == 'schedule'", alert)
        # Read-only by default; only the reporting job may write, and only issues.
        self.assertRegex(alert, r"(?m)^permissions:\n  contents: read$")
        self.assertEqual(re.findall(r"(?m)^\s+[a-z-]+: write$", alert), ["      issues: write"])
        # Run data reaches the script through env, never by interpolation.
        script = alert.split("        run: |\n", 1)[1]
        self.assertNotIn("${{", script)

    def test_libs_tags_are_cut_by_hand_from_main(self) -> None:
        tagger = (WORKFLOW.parent / "libs-tag.yml").read_text(encoding="utf-8")
        # Run by hand only, on main's head, which reached main through the queue (ADR-0013).
        self.assertIn("on:\n  workflow_dispatch:\n\n", tagger)
        self.assertIn("          ref: main\n", tagger)
        self.assertRegex(tagger, r"(?m)^permissions:\n  contents: read$")
        self.assertEqual(re.findall(r"(?m)^\s+[a-z-]+: write$", tagger), ["      contents: write"])
        # The tag logic lives in libs_tag.sh, which test_libs_tag runs against scratch repositories.
        self.assertIn("        run: bash .github/scripts/libs_tag.sh\n", tagger)

    def test_merge_queue_reaper_is_narrow(self) -> None:
        reaper = (WORKFLOW.parent / "merge-queue-reaper.yml").read_text(encoding="utf-8")
        # Main's copy runs on every CI request; no PR or queue check of its own.
        self.assertIn("on:\n  workflow_run:\n    workflows: [CI]\n    types: [requested]\n", reaper)
        self.assertNotRegex(reaper, r"(?m)^  (pull_request|pull_request_target|merge_group|push):")
        self.assertIn("    if: github.event.workflow_run.event == 'merge_group'\n", reaper)
        # A skipped (non-merge_group) run must not share, and so cancel, the sweep's group.
        self.assertIn("  group: merge-queue-reaper-${{ github.event.workflow_run.event }}\n", reaper)
        # Read-only by default; the sweep may only cancel runs.
        self.assertRegex(reaper, r"(?m)^permissions:\n  contents: read$")
        self.assertEqual(re.findall(r"(?m)^\s+[a-z-]+: write$", reaper), ["      actions: write"])
        self.assertNotRegex(reaper, r"(?m)^\s*-?\s*uses:")
        self.assertIn("timeout-minutes: 5", reaper)
        script = reaper.split("        run: |\n", 1)[1]
        self.assertNotIn("${{", script)

    @unittest.skipUnless(shutil.which("bash") and shutil.which("grep"), "bash is needed to run the sweep")
    def test_merge_queue_reaper_cancels_only_runs_whose_ref_is_gone(self) -> None:
        reaper = (WORKFLOW.parent / "merge-queue-reaper.yml").read_text(encoding="utf-8")
        script = textwrap.dedent(reaper.split("        run: |\n", 1)[1])
        fake_gh = textwrap.dedent("""\
            #!/usr/bin/env bash
            # Answers the three calls the sweep makes; --jq output is pre-shaped.
            # Run listings have a second page that only --paginate returns.
            args="$*"
            case "$args" in
              *"-X POST"*"/cancel"*) id="${args##*/runs/}"; echo "${id%%/*}" >> "$STATE/cancelled" ;;
              *"/matching-refs/"*) [ -z "${FAIL_REFS:-}" ] || exit 1; cat "$STATE/refs" ;;
              *"--paginate"*"status=in_progress"*) cat "$STATE/runs" "$STATE/runs2" ;;
              *"status=in_progress"*) cat "$STATE/runs" ;;
            esac
            """)

        def sweep(runs: str, refs: str, fail_refs: bool = False, runs2: str = "") -> tuple[int, list[str]]:
            with tempfile.TemporaryDirectory() as tmp:
                state = Path(tmp)
                (state / "gh").write_text(fake_gh)
                (state / "gh").chmod(0o755)
                (state / "runs").write_text(runs)
                (state / "runs2").write_text(runs2)
                (state / "refs").write_text(refs)
                env = dict(os.environ, PATH=f"{state}:{os.environ['PATH']}", STATE=str(state), GH_REPO="o/r")
                if fail_refs:
                    env["FAIL_REFS"] = "1"
                code = subprocess.run(["bash", "-c", script], env=env, capture_output=True, text=True).returncode
                cancelled = state / "cancelled"
                return code, cancelled.read_text().split() if cancelled.exists() else []

        live = "gh-readonly-queue/main/pr-7-aaa"
        dead = "gh-readonly-queue/main/pr-8-bbb"
        runs = f"101 {live}\n102 {dead}\n"
        self.assertEqual(sweep(runs, f"refs/heads/{live}\n"), (0, ["102"]))
        self.assertEqual(sweep(runs, f"refs/heads/{live}\nrefs/heads/{dead}\n"), (0, []))
        # A prefix of a live ref is not that ref.
        self.assertEqual(sweep("103 gh-readonly-queue/main/pr-7\n", f"refs/heads/{live}\n"), (0, ["103"]))
        self.assertEqual(sweep("", f"refs/heads/{live}\n"), (0, []), "nothing in flight, nothing to do")
        # A stale run past the first 100 of its status is still swept.
        self.assertEqual(sweep(runs, f"refs/heads/{live}\n", runs2="104 gh-readonly-queue/main/pr-9-ccc\n"), (0, ["102", "104"]))
        code, cancelled = sweep(runs, "", fail_refs=True)
        self.assertNotEqual(code, 0)
        self.assertEqual(cancelled, [], "an unreadable queue must never cancel anything")

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

    def test_tls_engine_jobs_run_when_their_own_ci_changes(self) -> None:
        # A skipped job reports success, so a PR that edits these jobs or the
        # scripts they run must also run them, or the edit is never tested.
        for path in (
            ".github/workflows/ci.yml",
            ".github/scripts/ci_plan.py",
            ".github/scripts/tls_fuzz_smoke.sh",
            ".github/scripts/live_internet_check.sh",
            ".github/scripts/limbo_differential.sh",
        ):
            with self.subTest(path=path):
                self.assertEqual(self._planner_map("--packages", "", input=f"{path}\n")["tls_engine"], "true")
        for path in (".github/scripts/test_ci_workflow.py", "docs/README.md", ".github/workflows/unrelated.yml"):
            with self.subTest(path=path):
                self.assertEqual(self._planner_map("--packages", "", input=f"{path}\n")["tls_engine"], "false")

    def test_rleval_service_jobs_run_when_their_own_ci_changes(self) -> None:
        for path in (".github/workflows/ci.yml", ".github/scripts/ci_plan.py"):
            with self.subTest(path=path):
                outputs = self._planner_map("--packages", "", input=f"{path}\n")
                self.assertEqual(outputs["rleval_service"], "true")
                self.assertEqual(outputs["rleval_image"], "true")
                smoke = self._planner_map("--emit-ci-smoke", input=f"{path}\n")
                self.assertEqual(smoke["rleval_service"], "true")
                self.assertEqual(smoke["rleval_image"], "true")
        for path in (".github/scripts/test_ci_workflow.py", "docs/README.md", ".github/workflows/unrelated.yml"):
            with self.subTest(path=path):
                outputs = self._planner_map("--packages", "", input=f"{path}\n")
                self.assertEqual(outputs["rleval_service"], "false")
                self.assertEqual(outputs["rleval_image"], "false")

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
                # ci.yml and ci_plan.py also select the native TLS engine and rleval service jobs.
                own = path in {".github/workflows/ci.yml", ".github/scripts/ci_plan.py"}
                for key in ("tls_engine", "rleval_service", "rleval_image"):
                    self.assertEqual(outputs[key], "true" if own else "false", key)
                win32 = own or path == ".github/scripts/test_ci_workflow.py"
                self.assertEqual(outputs["win32"], "true" if win32 else "false")
                self.assertTrue(
                    all(
                        outputs[key] == "false"
                        for key in SPECIALIZED_KEYS
                        - {"rand_platforms", "tls_engine", "rleval_service", "rleval_image", "win32"}
                    )
                )
        self.assertIn(
            "printf '%s\\n' \"$changed\" | python3 .github/scripts/ci_plan.py --emit-ci-smoke >> \"$GITHUB_OUTPUT\"",
            self.workflow,
        )

    def test_rand_platform_commands_and_required_gate_are_preserved(self) -> None:
        job = self.workflow.split("  rusty-rand-platforms:\n", 1)[1].split("\n  # Every other Rust job", 1)[0]
        self.assertIn("os: ${{ fromJSON(needs.plan.outputs.rand_os) }}", job)
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

    def _crypto_valgrind_step(self) -> str:
        job = self.workflow.split("  crypto-constant-time:\n", 1)[1]
        return job.split("      - name: Install valgrind\n", 1)[1].split("      - name:", 1)[0]

    def test_crypto_valgrind_setup_has_blocking_time_and_acquisition_bounds(self) -> None:
        step = self._crypto_valgrind_step()
        self.assertIn("        timeout-minutes: 10\n", step)
        self.assertIn("        shell: bash\n", step)
        self.assertNotIn("continue-on-error", step)
        self.assertIn("Acquire::Retries=3", step)
        self.assertIn("Acquire::http::Timeout=30", step)
        self.assertIn("Acquire::https::Timeout=30", step)
        self.assertIn("--error-on=any update", step)
        self.assertIn("install -y --no-install-recommends valgrind", step)
        self.assertIn("valgrind --version", step)
        self.assertIn("objdump --version | head -1", step)

    @unittest.skipUnless(shutil.which("bash"), "bash is needed to run valgrind setup")
    def test_crypto_valgrind_setup_propagates_apt_failures(self) -> None:
        script = textwrap.dedent(self._crypto_valgrind_step().split("        run: |\n", 1)[1])
        # Execute the real workflow body with shell functions at the sudo/tool
        # boundary: no root access, network, or package installation is needed.
        mocks = r'''
sudo() {
  printf '%s\n' "$*"
  case " $* " in
    *" update "*) return "$UPDATE_STATUS" ;;
    *" install "*) return "$INSTALL_STATUS" ;;
    *) return 99 ;;
  esac
}
valgrind() { echo valgrind-diagnostic; }
objdump() { echo objdump-diagnostic; }
'''
        # Include a timeout exit status as well as apt's usual error status.
        for update_status, install_status in ((0, 0), (100, 0), (0, 100), (124, 0), (0, 124)):
            with self.subTest(update=update_status, install=install_status):
                result = subprocess.run(
                    ["bash", "--noprofile", "--norc", "-e", "-o", "pipefail", "-c", mocks + script],
                    env={**os.environ, "UPDATE_STATUS": str(update_status), "INSTALL_STATUS": str(install_status)},
                    capture_output=True, text=True, timeout=10,
                )
                self.assertEqual(result.returncode, update_status or install_status)
                self.assertEqual(" install " in result.stdout, update_status == 0)
                self.assertEqual("valgrind-diagnostic" in result.stdout, result.returncode == 0)
                self.assertEqual("objdump-diagnostic" in result.stdout, result.returncode == 0)
                for call in result.stdout.splitlines():
                    if call.startswith("apt-get "):
                        self.assertIn("-o Acquire::Retries=3", call)
                        self.assertIn("-o Acquire::http::Timeout=30", call)
                        self.assertIn("-o Acquire::https::Timeout=30", call)
                        if " update" in call:
                            self.assertIn("--error-on=any update", call)


if __name__ == "__main__":
    unittest.main()
