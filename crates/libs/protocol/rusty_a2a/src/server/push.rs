//! Push notification delivery (spec Section 4.3): POSTs the current
//! [`Task`] to every [`TaskPushNotificationConfig`] registered for it
//! whenever its status or artifacts change.
//!
//! Delivery is best-effort and fire-and-forget: a slow, unreachable, or
//! error-returning webhook is logged (`tracing::warn!`) and otherwise has
//! no effect on task processing - in particular, it never blocks or fails
//! the [`AgentExecutor`](super::AgentExecutor) invocation that produced
//! the update.

use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use reqwest::Client;

use crate::types::{AuthenticationInfo, StreamResponse, Task, TaskPushNotificationConfig};

/// Spec Section 13.2: "Agents SHOULD implement reasonable timeout values
/// for webhook requests" (recommends 10-30s).
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);

/// One initial attempt plus this many retries.
const MAX_DELIVERY_ATTEMPTS: u32 = 4;

/// Base delay for the exponential backoff between retries (spec Section
/// 13.2: "Agents SHOULD implement retry with exponential backoff for
/// failed deliveries"): attempts are spaced `BASE_RETRY_DELAY * 2^n`
/// apart.
const BASE_RETRY_DELAY: Duration = Duration::from_millis(200);

#[derive(Clone)]
pub(crate) struct PushNotifier {
    client: Client,
    /// Spec Section 13.2 (SHOULD): reject a webhook URL that resolves to
    /// a private/loopback/link-local address, to guard against a
    /// malicious client registering a webhook that makes this agent
    /// probe its own internal network. Off by default - enabling it
    /// unconditionally would also reject the loopback addresses any
    /// local development or testing setup legitimately uses - see
    /// [`super::AgentServer::with_webhook_ssrf_protection`].
    ssrf_protection: bool,
}

impl PushNotifier {
    pub(crate) fn new() -> Self {
        PushNotifier {
            client: Client::builder()
                .timeout(REQUEST_TIMEOUT)
                .build()
                .expect("building the push-notification HTTP client should never fail"),
            ssrf_protection: false,
        }
    }

    pub(crate) fn set_ssrf_protection(&mut self, enabled: bool) {
        self.ssrf_protection = enabled;
    }

    /// If SSRF protection is enabled, rejects `url` the same way
    /// [`validate_webhook_url`] does - for use at registration time
    /// ([`super::engine::Engine::create_push_notification_config`] and
    /// the inline `taskPushNotificationConfig` path), so a client gets
    /// immediate feedback instead of a webhook that silently never
    /// fires.
    pub(crate) async fn check_webhook_url(&self, url: &str) -> std::result::Result<(), String> {
        if self.ssrf_protection {
            validate_webhook_url(url).await.map(|_| ())
        } else {
            Ok(())
        }
    }

    pub(crate) async fn notify(&self, config: &TaskPushNotificationConfig, task: &Task) {
        // Pinned once per delivery (covering every retry below) to the
        // exact addresses just validated, rather than trusting `reqwest`
        // to resolve `config.url`'s host again independently for the
        // real connection - see [`pinned_client`].
        let client = if self.ssrf_protection {
            let addrs = match validate_webhook_url(&config.url).await {
                Ok(addrs) => addrs,
                Err(reason) => {
                    tracing::warn!(
                        task_id = %task.id,
                        url = %config.url,
                        %reason,
                        "skipping push notification delivery to a disallowed webhook URL"
                    );
                    return;
                }
            };
            match pinned_client(&config.url, &addrs) {
                Ok(client) => client,
                Err(reason) => {
                    tracing::warn!(
                        task_id = %task.id,
                        url = %config.url,
                        %reason,
                        "skipping push notification delivery: failed to pin its validated address"
                    );
                    return;
                }
            }
        } else {
            self.client.clone()
        };

        // Spec Section 4.3.3: the webhook payload is a `StreamResponse`
        // object (i.e. `{"task": {...}}`), not the bare `Task`.
        let payload = StreamResponse::Task { task: task.clone() };

        for attempt in 1..=MAX_DELIVERY_ATTEMPTS {
            match self.attempt_delivery(&client, config, &payload).await {
                Ok(()) => return,
                Err(DeliveryFailure::Permanent(reason)) => {
                    tracing::warn!(
                        task_id = %task.id,
                        url = %config.url,
                        %reason,
                        "push notification delivery failed (not retrying)"
                    );
                    return;
                }
                Err(DeliveryFailure::Retryable(reason)) => {
                    tracing::warn!(
                        task_id = %task.id,
                        url = %config.url,
                        %reason,
                        attempt,
                        max_attempts = MAX_DELIVERY_ATTEMPTS,
                        "push notification delivery attempt failed"
                    );
                    if attempt == MAX_DELIVERY_ATTEMPTS {
                        tracing::warn!(
                            task_id = %task.id,
                            url = %config.url,
                            "giving up on push notification delivery after {attempt} attempts"
                        );
                        return;
                    }
                    tokio::time::sleep(BASE_RETRY_DELAY * 2u32.pow(attempt - 1)).await;
                }
            }
        }
    }

