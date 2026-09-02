//! Getting back into a conversation: resumption and hand-off primers.
//!
//! Two different moves. **Resume** reopens the original session in the tool
//! that owns it, with the transcript attached. **Primer** condenses a thread
//! into an opening message you can paste into any other tool — which is the
//! only option when the tool that produced it cannot resume, and often the
//! better one when you are switching tools deliberately.

use crate::index::Inventory;
use crate::model::{Conversation, Message, Role, SourceId};
use crate::{Error, Result};
use std::path::PathBuf;

/// A command to run, prepared but not executed. Returning it rather than
/// spawning keeps the decision to launch a terminal with the caller.
#[derive(Debug, Clone)]
pub struct ResumeCommand {
    pub program: String,
    pub args: Vec<String>,
    /// Where to run it. Falls back to the home directory when the original
    /// project folder no longer exists.
    pub cwd: PathBuf,
    /// True when the recorded project path was missing and we fell back —
    /// "even if the project folder has moved".
    pub project_moved: bool,
    /// Full transcript, for handing to the agent when it cannot reload the
    /// session itself.
    pub transcript: String,
}

impl ResumeCommand {
    /// The command as you would type it.
    pub fn display(&self) -> String {
        let mut out = self.program.clone();
        for a in &self.args {
            out.push(' ');
            if a.contains(' ') {
                out.push_str(&format!("\"{a}\""));
            } else {
                out.push_str(a);
            }
        }
        out
    }
}

impl Inventory {
    /// Build the command that reopens a conversation in its own tool.
    ///
    /// Only Claude Code and Codex support this; the other four have no
    /// documented way to reopen a session from outside, so they get a primer
    /// instead rather than a command that would silently do the wrong thing.
    pub fn resume(&self, conversation_id: i64) -> Result<ResumeCommand> {
        let (conversation, messages) = self.conversation(conversation_id)?;

        let (program, args) = match conversation.source {
            SourceId::ClaudeCode => (
                "claude".to_string(),
                vec!["--resume".to_string(), conversation.external_id.clone()],
            ),
            SourceId::Codex => (
                "codex".to_string(),
                vec!["resume".to_string(), conversation.external_id.clone()],
            ),
            other => {
                return Err(Error::other(format!(
                    "{} cannot reopen a session from outside the app — use `inv primer {}` \
                     to paste the thread into any tool instead",
                    other.display_name(),
                    conversation_id
                )))
            }
        };

        let recorded = conversation.project_path.as_ref().map(PathBuf::from);
        let (cwd, project_moved) = match recorded {
            Some(p) if p.is_dir() => (p, false),
            _ => (
                crate::paths::home_dir().unwrap_or_else(|| PathBuf::from(".")),
                conversation.project_path.is_some(),
            ),
        };

        Ok(ResumeCommand {
            program,
            args,
            cwd,
            project_moved,
            transcript: full_transcript(&conversation, &messages),
        })
    }

    /// A condensed primer: what was being solved, the last exchanges, and the
    /// code. Sized to paste as an opening message.
    pub fn primer(&self, conversation_id: i64) -> Result<String> {
        let (conversation, messages) = self.conversation(conversation_id)?;
        Ok(build_primer(&conversation, &messages))
    }
}

fn full_transcript(conversation: &Conversation, messages: &[Message]) -> String {
    let mut out = format!(
        "# {}\n_{} · {}_\n\n",
        conversation.title,
        conversation.source.display_name(),
        crate::format::timestamp(conversation.updated_at)
    );
    for m in messages {
        out.push_str(&format!("## {}\n{}\n\n", m.role.as_str(), m.text));
    }
    out
}

/// How many trailing exchanges the primer carries.
const TAIL_MESSAGES: usize = 6;
/// Per-message cap, so one enormous tool dump cannot crowd out the rest.
const MAX_CHARS_PER_MESSAGE: usize = 1_500;
const MAX_CODE_BLOCKS: usize = 4;

fn build_primer(conversation: &Conversation, messages: &[Message]) -> String {
    let mut out = String::new();

    out.push_str("Here is context from an earlier conversation I want to continue.\n\n");
    out.push_str(&format!(
        "**What I was working on:** {}\n",
        conversation.title
    ));
    if let Some(p) = &conversation.project_path {
        out.push_str(&format!("**Project:** {p}\n"));
    }
    if let Some(b) = &conversation.git_branch {
        out.push_str(&format!("**Branch:** {b}\n"));
    }
    out.push_str(&format!(
        "**Originally in:** {} · {} · {} messages\n\n",
        conversation.source.display_name(),
        crate::format::date(conversation.updated_at),
        conversation.message_count
    ));

    // The opening ask states the problem better than any summary of it.
    if let Some(first) = messages.iter().find(|m| m.role == Role::User) {
        out.push_str("**How it started:**\n");
        out.push_str(&quote(&truncate(&first.text, MAX_CHARS_PER_MESSAGE)));
        out.push_str("\n\n");
    }

    let tail_start = messages.len().saturating_sub(TAIL_MESSAGES);
    let tail: Vec<&Message> = messages[tail_start..]
        .iter()
        .filter(|m| m.role == Role::User || m.role == Role::Assistant)
        .collect();
    if !tail.is_empty() {
        out.push_str("**Where it got to:**\n\n");
        for m in tail {
            out.push_str(&format!(
                "*{}:* {}\n\n",
                m.role.as_str(),
                truncate(&m.text, MAX_CHARS_PER_MESSAGE)
            ));
        }
    }

    let blocks = code_blocks(messages);
    if !blocks.is_empty() {
        out.push_str("**Code from the conversation:**\n\n");
        for block in blocks.iter().take(MAX_CODE_BLOCKS) {
            out.push_str(block);
            out.push_str("\n\n");
        }
    }

    out.push_str("Please pick up from here.\n");
    out
}

/// Fenced blocks, most recent first — the later code is the code that survived.
fn code_blocks(messages: &[Message]) -> Vec<String> {
    let mut blocks = Vec::new();
    for m in messages.iter().rev() {
        let mut in_block = false;
        let mut current = String::new();
        for line in m.text.lines() {
            if line.trim_start().starts_with("```") {
                if in_block {
                    current.push_str("```");
                    if current.lines().count() > 2 {
                        blocks.push(std::mem::take(&mut current));
                    } else {
                        current.clear();
                    }
                    in_block = false;
                } else {
                    in_block = true;
                    current.push_str(line);
                    current.push('\n');
                }
                continue;
            }
            if in_block {
                current.push_str(line);
                current.push('\n');
            }
        }
        if blocks.len() >= MAX_CODE_BLOCKS {
            break;
        }
    }
    blocks
}

fn truncate(text: &str, max: usize) -> String {
    let text = text.trim();
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut s: String = text.chars().take(max).collect();
    s.push_str("\n… [truncated]");
    s
}

fn quote(text: &str) -> String {
    text.lines()
        .map(|l| format!("> {l}"))
        .collect::<Vec<_>>()
        .join("\n")
}
