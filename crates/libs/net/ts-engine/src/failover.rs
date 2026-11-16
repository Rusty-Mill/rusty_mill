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
        *self.urls.lock().unwrap_or_else(|e| e.into_inner()) = urls;
    }

    /// The engine's loop has ended: reconnect tasks should give up.
    pub(crate) fn stop(&self) {
        self.stopped.store(true, Ordering::Relaxed);
    }

    pub(crate) fn stopped(&self) -> bool {
        self.stopped.load(Ordering::Relaxed)
    }

    /// `primary` first, then each standby once, in advertised order.
    pub(crate) fn order(&self, primary: &str) -> Vec<String> {
        let mut out = vec![primary.to_string()];
        let list = self.urls.lock().unwrap_or_else(|e| e.into_inner()).clone();
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
    node: &ts_key::NodePrivate,
    trust: &Trust,
) -> Option<DerpClient> {
    let mut delay = MIN_BACKOFF;
    while !standbys.stopped() {
        for url in standbys.order(primary) {
            match DerpClient::connect_with(&url, node, trust).await {
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
    /// The control key seen at first start; a hub that answers with another
    /// is not the one this node joined, and gets nothing.
    pub pinned: ts_types::MachinePublic,
    pub node_key: ts_types::NodePublic,
    pub disco_key: ts_types::DiscoPublic,
    pub endpoints: Vec<std::net::SocketAddr>,
    pub standbys: Standbys,
    pub netmap_tx: mpsc::UnboundedSender<MapResponse>,
}

/// How one netmap stream ended.
struct Ended {
    /// Frames delivered before it ended.
    frames: usize,
    /// The engine stopped listening.
    engine_gone: bool,
    result: Result<(), ts_control::ClientError>,
}

impl ControlLink {
    /// Connects to `url` (fetching its key only) and checks the key.
    async fn open(&self, url: &str) -> Option<ControlClient> {
        let c = ControlClient::connect_with(
            url,
            self.machine.clone(),
            self.hostinfo.clone(),
            self.trust.clone(),
        )
        .await
        .inspect_err(|e| rusty_log::debug!(%url, "engine: control reconnect failed: {e}"))
        .ok()?;
        if c.control_key() != self.pinned {
            rusty_log::warn!(%url, "engine: refusing a control server with a different key");
            return None;
        }
        Some(c)
    }

    async fn stream(&self, client: &ControlClient) -> Ended {
        let (mut frames, mut engine_gone) = (0, false);
        let result = client
            .poll_netmap(
                self.node_key,
                self.disco_key,
                self.endpoints.clone(),
                |resp| {
                    if let Some(urls) = resp.standby_urls.clone() {
                        self.standbys.set(urls);
                    }
                    if self.netmap_tx.send(resp).is_err() {
                        engine_gone = true;
                        return std::ops::ControlFlow::Break(());
                    }
                    frames += 1;
                    std::ops::ControlFlow::Continue(())
                },
            )
            .await;
        Ended {
            frames,
            engine_gone,
            result,
        }
    }

    /// Streams from `client`. A hub that already knows this machine (the
    /// usual case after a restart or promotion) needs no pre-auth key, which
    /// may have expired or been rotated out; only one that refuses the poll
    /// without a single frame gets a registration and a second try.
    async fn follow(&self, client: &ControlClient) -> Ended {
        let first = self.stream(client).await;
        if first.frames > 0 || first.engine_gone || self.standbys.stopped() {
            return first;
        }
        if client.register(self.node_key, &self.authkey).await.is_err() {
            return first;
        }
        self.stream(client).await
    }

    /// Streams the netmap from `first`, then from whichever hub answers after
    /// it drops, until the engine stops listening.
    pub(crate) async fn run(self, first: ControlClient) {
        let mut current = Some(first);
        let mut delay = MIN_BACKOFF;
        while !self.standbys.stopped() {
            for url in self.standbys.order(&self.primary) {
                let client = match current.take() {
                    Some(c) => c,
                    None => match self.open(&url).await {
                        Some(c) => c,
                        None => continue,
                    },
                };
                let ended = self.follow(&client).await;
                if ended.engine_gone {
                    return;
                }
                if let Err(e) = ended.result {
                    rusty_log::warn!(%url, "engine: netmap poll ended: {e}");
                }
                if ended.frames > 0 {
                    delay = MIN_BACKOFF;
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

    #[rusty_tokio::test]
    async fn reconnecting_gives_up_once_the_engine_has_stopped() {
        let standbys = Standbys::default();
        let node = ts_key::NodePrivate::generate();
        let stopper = standbys.clone();
        rusty_tokio::spawn(async move {
            rusty_tokio::time::sleep(Duration::from_millis(300)).await;
            stopper.stop();
        });
        // Nothing listens on port 1: every attempt fails until the stop.
        let got = rusty_tokio::time::timeout(
            Duration::from_secs(10),
            reconnect_derp("http://127.0.0.1:1", &standbys, &node, &Trust::System),
        )
        .await
        .expect("it stops retrying");
        assert!(got.is_none());
    }
}
