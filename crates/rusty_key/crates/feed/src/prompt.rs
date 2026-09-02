//! The static, per-session system prompt producer (PRD 03 §System-prompt
//! construction). Built once per session; constant thereafter (drives prompt
//! caching). Phase-1 scope: identity + the H1 tool-use layer. Memory/H2/H3
//! layers and a prompt registry land in later phases.

use rk_config::HarnessLevel;

/// Produce the system prompt for `level`. Layers are additive by level
/// (the controlled-visibility ladder, ARCHITECTURE §3).
pub fn system_prompt(level: HarnessLevel) -> String {
    let mut s = String::from(
        "You are Rusty Keys, an autonomous engineering agent operating inside a \
         workspace. Use the provided tools to inspect and act. Prefer minimal, \
         reversible actions and report what you did, with evidence and limits.",
    );

    if level >= HarnessLevel::H1 {
        s.push_str(
            "\n\nTool-use protocol: call tools by name with JSON arguments. A blocked \
             or errored tool returns a structured result to observe and recover from, \
             not a hard stop. The workspace is the policy boundary; do not act outside it.",
        );
    }

    s
}

/// System prompt for the compaction summariser (PRD 06 / Phase 8).
pub const COMPACTION_SYSTEM: &str =
    "You compress an agent's conversation history. Produce a dense, factual \
     summary that preserves decisions, open questions, file paths, and the \
     current task state. Output ONLY the summary prose — no preamble.";

/// Build the summarisation prompt over a flattened `transcript`. Used for the
/// `session` (oldest half) and `full` (all) compaction tiers.
pub fn compaction_prompt(transcript: &str) -> String {
    format!(
        "Summarise the following conversation history so a future turn can \
         continue without it. Keep it under ~200 words.\n\n{transcript}"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compaction_prompt_embeds_transcript() {
        let p = compaction_prompt("user: hi\nassistant: hello");
        assert!(p.contains("user: hi"));
        assert!(p.contains("Summarise"));
    }

    #[test]
    fn h1_includes_tool_use_layer() {
        let p = system_prompt(HarnessLevel::H1);
        assert!(p.contains("Tool-use protocol"));
        assert!(p.contains("Rusty Keys"));
    }

    #[test]
    fn h0_omits_tool_use_layer() {
        let p = system_prompt(HarnessLevel::H0);
        assert!(!p.contains("Tool-use protocol"));
    }
}
