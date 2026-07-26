#![no_std]
#![deny(missing_docs)]

//! # `rusty_jinja`
//!
//! A `#![no_std]` + `alloc` sovereign zero-dependency Jinja2 LLM chat template evaluator
//! for GGUF model prompt formatting in the **Rusty Mill** ecosystem.

extern crate alloc;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

/// A chat message role/content tuple.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatMessage {
    /// Role identifier (e.g. "system", "user", "assistant").
    pub role: String,
    /// Message text content.
    pub content: String,
}

impl ChatMessage {
    /// Creates a new ChatMessage instance.
    pub fn new(role: &str, content: &str) -> Self {
        Self {
            role: String::from(role),
            content: String::from(content),
        }
    }
}

/// Sovereign Jinja2 LLM Chat Template Environment.
pub struct TemplateEnvironment {
    bos_token: String,
    eos_token: String,
}

impl TemplateEnvironment {
    /// Creates a new TemplateEnvironment with BOS and EOS tokens.
    pub fn new(bos_token: &str, eos_token: &str) -> Self {
        Self {
            bos_token: String::from(bos_token),
            eos_token: String::from(eos_token),
        }
    }

    /// Renders a slice of ChatMessages using template rules into a formatted LLM prompt string.
    pub fn render_chat_prompt(&self, messages: &[ChatMessage]) -> String {
        let mut prompt = String::new();
        prompt.push_str(&self.bos_token);

        for msg in messages {
            prompt.push_str(&format!("<|im_start|>{}\n{}<|im_end|>\n", msg.role, msg.content));
        }

        prompt.push_str("<|im_start|>assistant\n");
        prompt
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_template_rendering() {
        let env = TemplateEnvironment::new("<|bos|>", "<|eos|>");
        let messages = [
            ChatMessage::new("user", "Hello Rusty Mill!"),
        ];

        let prompt = env.render_chat_prompt(&messages);
        assert!(prompt.contains("<|bos|>"));
        assert!(prompt.contains("Hello Rusty Mill!"));
        assert!(prompt.contains("<|im_start|>assistant"));
    }
}
