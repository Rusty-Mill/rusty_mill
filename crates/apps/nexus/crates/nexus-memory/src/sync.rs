//! Client sync engine — push/pull against a `nexus-memory-hub`.
//!
//! Mirrors the hub's wire protocol (see `nexus-memory-hub`): the client pushes
//! its memories newer than a keyset cursor to `POST /sync/push`, and pulls
//! everyone else's from `GET /sync/pull` (excluding its own node), applying each
//! with last-write-wins ([`MemoryDb::upsert_lww`]). Cursors persist in
//! `sync_state` so each run resumes where it left off.
//!
//! Deletes propagate as tombstones (C36, #389): [`MemoryDb::delete`] flips
//! `status = 'deleted'` and bumps `updated_at` instead of issuing a SQL
//! `DELETE`, so the row is still visible to [`MemoryDb::list_since`] (this
//! module's push scan) and gets forwarded like any other edit. The hub needs
//! no changes — it stores records as opaque JSON keyed on `id`/`updated_at`,
//! so a `status: "deleted"` payload round-trips through push/pull exactly
//! like every other field. Known v1 limitation shared with regular edits: a
//! delete only pushes if this node authored the memory (`node_id` unset or
//! ours) — deleting a foreign-authored memory tombstones it locally but
//! doesn't propagate outward (see `push`'s authorship filter below); an
//! authorship-agnostic change outbox would be needed to close that gap.
//!
//! Config (`hub_url`, `secret`, `node_id`) is supplied per call by the caller
//! (CLI/MCP/shell/scheduler), so this stays decoupled from any config file.
//! `hub_url` is therefore attacker-reachable through any caller that can
//! trigger this IPC handler (e.g. a malicious MCP tool call), so it's run
//! through the same SSRF address-class guard `nexus-linkpreview` applies to
//! its fetches (see `is_blocked_address` below) before either `push` or
//! `pull` connects. Legitimate self-hosted hubs on a loopback/private
//! address need the caller to opt in explicitly via `allow_private_hub`
//! (arg) / `NEXUS_MEMORY_ALLOW_PRIVATE_HUB` (env) — off by default.

use std::net::{IpAddr, SocketAddr, ToSocketAddrs};
use std::time::Duration;

use serde_json::{json, Value};

use crate::db::MemoryDb;
use crate::model::Memory;

/// Per-request HTTP timeout.
const HTTP_TIMEOUT: Duration = Duration::from_secs(30);
/// Records per push/pull page.
const BATCH: usize = 200;
/// Epoch cursor — "everything" when no cursor is stored yet.
const EPOCH: &str = "1970-01-01T00:00:00+00:00";
/// Hard cap on a `/sync/pull` response body, checked before JSON-decoding.
/// Mirrors `nexus-linkpreview`'s `MAX_BODY_BYTES`: an attacker-controlled or
/// compromised hub could otherwise stream an unbounded body and OOM the
/// host before `serde_json` ever sees it. Generous relative to a
/// `BATCH`-record (200) page of plain-text `Memory` rows.
const MAX_PULL_BODY_BYTES: usize = 16 * 1024 * 1024;

const PUSH_TS: &str = "sync.push.updated_at";
const PUSH_ID: &str = "sync.push.id";
const PULL_TS: &str = "sync.pull.updated_at";
const PULL_ID: &str = "sync.pull.id";

/// Resolved hub connection config.
struct HubConfig {
    url: String,
    secret: String,
    node_id: String,
    /// Bypass the SSRF address-class guard for `url`. Off by default; only
    /// meant for a hub deliberately self-hosted on a loopback/private
    /// address. See `NEXUS_MEMORY_ALLOW_PRIVATE_HUB`.
    allow_private_hub: bool,
}

