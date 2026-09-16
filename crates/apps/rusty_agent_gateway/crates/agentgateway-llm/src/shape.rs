//! Request shaping from a route's `ai` policy.
//!
//! Everything here runs on the OpenAI-shaped body, before it is translated for
//! whichever provider the route names. That is the only place a rule written
//! once can mean the same thing for every provider: after translation there is
//! no `messages` array to prepend to, because Anthropic has hoisted the system
//! prompt out of it.
//!
//! # The order is a ladder, and it is the whole design
//!
//! Four of these can touch the same field, so which wins has to be stated
//! rather than discovered:
//!
//! 1. **`modelAliases`** resolves the name the caller used. It runs first
//!    because it is about *what the caller meant*, not about overriding them —
//!    everything below should see the resolved name.
//! 2. **`prompts`** shape the conversation.
//! 3. **`defaults`** fill in what the caller left out, and only that.
//! 4. **`overrides`** replace what the caller or the defaults set.
//! 5. The backend's own `model:` still wins over all of it, in
//!    [`crate::LlmBackend::handle`]. It is backend configuration rather than
//!    route policy — the most specific statement about where traffic goes —
//!    and it was already the rule before any of this existed.
//!
//! Read downwards, each step is "more specific wins", which is the only
//! ordering an operator can predict without reading the code.

pub mod caching;

use std::collections::BTreeMap;

use agentgateway_config::{AiPolicy, PromptMessage};
use serde_json::{Map, Value};

/// A route's `ai` policy, ready to apply.
#[derive(Debug, Clone, Default)]
pub struct Shape {
    aliases: BTreeMap<String, String>,
    prepend: Vec<Value>,
    append: Vec<Value>,
    defaults: BTreeMap<String, Value>,
    overrides: BTreeMap<String, Value>,
}

impl Shape {
    /// Compile a policy, or `None` when it asks for nothing this build acts on.
    ///
    /// `None` rather than an empty `Shape` so the request path can skip the
    /// work rather than walk four empty collections per call.
    pub fn new(policy: Option<&AiPolicy>) -> Option<Self> {
        let policy = policy?;
        let (prepend, append) = match &policy.prompts {
            Some(prompts) => (
                prompts.prepend.iter().map(message).collect::<Vec<_>>(),
                prompts.append.iter().map(message).collect::<Vec<_>>(),
            ),
            None => (Vec::new(), Vec::new()),
        };

        let shape = Shape {
            aliases: policy.model_aliases.clone(),
            prepend,
            append,
            defaults: policy.defaults.clone(),
            overrides: policy.overrides.clone(),
        };
        (!shape.is_empty()).then_some(shape)
    }

    fn is_empty(&self) -> bool {
        self.aliases.is_empty()
            && self.prepend.is_empty()
            && self.append.is_empty()
            && self.defaults.is_empty()
            && self.overrides.is_empty()
    }

    /// Apply the policy to an OpenAI-shaped request body.
    pub fn apply(&self, body: &mut Value) {
        let Some(object) = body.as_object_mut() else {
            // Not an object, so there is nothing to shape. The caller has
            // already decided what to do about that.
            return;
        };

        self.resolve_alias(object);
        self.add_prompts(object);

        for (key, value) in &self.defaults {
            object.entry(key.clone()).or_insert_with(|| value.clone());
        }
        for (key, value) in &self.overrides {
            object.insert(key.clone(), value.clone());
        }
    }

    /// Swap the name the caller used for the one it stands for.
    fn resolve_alias(&self, object: &mut Map<String, Value>) {
        let Some(asked) = object.get("model").and_then(Value::as_str) else {
            return;
        };
        let Some(real) = self.aliases.get(asked) else {
            return;
        };
        // An alias is not a rename of the whole vocabulary: a name that is not
        // one is passed through, so a route can alias `fast` without having to
        // enumerate every model a caller might otherwise ask for.
        let real = real.clone();
        tracing::debug!(alias = asked, model = %real, "resolved a model alias");
        object.insert("model".into(), Value::String(real));
    }

    /// Put the route's own messages around the caller's.
    fn add_prompts(&self, object: &mut Map<String, Value>) {
        if self.prepend.is_empty() && self.append.is_empty() {
            return;
        }
        // A body with no `messages` is malformed for this API and will be
        // refused downstream. Inventing the array here would turn a client bug
        // into a request that runs -- with only the operator's prompt in it.
        let Some(messages) = object.get_mut("messages").and_then(Value::as_array_mut) else {
            return;
        };

        messages.splice(0..0, self.prepend.iter().cloned());
        messages.extend(self.append.iter().cloned());
    }
}

fn message(configured: &PromptMessage) -> Value {
    serde_json::json!({"role": configured.role, "content": configured.content})
}

#[cfg(test)]
mod tests;
