//! The hook files setup writes whole for agents whose session-start hooks
//! are code: an OpenCode plugin and a Pi extension. Each runs
//! `rusty-mux-ctl agent hook` by absolute path once when the agent starts
//! and adds what it prints, nothing outside a block, to the system prompt.
//! Inside a block it also reports the agent's turns and tools with
//! `agent hook --event` (O2), without waiting for the answer and in order
//! ([`ORDERED`]). A file is ours
//! only when it starts with [`MARK`].

use rusty_json::Value;

use super::layout::Ctl;

/// The sender both files use, so reports reach the server in order: one
/// at a time, keeping only the newest while one runs. Plain JavaScript,
/// so it is valid TypeScript too.
pub(super) const ORDERED: &str = include_str!("ordered.js");

/// The first line of every file this module writes.
pub(super) const MARK: &str =
    "// Installed by `rusty-mux-ctl agent setup`; `rusty-mux-ctl agent remove` deletes it.\n";

/// The OpenCode plugin. `experimental.chat.system.transform` is OpenCode's
/// documented way to add system prompt lines; a ctl that fails or is gone
/// adds nothing, as a hook that prints nothing does. A new message or tool
/// reports `working`, a permission request `blocked`, an idle or failed
/// session `idle` or `error`.
pub(super) fn opencode(ctl: Ctl<'_>) -> String {
    format!(
        r#"{MARK}// Tells OpenCode which rusty_mux block it runs in, and rusty_mux what OpenCode does.
import {{ execFile }} from "node:child_process";

const CTL = {ctl};

{ORDERED}
const send = ordered((args, done) => execFile(CTL, args, {{ timeout: 5000 }}, () => done()));

const report = (state, msg, kind) => {{
  const args = ["agent", "hook", "--event", state, "--app", "opencode"];
  if (kind) args.push("--kind", kind);
  if (msg) args.push("--msg", String(msg));
  send(args);
}};

export const RustyMux = async () => {{
  const text = await new Promise((resolve) => {{
    execFile(CTL, {args}, {{ timeout: 5000 }}, (error, stdout) =>
      resolve(error ? "" : stdout.trim()),
    );
  }});
  if (!text) return {{}};
  return {{
    "experimental.chat.system.transform": async (_input, output) => {{
      output.system.push(text);
    }},
    "chat.message": async () => report("working"),
    "tool.execute.before": async (input) => report("working", input?.tool),
    "permission.ask": async (input) => report("blocked", input?.title, "permission"),
    event: async ({{ event }}) => {{
      if (event?.type === "session.idle") report("idle");
      if (event?.type === "session.error") report("error");
    }},
  }};
}};
"#,
        ctl = literal(ctl.path),
        args = list(ctl.args()),
    )
}

/// The Pi extension: `session_start` asks the ctl once, and
/// `before_agent_start` adds the answer to each run's system prompt, as a
/// prompt section (`appendSystemPrompt`) where Pi has them, else by
/// returning the whole prompt as older Pi wants. Inside a block, a run or
/// tool call reports `working` and a finished run `idle`.
pub(super) fn pi(ctl: Ctl<'_>) -> String {
    format!(
        r#"{MARK}// Tells Pi which rusty_mux block it runs in, and rusty_mux what Pi does.
const CTL = {ctl};

{ORDERED}
export default function rustyMux(pi: any) {{
  let text = "";
  const send = ordered((args, done) => {{
    pi.exec(CTL, args, {{ timeout: 5000 }}).then(() => done(), () => done());
  }});
  const report = (state: string, msg?: string) => {{
    if (!text) return;
    const args = ["agent", "hook", "--event", state, "--app", "pi"];
    if (msg) args.push("--msg", String(msg));
    send(args);
  }};
  pi.on("agent_start", async () => {{ report("working"); }});
  pi.on("tool_call", async (event: any) => {{ report("working", event?.toolName); }});
  pi.on("agent_end", async () => {{ report("idle"); }});
  pi.on("session_start", async () => {{
    const result = await pi.exec(CTL, {args}, {{ timeout: 5000 }});
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
        ctl = literal(ctl.path),
        args = list(ctl.args()),
    )
}

/// `words` as a JavaScript array of string literals, spaced as the hook
/// files always were: `["agent", "hook"]`.
fn list(words: &[&str]) -> String {
    let items: Vec<String> = words.iter().map(|w| literal(w)).collect();
    format!("[{}]", items.join(", "))
}

/// `text` as a JavaScript string literal: JSON's quoting is valid
/// JavaScript and TypeScript, so a path cannot end the string early.
fn literal(text: &str) -> String {
    Value::String(text.to_owned()).to_json_string()
}
