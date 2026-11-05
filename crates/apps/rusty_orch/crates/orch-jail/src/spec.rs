//! `jail.json`: what orch writes for one agent session and `lel-jail` reads.
//!
//! Sans-IO. [`JailFile`] is the wire shape (unknown fields refused);
//! [`JailSpec::parse`] validates it once, at the boundary, so the launcher
//! only ever sees a spec that holds the v0 contract (LEL D85).

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};

use rusty_sandbox::{HelperRequest, Limits, SandboxSpec, Sockets};
use rusty_serde::{Deserialize, Serialize};

/// Where every session cgroup must live.
pub const CGROUP_ROOT: &str = "/sys/fs/cgroup/";
/// Variables the mux could use to reach its control socket (D13).
const MUX_PREFIX: &str = "RUSTY_MUX_";

/// `jail.json` as written: plain strings, every field explicit.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[rusty_serde(deny_unknown_fields)]
pub struct JailFile {
    /// The session's cgroup directory, which the launcher joins.
    pub cgroup: String,
    /// The setup-error file orch created; empty after a successful exec.
    pub status: String,
    /// The agent's working directory, inside a write root.
    pub cwd: String,
    /// Readable and executable roots.
    pub read: Vec<String>,
    /// Writable roots.
    pub write: Vec<String>,
    /// Hard rlimits.
    pub limits: FileLimits,
    /// The agent's complete environment.
    pub env: BTreeMap<String, String>,
    /// The orch-minted token, by reference, so it never travels in the mux
    /// `Launch` or in this file.
    #[rusty_serde(default)]
    pub token: Option<FileToken>,
    /// The program to exec (looked up on the environment's `PATH`).
    pub program: String,
    /// Its arguments.
    pub args: Vec<String>,
}

/// The `limits` object of `jail.json`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[rusty_serde(deny_unknown_fields)]
pub struct FileLimits {
    /// CPU seconds, at least 1.
    pub cpu_secs: u64,
    /// Address-space limit in bytes.
    pub memory_bytes: u64,
    /// Largest writable file in bytes.
    pub file_bytes: u64,
    /// Open descriptor limit.
    pub open_files: u64,
    /// Process limit.
    pub processes: u64,
}

/// The `token` object of `jail.json`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[rusty_serde(deny_unknown_fields)]
pub struct FileToken {
    /// File holding the token, outside every root the agent can reach.
    pub file: String,
    /// Variable the agent receives it in.
    pub var: String,
}

/// A token the launcher reads and hands over in one variable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Token {
    /// File holding the token.
    pub file: PathBuf,
    /// Variable the agent receives it in.
    pub var: String,
}

/// A validated jail spec.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JailSpec {
    cgroup: PathBuf,
    status: PathBuf,
    sandbox: SandboxSpec,
    token: Option<Token>,
    program: OsString,
    args: Vec<OsString>,
}

impl JailSpec {
    /// Parses and validates `jail.json`.
    ///
    /// # Errors
    /// A message naming the first field that breaks the v0 contract.
    pub fn parse(json: &str) -> Result<Self, String> {
        let file: JailFile =
            rusty_serde::json::from_str(json).map_err(|e| format!("jail.json: {e}"))?;
        Self::validate(file)
    }

    /// Validates an already-decoded [`JailFile`].
    ///
    /// # Errors
    /// A message naming the first field that breaks the v0 contract.
    pub fn validate(file: JailFile) -> Result<Self, String> {
        let cgroup = cgroup_path(&file.cgroup)?;
        let status = absolute("status", &file.status)?;
        if let Some(name) = file.env.keys().find(|name| name.starts_with(MUX_PREFIX)) {
            return Err(format!(
                "env: {name} would point the agent at the mux (D13)"
            ));
        }
        if file.program.is_empty() {
            return Err("program: empty".into());
        }
        let l = file.limits;
        let cpu = core::time::Duration::from_secs(l.cpu_secs);
        // The helper enforces no wall clock; orch and the holder do (design §3).
        let limits = Limits::new(
            cpu,
            cpu,
            l.memory_bytes,
            l.file_bytes,
            l.open_files,
            l.processes,
        )
        .map_err(|e| format!("limits: {e}"))?;
        let sandbox = SandboxSpec::new(
            paths(&file.read),
            paths(&file.write),
            PathBuf::from(&file.cwd),
            file.env.into_iter().collect(),
            limits,
        )
        .map_err(|e| format!("sandbox: {e}"))?;
        let token = file.token.map(|t| token(&sandbox, t)).transpose()?;
        Ok(Self {
            cgroup,
            status,
            sandbox,
            token,
            program: file.program.into(),
            args: file.args.into_iter().map(Into::into).collect(),
        })
    }

