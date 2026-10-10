//! Change notifications. 2026-07-28 replaces per-resource
//! `resources/subscribe` with one long-lived `subscriptions/listen` request
//! whose response stream carries the notifications; the classic pair stays
//! for older peers.

use crate::codec::{field, object, opt_bool, opt_value, string, Obj, Wire};
use crate::page::ResultType;
use crate::rpc::RequestId;
use crate::version::Implementation;
use crate::{Error, Result};
use rusty_json::Value;

/// Method names for subscriptions and change notifications.
pub mod method {
    /// 2026-07-28: open a notification stream.
    pub const LISTEN: &str = "subscriptions/listen";
    /// 2026-07-28: first message on the stream, echoing what was granted.
    pub const ACKNOWLEDGED: &str = "notifications/subscriptions/acknowledged";
    /// Classic: subscribe to one resource.
    pub const SUBSCRIBE: &str = "resources/subscribe";
    /// Classic: drop that subscription.
    pub const UNSUBSCRIBE: &str = "resources/unsubscribe";
    /// A resource's contents changed.
    pub const RESOURCE_UPDATED: &str = "notifications/resources/updated";
    /// The tool list changed (no parameters).
    pub const TOOLS_LIST_CHANGED: &str = "notifications/tools/list_changed";
    /// The prompt list changed (no parameters).
    pub const PROMPTS_LIST_CHANGED: &str = "notifications/prompts/list_changed";
    /// The resource list changed (no parameters).
    pub const RESOURCES_LIST_CHANGED: &str = "notifications/resources/list_changed";
}

/// Which notifications a listener wants.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SubscriptionFilter {
    /// Tool list changes.
    pub tools_list_changed: Option<bool>,
    /// Prompt list changes.
    pub prompts_list_changed: Option<bool>,
    /// Resource list changes.
    pub resources_list_changed: Option<bool>,
    /// URIs of resources to follow.
    pub resource_subscriptions: Option<Vec<String>>,
}

impl Wire for SubscriptionFilter {
    fn to_value(&self) -> Value {
        Obj::new()
            .opt("toolsListChanged", self.tools_list_changed)
            .opt("promptsListChanged", self.prompts_list_changed)
            .opt("resourcesListChanged", self.resources_list_changed)
            .opt_value(
                "resourceSubscriptions",
                &self.resource_subscriptions.as_ref().map(|uris| {
                    Value::Array(uris.iter().map(|u| Value::from(u.as_str())).collect())
                }),
            )
            .done()
    }

    fn from_value(v: &Value) -> Result<Self> {
        const W: &str = "SubscriptionFilter";
        object(v, W)?;
        let resource_subscriptions = match v.get("resourceSubscriptions") {
            None | Some(Value::Null) => None,
            Some(list) => Some(
                list.as_array()
                    .ok_or_else(|| Error::decode(W, "\"resourceSubscriptions\" is not an array"))?
                    .iter()
                    .map(|u| {
                        u.as_str()
                            .map(String::from)
                            .ok_or_else(|| Error::decode(W, "a subscription is not a string"))
                    })
                    .collect::<Result<_>>()?,
            ),
        };
        Ok(Self {
            tools_list_changed: opt_bool(v, W, "toolsListChanged")?,
            prompts_list_changed: opt_bool(v, W, "promptsListChanged")?,
            resources_list_changed: opt_bool(v, W, "resourcesListChanged")?,
            resource_subscriptions,
        })
    }
}

/// Parameters of `subscriptions/listen`.
#[derive(Clone, Debug, PartialEq)]
pub struct ListenParams {
    /// Request `_meta`, raw JSON (see [`RequestMeta`](crate::RequestMeta)).
    pub meta: Option<Value>,
    /// What to be told about.
    pub notifications: SubscriptionFilter,
}

impl Wire for ListenParams {
    fn to_value(&self) -> Value {
        Obj::new()
            .opt_value("_meta", &self.meta)
            .set("notifications", self.notifications.to_value())
            .done()
    }

    fn from_value(v: &Value) -> Result<Self> {
        const W: &str = "ListenParams";
        object(v, W)?;
        Ok(Self {
            meta: opt_value(v, "_meta"),
            notifications: SubscriptionFilter::from_value(field(v, W, "notifications")?)?,
        })
    }
}