    async fn attempt_delivery(
        &self,
        client: &Client,
        config: &TaskPushNotificationConfig,
        payload: &StreamResponse,
    ) -> std::result::Result<(), DeliveryFailure> {
        let mut request = client.post(&config.url).json(payload);
        if let Some(token) = &config.token {
            request = request.header("X-A2A-Notification-Token", token);
        }
        if let Some(auth) = &config.authentication {
            request = apply_authentication(request, auth);
        }

        match request.send().await {
            Ok(response) if response.status().is_success() => Ok(()),
            Ok(response) => {
                let status = response.status();
                // 5xx and 429 are the receiver's own way of saying "try
                // again later"; anything else (4xx, notably) won't
                // succeed just by resending the identical request.
                if status.is_server_error() || status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                    Err(DeliveryFailure::Retryable(format!("non-2xx status {status}")))
                } else {
                    Err(DeliveryFailure::Permanent(format!("non-2xx status {status}")))
                }
            }
            Err(error) => Err(DeliveryFailure::Retryable(error.to_string())),
        }
    }
}

enum DeliveryFailure {
    /// Worth trying again: a network-level failure, or a status the
    /// receiver itself is using to ask for a retry (5xx, 429).
    Retryable(String),
    /// Resending the identical request won't change the outcome.
    Permanent(String),
}

fn apply_authentication(
    request: reqwest::RequestBuilder,
    auth: &AuthenticationInfo,
) -> reqwest::RequestBuilder {
    match &auth.credentials {
        Some(credentials) => request.header("Authorization", format!("{} {credentials}", auth.scheme)),
        None => request,
    }
}

/// Only ever called while SSRF protection is enabled (see
/// [`PushNotifier::check_webhook_url`]/[`PushNotifier::notify`]) - reject
/// a webhook URL whose host is (or resolves to) a private, loopback, or
/// link-local address - guards against SSRF, where a malicious client
/// registers a webhook that probes the agent's own internal network - and,
/// spec Section 13.2 (SHOULD, "Webhook URLs SHOULD use HTTPS to protect
/// payload confidentiality in transit"), require `https`: bundled under
/// this same opt-in flag since both guard the same delivery path, and
/// plain `http` would ship task content (which can carry message/artifact
/// data) in cleartext. A literal IP-address host is checked directly with
/// no DNS lookup involved; a hostname is resolved fresh every call
/// (rather than caching the result), so a webhook that resolved to a
/// private address the last time it was checked but a public one now is
/// re-evaluated correctly.
///
/// Returns the exact address(es) this check resolved and approved, so
/// callers (see [`pinned_client`]) can pin the real delivery connection
/// to one of them instead of letting the HTTP client perform its own,
/// later, independent DNS resolution - which would otherwise leave a
/// TOCTOU window for DNS rebinding (a hostname resolving to a public
/// address for this check, then to a private one moments later for an
/// independently-resolved real connection) to slip straight past this
/// check.
pub(crate) async fn validate_webhook_url(url: &str) -> std::result::Result<Vec<SocketAddr>, String> {
    let parsed = reqwest::Url::parse(url).map_err(|e| format!("invalid webhook URL: {e}"))?;
    match parsed.scheme() {
        "https" => {}
        other => {
            return Err(format!(
                "webhook URL scheme {other:?} is not allowed; only \"https\" is, to protect payload \
                 confidentiality in transit"
            ))
        }
    }
    let host = parsed
        .host_str()
        .ok_or_else(|| "webhook URL has no host".to_string())?;
    let port = parsed.port_or_known_default().unwrap_or(0);

    if let Ok(ip) = host.parse::<IpAddr>() {
        return if is_disallowed(ip) {
            Err(format!(
                "webhook URL host {host} is a private/loopback/link-local address, which is not allowed"
            ))
        } else {
            Ok(vec![SocketAddr::new(ip, port)])
        };
    }

    let addrs: Vec<SocketAddr> = tokio::net::lookup_host((host, port))
        .await
        .map_err(|e| format!("failed to resolve webhook URL host {host:?}: {e}"))?
        .collect();
    for addr in &addrs {
        if is_disallowed(addr.ip()) {
            return Err(format!(
                "webhook URL host {host:?} resolves to a private/loopback/link-local address ({}), which is not allowed",
                addr.ip()
            ));
        }
    }
    if addrs.is_empty() {
        Err(format!(
            "webhook URL host {host:?} did not resolve to any address"
        ))
    } else {
        Ok(addrs)
    }
}

