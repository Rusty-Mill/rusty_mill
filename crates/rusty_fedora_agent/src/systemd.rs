//! [`SystemController`] over `systemctl`/`journalctl`, shelled out to
//! through a `rustils` [`Spawner`] -- and, for `system_status`, direct
//! reads of the handful of `/proc` files that answer it (no subprocess
//! needed, and nothing there is scoped by the allowlist -- it's read-only
//! host info, not a controllable unit/package/path).

use std::sync::Arc;

use platform::process::Spawner;

use crate::allowlist::Allowlist;
use crate::domain::{
    JournalLine, JournalQuery, ServiceAction, ServiceSummary, SystemStatus, UnitType,
};
use crate::error::AgentError;
use crate::ports::SystemController;
use crate::process_util::run_checked;

/// Default line count for `fedora_read_journal` when the caller omits
/// `lines`.
const DEFAULT_JOURNAL_LINES: u32 = 100;

pub struct SystemdAdapter {
    spawner: Arc<dyn Spawner + Send + Sync>,
    allowlist: Arc<Allowlist>,
}

impl SystemdAdapter {
    pub fn new(spawner: Arc<dyn Spawner + Send + Sync>, allowlist: Arc<Allowlist>) -> Self {
        Self { spawner, allowlist }
    }
}

impl SystemController for SystemdAdapter {
    fn list_services(
        &self,
        unit_type: Option<UnitType>,
    ) -> Result<Vec<ServiceSummary>, AgentError> {
        let type_arg = match unit_type {
            Some(t) => t.as_systemctl_type().to_string(),
            None => "service,timer,socket".to_string(),
        };
        let args = vec![
            "list-units".to_string(),
            "--all".to_string(),
            "--no-legend".to_string(),
            "--plain".to_string(),
            format!("--type={type_arg}"),
        ];
        let stdout = run_checked(
            self.spawner.as_ref(),
            "systemctl list-units",
            "systemctl",
            &args,
        )?;
        Ok(parse_list_units(&stdout))
    }

    fn control_service(&self, name: &str, action: ServiceAction) -> Result<(), AgentError> {
        // Illegal unit names never reach `exec` -- checked before the
        // `Command` is ever built.
        self.allowlist.check_unit(name)?;

        let args = vec![action.as_systemctl_verb().to_string(), name.to_string()];
        run_checked(
            self.spawner.as_ref(),
            "systemctl control",
            "systemctl",
            &args,
        )?;
        Ok(())
    }

    fn read_journal(&self, query: JournalQuery) -> Result<Vec<JournalLine>, AgentError> {
        let mut args = vec![
            "--no-pager".to_string(),
            "-o".to_string(),
            "short-iso".to_string(),
        ];
        if let Some(unit) = &query.unit {
            args.push("-u".to_string());
            args.push(unit.clone());
        }
        args.push("-n".to_string());
        args.push(query.lines.unwrap_or(DEFAULT_JOURNAL_LINES).to_string());
        if let Some(since) = &query.since {
            args.push("--since".to_string());
            args.push(since.clone());
        }
        if let Some(priority) = query.priority {
            args.push("-p".to_string());
            args.push(priority.as_journalctl_value().to_string());
        }

        let stdout = run_checked(self.spawner.as_ref(), "journalctl", "journalctl", &args)?;
        Ok(stdout
            .lines()
            .map(|line| JournalLine {
                line: line.to_string(),
            })
            .collect())
    }

    fn system_status(&self) -> Result<SystemStatus, AgentError> {
        let uptime_seconds = read_uptime_seconds()?;
        let (load_average_1m, load_average_5m, load_average_15m) = read_load_average()?;
        let (mem_total_kb, mem_available_kb) = read_mem_info()?;

        Ok(SystemStatus {
            hostname: std::fs::read_to_string("/proc/sys/kernel/hostname")?
                .trim()
                .to_string(),
            kernel: std::fs::read_to_string("/proc/sys/kernel/osrelease")?
                .trim()
                .to_string(),
            os_pretty_name: read_os_pretty_name()?,
            uptime_seconds,
            load_average_1m,
            load_average_5m,
            load_average_15m,
            mem_total_kb,
            mem_available_kb,
        })
    }
}

/// Parses `systemctl list-units --no-legend --plain` output: whitespace-
/// separated `UNIT LOAD ACTIVE SUB DESCRIPTION...`, one unit per line.
fn parse_list_units(stdout: &str) -> Vec<ServiceSummary> {
    stdout
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let name = fields.next()?.to_string();
            let load_state = fields.next()?.to_string();
            let active_state = fields.next()?.to_string();
            let sub_state = fields.next()?.to_string();
            Some(ServiceSummary {
                name,
                load_state,
                active_state,
                sub_state,
            })
        })
        .collect()
}

