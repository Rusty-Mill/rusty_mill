//! Update checking — the only thing this product ever puts on the network.
//!
//! The policy lives here; the transport does not. `inventory-core` links no
//! HTTP client at all, which turns "the app makes no network calls" from a
//! promise into something you can verify from `cargo tree`. A shell that wants
//! update checks supplies an [`UpdateTransport`], and the user can switch it
//! off entirely.

use crate::db;
use crate::index::Inventory;
use crate::model::now_unix;
use crate::Result;

const SETTING_UPDATES: &str = "update_check_enabled";
const SETTING_LAST_CHECK: &str = "update_last_check_at";
const CHECK_INTERVAL_SECS: i64 = 24 * 3600;

/// Fetches a version manifest. Implemented outside the core so that the core
/// itself stays network-free.
pub trait UpdateTransport {
    /// Return the latest published version string, e.g. "1.0.2".
    fn latest_version(&self) -> Result<String>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateStatus {
    Disabled,
    /// Checked recently; no request was made.
    NotDue,
    UpToDate {
        current: String,
    },
    Available {
        current: String,
        latest: String,
    },
}

/// Compare dotted numeric versions. Non-numeric trailing parts (`-beta`) sort
/// before the same version without them.
pub fn is_newer(candidate: &str, current: &str) -> bool {
    let parts = |v: &str| -> (Vec<u64>, bool) {
        let core = v.trim().trim_start_matches('v');
        let (nums, pre) = match core.split_once(['-', '+']) {
            Some((n, _)) => (n, true),
            None => (core, false),
        };
        (
            nums.split('.')
                .map(|p| p.parse::<u64>().unwrap_or(0))
                .collect(),
            pre,
        )
    };
    let (a, a_pre) = parts(candidate);
    let (b, b_pre) = parts(current);

    for i in 0..a.len().max(b.len()) {
        let (x, y) = (
            a.get(i).copied().unwrap_or(0),
            b.get(i).copied().unwrap_or(0),
        );
        if x != y {
            return x > y;
        }
    }
    // Equal numerically: a release beats a pre-release of the same number.
    b_pre && !a_pre
}

impl Inventory {
    pub fn update_checks_enabled(&self) -> Result<bool> {
        Ok(db::get_setting(self.connection(), SETTING_UPDATES)?.as_deref() != Some("off"))
    }

    pub fn set_update_checks_enabled(&self, on: bool) -> Result<()> {
        db::set_setting(
            self.connection(),
            SETTING_UPDATES,
            if on { "on" } else { "off" },
        )
    }

    /// Check for a newer release, at most once a day.
    ///
    /// `force` bypasses the interval but not the user's opt-out: turning
    /// update checks off means no request is ever made.
    pub fn check_for_update(
        &self,
        transport: &dyn UpdateTransport,
        force: bool,
    ) -> Result<UpdateStatus> {
        if !self.update_checks_enabled()? {
            return Ok(UpdateStatus::Disabled);
        }
        let last: i64 = db::get_setting(self.connection(), SETTING_LAST_CHECK)?
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        if !force && now_unix() - last < CHECK_INTERVAL_SECS {
            return Ok(UpdateStatus::NotDue);
        }

        let latest = transport.latest_version()?;
        db::set_setting(
            self.connection(),
            SETTING_LAST_CHECK,
            &now_unix().to_string(),
        )?;

        let current = crate::VERSION.to_string();
        Ok(if is_newer(&latest, &current) {
            UpdateStatus::Available { current, latest }
        } else {
            UpdateStatus::UpToDate { current }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_versions() {
        assert!(is_newer("1.0.2", "1.0.1"));
        assert!(is_newer("1.1.0", "1.0.9"));
        assert!(is_newer("2.0.0", "1.99.99"));
        assert!(!is_newer("1.0.1", "1.0.1"));
        assert!(!is_newer("1.0.0", "1.0.1"));
        assert!(is_newer("v1.0.2", "1.0.1"));
        // A release supersedes its own pre-release; the reverse does not.
        assert!(is_newer("1.0.2", "1.0.2-beta"));
        assert!(!is_newer("1.0.2-beta", "1.0.2"));
        // Shorter is not automatically older.
        assert!(!is_newer("1.0", "1.0.0"));
    }
}
