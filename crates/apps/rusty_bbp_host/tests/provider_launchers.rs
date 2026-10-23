//! Real launcher process contracts; provider stubs require no login or network.
#![cfg(target_os = "linux")]
#![allow(clippy::unwrap_used, clippy::expect_used)]
use rusty_bbp::*;
use rusty_bbp_host::{admin, moderator, open_driver, profiles::ProfileSet};
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::Command as Proc;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Fixture {
    dir: PathBuf,
    repo: PathBuf,
    task: TaskId,
    principal: PrincipalId,
    turn: TurnId,
    config: PathBuf,
    reviewer: bool,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.dir).unwrap();
    }
}
impl Fixture {
    fn new(reviewer: bool) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "bbp_launcher_{}_{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        ));
        let repo = dir.join("source");
        std::fs::create_dir_all(&repo).unwrap();
        for args in [
            vec!["init", "-q"],
            vec!["config", "user.name", "Test"],
            vec!["config", "user.email", "test@example.invalid"],
        ] {
            assert!(Proc::new("git")
                .arg("-C")
                .arg(&repo)
                .args(args)
                .status()
                .unwrap()
                .success());
        }
        std::fs::write(repo.join("flag"), "no\n").unwrap();
        assert!(Proc::new("git")
            .arg("-C")
            .arg(&repo)
            .args(["add", "flag"])
            .status()
            .unwrap()
            .success());
        assert!(Proc::new("git")
            .arg("-C")
            .arg(&repo)
            .args(["commit", "-qm", "base"])
            .status()
            .unwrap()
            .success());
        let task = TaskId("task".into());
        let config: rusty_serde::json::Value =
            rusty_serde::json::from_str(include_str!("../../rusty_orch/docs/examples/bbp.json"))
                .unwrap();
        assert_eq!(
            config["assignments"]["maker"]["builtin_tools"].as_bool(),
            Some(true)
        );
        assert_eq!(
            config["assignments"]["auditor"]["builtin_tools"].as_bool(),
            Some(false)
        );
        let roster: Vec<Role> =
            rusty_serde::json::from_str(&rusty_serde::json::to_string(&config["roster"]).unwrap())
                .unwrap();
        assert_eq!(
            admin::open_task(
                &dir,
                &task,
                repo.to_str().unwrap(),
                b"flag",
                &PrincipalId("human".into()),
                &ProfileSet::shell("test", "true"),
                roster
            )
            .unwrap(),
            Response::Stored(ArtId(1))
        );
        for (role, vendor) in [("maker", "codex"), ("auditor", "claude")] {
            assert_eq!(
                admin::assign(
                    &dir,
                    &task,
                    &RoleId(role.into()),
                    &PrincipalId(role.into()),
                    vendor
                )
                .unwrap(),
                Response::Ok
            );
        }
        admin::tick(&dir, &task).unwrap();
        let (principal, turn) = if reviewer {
            // Boundary fixture for the launcher, not a substitute for engine/e2e tests.
            let mut d = open_driver(&dir, &task).unwrap();
            let prior = d.state.turn().unwrap().id;
            d.store
                .append(
                    &task,
                    d.state.rev,
                    &[
                        Event::TurnEnded {
                            turn: prior,
                            cause: TurnEnd::Aborted,
                        },
                        Event::StateChanged {
                            from: State::Planning,
                            to: State::Review,
                        },
                        Event::TurnGranted {
                            role: RoleId("auditor".into()),
                            turn: TurnId(99),
                            kind: TurnKind::Default,
                            deadline: Time(u64::MAX),
                            return_to: None,
                            slice: None,
                        },
                    ],
                )
                .unwrap();
            (PrincipalId("auditor".into()), TurnId(99))
        } else {
            (
                PrincipalId("maker".into()),
                open_driver(&dir, &task).unwrap().state.turn().unwrap().id,
            )
        };
        let config = moderator::write_mcp_config(
            &dir,
            &task,
            &PathBuf::from(env!("CARGO_BIN_EXE_bbp")),
            &principal,
            turn,
        )
        .unwrap();
        Self {
            dir,
            repo,
            task,
            principal,
            turn,
            config,
            reviewer,
        }
    }
    fn command(&self) -> Proc {
        let binary = if self.reviewer {
            env!("CARGO_BIN_EXE_claude-bbp-harness")
        } else {
            env!("CARGO_BIN_EXE_codex-bbp-harness")
        };
        let stub = self.dir.join("provider.sh");
        std::fs::write(&stub,"#!/bin/sh\nset -eu\ntest -z \"${OPENAI_API_KEY-}${ANTHROPIC_API_KEY-}${GH_TOKEN-}${BBP_DIR-}\"\nprintf '%s\\0' \"$@\" > \"$CAPTURE_ARGS\"\npwd > \"$CAPTURE_CWD\"\nexit 42\n").unwrap();
        std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o700)).unwrap();
        let mut cmd = Proc::new(binary);
        cmd.env("BBP_DIR", &self.dir)
            .env("BBP_TASK", &self.task.0)
            .env("BBP_PRINCIPAL", &self.principal.0)
            .env("BBP_TURN", self.turn.0.to_string())
            .env("BBP_BIN", env!("CARGO_BIN_EXE_bbp"))
            .env("OPENAI_API_KEY", "sentinel")
            .env("ANTHROPIC_API_KEY", "sentinel")
            .env("GH_TOKEN", "sentinel")
            .env("CAPTURE_ARGS", self.dir.join("args"))
            .env("CAPTURE_CWD", self.dir.join("cwd"))
            .args([
                "--provider-bin",
                stub.to_str().unwrap(),
                "--mcp-config",
                self.config.to_str().unwrap(),
            ]);
        cmd
    }
    fn captured_args(&self) -> Vec<String> {
        String::from_utf8(std::fs::read(self.dir.join("args")).unwrap())
            .unwrap()
            .split('\0')
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .collect()
    }
}

