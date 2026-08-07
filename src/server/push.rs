//! Push notification delivery (spec Section 4.3): POSTs the current
//! [`Task`] to every [`TaskPushNotificationConfig`] registered for it
//! whenever its status or artifacts change.
//!
//! Delivery is best-effort and fire-and-forget: a slow, unreachable, or
//! error-returning webhook is logged (`tracing::warn!`) and otherwise has
//! no effect on task processing - in particular, it never blocks or fails
//! the [`AgentExecutor`](super::AgentExecutor) invocation that produced
//! the update.

use std::net::IpAddr;
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
            validate_webhook_url(url).await
        } else {
            Ok(())
        }
    }

    pub(crate) async fn notify(&self, config: &TaskPushNotificationConfig, task: &Task) {
        if self.ssrf_protection {
            if let Err(reason) = validate_webhook_url(&config.url).await {
                tracing::warn!(
                    task_id = %task.id,
                    url = %config.url,
                    %reason,
                    "skipping push notification delivery to a disallowed webhook URL"
                );
                return;
            }
        }

        // Spec Section 4.3.3: the webhook payload is a `StreamResponse`
        // object (i.e. `{"task": {...}}`), not the bare `Task`.
        let payload = StreamResponse::Task { task: task.clone() };

        for attempt in 1..=MAX_DELIVERY_ATTEMPTS {
            match self.attempt_delivery(config, &payload).await {
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
        config: &TaskPushNotificationConfig,
        payload: &StreamResponse,
    ) -> std::result::Result<(), DeliveryFailure> {
        let mut request = self.client.post(&config.url).json(payload);
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

/// Spec Section 13.2 (SHOULD): reject a webhook URL whose host is (or
/// resolves to) a private, loopback, or link-local address - guards
/// against SSRF, where a malicious client registers a webhook that
/// probes the agent's own internal network. A literal IP-address host is
/// checked directly with no DNS lookup involved; a hostname is resolved
/// fresh every call (rather than caching the result) so this also catches
/// DNS rebinding - a hostname that resolved to a public address at
/// registration time but a private one by the time of a later delivery.
pub(crate) async fn validate_webhook_url(url: &str) -> std::result::Result<(), String> {
    let parsed = reqwest::Url::parse(url).map_err(|e| format!("invalid webhook URL: {e}"))?;
    match parsed.scheme() {
        "http" | "https" => {}
        other => return Err(format!("unsupported webhook URL scheme {other:?}")),
    }
    let host = parsed
        .host_str()
        .ok_or_else(|| "webhook URL has no host".to_string())?;

    if let Ok(ip) = host.parse::<IpAddr>() {
        return if is_disallowed(ip) {
            Err(format!(
                "webhook URL host {host} is a private/loopback/link-local address, which is not allowed"
            ))
        } else {
            Ok(())
        };
    }

    let port = parsed.port_or_known_default().unwrap_or(0);
    let addrs = tokio::net::lookup_host((host, port))
        .await
        .map_err(|e| format!("failed to resolve webhook URL host {host:?}: {e}"))?;
    let mut resolved_any = false;
    for addr in addrs {
        resolved_any = true;
        if is_disallowed(addr.ip()) {
            return Err(format!(
                "webhook URL host {host:?} resolves to a private/loopback/link-local address ({}), which is not allowed",
                addr.ip()
            ));
        }
    }
    if resolved_any {
        Ok(())
    } else {
        Err(format!(
            "webhook URL host {host:?} did not resolve to any address"
        ))
    }
}

fn is_disallowed(ip: IpAddr) -> bool {
    match ip.to_canonical() {
        IpAddr::V4(v4) => {
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
        }
        IpAddr::V6(v6) => v6.is_loopback() || v6.is_unspecified() || v6.is_unicast_link_local(),
    }
}