/// `notifications/subscriptions/acknowledged`: what the server granted.
#[derive(Clone, Debug, PartialEq)]
pub struct AcknowledgedParams {
    /// `_meta`, raw JSON.
    pub meta: Option<Value>,
    /// The filter now in force.
    pub notifications: SubscriptionFilter,
}

impl Wire for AcknowledgedParams {
    fn to_value(&self) -> Value {
        Obj::new()
            .opt_value("_meta", &self.meta)
            .set("notifications", self.notifications.to_value())
            .done()
    }

    fn from_value(v: &Value) -> Result<Self> {
        const W: &str = "AcknowledgedParams";
        object(v, W)?;
        Ok(Self {
            meta: opt_value(v, "_meta"),
            notifications: SubscriptionFilter::from_value(field(v, W, "notifications")?)?,
        })
    }
}

const SUBSCRIPTION_ID: &str = "io.modelcontextprotocol/subscriptionId";
const SERVER_INFO: &str = "io.modelcontextprotocol/serverInfo";

/// The final result of `subscriptions/listen`, sent when the stream ends.
/// Its `_meta` must name the subscription.
#[derive(Clone, Debug, PartialEq)]
pub struct ListenResult {
    /// `resultType`, normally `complete`.
    pub result_type: ResultType,
    /// `_meta`, a JSON object holding at least the subscription id.
    pub meta: Value,
}

impl ListenResult {
    /// A completed listen for subscription `id`.
    pub fn complete(id: &RequestId) -> Self {
        let mut meta = Value::object();
        meta.insert(SUBSCRIPTION_ID, id.to_value());
        Self {
            result_type: ResultType(ResultType::COMPLETE.to_owned()),
            meta,
        }
    }

    /// Record the server's identity in `_meta`.
    pub fn with_server_info(mut self, info: &Implementation) -> Self {
        self.meta.insert(SERVER_INFO, info.to_value());
        self
    }

    /// The subscription this ends.
    pub fn subscription_id(&self) -> Option<RequestId> {
        RequestId::from_value(self.meta.get(SUBSCRIPTION_ID)?).ok()
    }
}

impl Wire for ListenResult {
    fn to_value(&self) -> Value {
        Obj::new()
            .set("resultType", self.result_type.0.as_str())
            .set("_meta", self.meta.clone())
            .done()
    }

    fn from_value(v: &Value) -> Result<Self> {
        const W: &str = "ListenResult";
        object(v, W)?;
        let meta = object(field(v, W, "_meta")?, "ListenResult _meta")?.clone();
        RequestId::from_value(
            meta.get(SUBSCRIPTION_ID)
                .ok_or_else(|| Error::decode(W, "_meta has no subscription id"))?,
        )?;
        Ok(Self {
            result_type: ResultType(string(v, W, "resultType")?),
            meta,
        })
    }
}

/// `resources/subscribe` and `resources/unsubscribe` (classic): one URI.
#[derive(Clone, Debug, PartialEq)]
pub struct SubscribeParams {
    /// The resource to follow or stop following.
    pub uri: String,
    /// Request `_meta`, raw JSON.
    pub meta: Option<Value>,
}

impl SubscribeParams {
    /// Parameters for `uri`.
    pub fn new(uri: impl Into<String>) -> Self {
        Self {
            uri: uri.into(),
            meta: None,
        }
    }
}

impl Wire for SubscribeParams {
    fn to_value(&self) -> Value {
        Obj::new()
            .set("uri", self.uri.as_str())
            .opt_value("_meta", &self.meta)
            .done()
    }

    fn from_value(v: &Value) -> Result<Self> {
        object(v, "SubscribeParams")?;
        Ok(Self {
            uri: string(v, "SubscribeParams", "uri")?,
            meta: opt_value(v, "_meta"),
        })
    }
}

/// `notifications/resources/updated`.
#[derive(Clone, Debug, PartialEq)]
pub struct ResourceUpdatedParams {
    /// The resource that changed.
    pub uri: String,
    /// `_meta`, raw JSON.
    pub meta: Option<Value>,
}

impl Wire for ResourceUpdatedParams {
    fn to_value(&self) -> Value {
        Obj::new()
            .set("uri", self.uri.as_str())
            .opt_value("_meta", &self.meta)
            .done()
    }

    fn from_value(v: &Value) -> Result<Self> {
        object(v, "ResourceUpdatedParams")?;
        Ok(Self {
            uri: string(v, "ResourceUpdatedParams", "uri")?,
            meta: opt_value(v, "_meta"),
        })
    }
}
