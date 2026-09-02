//! ADR-0035 / eval-plan §4.1: the controlled-visibility ablation eval-substrate,
//! exercised via golden-episode replay. Asserts the three stages — per-episode
//! isolation, level-visibility, and evaluator-side adjudication at *every* level
//! independent of the agent's self-report (paper Table 5).

use rk_app::{run_episode, GoldenEpisode};
use rk_compose::EpisodeOutcome;
use rk_config::HarnessLevel;
use rk_kernel::fake::{FakeLanguageModel, Scripted};

fn passing_checks() -> String {
    // The evaluator's check: a freshly-computed assertion over the workspace.
    "[[check]]\nname = \"present\"\ncommand = \"cat marker.txt\"\nexpected_substring = \"DONE\"\ncovers = [\"req-1\"]\n".to_string()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn h0_is_adjudicated_by_evaluator_with_no_agent_report() {
    // Paper Table 5: H0 can earn autonomous_verified_success when the evaluator's
    // checks pass — with NO agent verification report at all. The agent here just
    // emits text (no tools, no verification_report), yet the evaluator labels it.
    let ep = GoldenEpisode {
        name: "h0-eval".into(),
        level: HarnessLevel::H0,
        commit: "fixed".into(),
        files: vec![("marker.txt".into(), "DONE\n".into())],
        checks_toml: Some(passing_checks()),
        prompts: vec!["solve it".into()],
    };
    let model = FakeLanguageModel::new(vec![vec![Scripted::Text("done".into())]]);
    let out = run_episode(&ep, model).await.unwrap();

    // Evaluator verifies behaviour independent of the (absent) agent report.
    assert_eq!(
        out.evaluator_outcome,
        EpisodeOutcome::AutonomousVerifiedSuccess
    );
    let _ = std::fs::remove_dir_all(&out.workspace);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn evaluator_fails_when_its_own_checks_fail() {
    // The agent claims success (text reply), but the evaluator's check fails →
    // the comparable label is Failed, not the agent's optimistic self-report.
    let ep = GoldenEpisode {
        name: "eval-fail".into(),
        level: HarnessLevel::H1,
        commit: "fixed".into(),
        files: vec![("marker.txt".into(), "WRONG\n".into())],
        checks_toml: Some(passing_checks()),
        prompts: vec!["solve it".into()],
    };
    let model = FakeLanguageModel::new(vec![vec![Scripted::Text("all good!".into())]]);
    let out = run_episode(&ep, model).await.unwrap();
    assert_eq!(out.evaluator_outcome, EpisodeOutcome::Failed);
    let _ = std::fs::remove_dir_all(&out.workspace);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn each_episode_runs_in_its_own_isolated_workspace() {
    // Two episodes of the same name get distinct fresh workspaces (Stage 1),
    // so there is no shared tree / cross-episode contamination.
    let mk = || GoldenEpisode {
        name: "iso".into(),
        level: HarnessLevel::H1,
        commit: "fixed".into(),
        files: vec![("marker.txt".into(), "DONE\n".into())],
        checks_toml: Some(passing_checks()),
        prompts: vec!["go".into()],
    };
    let a = run_episode(
        &mk(),
        FakeLanguageModel::new(vec![vec![Scripted::Text("a".into())]]),
    )
    .await
    .unwrap();
    let b = run_episode(
        &mk(),
        FakeLanguageModel::new(vec![vec![Scripted::Text("b".into())]]),
    )
    .await
    .unwrap();
    assert_ne!(a.workspace, b.workspace);
    // Each workspace is self-contained (its own .rustykeys/).
    assert!(a.workspace.join(".rustykeys").exists());
    assert!(b.workspace.join(".rustykeys").exists());
    let _ = std::fs::remove_dir_all(&a.workspace);
    let _ = std::fs::remove_dir_all(&b.workspace);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn no_checks_means_no_evaluator_evidence() {
    // Without a checks.toml the evaluator has nothing to adjudicate on → the
    // outcome is UnverifiedSuccess (not a free pass).
    let ep = GoldenEpisode {
        name: "no-checks".into(),
        level: HarnessLevel::H1,
        commit: "fixed".into(),
        files: vec![],
        checks_toml: None,
        prompts: vec!["go".into()],
    };
    let model = FakeLanguageModel::new(vec![vec![Scripted::Text("done".into())]]);
    let out = run_episode(&ep, model).await.unwrap();
    assert_eq!(out.evaluator_outcome, EpisodeOutcome::UnverifiedSuccess);
    let _ = std::fs::remove_dir_all(&out.workspace);
}