    /// The cgroup directory to join.
    #[must_use]
    pub fn cgroup(&self) -> &Path {
        &self.cgroup
    }

    /// The setup-error file.
    #[must_use]
    pub fn status(&self) -> &Path {
        &self.status
    }

    /// The token reference, if the session has one.
    #[must_use]
    pub fn token(&self) -> Option<&Token> {
        self.token.as_ref()
    }

    /// The helper request, with the token's value added to the environment.
    /// Sockets are fixed to [`Sockets::InternetOnly`]: no Unix or abstract
    /// socket can be created, so a leaked mux path is useless (design §4).
    #[must_use]
    pub fn helper_request(&self, token_value: Option<String>) -> HelperRequest {
        let limits = self.sandbox.limits();
        let mut env = self.sandbox.env().to_vec();
        if let (Some(token), Some(value)) = (&self.token, token_value) {
            env.push((token.var.clone(), value));
        }
        HelperRequest {
            status: self.status.clone(),
            cwd: self.sandbox.cwd().clone(),
            cpu_secs: limits.cpu().as_secs(),
            memory_bytes: limits.memory_bytes(),
            file_bytes: limits.file_bytes(),
            open_files: limits.open_files(),
            processes: limits.processes(),
            read: self.sandbox.read_roots().to_vec(),
            write: self.sandbox.write_roots().to_vec(),
            env,
            sockets: Sockets::InternetOnly,
            program: self.program.clone(),
            args: self.args.clone(),
        }
    }
}

fn paths(raw: &[String]) -> Vec<PathBuf> {
    raw.iter().map(PathBuf::from).collect()
}

fn absolute(field: &str, raw: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(raw);
    if !path.is_absolute() || path.components().any(|c| c == Component::ParentDir) {
        return Err(format!(
            "{field}: {raw:?} is not an absolute path without .."
        ));
    }
    Ok(path)
}

fn cgroup_path(raw: &str) -> Result<PathBuf, String> {
    let path = absolute("cgroup", raw)?;
    let root = Path::new(CGROUP_ROOT);
    if !path.starts_with(root) || path == root {
        return Err(format!("cgroup: {raw:?} is not below {CGROUP_ROOT}"));
    }
    Ok(path)
}

