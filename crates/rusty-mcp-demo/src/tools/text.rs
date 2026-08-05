//! Text utilities.
//!
//! A second router, to show that adding a topic means adding a module and one
//! `+` term — no central registry to update.

use rmcp::{Json, handler::server::wrapper::Parameters, tool, tool_router};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::server::DemoServer;

/// A blob of text to operate on.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct TextInput {
    /// The text to process.
    pub text: String,
}

/// Counts describing a piece of text.
#[derive(Debug, Serialize, JsonSchema)]
pub struct TextStats {
    /// Whitespace-separated words.
    pub words: usize,
    /// Unicode scalar values, not bytes.
    pub characters: usize,
    /// Lines, counting a trailing newline as terminating rather than starting one.
    pub lines: usize,
}

#[tool_router(router = text_tools, vis = "pub(crate)")]
impl DemoServer {
    /// Convert text to a URL-safe slug.
    #[tool(description = "Convert text into a lowercase, hyphen-separated slug.")]
    pub async fn slugify(&self, Parameters(TextInput { text }): Parameters<TextInput>) -> String {
        slugify_str(&text)
    }

    /// Count words, characters and lines.
    #[tool(description = "Count the words, characters and lines in some text.")]
    pub async fn text_stats(
        &self,
        Parameters(TextInput { text }): Parameters<TextInput>,
    ) -> Json<TextStats> {
        Json(TextStats {
            words: text.split_whitespace().count(),
            characters: text.chars().count(),
            lines: if text.is_empty() {
                0
            } else {
                text.lines().count()
            },
        })
    }
}

/// Lowercase, keep alphanumerics, collapse everything else into single hyphens.
fn slugify_str(text: &str) -> String {
    let mut slug = String::with_capacity(text.len());
    let mut pending_hyphen = false;

    for ch in text.chars() {
        if ch.is_alphanumeric() {
            if pending_hyphen && !slug.is_empty() {
                slug.push('-');
            }
            pending_hyphen = false;
            slug.extend(ch.to_lowercase());
        } else {
            pending_hyphen = true;
        }
    }

    slug
}

#[cfg(test)]
mod tests {
    use super::slugify_str;

    #[test]
    fn slugifies() {
        assert_eq!(slugify_str("Hello, World!"), "hello-world");
        assert_eq!(
            slugify_str("  leading and trailing  "),
            "leading-and-trailing"
        );
        assert_eq!(slugify_str("multiple---separators"), "multiple-separators");
        assert_eq!(slugify_str("Ünïcödé Tëxt"), "ünïcödé-tëxt");
        assert_eq!(slugify_str("!!!"), "");
        assert_eq!(slugify_str(""), "");
    }
}