/// Resolve hub config: explicit `args` first, then `NEXUS_MEMORY_*` env vars.
/// Letting the secret come from the environment means callers (dashboard, MCP,
/// a scheduler) can trigger `sync` without passing it in IPC args.
fn parse_config(args: &Value) -> Result<HubConfig, String> {
    let resolve = |arg_key: &str, env_key: &str| -> Option<String> {
        args.get(arg_key)
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| std::env::var(env_key).ok())
            .filter(|s| !s.trim().is_empty())
    };
    let url = resolve("hub_url", "NEXUS_MEMORY_HUB_URL")
        .ok_or_else(|| "sync: no hub_url (pass it or set NEXUS_MEMORY_HUB_URL)".to_string())?;
    let secret = resolve("secret", "NEXUS_MEMORY_SYNC_SECRET")
        .ok_or_else(|| "sync: no secret (pass it or set NEXUS_MEMORY_SYNC_SECRET)".to_string())?;
    let node_id = resolve("node_id", "NEXUS_MEMORY_NODE_ID")
        .ok_or_else(|| "sync: no node_id (pass it or set NEXUS_MEMORY_NODE_ID)".to_string())?;
    let allow_private_hub = args
        .get("allow_private_hub")
        .and_then(Value::as_bool)
        .unwrap_or(false)
        || std::env::var("NEXUS_MEMORY_ALLOW_PRIVATE_HUB")
            .is_ok_and(|v| matches!(v.trim(), "1" | "true" | "yes"));
    Ok(HubConfig {
        url: url.trim_end_matches('/').to_string(),
        secret,
        node_id,
        allow_private_hub,
    })
}

/// Return `true` if `ip` is a non-public address that an outbound hub
/// connection must refuse: loopback (`127.0.0.0/8`, `::1`), link-local
/// (`169.254.0.0/16`, `fe80::/10` — also covers the AWS EC2 metadata IP
/// `169.254.169.254`), RFC1918 private (`10/8`, `172.16/12`, `192.168/16`),
/// shared address space (`100.64/10`, RFC6598), IPv4 broadcast
/// (`255.255.255.255`), IPv6 ULA (`fc00::/7`), unspecified (`0.0.0.0`,
/// `::`), multicast, or IPv4-mapped IPv6 of any of the above.
///
/// Mirrors `nexus-linkpreview::is_blocked_address` (issue #78) — duplicated
/// here rather than extracted into a shared crate since the two callers use
/// different reqwest clients (blocking vs. async) and pulling out a shared
/// crate would widen this fix beyond `nexus-memory`.
fn is_blocked_address(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            if v4.is_loopback() || v4.is_unspecified() || v4.is_multicast() || v4.is_broadcast() {
                return true;
            }
            let octs = v4.octets();
            v4.is_private()
                || v4.is_link_local()
                || (octs[0] == 100 && (64..128).contains(&octs[1]))
                || octs[0] == 0
        }
        IpAddr::V6(v6) => {
            if v6.is_loopback() || v6.is_unspecified() || v6.is_multicast() {
                return true;
            }
            if let Some(mapped) = v6.to_ipv4_mapped() {
                return is_blocked_address(IpAddr::V4(mapped));
            }
            let segs = v6.segments();
            if (segs[0] & 0xfe00) == 0xfc00 {
                return true;
            }
            if (segs[0] & 0xffc0) == 0xfe80 {
                return true;
            }
            false
        }
    }
}

/// Resolve `host`/`port`, refusing (unless `allow_private`) any resolved
/// address that's non-public per [`is_blocked_address`]. Returns the first
/// resolved address so the caller can pin the connection to it.
fn resolve_public_address(host: &str, port: u16, allow_private: bool) -> Result<IpAddr, String> {
    let addrs: Vec<SocketAddr> = (host, port)
        .to_socket_addrs()
        .map_err(|e| format!("sync: DNS resolution failed for hub host {host}: {e}"))?
        .collect();
    if !allow_private {
        for addr in &addrs {
            let ip = addr.ip();
            if is_blocked_address(ip) {
                return Err(format!(
                    "sync: hub_url host {host} resolves to non-public address {ip} — refused \
                     (set allow_private_hub to override for a self-hosted hub)"
                ));
            }
        }
    }
    addrs
        .first()
        .map(SocketAddr::ip)
        .ok_or_else(|| format!("sync: hub_url host {host} resolved to no addresses"))
}

/// Validate that `url` (already known to be http/https) doesn't resolve to
/// a non-public address, unless `allow_private` opts out. Returns the
/// validated IP so the caller can pin the connection to it.
fn validate_url_target(url: &reqwest::Url, allow_private: bool) -> Result<IpAddr, String> {
    let host = url
        .host_str()
        .ok_or_else(|| format!("sync: hub_url has no host: {url}"))?;
    if let Ok(ip) = host.parse::<IpAddr>() {
        if !allow_private && is_blocked_address(ip) {
            return Err(format!(
                "sync: hub_url targets non-public address {ip} — refused (set \
                 allow_private_hub to override for a self-hosted hub)"
            ));
        }
        return Ok(ip);
    }
    let port = url
        .port_or_known_default()
        .unwrap_or(if url.scheme() == "https" { 443 } else { 80 });
    resolve_public_address(host, port, allow_private)
}

