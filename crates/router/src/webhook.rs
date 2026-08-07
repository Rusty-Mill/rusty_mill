use std::time::Duration;

use ring::hmac;
use serde::Serialize;

use crate::config::{BudgetPeriod, WebhookConfig};

/// A budget-related event this router can push to an operator's own
/// endpoint, so crossing a budget surfaces as more than a `402` on the
/// client's next request and a Prometheus counter.
#[derive(Debug, Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
// Every variant sharing a "Budget" prefix is intentional -- this enum is
// scoped to budget-related events by design, not a naming accident.
#[allow(clippy::enum_variant_names)]
enum WebhookEvent {
    BudgetExceeded {
        client: String,
        spent_usd: f64,
        budget_usd: f64,
        period: BudgetPeriod,
    },
    BudgetReset {
        client: String,
        budget_usd: f64,
        period: BudgetPeriod,
    },
    /// A client's tracked spend just crossed its configured
    /// `budget_warning_threshold` -- a heads-up before `BudgetExceeded`,
    /// not a second limit. Fires once per crossing, same rule
    /// `BudgetExceeded` already follows.
    BudgetWarning {
        client: String,
        spent_usd: f64,
        budget_usd: f64,
        warning_threshold: f64,
        period: BudgetPeriod,
    },
}

/// Backoff policy for retrying a failed webhook delivery. Only a 5xx
/// response or a network error is retryable -- any other status (a 4xx,
/// for instance) is treated as permanent. Same doubling-capped-at-max
/// shape `[mcp]` upstream reconnect already uses, just bounded by default
/// (`max_retries`) rather than optionally unbounded, since a webhook
/// receiver that's actually down shouldn't be retried forever.
#[derive(Debug, Clone, Copy)]
pub(crate) struct RetryPolicy {
    initial_backoff: Duration,
    max_backoff: Duration,
    max_retries: u32,
}

impl From<&WebhookConfig> for RetryPolicy {
    fn from(config: &WebhookConfig) -> Self {
        Self {
            initial_backoff: Duration::from_secs(config.retry_backoff_secs),
            max_backoff: Duration::from_secs(config.retry_backoff_max_secs),
            max_retries: config.max_retries,
        }
    }
}

impl RetryPolicy {
    /// No retry -- a single delivery attempt, same as this router's
    /// original webhook behavior. Used by tests that don't exercise retry
    /// behavior; production construction always goes through `From<&WebhookConfig>`,
    /// which sends the same single-attempt behavior via `max_retries: 0`
    /// when an operator sets it in config.
    #[cfg(test)]
    pub(crate) fn none() -> Self {
        Self {
            initial_backoff: Duration::from_millis(1),
            max_backoff: Duration::from_millis(1),
            max_retries: 0,
        }
    }

    #[cfg(test)]
    pub(crate) fn for_test(initial_backoff_ms: u64, max_retries: u32) -> Self {
        Self {
            initial_backoff: Duration::from_millis(initial_backoff_ms),
            max_backoff: Duration::from_millis(initial_backoff_ms * 10),
            max_retries,
        }
    }
}

/// Doubles `current`, capped at `max` -- no jitter, same as MCP's own
/// reconnect backoff (a handful of webhook retries isn't a
/// thundering-herd concern at this scale).
fn next_backoff(current: Duration, max: Duration) -> Duration {
    (current * 2).min(max)
}

/// Hex-encoded HMAC-SHA256 over `body`, keyed by `secret`.
fn sign_hex(secret: &str, body: &[u8]) -> String {
    let key = hmac::Key::new(hmac::HMAC_SHA256, secret.as_bytes());
    let tag = hmac::sign(&key, body);
    tag.as_ref().iter().map(|b| format!("{b:02x}")).collect()
}

/// Fires `[webhook]`-configured POSTs on budget events. Delivery is
/// fire-and-forget (spawned, not awaited) so a slow or unreachable
/// receiver never adds latency to the request that triggered the event --
/// same non-blocking contract as `record_usage`'s persistence writes. A
/// delivery failure (after exhausting `retry_policy`) is only logged,
/// never surfaced to the client.
pub(crate) struct WebhookNotifier {
    client: reqwest::Client,
    url: String,
    /// The exact value to send as this POST's `Authorization` header
    /// (e.g. `"Bearer <token>"`), so the receiver can verify the request
    /// came from this router. `None` sends no `Authorization` header.
    auth_header: Option<String>,
    /// HMAC-SHA256 secret, resolved from `signing_secret_env`. `None`
    /// sends no `X-RP-Signature` header.
    signing_secret: Option<String>,
    retry_policy: RetryPolicy,
}

