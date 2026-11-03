//! The dedicated container a host-cleared provider runs in (#594, Nano
//! 2026-11-03): Codex runs only inside it, through `podman exec`, never on
//! the runner's host.
//!
//! The operator names the container and its exact mounts in the
//! assertions file ([`super::assertions`]). Before every call, the runner
//! must find that container through its own rootless podman, with
//! exactly those mounts, and confirm that its `/etc/machine-id` is mounted
//! read-only from a file the runner cannot change and holds the named id.

use std::path::PathBuf;

/// Where a container's machine id is mounted.
pub const MACHINE_ID_PATH: &str = "/etc/machine-id";

/// A container the operator cleared for one provider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContainerSpec {
    /// The container's name in the runner's podman.
    pub name: String,
    /// What the container's `/etc/machine-id` must hold.
    pub machine_id: String,
    /// Every mount the container may have, and no others. Always includes
    /// the read-only bind of [`MACHINE_ID_PATH`].
    pub mounts: Vec<Mount>,
}

/// One mount the operator allows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mount {
    /// A host path bound into the container.
    Bind {
        source: PathBuf,
        destination: String,
        rw: bool,
    },
    /// A named podman volume.
    Volume { name: String, destination: String },
}

impl Mount {
    /// Where the mount appears inside the container.
    pub fn destination(&self) -> &str {
        match self {
            Self::Bind { destination, .. } | Self::Volume { destination, .. } => destination,
        }
    }
}

impl ContainerSpec {
    /// The host file holding the container's machine id: the source of
    /// the read-only bind at [`MACHINE_ID_PATH`].
    pub fn machine_id_source(&self) -> Option<&PathBuf> {
        self.mounts.iter().find_map(|m| match m {
            Mount::Bind {
                source,
                destination,
                rw: false,
            } if destination == MACHINE_ID_PATH => Some(source),
            _ => None,
        })
    }
}