/// Whether a webhook may not be delivered to `ip`: anything not a
/// globally routed unicast address (design review 3.6 widened this; IPv6
/// unique-local `fc00::/7`, for one, used to pass). An IPv4 address
/// embedded in IPv6 (mapped `::ffff:0:0/96`, NAT64 `64:ff9b::/96`) is
/// judged as the IPv4 address it reaches.
fn is_disallowed(ip: IpAddr) -> bool {
    match ip.to_canonical() {
        IpAddr::V4(v4) => is_disallowed_v4(v4),
        IpAddr::V6(v6) => {
            let seg = v6.segments();
            if seg[..6] == [0x64, 0xff9b, 0, 0, 0, 0] {
                let [a, b] = seg[6].to_be_bytes();
                let [c, d] = seg[7].to_be_bytes();
                return is_disallowed_v4(std::net::Ipv4Addr::new(a, b, c, d));
            }
            v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || v6.is_unicast_link_local()
                || (seg[0] & 0xfe00) == 0xfc00 // unique local, fc00::/7
                || (seg[0] & 0xffc0) == 0xfec0 // deprecated site-local, fec0::/10
        }
    }
}

fn is_disallowed_v4(v4: std::net::Ipv4Addr) -> bool {
    let [a, b, ..] = v4.octets();
    v4.is_loopback()
        || v4.is_private()
        || v4.is_link_local() // includes the 169.254.169.254 metadata service
        || v4.is_unspecified()
        || v4.is_broadcast()
        || v4.is_multicast()
        || a == 0 // "this network", 0.0.0.0/8
        || (a == 100 && (64..128).contains(&b)) // shared address space, 100.64.0.0/10
        || (a == 192 && b == 0 && v4.octets()[2] == 0) // IETF protocol assignments
        || (a == 198 && (b == 18 || b == 19)) // benchmarking, 198.18.0.0/15
        || a >= 240 // reserved, 240.0.0.0/4
}

/// Builds a one-off HTTP client whose only allowed DNS resolution for the
/// webhook's host is `addrs` - the exact addresses
/// [`validate_webhook_url`] already resolved and approved - so every
/// attempt one [`PushNotifier::notify`] call makes (including retries)
/// connects to one of them instead of letting `reqwest` perform its own,
/// independent DNS resolution and possibly land on a different (rebound)
/// address than the one that was actually validated.
fn pinned_client(url: &str, addrs: &[SocketAddr]) -> std::result::Result<Client, String> {
    let parsed = reqwest::Url::parse(url).map_err(|e| format!("invalid webhook URL: {e}"))?;
    let host = parsed
        .host_str()
        .ok_or_else(|| "webhook URL has no host".to_string())?;
    // No redirects (design review 3.6): a 3xx would send the delivery to a
    // host none of this validation ever saw -- `resolve_to_addrs` pins
    // only the original host. A webhook that redirects is a failed
    // delivery, retried like any other non-2xx.
    Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .resolve_to_addrs(host, addrs)
        .build()
        .map_err(|e| format!("failed to build a DNS-pinned HTTP client for {host:?}: {e}"))
}

#[cfg(test)]
mod tests {
    use std::net::Ipv4Addr;

    use super::*;

