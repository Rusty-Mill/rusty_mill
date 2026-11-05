//! The jail an orch-launched agent runs in when a mux block hosts its pty
//! (Let Em Loose D82, mux-hosted-agents-design.md §2 step 5 and §4).
//!
//! orch writes a [`JailFile`] to `<dir>/builders/<claim>/<n>/jail.json` and
//! asks the mux to run `lel-jail <jail.json>` on a pty, with the slot lock
//! on fd 3 and an ACP socket on fd 4. `lel-jail` joins the session's
//! cgroup, keeps only fds 0-4, then confines itself with rusty_sandbox
//! (Landlock, seccomp refusing every socket but IPv4/IPv6, rlimits) and
//! execs the agent. v0 adds no namespaces; the #726 layers come later
//! behind the same file (D85).

//!
//! Linux only: a jail spec names Linux paths (`/sys/fs/cgroup/...`, roots
//! Landlock confines), and `rusty_sandbox` judges them by the host's path
//! rules, so the spec and the launcher are built for Linux and nowhere
//! else (D85). Elsewhere `lel-jail` refuses with exit code 125.

#[cfg(target_os = "linux")]
pub mod spec;

#[cfg(target_os = "linux")]
pub mod launch;

#[cfg(target_os = "linux")]
pub use spec::{FileLimits, FileToken, JailFile, JailSpec, Resolved, Token, CGROUP_ROOT};
