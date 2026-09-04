//! [`PackageController`] over `dnf`, shelled out to through a `rustils`
//! [`Spawner`]. Installs/removes can run long, so `install`/`remove` spawn
//! a background OS thread and return a [`TaskId`] immediately -- this
//! agent has no async runtime (see the crate README for why), so "return
//! immediately, poll later" is a thread plus an in-memory registry rather
//! than a spawned future.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;

use platform::process::{ExitStatus, Spawner};

use crate::allowlist::Allowlist;
use crate::domain::{PackageUpdate, TaskId, TaskState, TaskStatus};
use crate::error::AgentError;
use crate::ports::PackageController;
use crate::process_util::run_captured;

type TaskRegistry = Mutex<HashMap<TaskId, TaskStatus>>;

pub struct DnfController {
    spawner: Arc<dyn Spawner + Send + Sync>,
    allowlist: Arc<Allowlist>,
    tasks: Arc<TaskRegistry>,
    next_task_id: AtomicU64,
}

impl DnfController {
    pub fn new(spawner: Arc<dyn Spawner + Send + Sync>, allowlist: Arc<Allowlist>) -> Self {
        Self {
            spawner,
            allowlist,
            tasks: Arc::new(Mutex::new(HashMap::new())),
            next_task_id: AtomicU64::new(1),
        }
    }

    fn new_task_id(&self) -> TaskId {
        let n = self.next_task_id.fetch_add(1, Ordering::Relaxed);
        TaskId(format!("task-{n}"))
    }

    /// Checks every package against the allowlist -- the whole batch is
    /// rejected if any one package fails, rather than silently dropping it
    /// from the command -- then spawns a background thread that runs
    /// `dnf <verb> -y <packages...>` and records the outcome.
    fn run_dnf(
        &self,
        verb: &'static str,
        op: &'static str,
        packages: &[String],
    ) -> Result<TaskId, AgentError> {
        if packages.is_empty() {
            return Err(AgentError::InvalidRequest(
                "packages must not be empty".to_string(),
            ));
        }
        for package in packages {
            self.allowlist.check_package(package)?;
        }

        let task_id = self.new_task_id();
        lock_tasks(&self.tasks).insert(
            task_id.clone(),
            TaskStatus {
                id: task_id.clone(),
                state: TaskState::Running,
                stdout: None,
                stderr: None,
                exit_code: None,
            },
        );

        let spawner = self.spawner.clone();
        let tasks = self.tasks.clone();
        let task_id_for_thread = task_id.clone();
        let mut args = vec![verb.to_string(), "-y".to_string()];
        args.extend(packages.iter().cloned());

        thread::spawn(move || {
            let status = match run_captured(spawner.as_ref(), op, "dnf", &args) {
                Ok(output) => TaskStatus {
                    id: task_id_for_thread.clone(),
                    state: if output.status.success() {
                        TaskState::Succeeded
                    } else {
                        TaskState::Failed
                    },
                    exit_code: exit_code_of(output.status),
                    stdout: Some(output.stdout),
                    stderr: Some(output.stderr),
                },
                Err(err) => TaskStatus {
                    id: task_id_for_thread.clone(),
                    state: TaskState::Failed,
                    stdout: None,
                    stderr: Some(err.to_string()),
                    exit_code: None,
                },
            };
            lock_tasks(&tasks).insert(task_id_for_thread, status);
        });

        Ok(task_id)
    }
}

