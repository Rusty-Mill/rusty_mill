//! Server-side persistence for tasks and push notification configurations.
//!
//! [`InMemoryTaskStore`] is a process-local, non-persistent implementation
//! suitable for examples, tests, and single-instance deployments.
//! Production deployments needing durability or multi-instance fan-out
//! should implement [`TaskStore`] against a real datastore.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::types::{ListTasksRequest, Task, TaskPushNotificationConfig};

/// Storage for [`Task`] records and their push notification
/// configurations.
#[async_trait]
pub trait TaskStore: Send + Sync {
    async fn get(&self, id: &str) -> Option<Task>;
    async fn put(&self, task: Task);

    /// Returns `(page, next_page_token, total_matching)`: the page of
    /// tasks matching `filter`, an opaque cursor for the next page (empty
    /// if this was the last page), and the total count of tasks matching
    /// the filter across all pages.
    async fn list(&self, filter: &ListTasksRequest) -> (Vec<Task>, String, i64);

    async fn put_push_config(&self, config: TaskPushNotificationConfig) -> TaskPushNotificationConfig;
    async fn get_push_config(&self, task_id: &str, id: &str) -> Option<TaskPushNotificationConfig>;
    async fn list_push_configs(&self, task_id: &str) -> Vec<TaskPushNotificationConfig>;
    /// Returns `true` if a configuration was found and removed.
    async fn delete_push_config(&self, task_id: &str, id: &str) -> bool;
}

/// A simple in-memory [`TaskStore`], backed by a `HashMap` behind a
/// `tokio::sync::RwLock`. Data does not survive a process restart.
#[derive(Default)]
pub struct InMemoryTaskStore {
    tasks: RwLock<HashMap<String, Task>>,
    push_configs: RwLock<HashMap<String, Vec<TaskPushNotificationConfig>>>,
}

impl InMemoryTaskStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn shared() -> Arc<Self> {
        Arc::new(Self::new())
    }
}

#[async_trait]
impl TaskStore for InMemoryTaskStore {
    async fn get(&self, id: &str) -> Option<Task> {
        self.tasks.read().await.get(id).cloned()
    }

    async fn put(&self, task: Task) {
        self.tasks.write().await.insert(task.id.clone(), task);
    }

    async fn list(&self, filter: &ListTasksRequest) -> (Vec<Task>, String, i64) {
        let tasks = self.tasks.read().await;
        let mut matching: Vec<Task> = tasks
            .values()
            .filter(|t| {
                if let Some(ctx) = &filter.context_id {
                    if t.context_id.as_deref() != Some(ctx.as_str()) {
                        return false;
                    }
                }
                if let Some(status) = filter.status {
                    if t.status.state != status {
                        return false;
                    }
                }
                if let Some(after) = filter.status_timestamp_after {
                    match t.status.timestamp {
                        Some(ts) if ts >= after => {}
                        _ => return false,
                    }
                }
                true
            })
            .cloned()
            .collect();
        matching.sort_by(|a, b| a.id.cmp(&b.id));
        let total = matching.len() as i64;

        let page_size = filter.page_size.unwrap_or(50).clamp(1, 100) as usize;
        let start = filter
            .page_token
            .as_ref()
            .and_then(|t| matching.iter().position(|task| &task.id == t))
            .unwrap_or(0);
        let end = (start + page_size).min(matching.len());
        let next_page_token = if end < matching.len() {
            matching[end].id.clone()
        } else {
            String::new()
        };
        let mut page = matching[start..end].to_vec();

        if !filter.include_artifacts.unwrap_or(false) {
            for t in &mut page {
                t.artifacts.clear();
            }
        }
        (page, next_page_token, total)
    }

    async fn put_push_config(&self, mut config: TaskPushNotificationConfig) -> TaskPushNotificationConfig {
        if config.id.is_none() {
            config.id = Some(Uuid::new_v4().to_string());
        }
        let task_id = config.task_id.clone().unwrap_or_default();
        let mut configs = self.push_configs.write().await;
        let entry = configs.entry(task_id).or_default();
        entry.retain(|c| c.id != config.id);
        entry.push(config.clone());
        config
    }

    async fn get_push_config(&self, task_id: &str, id: &str) -> Option<TaskPushNotificationConfig> {
        self.push_configs
            .read()
            .await
            .get(task_id)?
            .iter()
            .find(|c| c.id.as_deref() == Some(id))
            .cloned()
    }

    async fn list_push_configs(&self, task_id: &str) -> Vec<TaskPushNotificationConfig> {
        self.push_configs
            .read()
            .await
            .get(task_id)
            .cloned()
            .unwrap_or_default()
    }

    async fn delete_push_config(&self, task_id: &str, id: &str) -> bool {
        let mut configs = self.push_configs.write().await;
        if let Some(list) = configs.get_mut(task_id) {
            let before = list.len();
            list.retain(|c| c.id.as_deref() != Some(id));
            return list.len() != before;
        }
        false
    }
}