#[test]
fn author_execs_in_an_isolated_clone_and_preserves_provider_exit_status() {
    let f = Fixture::new(false);
    assert_eq!(f.command().status().unwrap().code(), Some(42));
    let cwd = PathBuf::from(std::fs::read_to_string(f.dir.join("cwd")).unwrap().trim());
    assert_ne!(cwd, f.repo);
    assert!(f
        .captured_args()
        .windows(2)
        .any(|args| args == ["--sandbox", "workspace-write"]));
    assert_eq!(std::fs::read_to_string(cwd.join("flag")).unwrap(), "no\n");
    assert!(Proc::new("git")
        .arg("-C")
        .arg(cwd)
        .args(["remote"])
        .output()
        .unwrap()
        .stdout
        .is_empty());
    assert!(f
        .captured_args()
        .iter()
        .any(|a| a.contains("required=true") && a.contains("BBP_TURN")));
    assert_eq!(
        std::fs::read_to_string(f.repo.join("flag")).unwrap(),
        "no\n"
    );
}

#[test]
fn reviewer_execs_with_an_empty_toolset_and_only_its_bound_mcp_configuration() {
    let f = Fixture::new(true);
    assert_eq!(f.command().status().unwrap().code(), Some(42));
    let cwd = PathBuf::from(std::fs::read_to_string(f.dir.join("cwd")).unwrap().trim());
    assert!(std::fs::read_dir(cwd).unwrap().next().is_none());
    // Preserve the empty --tools argument; it must not become a missing flag.
    let bytes = std::fs::read(f.dir.join("args")).unwrap();
    let args: Vec<_> = bytes
        .split(|b| *b == 0)
        .map(|s| String::from_utf8_lossy(s).into_owned())
        .collect();
    assert!(args.windows(2).any(|w| w == ["--tools", ""]));
    assert!(args.iter().any(|s| s == "--strict-mcp-config"));
    let config = &args[args.iter().position(|s| s == "--mcp-config").unwrap() + 1];
    let json: rusty_serde::json::Value =
        rusty_serde::json::from_str(&std::fs::read_to_string(config).unwrap()).unwrap();
    assert_eq!(
        json.get("mcpServers")
            .unwrap()
            .get("bbp")
            .unwrap()
            .get("env")
            .unwrap()
            .get("BBP_TURN")
            .unwrap()
            .as_str(),
        Some("99")
    );
}

