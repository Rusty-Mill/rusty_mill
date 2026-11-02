//! The hook files setup writes whole for agents whose session-start hooks
//! are code: an OpenCode plugin and a Pi extension. Each runs
//! `rusty-mux-ctl agent hook` by absolute path once when the agent starts
//! and adds what it prints, nothing outside a block, to the system prompt.
//! A file is ours only when it starts with [`MARK`].

use rusty_json::Value;

/// The first line of every file this module writes.
pub(super) const MARK: &str =
    "// Installed by `rusty-mux-ctl agent setup`; `rusty-mux-ctl agent remove` deletes it.\n";

/// The OpenCode plugin. `experimental.chat.system.transform` is OpenCode's
/// documented way to add system prompt lines; a ctl that fails or is gone
/// adds nothing, as a hook that prints nothing does.
pub(super) fn opencode(ctl: &str) -> String {
    format!(
        r#"{MARK}// Tells OpenCode which rusty_mux block it runs in.
import {{ execFile }} from "node:child_process";

const CTL = {ctl};

export const RustyMux = async () => {{
  const text = await new Promise((resolve) => {{
    execFile(CTL, ["agent", "hook"], {{ timeout: 5000 }}, (error, stdout) =>
      resolve(error ? "" : stdout.trim()),
    );
  }});
  if (!text) return {{}};
  return {{
    "experimental.chat.system.transform": async (_input, output) => {{
      output.system.push(text);
    }},
  }};
}};
"#,
        ctl = literal(ctl)
    )
}

/// The Pi extension: `session_start` asks the ctl once, and
/// `before_agent_start` adds the answer to each run's system prompt, as a
/// prompt section (`appendSystemPrompt`) where Pi has them, else by
/// returning the whole prompt as older Pi wants.
pub(super) fn pi(ctl: &str) -> String {
    format!(
        r#"{MARK}// Tells Pi which rusty_mux block it runs in.
const CTL = {ctl};

export default function rustyMux(pi: any) {{
  let text = "";
  pi.on("session_start", async () => {{
    const result = await pi.exec(CTL, ["agent", "hook"], {{ timeout: 5000 }});
    text = result.code === 0 ? result.stdout.trim() : "";
  }});
  pi.on("before_agent_start", async (event: any) => {{
    if (!text) return undefined;
    const options = event.systemPromptOptions;
    if (!options) return {{ systemPrompt: `${{event.systemPrompt}}\n\n${{text}}` }};
    options.appendSystemPrompt = options.appendSystemPrompt
      ? `${{options.appendSystemPrompt}}\n\n${{text}}`
      : text;
    return undefined;
  }});
}}
"#,
        ctl = literal(ctl)
    )
}

/// `text` as a JavaScript string literal: JSON's quoting is valid
/// JavaScript and TypeScript, so a path cannot end the string early.
fn literal(text: &str) -> String {
    Value::String(text.to_owned()).to_json_string()
}