impl PackageController for DnfController {
    fn list_updates(&self) -> Result<Vec<PackageUpdate>, AgentError> {
        let args = vec!["check-update".to_string()];
        // `dnf check-update` exits 100 when updates *are* available (0 =
        // none, 1 = error) -- not a failure, so this can't go through the
        // usual "non-zero is an error" helper.
        let output = run_captured(self.spawner.as_ref(), "dnf check-update", "dnf", &args)?;
        match output.status {
            ExitStatus::Code(0) => Ok(Vec::new()),
            ExitStatus::Code(100) => {
                let updates = parse_check_update(&output.stdout);
                if updates.is_empty() {
                    // Exit 100 means dnf itself is telling us updates ARE
                    // available; zero parsed rows means our parser didn't
                    // recognize this dnf version's output format, not that
                    // there truly are none -- that contradiction is exactly
                    // what DnfParse exists to surface rather than silently
                    // reporting "no updates" when there are some.
                    Err(AgentError::DnfParse(format!(
                        "exit 100 (updates available) but no rows parsed from: {}",
                        output.stdout
                    )))
                } else {
                    Ok(updates)
                }
            }
            _ => Err(AgentError::CommandFailed {
                op: "dnf check-update",
                exit_code: exit_code_of(output.status),
                stderr: output.stderr,
            }),
        }
    }

    fn install(&self, packages: &[String]) -> Result<TaskId, AgentError> {
        self.run_dnf("install", "dnf install", packages)
    }

    fn remove(&self, packages: &[String]) -> Result<TaskId, AgentError> {
        self.run_dnf("remove", "dnf remove", packages)
    }

    fn task_status(&self, id: &TaskId) -> Result<TaskStatus, AgentError> {
        lock_tasks(&self.tasks)
            .get(id)
            .cloned()
            .ok_or_else(|| AgentError::UnknownTask(id.0.clone()))
    }
}

fn lock_tasks(tasks: &TaskRegistry) -> MutexGuard<'_, HashMap<TaskId, TaskStatus>> {
    // Recover rather than panic if a previous task thread poisoned the
    // lock -- a lost stdout/stderr capture on one crashed task shouldn't
    // wedge every later `install`/`remove`/`task_status` call.
    tasks
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn exit_code_of(status: ExitStatus) -> Option<i32> {
    match status {
        ExitStatus::Code(code) => Some(code),
        _ => None,
    }
}