    #[tokio::test]
    async fn validate_webhook_url_reports_the_exact_address_it_checked() {
        let addrs = validate_webhook_url("https://203.0.113.5/hook")
            .await
            .expect("a public-looking IP literal should validate");
        assert_eq!(
            addrs,
            vec![SocketAddr::new(IpAddr::V4(Ipv4Addr::new(203, 0, 113, 5)), 443)]
        );
    }

    /// Regression test for the DNS-rebinding TOCTOU: delivery must go
    /// through a client pinned to the exact address
    /// [`validate_webhook_url`] already validated, not one that performs
    /// its own later, independent DNS resolution. Proven here by pointing
    /// [`pinned_client`] at a hostname reserved by RFC 2606 to never
    /// resolve via real DNS (`.invalid`) - if the delivery client fell
    /// back to resolving it for real (the bug this guards against), the
    /// request would fail with a resolution error and the local listener
    /// below would never see a connection.
    #[tokio::test]
    async fn pinned_client_connects_to_the_validated_address_not_a_fresh_dns_lookup() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let url = format!("https://webhook.invalid:{port}/hook");
        let addrs = vec![SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port)];

        let client = pinned_client(&url, &addrs).expect("building a pinned client should succeed");
        let accept =
            tokio::spawn(
                async move { tokio::time::timeout(Duration::from_secs(5), listener.accept()).await },
            );

        // The request itself will still fail (there's no real TLS server
        // behind `listener`), which is fine - what's under test is
        // whether the TCP connection was even attempted against the
        // pinned address, which only happens if DNS resolution was
        // actually overridden rather than performed fresh against the
        // unresolvable `webhook.invalid` host.
        let _ = client.get(&url).send().await;

        let accepted = accept.await.expect("accept task panicked");
        assert!(
            accepted.is_ok(),
            "pinned client never connected to the validated address - it must have performed its own DNS lookup instead"
        );
    }

    /// Design review 3.6: address classes a webhook must not reach. IPv6
    /// unique-local (`fc00::/7`) and NAT64-embedded private IPv4 used to
    /// pass the SSRF filter.
    #[test]
    fn non_global_addresses_are_disallowed() {
        let blocked = [
            "127.0.0.1",
            "10.1.2.3",
            "172.16.0.1",
            "192.168.1.1",
            "169.254.169.254",
            "100.64.0.1",
            "0.1.2.3",
            "198.18.0.1",
            "224.0.0.1",
            "240.0.0.1",
            "192.0.0.8",
            "::1",
            "::",
            "fe80::1",
            "fc00::1",
            "fd12:3456::1",
            "fec0::1",
            "ff02::1",
            "::ffff:10.0.0.1",
            "64:ff9b::a9fe:a9fe",
        ];
        for ip in blocked {
            assert!(is_disallowed(ip.parse().unwrap()), "{ip} should be blocked");
        }
        for ip in ["203.0.114.7", "8.8.8.8", "2606:4700::1111", "64:ff9b::808:808"] {
            assert!(!is_disallowed(ip.parse().unwrap()), "{ip} should be allowed");
        }
    }

    /// Design review 3.6: the pinned delivery client used reqwest's
    /// default redirect policy, so a 3xx sent the webhook payload to a
    /// host the SSRF check never saw. Redirects are not followed now.
    #[tokio::test]
    async fn the_pinned_client_does_not_follow_redirects() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let internal = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let internal_port = internal.local_addr().unwrap().port();
        let hook = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = hook.local_addr().unwrap().port();
        tokio::spawn(async move {
            let (mut sock, _) = hook.accept().await.unwrap();
            let mut buf = [0u8; 1024];
            let _ = sock.read(&mut buf).await;
            let reply = format!(
                "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:{internal_port}/admin\r\nContent-Length: 0\r\n\r\n"
            );
            sock.write_all(reply.as_bytes()).await.unwrap();
        });
        let url = format!("http://webhook.invalid:{port}/hook");
        let addrs = vec![SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port)];
        let client = pinned_client(&url, &addrs).unwrap();
        let resp = client.post(&url).send().await.unwrap();
        assert_eq!(resp.status().as_u16(), 302);
        let followed = tokio::time::timeout(Duration::from_millis(300), internal.accept()).await;
        assert!(followed.is_err(), "the redirect target must not be contacted");
    }
}
