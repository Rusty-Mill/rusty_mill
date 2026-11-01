# Agent placement rules

Where an agent's commands act and where its new blocks go, as
`rusty-mux-ctl` and `rusty-mux-mcp` behave today, and the conventions the
rusty-mux skill asks agents to follow (C34). The skill is
`crates/apps/rusty_mux/crates/ctl/src/agent/SKILL.md`; `rusty-mux-ctl agent
setup` installs it.

## How a target is chosen

1. **Named wins.** A target given on the command line is used, and nothing
   else is consulted. It is a full id, a case-insensitive name, a position
   from 1, or the last four or more characters of an id, tried in that order;
   `SESSION/WINDOW/BLOCK` names the parents too ([`ctl/src/target.rs`](../../../crates/apps/rusty_mux/crates/ctl/src/target.rs)).
   Without a session part, windows and blocks are looked up in the default
   session.
2. **Here.** A missing session, window or block flag takes the one the
   command runs in, from `RUSTY_MUX_SESSION`, `RUSTY_MUX_WINDOW` and
   `RUSTY_MUX_BLOCK`, which the server sets for every block's program.
3. **Otherwise refused.** Outside a block, a missing target is a usage error.
   A target that matches nothing or more than one thing exits 3, an ambiguous
   one listing the ids it matched; ctl never guesses.

There is no "focused" fallback: the server does not track which block a
person is looking at. If window layout (C12) adds focus, it becomes a third
rule between "here" and "refused", and this page changes with it.

## Where an agent puts its work

- New blocks go in the agent's own window, the one it runs in, unless the
  person names another. `run` and `create-block` default there through rule 2,
  and the server places them beside the window's largest block without
  disturbing a zoom. `split-block --side SIDE` puts a block beside a chosen
  one, the agent's own by default, and unzooms the window so it shows; use it
  only when the person should see the block.
- An agent names every block it creates (`rename-block`), so later commands
  target it by name. Positions shift when blocks close.
- An agent types (`input`, `terminal.send-keys`) only into blocks it created,
  or ones the person pointed it at. Typing needs the input lease, so a person
  typing in a block holds it and the agent's request is refused rather than
  interleaved.
- An agent closes the blocks it created when done, unless asked to keep them.
- An agent waits with `wait` or `run --wait` rather than polling the screen.

## The session-start hook

`agent setup` adds a hook that runs `rusty-mux-ctl agent hook` when the
agent starts: a `SessionStart` hook for Claude Code and Codex, a plugin for
OpenCode and an extension for Pi, which add its text to the system prompt.
Inside a block it prints one paragraph naming the block, window and session
and pointing at the skill; elsewhere it prints nothing, so the hook costs
nothing outside rusty_mux. Codex runs a new hook only after it is trusted
(`/hooks` in Codex).
