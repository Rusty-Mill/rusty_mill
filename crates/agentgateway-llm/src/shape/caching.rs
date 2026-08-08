//! Provider cache breakpoints from `ai.promptCaching`.
//!
//! Unlike the rest of [`super`], this runs *after* translation. A cache
//! breakpoint is a provider-specific annotation on a provider-specific shape —
//! Anthropic's `cache_control` on a content block — and there is nothing in
//! the OpenAI request to hang it on.
//!
//! # Only Anthropic
//!
//! OpenAI caches long prefixes by itself and takes no configuration for it, so
//! this is a no-op there rather than an error: a route that sets
//! `promptCaching` and later switches provider should not stop starting.
//!
//! # What a breakpoint costs when it is wrong
//!
//! Nothing. Anthropic will not cache a prefix below its own minimum and
//! ignores the marker, so `minTokens` is an optimisation for the operator's
//! own noise rather than a correctness rule. That is why estimating the length
//! rather than tokenising it is acceptable here, and why the estimate is
//! documented rather than hidden: being wrong costs a marker nobody sees.

use agentgateway_config::PromptCaching;
use serde_json::{Value, json};

/// Roughly how many characters a token is worth in English prose.
///
/// A real tokeniser would mean shipping a vocabulary per model to decide
/// whether to add an annotation the provider ignores when it is wrong. This is
/// the cheapest thing that is right often enough to be useful.
const CHARS_PER_TOKEN: u64 = 4;

/// A compiled `promptCaching` policy.
#[derive(Debug, Clone)]
pub struct Caching {
    system: bool,
    messages: bool,
    min_tokens: Option<u64>,
    message_offset: usize,
}

impl Caching {
    /// Compile a policy, or `None` when it would mark nothing.
    pub fn new(policy: Option<&PromptCaching>) -> Option<Self> {
        let policy = policy?;
        // `cacheTools` is deliberately absent: this build does not translate
        // `tools` to Anthropic at all, so there is no tool block to mark.
        // `Config::lint` says so rather than this silently doing nothing.
        if !policy.cache_system && !policy.cache_messages {
            return None;
        }
        Some(Caching {
            system: policy.cache_system,
            messages: policy.cache_messages,
            min_tokens: policy.min_tokens,
            message_offset: policy.cache_message_offset.unwrap_or(0),
        })
    }

    /// Annotate an Anthropic Messages request.
    pub fn apply(&self, body: &mut Value) {
        if let Some(minimum) = self.min_tokens
            && estimated_tokens(body) < minimum
        {
            return;
        }
        let Some(object) = body.as_object_mut() else {
            return;
        };

        if self.system
            && let Some(system) = object.get_mut("system")
        {
            // Anthropic takes `system` as either a string or a list of blocks,
            // and only a block can carry `cache_control`, so a string is
            // promoted rather than left unmarkable.
            if let Some(text) = system.as_str() {
                *system = json!([{
                    "type": "text",
                    "text": text,
                    "cache_control": {"type": "ephemeral"},
                }]);
            } else if let Some(blocks) = system.as_array_mut() {
                mark_last_block(blocks);
            }
        }

        if self.messages
            && let Some(messages) = object.get_mut("messages").and_then(Value::as_array_mut)
            && !messages.is_empty()
        {
            // Counting back from the end, because what a conversation adds is
            // a new turn at the end and the breakpoint wants to sit behind the
            // part that changes. An offset past the start marks the first
            // message rather than nothing: the intent is "as far back as
            // asked", and refusing to mark anything would be a silent no-op.
            let index = messages.len().saturating_sub(1 + self.message_offset);
            if let Some(content) = messages[index].get_mut("content") {
                if let Some(text) = content.as_str() {
                    *content = json!([{
                        "type": "text",
                        "text": text,
                        "cache_control": {"type": "ephemeral"},
                    }]);
                } else if let Some(blocks) = content.as_array_mut() {
                    mark_last_block(blocks);
                }
            }
        }
    }
}

/// Put `cache_control` on the final block of a list.
///
/// The breakpoint covers everything up to and including where it sits, so the
/// last block is the one that caches the whole list.
fn mark_last_block(blocks: &mut [Value]) {
    if let Some(last) = blocks.last_mut()
        && let Some(object) = last.as_object_mut()
    {
        object.insert("cache_control".into(), json!({"type": "ephemeral"}));
    }
}

/// A rough token count for the whole request.
///
/// Serialized length over [`CHARS_PER_TOKEN`]. See the module docs for why an
/// estimate is enough.
fn estimated_tokens(body: &Value) -> u64 {
    let characters = serde_json::to_string(body).map(|s| s.len()).unwrap_or(0) as u64;
    characters / CHARS_PER_TOKEN
}

#[cfg(test)]
mod tests;