/// Compute the DNS pin for `url` given the IP that just passed the SSRF
/// guard: the `(domain, socket_addr)` pair fed into
/// `ClientBuilder::resolve` so reqwest connects to exactly the validated
/// address instead of re-resolving (DNS rebinding). Returns `None` when the
/// URL's host is an IP literal — there's no DNS lookup to rebind.
fn dns_pin(url: &reqwest::Url, validated_ip: IpAddr) -> Option<(String, SocketAddr)> {
    let host = url.host_str()?;
    let bare = host
        .strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(host);
    if bare.parse::<IpAddr>().is_ok() {
        return None;
    }
    let port = url
        .port_or_known_default()
        .unwrap_or(if url.scheme() == "https" { 443 } else { 80 });
    Some((host.to_string(), SocketAddr::new(validated_ip, port)))
}

/// Run one full sync cycle (push then pull) against the hub in `args`.
/// Returns `{ pushed, pulled }`.
pub(crate) async fn sync(db: MemoryDb, args: &Value) -> Result<Value, String> {
    let cfg = parse_config(args)?;
    let parsed_url =
        reqwest::Url::parse(&cfg.url).map_err(|e| format!("sync: invalid hub_url: {e}"))?;
    if !matches!(parsed_url.scheme(), "http" | "https") {
        return Err(format!("sync: hub_url must be http or https: {}", cfg.url));
    }
    // SSRF guard — refuse a hub host that resolves to loopback / link-local
    // / private / metadata IP unless the caller explicitly opted in. Runs
    // once here (rather than duplicated in `push`/`pull`) since both share
    // this one client, pinned to the address that passed the check.
    let validated_ip = validate_url_target(&parsed_url, cfg.allow_private_hub)?;
    let mut builder = reqwest::Client::builder()
        .timeout(HTTP_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none());
    if let Some((domain, addr)) = dns_pin(&parsed_url, validated_ip) {
        builder = builder.resolve(&domain, addr);
    }
    let client = builder
        .build()
        .map_err(|e| format!("sync: http client: {e}"))?;
    let pushed = push(&db, &client, &cfg).await?;
    let pulled = pull(&db, &client, &cfg).await?;
    Ok(json!({ "pushed": pushed, "pulled": pulled }))
}

/// Push local memories newer than the stored push cursor, advancing it.
///
/// Only memories authored here (`node_id` unset or equal to our node) are sent,
/// and each is stamped with our `node_id` so other nodes record us as the
/// author — that author stamp is what lets every node skip re-pushing memories
/// it merely pulled (no echo). The cursor still advances over the whole scanned
/// page, so foreign rows are seen once and never re-scanned.
///
/// v1 limitation: edits *here* to a memory authored *elsewhere* are not pushed
/// (its `node_id` stays foreign); whole-store authorship-agnostic sync would
/// need an explicit change outbox.
async fn push(db: &MemoryDb, client: &reqwest::Client, cfg: &HubConfig) -> Result<u64, String> {
    let mut ts = db
        .sync_state_get(PUSH_TS)
        .map_err(de)?
        .unwrap_or_else(|| EPOCH.to_string());
    let mut id = db.sync_state_get(PUSH_ID).map_err(de)?.unwrap_or_default();
    let mut total = 0_u64;
    loop {
        let batch = db.list_since(&ts, &id, BATCH).map_err(de)?;
        if batch.is_empty() {
            break;
        }
        let records: Vec<Value> = batch
            .iter()
            .filter(|m| m.node_id.as_deref().is_none_or(|n| n == cfg.node_id))
            .filter_map(|m| {
                let mut v = serde_json::to_value(m).ok()?;
                v.as_object_mut()?
                    .insert("node_id".to_string(), json!(cfg.node_id));
                Some(v)
            })
            .collect();
        if !records.is_empty() {
            let sent = records.len() as u64;
            let resp = client
                .post(format!("{}/sync/push", cfg.url))
                .bearer_auth(&cfg.secret)
                .json(&json!({ "node_id": cfg.node_id, "records": records }))
                .send()
                .await
                .map_err(|e| format!("sync push: {e}"))?;
            if !resp.status().is_success() {
                return Err(format!("sync push: HTTP {}", resp.status()));
            }
            total += sent;
        }
        // Advance the cursor over the whole scanned page (keyset order), so
        // skipped foreign rows are not re-scanned next time.
        if let Some(last) = batch.last() {
            ts = last.updated_at.to_rfc3339();
            id = last.id.to_string();
            db.sync_state_set(PUSH_TS, &ts).map_err(de)?;
            db.sync_state_set(PUSH_ID, &id).map_err(de)?;
        }
        if batch.len() < BATCH {
            break;
        }
    }
    Ok(total)
}

