//! What keeps one peer from using up the hub: deadlines, caps on concurrent
//! connections, and a per-sender relay rate. Defaults suit a small mesh on
//! the open internet; tests shrink them.

use std::collections::HashMap;
use std::hash::Hash;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// The hub's resource limits.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Limits {
    /// Longest to wait for an HTTP request head or a request body.
    pub head_timeout: Duration,
    /// Longest for the Noise handshake or the DERP handshake.
    pub handshake_timeout: Duration,
    /// Concurrent TCP connections the hub serves; more are closed.
    pub max_connections: usize,
    /// Concurrent netmap streams; more get `503`.
    pub max_map_streams: usize,
    /// Concurrent netmap streams one machine may hold; more get `503`.
    pub max_map_streams_per_machine: usize,
    /// Concurrent HTTP/2 streams on one Noise connection.
    pub max_streams_per_connection: u32,
    /// Concurrent DERP sessions; more are refused.
    pub max_derp_sessions: usize,
    /// Sustained relay rate per sending node, in bytes per second.
    pub derp_bytes_per_sec: u32,
    /// Burst a sending node may relay at once; at least one packet (64 KiB).
    pub derp_burst: u32,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            head_timeout: Duration::from_secs(10),
            handshake_timeout: Duration::from_secs(10),
            max_connections: 4096,
            max_map_streams: 1024,
            max_map_streams_per_machine: 2,
            max_streams_per_connection: 16,
            max_derp_sessions: 2048,
            derp_bytes_per_sec: 8 << 20,
            derp_burst: 2 << 20,
        }
    }
}

/// A count of live things, capped. Cloning shares the count.
#[derive(Debug, Clone, Default)]
pub struct Counter(Arc<AtomicUsize>);

/// One slot of a [`Counter`]; frees it on drop.
#[derive(Debug)]
pub struct Slot(Arc<AtomicUsize>);

impl Counter {
    /// Takes a slot unless `limit` are already in use.
    pub fn acquire(&self, limit: usize) -> Option<Slot> {
        let taken = self
            .0
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < limit).then_some(n + 1)
            })
            .is_ok();
        taken.then(|| Slot(self.0.clone()))
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

/// Counts of live things per key, each capped. Cloning shares the counts.
#[derive(Debug)]
pub struct Keyed<K>(Arc<Mutex<HashMap<K, usize>>>);

impl<K> Default for Keyed<K> {
    fn default() -> Self {
        Self(Arc::default())
    }
}

/// One slot of a [`Keyed`] counter; frees it on drop.
#[derive(Debug)]
pub struct KeySlot<K: Hash + Eq> {
    counts: Arc<Mutex<HashMap<K, usize>>>,
    key: K,
}

impl<K: Hash + Eq + Clone> Keyed<K> {
    /// Takes a slot for `key` unless it already holds `limit`.
    pub fn acquire(&self, key: &K, limit: usize) -> Option<KeySlot<K>> {
        let mut counts = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let held = counts.entry(key.clone()).or_default();
        if *held >= limit {
            if *held == 0 {
                counts.remove(key);
            }
            return None;
        }
        *held += 1;
        Some(KeySlot {
            counts: self.0.clone(),
            key: key.clone(),
        })
    }
}

impl<K: Hash + Eq> Drop for KeySlot<K> {
    fn drop(&mut self) {
        let mut counts = self.counts.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(held) = counts.get_mut(&self.key) {
            *held -= 1;
            if *held == 0 {
                counts.remove(&self.key);
            }
        }
    }
}

/// A token bucket: `rate` bytes per second up to `burst`. A packet that
/// does not fit is refused, not delayed; the tunnel above is lossy by design.
#[derive(Debug)]
pub struct Bucket {
    rate: f64,
    burst: f64,
    tokens: f64,
    last: Instant,
}

impl Bucket {
    pub fn new(rate: u32, burst: u32, now: Instant) -> Self {
        Self {
            rate: f64::from(rate),
            burst: f64::from(burst),
            tokens: f64::from(burst),
            last: now,
        }
    }

    /// Spends `bytes` if the bucket holds them at `now`.
    pub fn allow(&mut self, bytes: usize, now: Instant) -> bool {
        let elapsed = now.saturating_duration_since(self.last).as_secs_f64();
        self.tokens = (self.tokens + elapsed * self.rate).min(self.burst);
        self.last = now;
        let need = bytes as f64;
        if need > self.tokens {
            return false;
        }
        self.tokens -= need;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_counter_hands_out_at_most_its_limit_and_frees_on_drop() {
        let c = Counter::default();
        let a = c.acquire(2).expect("first");
        let _b = c.acquire(2).expect("second");
        assert!(c.acquire(2).is_none(), "third is over the limit");
        drop(a);
        assert!(c.acquire(2).is_some(), "a freed slot is reusable");
    }

    #[test]
    fn a_zero_limit_admits_nothing() {
        assert!(Counter::default().acquire(0).is_none());
    }

    #[test]
    fn a_bucket_allows_a_burst_then_refills_at_its_rate() {
        let t0 = Instant::now();
        let mut b = Bucket::new(1000, 3000, t0);
        assert!(b.allow(3000, t0), "the burst");
        assert!(!b.allow(1, t0), "empty");
        let t1 = t0 + Duration::from_millis(500);
        assert!(b.allow(500, t1), "half a second refills 500 bytes");
        assert!(!b.allow(1, t1), "and no more");
    }

    #[test]
    fn a_bucket_never_holds_more_than_its_burst() {
        let t0 = Instant::now();
        let mut b = Bucket::new(1000, 2000, t0);
        let later = t0 + Duration::from_secs(3600);
        assert!(b.allow(2000, later));
        assert!(!b.allow(1, later));
    }

    #[test]
    fn a_packet_larger_than_the_burst_is_always_refused() {
        let t0 = Instant::now();
        let mut b = Bucket::new(1000, 100, t0);
        assert!(!b.allow(101, t0 + Duration::from_secs(10)));
    }
}
