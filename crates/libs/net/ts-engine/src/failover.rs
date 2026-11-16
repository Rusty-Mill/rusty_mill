//! Failover: keep control and DERP alive across a hub restart or promotion.
//!
//! The hub advertises its standbys in every netmap (`StandbyURLs`). When a
//! connection drops, the engine retries the URL it was given, then each
//! standby, with a short backoff, until one answers or the engine is gone.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ts_control::ControlClient;
use ts_derp::DerpClient;
use ts_key::NodeState;
use ts_types::tailcfg::{Hostinfo, MapResponse};

use crate::Trust;
use rusty_tokio::sync::mpsc;

const MIN_BACKOFF: Duration = Duration::from_millis(500);
const MAX_BACKOFF: Duration = Duration::from_secs(5);

/// Standby URLs the hub last advertised, and whether the engine has stopped;
/// shared by the engine and its poll task.
#[derive(Clone, Default)]
pub(crate) struct Standbys {
    urls: Arc<Mutex<Vec<String>>>,
    stopped: Arc<AtomicBool>,
}

impl Standbys {
    /// Replaces the list (a netmap without one leaves it alone).
    pub(crate) fn set(&self, urls: Vec<String>) {
        if let Ok(mut g) = self.urls.lock() {
            *g = urls;
        }
    }

    /// The engine's loop has ended: reconnect tasks should give up.
    pub(crate) fn stop(&self) {
        self.stopped.store(true, Ordering::Relaxed);
    }

    fn stopped(&self) -> bool {
        self.stopped.load(Ordering::Relaxed)
    }

    /// `primary` first, then each standby once, in advertised order.
    pub(crate) fn order(&self, primary: &str) -> Vec<String> {
        let mut out = vec![primary.to_string()];
        let list = self.urls.lock().map(|g| g.clone()).unwrap_or_default();
        for u in list {
            if !out.contains(&u) {
                out.push(u);
            }
        }
        out
    }
}

fn next(delay: Duration) -> Duration {
    (delay * 2).min(MAX_BACKOFF)
}

/// Reconnects to DERP, trying `primary` then the standbys, until one answers.
/// `None` once the engine has stopped.
pub(crate) async fn reconnect_derp(
    primary: &str,
    standbys: &Standbys,
    state: &NodeState,
    trust: &Trust,
) -> Option<DerpClient> {
    let mut delay = MIN_BACKOFF;
    while !standbys.stopped() {
        for url in standbys.order(primary) {
            match DerpClient::connect_with(&url, &state.node, trust).await {
                Ok(c) => {
                    rusty_log::info!(%url, "engine: reconnected to DERP");
                    return Some(c);
                }
                Err(e) => rusty_log::debug!(%url, "engine: DERP reconnect failed: {e}"),
            }
        }
        rusty_tokio::time::sleep(delay).await;
        delay = next(delay);
    }
    None
}

/// Everything the control poll task needs to reconnect.
pub(crate) struct ControlLink {
    pub primary: String,
    pub authkey: String,
    pub hostinfo: Hostinfo,
    pub trust: Trust,
    pub machine: ts_key::MachinePrivate,
    pub node_key: ts_types::NodePublic,
    pub disco_key: ts_types::DiscoPublic,
    pub endpoints: Vec<std::net::SocketAddr>,
    pub standbys: Standbys,
    pub netmap_tx: mpsc::UnboundedSender<MapResponse>,
}

impl ControlLink {
    async fn open(&self, url: &str) -> Result<ControlClient, ts_control::ClientError> {
        let c = ControlClient::connect_with(
            url,
            self.machine.clone(),
            self.hostinfo.clone(),
            self.trust.clone(),
        )
        .await?;
        c.register(self.node_key, &self.authkey).await?;
        Ok(c)
    }

    /// Streams the netmap from `first`, then from whichever hub answers after
    /// it drops, until the engine stops listening.
    pub(crate) async fn run(self, first: ControlClient) {
        let mut current = Some(first);
        let mut delay = MIN_BACKOFF;
        while !self.standbys.stopped() {
            let urls = self.standbys.order(&self.primary);
            for url in urls {
                let client = match current.take() {
                    Some(c) => c,
                    None => match self.open(&url).await {
                        Ok(c) => {
                            rusty_log::info!(%url, "engine: control reconnected");
                            c
                        }
                        Err(e) => {
                            rusty_log::debug!(%url, "engine: control reconnect failed: {e}");
                            continue;
                        }
                    },
                };
                delay = MIN_BACKOFF;
                let r = client
                    .poll_netmap(
                        self.node_key,
                        self.disco_key,
                        self.endpoints.clone(),
                        |resp| {
                            if let Some(urls) = resp.standby_urls.clone() {
                                self.standbys.set(urls);
                            }
                            match self.netmap_tx.send(resp) {
                                Ok(()) => std::ops::ControlFlow::Continue(()),
                                Err(_) => std::ops::ControlFlow::Break(()),
                            }
                        },
                    )
                    .await;
                if self.standbys.stopped() {
                    return;
                }
                if let Err(e) = r {
                    rusty_log::warn!("engine: netmap poll ended: {e}");
                }
                break; // restart from the primary
            }
            rusty_tokio::time::sleep(delay).await;
            delay = next(delay);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn order_puts_the_primary_first_and_drops_duplicates() {
        let s = Standbys::default();
        assert_eq!(s.order("a"), ["a"]);
        s.set(vec!["b".into(), "a".into(), "c".into(), "b".into()]);
        assert_eq!(s.order("a"), ["a", "b", "c"]);
    }

    #[test]
    fn backoff_doubles_up_to_the_cap() {
        assert_eq!(next(MIN_BACKOFF), Duration::from_secs(1));
        assert_eq!(next(Duration::from_secs(4)), MAX_BACKOFF);
    }
}
