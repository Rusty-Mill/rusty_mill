"""Run Linux acceptance cases and retain a result for every mode."""

from __future__ import annotations

import argparse
from dataclasses import asdict, dataclass
import json
from pathlib import Path
import re
import subprocess


@dataclass(frozen=True)
class Case:
    """One exact Rust test and the evidence it must emit after its assertions."""

    mode: str
    package: str
    target: tuple[str, ...]
    test: str

    def command(self) -> list[str]:
        """Select exactly one opt-in test, without swallowing its output."""
        return ["cargo", "test", "--locked", "-p", self.package, *self.target,
                self.test, "--", "--ignored", "--exact", "--nocapture"]


CASES = (
    Case("lifecycle", "orch-bbp", ("--lib",),
         "tests_confined::confined_orch_cli_lifecycle_resumes_and_records_receipt"),
    Case("pause-adoption", "orch-bbp", ("--lib",),
         "tests_e2e::no_progress_pause_survives_orch_processes_without_duplicate_task_or_charge"),
    Case("stale-holder", "orch-bbp", ("--lib",),
         "tests_e2e::orch_binary_from_a_stale_state_copy_is_fenced_out_of_the_task"),
    Case("millboard-link", "orch-bbp", ("--lib",),
         "tests_e2e::orch_binary_records_each_generation_with_its_millboard_card"),
    Case("slot-admission", "orch-bbp", ("--lib",),
         "tests_e2e::orch_binary_admits_two_goals_through_one_agent_slot"),
    Case("orch-cancel", "orch-bbp", ("--lib",),
         "tests_e2e::orch_binary_cancel_reaps_a_live_turn_frees_its_slot_and_retires_the_goal"),
    Case("orch-cancel-moderator-lost", "orch-bbp", ("--lib",),
         "tests_e2e::orch_binary_cancel_stops_the_writer_a_dead_moderator_left_behind"),
    Case("orch-cancel-mid-cancel", "orch-bbp", ("--lib",),
         "tests_e2e::orch_binary_cancel_completes_when_the_moderator_dies_while_it_waits"),
    Case("orch-cancel-unrecorded-open", "orch-bbp", ("--lib",),
         "tests_e2e::orch_binary_cancel_finds_work_started_before_its_handle_was_checkpointed"),
    Case("harness-confined", "orch-bbp", ("--lib",),
         "tests_e2e::orch_binary_confines_a_sandboxed_pool_member_away_from_the_bbp_directory"),
    Case("bbp-confined", "rusty_bbp_host", ("--test", "confined_harness"),
         "a_confined_harness_cannot_reach_the_task_directory_yet_plans_through_its_relay"),
    Case("shared-worktrees", "rusty_bbp_host", ("--test", "confined_harness"),
         "two_confined_tasks_share_one_object_store_but_not_their_worktrees"),
    Case("launcher-confined", "rusty_bbp_host", ("--test", "confined_harness"),
         "a_confined_codex_launcher_checks_its_turn_and_plans_from_a_shared_clone"),
    Case("session-confined", "rusty_bbp_host", ("--test", "confined_harness"),
         "a_confined_session_keeps_its_home_and_one_fresh_workspace_across_turns"),
    Case("rust-lld", "rusty_bbp_host", ("--test", "rust_lld_profile"),
         "confined_rust_lld_profile_passes_under_one_gib"),
    Case("maintenance", "rusty_orch", ("--lib",),
         "maintenance::e2e::sandboxed_maintenance_recovers_and_prepares_reviewed_pr_without_merging"),
    *(Case(mode, "rusty_bbp_host", ("--test", "moderator_e2e"),
           f"confined_{mode}_recovery") for mode in ("timeout", "cancel", "kill")),
)


@dataclass(frozen=True)
class Result:
    """Portable per-mode evidence, including failure rather than just exit status."""

    mode: str
    passed: bool
    detail: str


def assess(mode: str, returncode: int, output: str) -> Result:
    """Reject refusal, zero/skipped tests, and missing completion evidence."""
    if returncode != 0:
        return Result(mode, False, f"test exited {returncode}")
    if "NotEnforced" in output:
        return Result(mode, False, "sandbox refusal")
    if not re.search(r"test result: ok\. 1 passed; 0 failed; 0 ignored;", output):
        return Result(mode, False, "expected exactly one executed test")
    if f"BBP_ACCEPTANCE PASS {mode}" not in output.splitlines():
        return Result(mode, False, "missing completion evidence")
    return Result(mode, True, "acceptance assertions completed")


def run_case(case: Case, directory: Path) -> Result:
    """Run one case with a deadline and preserve output even on failure."""
    try:
        completed = subprocess.run(case.command(), stdout=subprocess.PIPE,
                                   stderr=subprocess.STDOUT, timeout=180, check=False)
        output = completed.stdout.decode("utf-8", errors="replace")
        result = assess(case.mode, completed.returncode, output)
    except subprocess.TimeoutExpired as error:
        output = (error.stdout or b"").decode("utf-8", errors="replace")
        result = Result(case.mode, False, "acceptance deadline exceeded")
    except OSError as error:
        output = str(error)
        result = Result(case.mode, False, f"could not start test: {error}")
    (directory / f"{case.mode}.log").write_text(output, encoding="utf-8")
    print(output, flush=True)
    print(f"{case.mode}: {'PASS' if result.passed else 'FAIL'} ({result.detail})", flush=True)
    return result


def main() -> int:
    """Attempt every mode, write machine/human results, and fail on any gap."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    results = [run_case(case, args.output) for case in CASES]
    (args.output / "results.json").write_text(
        json.dumps([asdict(result) for result in results], indent=2) + "\n", encoding="utf-8")
    rows = ["| Mode | Result | Evidence |", "| --- | --- | --- |"]
    rows.extend(f"| {r.mode} | {'PASS' if r.passed else 'FAIL'} | {r.detail} |" for r in results)
    (args.output / "summary.md").write_text("\n".join(rows) + "\n", encoding="utf-8")
    return 0 if all(result.passed for result in results) else 1


if __name__ == "__main__":
    raise SystemExit(main())