fn token(sandbox: &SandboxSpec, raw: FileToken) -> Result<Token, String> {
    let file = absolute("token.file", &raw.file)?;
    if sandbox.can_reach(&file) {
        return Err(format!("token.file: {} is inside a sandbox root", raw.file));
    }
    if raw.var.is_empty() || raw.var.contains('=') || raw.var.starts_with(MUX_PREFIX) {
        return Err(format!("token.var: {:?} is not a usable name", raw.var));
    }
    if sandbox.env().iter().any(|(name, _)| *name == raw.var) {
        return Err(format!("token.var: {} is already in env", raw.var));
    }
    Ok(Token { file, var: raw.var })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn good() -> String {
        r#"{
          "cgroup": "/sys/fs/cgroup/lel/t/c/1",
          "status": "/srv/b/c/1/status",
          "cwd": "/srv/wt/c",
          "read": ["/usr", "/lib"],
          "write": ["/srv/wt/c"],
          "limits": {"cpu_secs": 60, "memory_bytes": 1073741824, "file_bytes": 1048576,
                     "open_files": 256, "processes": 64},
          "env": {"PATH": "/usr/bin", "TERM": "xterm-256color"},
          "token": {"file": "/srv/tokens/c-1", "var": "LEL_TOKEN"},
          "program": "agent",
          "args": ["--acp-fd", "4"]
        }"#
        .into()
    }

    fn with(from: &str, to: &str) -> Result<JailSpec, String> {
        let json = good();
        assert!(json.contains(from), "fixture lacks {from}");
        JailSpec::parse(&json.replace(from, to))
    }

    #[test]
    fn a_good_spec_becomes_an_internet_only_request_with_the_token() {
        let spec = JailSpec::parse(&good()).unwrap();
        assert_eq!(spec.cgroup(), Path::new("/sys/fs/cgroup/lel/t/c/1"));
        let req = spec.helper_request(Some("secret".into()));
        assert_eq!(req.sockets, Sockets::InternetOnly);
        assert!(req.env.contains(&("LEL_TOKEN".into(), "secret".into())));
        assert_eq!(req.args, [OsString::from("--acp-fd"), "4".into()]);
        assert_eq!(req.cpu_secs, 60);
    }

    #[test]
    fn an_unknown_field_is_refused() {
        let err = with(r#""program""#, r#""sockets": "internet", "program""#).unwrap_err();
        assert!(err.contains("jail.json"), "{err}");
    }

    #[test]
    fn relative_and_escaping_paths_are_refused() {
        assert!(with("/srv/b/c/1/status", "status")
            .unwrap_err()
            .contains("status"));
        assert!(with(r#""/usr""#, r#""usr""#)
            .unwrap_err()
            .contains("sandbox"));
        let err = with("/sys/fs/cgroup/lel/t/c/1", "/sys/fs/cgroup/../x").unwrap_err();
        assert!(err.contains("cgroup"), "{err}");
    }

    #[test]
    fn a_cgroup_outside_the_cgroup_tree_is_refused() {
        assert!(with("/sys/fs/cgroup/lel/t/c/1", "/tmp/cg").is_err());
        assert!(with("/sys/fs/cgroup/lel/t/c/1", "/sys/fs/cgroup/").is_err());
    }

    #[test]
    fn a_cwd_outside_the_write_roots_is_refused() {
        let err = with(r#""cwd": "/srv/wt/c""#, r#""cwd": "/usr""#).unwrap_err();
        assert!(err.contains("sandbox"), "{err}");
    }

    #[test]
    fn a_mux_variable_is_refused_in_env_and_as_the_token_var() {
        let err = with(r#""TERM""#, r#""RUSTY_MUX_SOCKET""#).unwrap_err();
        assert!(err.contains("D13"), "{err}");
        let err = with(r#""LEL_TOKEN""#, r#""RUSTY_MUX_SOCKET""#).unwrap_err();
        assert!(err.contains("token.var"), "{err}");
    }

    #[test]
    fn a_token_inside_a_root_or_shadowing_env_is_refused() {
        let err = with("/srv/tokens/c-1", "/srv/wt/c/token").unwrap_err();
        assert!(err.contains("inside a sandbox root"), "{err}");
        let err = with(r#""LEL_TOKEN""#, r#""PATH""#).unwrap_err();
        assert!(err.contains("already in env"), "{err}");
    }

    #[test]
    fn zero_limits_and_an_empty_program_are_refused() {
        assert!(with(r#""cpu_secs": 60"#, r#""cpu_secs": 0"#)
            .unwrap_err()
            .contains("limits"));
        assert!(with(r#""program": "agent""#, r#""program": """#)
            .unwrap_err()
            .contains("program"));
    }

    #[test]
    fn a_spec_without_a_token_adds_no_variable() {
        let json = good().replace(
            r#""token": {"file": "/srv/tokens/c-1", "var": "LEL_TOKEN"},"#,
            "",
        );
        let spec = JailSpec::parse(&json).unwrap();
        assert!(spec.token().is_none());
        assert_eq!(spec.helper_request(None).env.len(), 2);
    }
}