impl WebhookNotifier {
    pub(crate) fn new(
        url: String,
        auth_header: Option<String>,
        timeout_secs: u64,
        signing_secret: Option<String>,
        retry_policy: RetryPolicy,
    ) -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(timeout_secs))
                .build()
                .expect("reqwest client should build with a timeout configured"),
            url,
            auth_header,
            signing_secret,
            retry_policy,
        }
    }

    /// Builds a `WebhookNotifier` straight from `[webhook]` config plus the
    /// already-resolved `auth_header`/`signing_secret` values (env var
    /// resolution is the caller's job, same division of responsibility as
    /// every other `*_env` field in this crate).
    pub(crate) fn from_config(
        config: &WebhookConfig,
        auth_header: Option<String>,
        signing_secret: Option<String>,
    ) -> Self {
        Self::new(
            config.url.clone(),
            auth_header,
            config.timeout_secs,
            signing_secret,
            RetryPolicy::from(config),
        )
    }

    fn send(&self, event: WebhookEvent) {
        let body = match serde_json::to_vec(&event) {
            Ok(b) => b,
            Err(e) => {
                tracing::warn!(error = %e, "failed to serialize webhook event; not sending");
                return;
            }
        };
        let client = self.client.clone();
        let url = self.url.clone();
        let auth_header = self.auth_header.clone();
        let signature = self
            .signing_secret
            .as_deref()
            .map(|secret| format!("sha256={}", sign_hex(secret, &body)));
        let policy = self.retry_policy;

        tokio::spawn(async move {
            let mut backoff = policy.initial_backoff;
            let mut attempt = 0u32;
            loop {
                let mut req = client
                    .post(&url)
                    .header(reqwest::header::CONTENT_TYPE, "application/json")
                    .body(body.clone());
                if let Some(auth) = &auth_header {
                    req = req.header(reqwest::header::AUTHORIZATION, auth.clone());
                }
                if let Some(sig) = &signature {
                    req = req.header("X-RP-Signature", sig.clone());
                }

                let retryable = match req.send().await {
                    Ok(resp) if resp.status().is_success() => return,
                    Ok(resp) => {
                        let status = resp.status();
                        let will_retry = status.is_server_error() && attempt < policy.max_retries;
                        tracing::warn!(%url, %status, attempt, will_retry, "webhook delivery failed");
                        status.is_server_error()
                    }
                    Err(e) => {
                        let will_retry = attempt < policy.max_retries;
                        tracing::warn!(%url, error = %e, attempt, will_retry, "webhook delivery failed");
                        true
                    }
                };

                if !retryable || attempt >= policy.max_retries {
                    return;
                }
                tokio::time::sleep(backoff).await;
                backoff = next_backoff(backoff, policy.max_backoff);
                attempt += 1;
            }
        });
    }

    /// A client's tracked spend just reached or passed its configured
    /// `budget_usd` as a result of a request that was already let through
    /// (the request that pushed it over is charged before this fires, not
    /// blocked by it -- the `402` starts on the *next* request).
    pub(crate) fn notify_budget_exceeded(
        &self,
        client_name: &str,
        spent_usd: f64,
        budget_usd: f64,
        period: BudgetPeriod,
    ) {
        self.send(WebhookEvent::BudgetExceeded {
            client: client_name.to_string(),
            spent_usd,
            budget_usd,
            period,
        });
    }

    /// An operator manually reset a client's spend via the admin API
    /// (`POST /v1/admin/clients/{name}/reset-spend`).
    pub(crate) fn notify_budget_reset(
        &self,
        client_name: &str,
        budget_usd: f64,
        period: BudgetPeriod,
    ) {
        self.send(WebhookEvent::BudgetReset {
            client: client_name.to_string(),
            budget_usd,
            period,
        });
    }

    /// A client's tracked spend just crossed `warning_threshold *
    /// budget_usd`, on the specific request that pushed it over that
    /// fraction -- same "only on the crossing request" rule
    /// `notify_budget_exceeded` follows.
    pub(crate) fn notify_budget_warning(
        &self,
        client_name: &str,
        spent_usd: f64,
        budget_usd: f64,
        warning_threshold: f64,
        period: BudgetPeriod,
    ) {
        self.send(WebhookEvent::BudgetWarning {
            client: client_name.to_string(),
            spent_usd,
            budget_usd,
            warning_threshold,
            period,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_backoff_doubles_each_time() {
        let max = Duration::from_secs(60);
        let a = Duration::from_secs(1);
        let b = next_backoff(a, max);
        let c = next_backoff(b, max);
        assert_eq!(b, Duration::from_secs(2));
        assert_eq!(c, Duration::from_secs(4));
    }

    #[test]
    fn next_backoff_caps_at_max() {
        let max = Duration::from_secs(60);
        assert_eq!(next_backoff(Duration::from_secs(50), max), max);
        assert_eq!(next_backoff(max, max), max);
    }

    #[test]
    fn sign_hex_is_deterministic_and_key_dependent() {
        let body = br#"{"event":"budget_exceeded"}"#;
        let a = sign_hex("secret-one", body);
        let b = sign_hex("secret-one", body);
        let c = sign_hex("secret-two", body);
        assert_eq!(a, b);
        assert_ne!(a, c);
        // HMAC-SHA256 -> 32 bytes -> 64 hex chars.
        assert_eq!(a.len(), 64);
    }
}