fn read_uptime_seconds() -> Result<u64, AgentError> {
    let text = std::fs::read_to_string("/proc/uptime")?;
    let first = text
        .split_whitespace()
        .next()
        .ok_or_else(|| AgentError::InvalidRequest("/proc/uptime had no fields".to_string()))?;
    let seconds: f64 = first
        .parse()
        .map_err(|_| AgentError::InvalidRequest(format!("unparseable /proc/uptime: {first}")))?;
    Ok(seconds as u64)
}

fn read_load_average() -> Result<(f64, f64, f64), AgentError> {
    let text = std::fs::read_to_string("/proc/loadavg")?;
    let mut fields = text.split_whitespace();
    let parse_next = |fields: &mut std::str::SplitWhitespace| -> Result<f64, AgentError> {
        fields
            .next()
            .ok_or_else(|| {
                AgentError::InvalidRequest("/proc/loadavg had too few fields".to_string())
            })?
            .parse()
            .map_err(|_| AgentError::InvalidRequest("unparseable /proc/loadavg".to_string()))
    };
    let one = parse_next(&mut fields)?;
    let five = parse_next(&mut fields)?;
    let fifteen = parse_next(&mut fields)?;
    Ok((one, five, fifteen))
}

fn read_mem_info() -> Result<(u64, u64), AgentError> {
    let text = std::fs::read_to_string("/proc/meminfo")?;
    let mut total = None;
    let mut available = None;
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("MemTotal:") {
            total = parse_kb_field(rest);
        } else if let Some(rest) = line.strip_prefix("MemAvailable:") {
            available = parse_kb_field(rest);
        }
    }
    match (total, available) {
        (Some(total), Some(available)) => Ok((total, available)),
        _ => Err(AgentError::InvalidRequest(
            "/proc/meminfo missing MemTotal/MemAvailable".to_string(),
        )),
    }
}

fn parse_kb_field(rest: &str) -> Option<u64> {
    rest.split_whitespace().next()?.parse().ok()
}

fn read_os_pretty_name() -> Result<String, AgentError> {
    let text = std::fs::read_to_string("/etc/os-release")?;
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("PRETTY_NAME=") {
            return Ok(rest.trim_matches('"').to_string());
        }
    }
    Err(AgentError::InvalidRequest(
        "/etc/os-release missing PRETTY_NAME".to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::allowlist::AllowlistConfig;
    use platform::process::ExitStatus;
    use platform_mock::process::MockSpawner;

    fn allowlist(units: &[&str]) -> Arc<Allowlist> {
        Arc::new(Allowlist::new(AllowlistConfig {
            units: units.iter().map(|s| s.to_string()).collect(),
            packages: Vec::new(),
            config_path_prefixes: Vec::new(),
        }))
    }

    #[test]
    fn parses_list_units_output() {
        let out = "ollama.service loaded active running Ollama server\n\
                    cockpit.socket  loaded active listening Cockpit socket\n";
        let parsed = parse_list_units(out);
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].name, "ollama.service");
        assert_eq!(parsed[0].active_state, "active");
        assert_eq!(parsed[1].sub_state, "listening");
    }

    #[test]
    fn control_service_rejects_a_unit_outside_the_allowlist() {
        let spawner: Arc<dyn Spawner + Send + Sync> = Arc::new(MockSpawner::new());
        let adapter = SystemdAdapter::new(spawner, allowlist(&["ollama.service"]));

        let err = adapter
            .control_service("sshd.service", ServiceAction::Stop)
            .expect_err("sshd.service is not allowlisted");
        assert!(matches!(err, AgentError::UnitNotAllowed(_)));
    }

    #[test]
    fn control_service_never_spawns_when_the_unit_is_rejected() {
        let mock = MockSpawner::new();
        let spawned = mock.spawned.clone();
        let spawner: Arc<dyn Spawner + Send + Sync> = Arc::new(mock);
        let adapter = SystemdAdapter::new(spawner, allowlist(&["ollama.service"]));

        let _ = adapter.control_service("sshd.service", ServiceAction::Stop);
        assert!(
            spawned.lock().expect("lock").is_empty(),
            "an illegal unit name must never reach the spawner"
        );
    }

    #[test]
    fn control_service_allows_an_allowlisted_unit() {
        let spawner: Arc<dyn Spawner + Send + Sync> =
            Arc::new(MockSpawner::new().script("systemctl", ExitStatus::Code(0)));
        let adapter = SystemdAdapter::new(spawner, allowlist(&["ollama.service"]));

        adapter
            .control_service("ollama.service", ServiceAction::Restart)
            .expect("allowlisted unit succeeds");
    }

    #[test]
    fn read_journal_parses_lines() {
        let spawner: Arc<dyn Spawner + Send + Sync> =
            Arc::new(MockSpawner::new().script_with_output(
                "journalctl",
                ExitStatus::Code(0),
                b"line one\nline two\n".to_vec(),
            ));
        let adapter = SystemdAdapter::new(spawner, allowlist(&[]));

        let lines = adapter
            .read_journal(JournalQuery::default())
            .expect("journal read succeeds");
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].line, "line one");
    }
}
