//! The heavy HTML panels of an analysis (3D viewer, scoring report, ballchasing
//! dashboard: ~93% of its bytes), held server-side so `/api/analyze` can return
//! just the data and the UI fetches a panel the first time its tab is opened.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use crate::pipeline::Analysis;
use crate::store::AccountId;

/// Analyses whose panels are kept; each is ~9 MB, so this bounds memory.
const KEEP: usize = 4;

/// The three panels of one analysis.
#[derive(Debug, Default, PartialEq)]
pub struct Panels {
    viewer: String,
    scoring: String,
    ballchasing: String,
}

impl Panels {
    /// Move the panels out of `a`, leaving them empty (so they drop out of its JSON).
    pub fn take(a: &mut Analysis) -> Self {
        Self {
            viewer: std::mem::take(&mut a.viewer_html),
            scoring: std::mem::take(&mut a.scoring_html),
            ballchasing: std::mem::take(&mut a.ballchasing_html),
        }
    }

    /// One panel by its URL name, or `None` for an unknown name.
    pub fn get(&self, name: &str) -> Option<&str> {
        match name {
            "viewer" => Some(&self.viewer),
            "scoring" => Some(&self.scoring),
            "ballchasing" => Some(&self.ballchasing),
            _ => None,
        }
    }
}

/// The most recent analyses' panels, per account: one account can never read
/// another's, even given the id.
#[derive(Default)]
pub struct PanelCache {
    entries: Mutex<VecDeque<(String, String, Arc<Panels>)>>,
}

impl PanelCache {
    /// Keep `panels` for `account`'s analysis `id`, evicting the oldest past `KEEP`.
    pub fn put(&self, account: &AccountId, id: &str, panels: Panels) {
        let Ok(mut q) = self.entries.lock() else {
            return;
        };
        q.retain(|(a, i, _)| !(a == account.as_str() && i == id));
        q.push_back((account.as_str().into(), id.into(), Arc::new(panels)));
        while q.len() > KEEP {
            q.pop_front();
        }
    }

    pub fn get(&self, account: &AccountId, id: &str) -> Option<Arc<Panels>> {
        let q = self.entries.lock().ok()?;
        q.iter()
            .find(|(a, i, _)| a == account.as_str() && i == id)
            .map(|(_, _, p)| Arc::clone(p))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn acct(s: &str) -> AccountId {
        AccountId::new(s).expect("valid account")
    }
    fn panels(tag: &str) -> Panels {
        Panels {
            viewer: tag.into(),
            ..Panels::default()
        }
    }

    #[test]
    fn a_panel_is_served_only_to_its_owner_and_only_by_known_name() {
        let cache = PanelCache::default();
        cache.put(&acct("ann"), "r1", panels("v"));
        let p = cache.get(&acct("ann"), "r1").expect("owner sees it");
        assert_eq!((p.get("viewer"), p.get("nope")), (Some("v"), None));
        assert!(
            cache.get(&acct("bob"), "r1").is_none(),
            "not another account's"
        );
        assert!(cache.get(&acct("ann"), "r2").is_none());
    }

    #[test]
    fn the_oldest_analyses_are_evicted_and_a_resubmit_replaces() {
        let cache = PanelCache::default();
        let a = acct("ann");
        for i in 0..=KEEP {
            cache.put(&a, &format!("r{i}"), panels("x"));
        }
        assert!(cache.get(&a, "r0").is_none(), "oldest evicted");
        assert!(cache.get(&a, &format!("r{KEEP}")).is_some());
        cache.put(&a, "r1", panels("new"));
        assert_eq!(cache.get(&a, "r1").unwrap().get("viewer"), Some("new"));
    }
}
