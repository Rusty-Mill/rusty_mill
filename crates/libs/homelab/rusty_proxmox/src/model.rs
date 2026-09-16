//! Small enums describing which guest type and which action -- everything
//! else this crate hands back is Proxmox's own JSON, unopinionated, since
//! the exact response shape differs by node/guest configuration and is
//! already documented by the Proxmox API viewer at
//! `https://<host>:8006/pve-docs/api-viewer/`.

use std::fmt;

/// A Proxmox guest is either a QEMU/KVM virtual machine or an LXC container
/// -- distinct resource types under distinct API paths
/// (`/nodes/{node}/qemu/...` vs. `/nodes/{node}/lxc/...`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestKind {
    /// A QEMU/KVM virtual machine.
    Qemu,
    /// An LXC container.
    Lxc,
}

impl GuestKind {
    /// The path segment Proxmox uses for this guest type.
    pub fn as_str(self) -> &'static str {
        match self {
            GuestKind::Qemu => "qemu",
            GuestKind::Lxc => "lxc",
        }
    }
}

impl fmt::Display for GuestKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A power action on a guest, sent as `POST .../status/{action}`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerAction {
    /// Start the guest.
    Start,
    /// Hard stop (pull the power cord -- no clean shutdown inside the guest).
    Stop,
    /// Ask the guest to shut down cleanly (ACPI event for QEMU, `init 0` for
    /// LXC).
    Shutdown,
    /// Reboot the guest.
    Reboot,
    /// Suspend the guest to disk/RAM.
    Suspend,
    /// Resume a suspended guest.
    Resume,
}

impl PowerAction {
    /// The path segment Proxmox uses for this action.
    pub fn as_str(self) -> &'static str {
        match self {
            PowerAction::Start => "start",
            PowerAction::Stop => "stop",
            PowerAction::Shutdown => "shutdown",
            PowerAction::Reboot => "reboot",
            PowerAction::Suspend => "suspend",
            PowerAction::Resume => "resume",
        }
    }
}

impl fmt::Display for PowerAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}
