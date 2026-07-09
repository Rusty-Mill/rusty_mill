//! On-disk node identity state.
//!
//! Our own JSON format (not Go's `tailscaled.state`): one file,
//! `ts-rs.state.json`, holding the three private keys. Created with mode
//! 0600. Compatibility with Go state files is a non-goal — identities are
//! per-daemon, not portable.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::{DiscoPrivate, MachinePrivate, NodePrivate};

const STATE_FILE: &str = "ts-rs.state.json";

#[derive(Debug, thiserror::Error)]
pub enum StateError {
    #[error("state I/O error on {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("corrupt state file {path}: {reason}")]
    Corrupt { path: PathBuf, reason: String },
}

/// The persistent identity of one node.
pub struct NodeState {
    pub machine: MachinePrivate,
    pub node: NodePrivate,
    pub disco: DiscoPrivate,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct StateFile {
    machine_key: String,
    node_key: String,
    disco_key: String,
}

impl NodeState {
    /// Generates a fresh identity.
    pub fn generate() -> Self {
        Self {
            machine: MachinePrivate::generate(),
            node: NodePrivate::generate(),
            disco: DiscoPrivate::generate(),
        }
    }

    /// Loads state from `dir`, or generates-and-saves a fresh identity if no
    /// state file exists yet.
    pub fn load_or_generate(dir: &Path) -> Result<Self, StateError> {
        let path = dir.join(STATE_FILE);
        match fs::read_to_string(&path) {
            Ok(contents) => Self::parse(&contents, &path),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let state = Self::generate();
                state.save(dir)?;
                Ok(state)
            }
            Err(source) => Err(StateError::Io { path, source }),
        }
    }

    fn parse(contents: &str, path: &Path) -> Result<Self, StateError> {
        let corrupt = |reason: String| StateError::Corrupt {
            path: path.to_path_buf(),
            reason,
        };
        let raw: StateFile = serde_json::from_str(contents).map_err(|e| corrupt(e.to_string()))?;
        Ok(Self {
            machine: raw
                .machine_key
                .parse()
                .map_err(|e| corrupt(format!("machine_key: {e}")))?,
            node: raw
                .node_key
                .parse()
                .map_err(|e| corrupt(format!("node_key: {e}")))?,
            disco: raw
                .disco_key
                .parse()
                .map_err(|e| corrupt(format!("disco_key: {e}")))?,
        })
    }

    /// Writes the state file (mode 0600) into `dir`, creating `dir` if
    /// needed.
    pub fn save(&self, dir: &Path) -> Result<(), StateError> {
        let path = dir.join(STATE_FILE);
        let io_err = |source| StateError::Io {
            path: path.clone(),
            source,
        };
        fs::create_dir_all(dir).map_err(io_err)?;
        let raw = StateFile {
            machine_key: self.machine.to_state_string(),
            node_key: self.node.to_state_string(),
            disco_key: self.disco.to_state_string(),
        };
        let json = serde_json::to_string_pretty(&raw).expect("state serializes");

        let mut opts = fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut f = opts.open(&path).map_err(io_err)?;
        f.write_all(json.as_bytes()).map_err(io_err)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_or_generate_round_trips() {
        let dir = std::env::temp_dir().join(format!("ts-key-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);

        let first = NodeState::load_or_generate(&dir).unwrap();
        let second = NodeState::load_or_generate(&dir).unwrap();
        assert_eq!(first.machine.to_bytes(), second.machine.to_bytes());
        assert_eq!(first.node.to_bytes(), second.node.to_bytes());
        assert_eq!(first.disco.to_bytes(), second.disco.to_bytes());

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(dir.join(STATE_FILE))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600, "state file must be 0600");
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn corrupt_state_is_an_error_not_a_panic() {
        let dir = std::env::temp_dir().join(format!("ts-key-corrupt-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(STATE_FILE), "{not json").unwrap();
        assert!(matches!(
            NodeState::load_or_generate(&dir),
            Err(StateError::Corrupt { .. })
        ));
        let _ = fs::remove_dir_all(&dir);
    }
}
