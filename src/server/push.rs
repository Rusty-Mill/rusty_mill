//! Push notification delivery (spec Section 4.3): POSTs the current
//! [`Task`] to every [`TaskPushNotificationConfig`] registered for it
//! whenever its status or artifacts change.
//!
//! Delivery is best-effort and fire-and-forget: a slow, unreachable, or
//! error-returning webhook is logged (`tracing::warn!`) and otherwise has
//! no effect on task processing - in particular, it never blocks or fails
//! the [`AgentExecutor`](super::AgentExecutor) invocation that produced
//! the update.

use reqwest::Client;

use crate::types::{AuthenticationInfo, Task, TaskPushNotificationConfig};

#[derive(Clone)]
pub(crate) struct PushNotifier {
    client: Client,
}

impl PushNotifier {
    pub(crate) fn new() -> Self {
        PushNotifier {
            client: Client::new(),
        }
    }

    pub(crate) async fn notify(&self, config: &TaskPushNotificationConfig, task: &Task) {
        let mut request = self.client.post(&config.url).json(task);
        if let Some(token) = &config.token {
            request = request.header("X-A2A-Notification-Token", token);
        }
        if let Some(auth) = &config.authentication {
            request = apply_authentication(request, auth);
        }

        match request.send().await {
            Ok(response) if !response.status().is_success() => {
                tracing::warn!(
                    task_id = %task.id,
                    url = %config.url,
                    status = %response.status(),
                    "push notification webhook returned a non-2xx status"
                );
            }
            Err(error) => {
                tracing::warn!(
                    task_id = %task.id,
                    url = %config.url,
                    %error,
                    "push notification delivery failed"
                );
            }
            Ok(_) => {}
        }
    }
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
