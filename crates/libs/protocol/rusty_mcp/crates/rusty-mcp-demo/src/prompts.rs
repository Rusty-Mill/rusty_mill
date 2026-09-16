//! Prompts exposed by the demo server.
//!
//! Prompts are user-initiated templates — the client typically surfaces them as
//! slash commands or menu entries, so the model never picks one on its own.
//! That is the distinction from tools, and it is why prompt arguments should
//! read like something a person fills in.
//!
//! `rmcp` gives prompts the same router treatment as tools, so this composes
//! exactly like `tools/`: one module per topic, merged with `+`.

use rmcp::{
    handler::server::wrapper::Parameters,
    model::{PromptMessage, Role},
    prompt, prompt_router,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Arguments for the `summarize` prompt.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct SummarizeArgs {
    /// The text to summarize.
    pub text: String,
    /// Roughly how many sentences the summary should run to.
    pub sentences: Option<u32>,
}

/// Arguments for the `explain-error` prompt.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ExplainErrorArgs {
    /// The error message or stack trace to explain.
    pub error: String,
    /// Language or runtime it came from, if known.
    pub language: Option<String>,
}

// Unlike `tool_router`, `prompt_router` takes the router name as a
// string literal rather than an identifier.
#[prompt_router(router = "demo_prompts", vis = "pub(crate)")]
impl crate::server::DemoServer {
    /// Summarize a piece of text.
    #[prompt(
        name = "summarize",
        description = "Summarize text in a given number of sentences."
    )]
    pub async fn summarize(
        &self,
        Parameters(SummarizeArgs { text, sentences }): Parameters<SummarizeArgs>,
    ) -> Vec<PromptMessage> {
        let sentences = sentences.unwrap_or(3);

        vec![PromptMessage::new_text(
            Role::User,
            format!("Summarize the following in about {sentences} sentences.\n\n{text}"),
        )]
    }

    /// Explain an error message.
    #[prompt(
        name = "explain-error",
        description = "Explain an error message and suggest fixes."
    )]
    pub async fn explain_error(
        &self,
        Parameters(ExplainErrorArgs { error, language }): Parameters<ExplainErrorArgs>,
    ) -> Vec<PromptMessage> {
        let context = match language {
            Some(language) => format!(" This is {language} code."),
            None => String::new(),
        };

        vec![PromptMessage::new_text(
            Role::User,
            format!("Explain what this error means and how to fix it.{context}\n\n{error}"),
        )]
    }
}