#[test]
fn tampered_mcp_and_stale_turn_are_refused_before_provider_start() {
    for stale in [false, true] {
        let f = Fixture::new(false);
        let mut cmd = f.command();
        if stale {
            let mut d = open_driver(&f.dir, &f.task).unwrap();
            d.dispatch(
                &Command::AbortTurn { turn: Some(f.turn) },
                rusty_bbp_host::now(),
            )
            .unwrap();
        } else {
            let text = std::fs::read_to_string(&f.config)
                .unwrap()
                .replace("\"mcp\"", "\"human\"");
            std::fs::write(&f.config, text).unwrap();
        }
        assert!(!cmd.status().unwrap().success());
        assert!(!f.dir.join("args").exists());
    }
}

#[test]
fn reviewer_cannot_receive_provider_flag_overrides() {
    let f = Fixture::new(true);
    assert!(!f
        .command()
        .args(["--tools", "Bash"])
        .status()
        .unwrap()
        .success());
    assert!(!f.dir.join("args").exists());
}

#[test]
fn reviewer_refuses_a_same_vendor_candidate_writer_before_provider_start() {
    let f = Fixture::new(true);
    let mut d = open_driver(&f.dir, &f.task).unwrap();
    d.store
        .append(
            &f.task,
            d.state.rev,
            &[Event::Assigned {
                role: RoleId("maker".into()),
                principal: Principal {
                    id: PrincipalId("maker".into()),
                    kind: PrincipalKind::Agent {
                        role: RoleId("maker".into()),
                        vendor: "claude".into(),
                    },
                },
            }],
        )
        .unwrap();
    assert!(!f.command().status().unwrap().success());
    assert!(!f.dir.join("args").exists());
}

#[test]
fn inherited_git_context_cannot_redirect_author_commands_or_external_index() {
    let f = Fixture::new(false);
    let external_index = f.dir.join("external.index");
    let original = std::fs::read(f.repo.join(".git/index")).unwrap();
    std::fs::write(&external_index, &original).unwrap();
    let mut command = f.command();
    std::fs::write(
        f.dir.join("provider.sh"),
        r#"#!/bin/sh
set -eu
test -z "${GIT_DIR-}${GIT_WORK_TREE-}${GIT_INDEX_FILE-}${GIT_CONFIG_COUNT-}"
test "$GIT_CONFIG_NOSYSTEM" = 1
test "$GIT_CONFIG_GLOBAL" = /dev/null
git rev-parse --show-toplevel > "$CAPTURE_CWD"
printf 'changed\n' > flag
git diff -- flag > "$CAPTURE_DIFF"
git add flag
"#,
    )
    .unwrap();
    let output = command
        .env("GIT_DIR", f.repo.join(".git"))
        .env("GIT_WORK_TREE", &f.repo)
        .env("GIT_INDEX_FILE", &external_index)
        .env("GIT_CONFIG_COUNT", "1")
        .env("GIT_CONFIG_KEY_0", "core.worktree")
        .env("GIT_CONFIG_VALUE_0", &f.repo)
        .env("CAPTURE_DIFF", f.dir.join("diff"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let cwd = PathBuf::from(std::fs::read_to_string(f.dir.join("cwd")).unwrap().trim());
    assert_ne!(cwd, f.repo);
    assert!(cwd.starts_with(f.dir.join("harness")));
    assert!(std::fs::read_to_string(f.dir.join("diff"))
        .unwrap()
        .contains("+changed"));
    assert_eq!(std::fs::read(&external_index).unwrap(), original);
    assert_eq!(
        std::fs::read_to_string(f.repo.join("flag")).unwrap(),
        "no\n"
    );
}
