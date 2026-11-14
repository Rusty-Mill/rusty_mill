//! `--pam`: the site's PAM policy decides whether an account may be served
//! (D266, D273). Account checks only; see `platform_linux::Pam`.
//!
//! A check runs on the blocking pool under [`DEADLINE`] (`pam_ldap` can
//! hang). A timeout leaves its thread running, so at most [`MAX_CHECKS`]
//! run at once, counted until the thread ends, and one more is refused.
//! Every error, timeout and refusal is an `Err`: PAM can only narrow
//! admission.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

/// How long one check may take before the connection is refused.
pub(crate) const DEADLINE: Duration = Duration::from_secs(5);
/// Checks that may be running (or hung) at once.
pub(crate) const MAX_CHECKS: usize = 16;

type Check = dyn Fn(&str, Option<&str>) -> Result<(), String> + Send + Sync;

/// The loaded PAM policy for one service. Cheap to clone.
#[derive(Clone)]
pub struct PamGate {
    check: Arc<Check>,
    running: Arc<AtomicUsize>,
}

impl PamGate {
    /// Loads libpam and checks the service file and a probe `pam_start`.
    /// Any `Err` means the front door must not start (fail closed).
    pub fn open(service: &str) -> Result<PamGate, String> {
        let pam = platform_linux::Pam::open(service)?;
        let service = service.to_owned();
        Ok(PamGate::new(move |account, ruser| {
            pam.check(&service, account, ruser)
        }))
    }

    /// A gate over `check(account, ruser)`; `ruser` is the peer's local
    /// user, or `None` for a tailnet peer.
    pub fn new(
        check: impl Fn(&str, Option<&str>) -> Result<(), String> + Send + Sync + 'static,
    ) -> PamGate {
        PamGate {
            check: Arc::new(check),
            running: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// `Ok` if the policy lets `account` in now.
    pub(crate) async fn admit(&self, account: &str, ruser: Option<&str>) -> Result<(), String> {
        let slot = Slot::take(&self.running)?;
        let (check, account, ruser) = (
            self.check.clone(),
            account.to_owned(),
            ruser.map(str::to_owned),
        );
        let job = rusty_tokio::spawn_blocking(move || {
            let _slot = slot;
            check(&account, ruser.as_deref())
        });
        match rusty_tokio::time::timeout(DEADLINE, job).await {
            Ok(Ok(verdict)) => verdict,
            Ok(Err(e)) => Err(format!("the PAM check failed: {e}")),
            Err(_) => Err("the PAM check timed out".into()),
        }
    }
}

impl std::fmt::Debug for PamGate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PamGate")
    }
}

impl PartialEq for PamGate {
    fn eq(&self, other: &PamGate) -> bool {
        Arc::ptr_eq(&self.check, &other.check)
    }
}
impl Eq for PamGate {}

/// One of the [`MAX_CHECKS`] places; given back when dropped.
struct Slot(Arc<AtomicUsize>);

impl Slot {
    fn take(running: &Arc<AtomicUsize>) -> Result<Slot, String> {
        if running.fetch_add(1, Ordering::SeqCst) >= MAX_CHECKS {
            running.fetch_sub(1, Ordering::SeqCst);
            return Err("too many PAM checks running".into());
        }
        Ok(Slot(running.clone()))
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_seventeenth_check_is_refused_until_one_ends() {
        let running = Arc::new(AtomicUsize::new(0));
        let slots: Vec<Slot> = (0..MAX_CHECKS)
            .map(|_| Slot::take(&running).unwrap())
            .collect();
        assert!(Slot::take(&running).is_err());
        assert_eq!(running.load(Ordering::SeqCst), MAX_CHECKS);
        drop(slots);
        assert!(Slot::take(&running).is_ok());
    }
}
