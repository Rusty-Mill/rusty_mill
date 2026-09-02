//! Weighted endpoint selection.
//!
//! Deterministic round-robin over a weighted ring rather than random choice.
//! Randomness only reaches the configured ratio in expectation, which means a
//! low-traffic route can sit lopsided for a long time and a test can never
//! assert anything exactly. Round-robin hits the ratio precisely every cycle.

use std::sync::atomic::{AtomicU64, Ordering};

use http::uri::Authority;

/// Failure to build an endpoint set.
#[derive(Debug, thiserror::Error)]
pub enum BalanceError {
    /// A `host` value HTTP cannot parse as `host:port`.
    #[error("{at}: `{value}` is not a valid host:port authority")]
    Authority {
        /// Where in the configuration it came from.
        at: String,
        /// The offending text.
        value: String,
    },

    /// Every backend on the route has weight zero.
    #[error(
        "{at}: every backend has weight 0, so the route can never send traffic anywhere; \
         remove the route or give a backend a non-zero weight"
    )]
    NoWeight {
        /// Where in the configuration it came from.
        at: String,
    },

    /// The route named no backends at all.
    #[error("{at}: no backends")]
    Empty {
        /// Where in the configuration it came from.
        at: String,
    },
}

/// A weighted set of upstream authorities.
#[derive(Debug)]
pub struct Endpoints {
    /// `(authority, cumulative_weight)`, so selection is one scan.
    entries: Vec<(Authority, u32)>,
    total: u32,
    cursor: AtomicU64,
}

impl Endpoints {
    /// Build from `(host, weight)` pairs.
    ///
    /// A backend with weight 0 receives no traffic, per Gateway API. That is
    /// how a backend is drained without deleting its configuration — so it is
    /// dropped from the ring rather than treated as an error.
    pub fn new<'a>(
        backends: impl IntoIterator<Item = (&'a str, u32)>,
        at: &str,
    ) -> Result<Self, BalanceError> {
        let mut entries = Vec::new();
        let mut total: u32 = 0;
        let mut saw_any = false;

        for (host, weight) in backends {
            saw_any = true;
            let authority = Authority::try_from(host).map_err(|_| BalanceError::Authority {
                at: at.to_string(),
                value: host.to_string(),
            })?;
            if weight == 0 {
                continue;
            }
            total = total.saturating_add(weight);
            entries.push((authority, total));
        }

        if !saw_any {
            return Err(BalanceError::Empty { at: at.to_string() });
        }
        if entries.is_empty() {
            return Err(BalanceError::NoWeight { at: at.to_string() });
        }

        Ok(Endpoints {
            entries,
            total,
            cursor: AtomicU64::new(0),
        })
    }

    /// The next endpoint to try.
    pub fn next(&self) -> &Authority {
        self.pick(self.cursor.fetch_add(1, Ordering::Relaxed))
    }

    /// The endpoint for a given step of the cycle.
    fn pick(&self, step: u64) -> &Authority {
        let offset = (step % u64::from(self.total)) as u32;
        for (authority, cumulative) in &self.entries {
            if offset < *cumulative {
                return authority;
            }
        }
        // Unreachable while `total` is the last cumulative weight, but a
        // panicking proxy is worse than a slightly unfair one.
        &self.entries[0].0
    }

    /// How many endpoints can receive traffic.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether no endpoint can receive traffic.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn counts(endpoints: &Endpoints, requests: usize) -> Vec<(String, usize)> {
        let mut tally: Vec<(String, usize)> = Vec::new();
        for _ in 0..requests {
            let picked = endpoints.next().to_string();
            match tally.iter_mut().find(|(name, _)| *name == picked) {
                Some((_, count)) => *count += 1,
                None => tally.push((picked, 1)),
            }
        }
        tally
    }

    #[test]
    fn equal_weights_alternate_evenly() {
        let endpoints = Endpoints::new([("a:80", 1), ("b:80", 1)], "test").expect("should build");
        let tally = counts(&endpoints, 100);
        assert_eq!(tally.len(), 2);
        for (name, count) in tally {
            assert_eq!(count, 50, "{name} should get exactly half");
        }
    }

    #[test]
    fn weights_are_honoured_exactly_over_a_cycle() {
        let endpoints = Endpoints::new([("a:80", 1), ("b:80", 9)], "test").expect("should build");
        let tally = counts(&endpoints, 100);

        let a = tally.iter().find(|(n, _)| n == "a:80").expect("a").1;
        let b = tally.iter().find(|(n, _)| n == "b:80").expect("b").1;
        assert_eq!((a, b), (10, 90), "round-robin hits the ratio exactly");
    }

    #[test]
    fn a_zero_weight_backend_is_drained_not_rejected() {
        // Weight 0 is how a backend is taken out of rotation without deleting
        // its configuration.
        let endpoints = Endpoints::new([("a:80", 0), ("b:80", 1)], "test").expect("should build");
        assert_eq!(endpoints.len(), 1);
        let tally = counts(&endpoints, 10);
        assert_eq!(tally, vec![("b:80".to_string(), 10)]);
    }

    #[test]
    fn all_weights_zero_is_a_config_error() {
        let err = Endpoints::new([("a:80", 0), ("b:80", 0)], "route[0]")
            .expect_err("a route that can never route is a mistake");
        assert!(err.to_string().contains("route[0]"), "got: {err}");
        assert!(err.to_string().contains("weight 0"), "got: {err}");
    }

    #[test]
    fn an_unparseable_host_is_rejected_at_build_time() {
        let err = Endpoints::new([("not a host", 1)], "route[0]").expect_err("should not build");
        assert!(err.to_string().contains("not a host"), "got: {err}");
    }

    #[test]
    fn selection_wraps_without_overflowing() {
        let endpoints = Endpoints::new([("a:80", 1), ("b:80", 1)], "test").expect("build");
        // Near the wrap point of the cursor: picking must stay in range.
        assert!(!endpoints.pick(u64::MAX).to_string().is_empty());
    }
}