/// Pull remote memories newer than the stored pull cursor, applying each with
/// last-write-wins and advancing the cursor.
async fn pull(db: &MemoryDb, client: &reqwest::Client, cfg: &HubConfig) -> Result<u64, String> {
    let mut ts = db
        .sync_state_get(PULL_TS)
        .map_err(de)?
        .unwrap_or_else(|| EPOCH.to_string());
    let mut id = db.sync_state_get(PULL_ID).map_err(de)?;
    let mut total = 0_u64;
    loop {
        let mut req = client
            .get(format!("{}/sync/pull", cfg.url))
            .bearer_auth(&cfg.secret)
            .query(&[
                ("since", ts.as_str()),
                ("exclude_node", cfg.node_id.as_str()),
                ("limit", "200"),
            ]);
        if let Some(sid) = id.as_deref() {
            req = req.query(&[("since_id", sid)]);
        }
        let resp = req.send().await.map_err(|e| format!("sync pull: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("sync pull: HTTP {}", resp.status()));
        }
        let bytes = read_capped_body(resp, MAX_PULL_BODY_BYTES).await?;
        let body: Value =
            serde_json::from_slice(&bytes).map_err(|e| format!("sync pull: decode: {e}"))?;
        let records = body
            .get("records")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if records.is_empty() {
            break;
        }
        // Advance the cursor across the whole page (even records we can't decode,
        // so a bad row never wedges the cursor), and apply the decodable ones.
        for rec in &records {
            if let Some(u) = rec.get("updated_at").and_then(Value::as_str) {
                ts = u.to_string();
            }
            if let Some(i) = rec.get("id").and_then(Value::as_str) {
                id = Some(i.to_string());
            }
            if let Ok(m) = serde_json::from_value::<Memory>(rec.clone()) {
                let _ = db.upsert_lww(&m).map_err(de)?;
            }
        }
        db.sync_state_set(PULL_TS, &ts).map_err(de)?;
        db.sync_state_set(PULL_ID, id.as_deref().unwrap_or(""))
            .map_err(de)?;
        total += records.len() as u64;
        if records.len() < BATCH {
            break;
        }
    }
    Ok(total)
}

/// Read `resp`'s body into memory, refusing anything over `cap` bytes
/// instead of buffering an unbounded stream before JSON-decoding it
/// (mirrors `nexus-linkpreview`'s `MAX_BODY_BYTES`, issue #78). Checks
/// `Content-Length` up front as a fast rejection, then still enforces the
/// cap against the actual bytes streamed in case the header is absent or
/// understates the real size.
async fn read_capped_body(mut resp: reqwest::Response, cap: usize) -> Result<Vec<u8>, String> {
    if let Some(len) = resp.content_length() {
        if len > cap as u64 {
            return Err(format!(
                "sync pull: response body too large ({len} bytes, cap {cap})"
            ));
        }
    }
    let mut buf = Vec::with_capacity(cap.min(64 * 1024));
    while let Some(chunk) = resp.chunk().await.map_err(|e| format!("sync pull: {e}"))? {
        if buf.len() + chunk.len() > cap {
            return Err(format!(
                "sync pull: response body exceeded cap of {cap} bytes"
            ));
        }
        buf.extend_from_slice(&chunk);
    }
    Ok(buf)
}

/// Map a db error to the engine's `String` error.
// By-value so it can be used directly as `.map_err(de)` at every call
// site (10+) instead of `.map_err(|e| de(&e))` everywhere.
#[allow(clippy::needless_pass_by_value)]
fn de(e: crate::db::MemoryDbError) -> String {
    format!("sync: db: {e}")
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use nexus_memory_hub::{AppState, HubStore};

    use super::*;

    const SECRET: &str = "test-secret";

    /// Spin up a real in-memory hub on a loopback ephemeral port; returns
    /// its base URL and its store (for direct seeding).
    async fn spawn_hub() -> (String, HubStore) {
        let store = HubStore::open_in_memory().unwrap();
        let state = AppState {
            store: store.clone(),
            secret: Arc::new(SECRET.to_string()),
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            nexus_memory_hub::serve(listener, state).await.unwrap();
        });
        (format!("http://{addr}"), store)
    }

    #[tokio::test]
    async fn sync_reports_unreachable_hub() {
        let db = MemoryDb::open_in_memory().unwrap();
        db.insert(&Memory::new("to push")).unwrap();
        // Port 1 refuses fast — deterministic connection failure. Loopback
        // is otherwise blocked by the SSRF guard (see the tests below), so
        // this test — about transport failure, not the guard — opts out.
        let err = sync(
            db,
            &json!({
                "hub_url": "http://127.0.0.1:1",
                "secret": "s",
                "node_id": "n",
                "allow_private_hub": true,
            }),
        )
        .await
        .unwrap_err();
        assert!(
            err.contains("sync push") || err.contains("sync pull"),
            "got: {err}"
        );
    }

    #[test]
    fn parse_config_prefers_args_and_trims_trailing_slash() {
        let cfg = parse_config(&json!({
            "hub_url": "http://host:8765/",
            "secret": "s",
            "node_id": "n"
        }))
        .unwrap();
        assert_eq!(cfg.url, "http://host:8765");
        assert_eq!(cfg.secret, "s");
        assert_eq!(cfg.node_id, "n");
        assert!(!cfg.allow_private_hub);
    }

    #[test]
    fn parse_config_reads_allow_private_hub_flag() {
        let cfg = parse_config(&json!({
            "hub_url": "http://host:8765/",
            "secret": "s",
            "node_id": "n",
            "allow_private_hub": true,
        }))
        .unwrap();
        assert!(cfg.allow_private_hub);
    }

    /// Pure-guard coverage for every address class the finding calls out
    /// (loopback, the AWS metadata link-local IP, RFC1918) — no network
    /// involved, since a literal-IP host short-circuits DNS entirely. The
    /// `allow_private_hub` override is proven to bypass the same check.
    #[test]
    fn validate_url_target_blocks_loopback_link_local_and_private() {
        for raw in [
            "http://127.0.0.1/x",
            "http://169.254.169.254/latest/meta-data/",
            "http://10.1.2.3/",
            "http://192.168.1.1/",
        ] {
            let url = reqwest::Url::parse(raw).unwrap();
            let err = validate_url_target(&url, false).unwrap_err();
            assert!(err.contains("non-public"), "{raw}: got {err}");
            assert!(validate_url_target(&url, true).is_ok(), "{raw}: override");
        }
    }

    /// End-to-end proof: a *real, working* hub listening on loopback is
    /// refused before `sync` ever connects to it. Pre-fix, this hub would
    /// have answered the (empty) push/pull cycle successfully — `sync`
    /// would return `Ok`, not `Err`, since there was nothing to block on.
    #[tokio::test]
    async fn sync_rejects_loopback_hub_without_override() {
        let (hub_url, _store) = spawn_hub().await;
        let db = MemoryDb::open_in_memory().unwrap();
        let err = sync(
            db,
            &json!({ "hub_url": hub_url, "secret": SECRET, "node_id": "n" }),
        )
        .await
        .unwrap_err();
        assert!(err.contains("non-public"), "got: {err}");
    }

    /// Same loopback hub, but with the explicit opt-out: sync must still
    /// work for a deliberately self-hosted local hub.
    #[tokio::test]
    async fn sync_allows_loopback_hub_with_explicit_override() {
        let (hub_url, _store) = spawn_hub().await;
        let db = MemoryDb::open_in_memory().unwrap();
        let result = sync(
            db,
            &json!({
                "hub_url": hub_url,
                "secret": SECRET,
                "node_id": "n",
                "allow_private_hub": true,
            }),
        )
        .await
        .unwrap();
        assert_eq!(result["pulled"], 0);
    }

    /// A hub response whose body exceeds `MAX_PULL_BODY_BYTES` is refused
    /// instead of being buffered whole and handed to `serde_json`. Uses
    /// `allow_private_hub` to isolate the cap from the address guard above.
    #[tokio::test]
    async fn sync_pull_rejects_oversized_response_body() {
        let (hub_url, store) = spawn_hub().await;
        let oversized = "x".repeat(MAX_PULL_BODY_BYTES + 1024);
        store
            .push(
                "seed-node",
                &[json!({
                    "id": "11111111-1111-1111-1111-111111111111",
                    "updated_at": "2024-01-01T00:00:00+00:00",
                    "junk": oversized,
                })],
            )
            .unwrap();
        let db = MemoryDb::open_in_memory().unwrap();
        let err = sync(
            db,
            &json!({
                "hub_url": hub_url,
                "secret": SECRET,
                "node_id": "n",
                "allow_private_hub": true,
            }),
        )
        .await
        .unwrap_err();
        assert!(
            err.contains("too large") || err.contains("exceeded cap"),
            "got: {err}"
        );
    }
}