/// Parses `dnf check-update` output: a header, a blank line, then
/// `name.arch  new-version  repo` rows (whitespace-separated) until a
/// trailing blank line and/or an "Obsoleting Packages" section. Any line
/// that doesn't split into exactly three whitespace-separated fields is
/// skipped rather than erroring -- headers and section breaks are exactly
/// that shape.
fn parse_check_update(stdout: &str) -> Vec<PackageUpdate> {
    stdout
        .lines()
        .filter_map(|line| {
            let fields: Vec<&str> = line.split_whitespace().collect();
            if fields.len() != 3 {
                return None;
            }
            let name = fields[0]
                .rsplit_once('.')
                .map(|(name, _arch)| name)
                .unwrap_or(fields[0]);
            Some(PackageUpdate {
                name: name.to_string(),
                // `dnf check-update` reports only the candidate version,
                // not what's currently installed.
                current_version: String::new(),
                new_version: fields[1].to_string(),
                repo: fields[2].to_string(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::allowlist::AllowlistConfig;
    use platform_mock::process::MockSpawner;
    use std::time::{Duration, Instant};

    fn allowlist(packages: &[&str]) -> Arc<Allowlist> {
        Arc::new(Allowlist::new(AllowlistConfig {
            units: Vec::new(),
            packages: packages.iter().map(|s| s.to_string()).collect(),
            config_path_prefixes: Vec::new(),
        }))
    }

    fn wait_for_completion(controller: &DnfController, id: &TaskId) -> TaskStatus {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let status = controller.task_status(id).expect("task exists");
            if status.state != TaskState::Running || Instant::now() > deadline {
                return status;
            }
            thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn install_rejects_a_package_outside_the_allowlist() {
        let spawner: Arc<dyn Spawner + Send + Sync> = Arc::new(MockSpawner::new());
        let controller = DnfController::new(spawner, allowlist(&["htop"]));

        let err = controller
            .install(&["nmap".to_string()])
            .expect_err("nmap is not allowlisted");
        assert!(matches!(err, AgentError::PackageNotAllowed(_)));
    }

    #[test]
    fn install_rejects_the_whole_batch_if_any_package_is_disallowed() {
        let mock = MockSpawner::new();
        let spawned = mock.spawned.clone();
        let spawner: Arc<dyn Spawner + Send + Sync> = Arc::new(mock);
        let controller = DnfController::new(spawner, allowlist(&["htop"]));

        let err = controller
            .install(&["htop".to_string(), "nmap".to_string()])
            .expect_err("nmap in the batch is not allowlisted");
        assert!(matches!(err, AgentError::PackageNotAllowed(_)));
        assert!(
            spawned.lock().expect("lock").is_empty(),
            "no dnf command should run when any package in the batch is disallowed"
        );
    }

    #[test]
    fn a_successful_install_task_transitions_to_succeeded() {
        let spawner: Arc<dyn Spawner + Send + Sync> =
            Arc::new(MockSpawner::new().script_with_output(
                "dnf",
                ExitStatus::Code(0),
                b"Installed: htop\n".to_vec(),
            ));
        let controller = DnfController::new(spawner, allowlist(&["htop"]));

        let task_id = controller
            .install(&["htop".to_string()])
            .expect("allowlisted install starts");
        let status = wait_for_completion(&controller, &task_id);

        assert_eq!(status.state, TaskState::Succeeded);
        assert_eq!(status.exit_code, Some(0));
        assert!(status.stdout.unwrap_or_default().contains("Installed"));
    }

    #[test]
    fn a_failing_remove_task_transitions_to_failed() {
        let spawner: Arc<dyn Spawner + Send + Sync> =
            Arc::new(MockSpawner::new().script("dnf", ExitStatus::Code(1)));
        let controller = DnfController::new(spawner, allowlist(&["htop"]));

        let task_id = controller
            .remove(&["htop".to_string()])
            .expect("allowlisted remove starts");
        let status = wait_for_completion(&controller, &task_id);

        assert_eq!(status.state, TaskState::Failed);
        assert_eq!(status.exit_code, Some(1));
    }

    #[test]
    fn task_status_on_an_unknown_id_is_a_404_shaped_error() {
        let spawner: Arc<dyn Spawner + Send + Sync> = Arc::new(MockSpawner::new());
        let controller = DnfController::new(spawner, allowlist(&[]));

        let err = controller
            .task_status(&TaskId("nonexistent".to_string()))
            .expect_err("no such task");
        assert!(matches!(err, AgentError::UnknownTask(_)));
    }

    #[test]
    fn list_updates_with_no_updates_available_is_an_empty_list() {
        let spawner: Arc<dyn Spawner + Send + Sync> =
            Arc::new(MockSpawner::new().script("dnf", ExitStatus::Code(0)));
        let controller = DnfController::new(spawner, allowlist(&[]));

        assert_eq!(controller.list_updates().expect("no updates"), Vec::new());
    }

    #[test]
    fn list_updates_parses_available_updates() {
        let stdout = b"\n\
            htop.x86_64                3.3.1-1.fc43            updates\n\
            Obsoleting Packages\n\
            \n"
        .to_vec();
        let spawner: Arc<dyn Spawner + Send + Sync> =
            Arc::new(MockSpawner::new().script_with_output("dnf", ExitStatus::Code(100), stdout));
        let controller = DnfController::new(spawner, allowlist(&[]));

        let updates = controller.list_updates().expect("updates parse");
        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0].name, "htop");
        assert_eq!(updates[0].new_version, "3.3.1-1.fc43");
        assert_eq!(updates[0].repo, "updates");
    }

    #[test]
    fn list_updates_with_exit_100_but_no_parseable_rows_is_a_dnf_parse_error() {
        // Exit 100 promises updates are available; output this parser
        // can't make sense of contradicts that rather than meaning zero.
        let spawner: Arc<dyn Spawner + Send + Sync> =
            Arc::new(MockSpawner::new().script_with_output(
                "dnf",
                ExitStatus::Code(100),
                b"some future dnf output format we don't understand\n".to_vec(),
            ));
        let controller = DnfController::new(spawner, allowlist(&[]));

        let err = controller
            .list_updates()
            .expect_err("unparseable exit-100 output should not silently mean no updates");
        assert!(matches!(err, AgentError::DnfParse(_)));
    }
}
