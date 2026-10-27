//! `watch`: runs the config's `on` rules for each event (see
//! [`rusty_mux_script::Rule`]).
//!
//! Each rule an event matches runs as `rusty-mux-ctl` would run its chain,
//! on a connection of its own, in the order the rules are defined. A rule
//! that fails is reported and the watch goes on.

use std::io::{self, Write};

use rusty_mux_core::Event;
use rusty_mux_proto::Features;
use rusty_mux_script::Config;

use crate::{Env, Exit, Stop, config, render, run_until};

/// What `watch` needs to run rules.
pub(crate) struct Watcher<'a> {
    pub(crate) config: &'a Config,
    /// With the socket being watched.
    pub(crate) env: &'a Env,
    pub(crate) json: bool,
    pub(crate) stop: &'a Stop,
}

impl Watcher<'_> {
    /// Prints `event` if a rule matches it, then runs each matching rule.
    /// Only failing to write stops the watch.
    pub(crate) fn event(
        &self,
        seq: u64,
        event: &Event,
        out: &mut dyn Write,
        err: &mut dyn Write,
    ) -> io::Result<()> {
        let (kind, mut fields) = render::event_fields(event);
        let rules: Vec<_> = self
            .config
            .rules
            .iter()
            .enumerate()
            .filter(|(_, rule)| rule.event == kind)
            .collect();
        if rules.is_empty() {
            return Ok(());
        }
        writeln!(out, "{}", render::event_line(seq, event, self.json))?;
        if let Some((name, method)) = read_field(kind) {
            let Some(value) = self.read(method, &fields, err) else {
                writeln!(err, "rusty-mux-ctl: skipping event {seq} {kind}: no {name}")?;
                return Ok(());
            };
            fields.push((name.to_owned(), value));
        }
        for (i, rule) in rules {
            let ran = match rule.calls(self.config, &fields) {
                Ok(calls) => self.run(&config::chain(&calls), self.json, out, err) == Exit::Ok,
                Err(reason) => {
                    writeln!(err, "rusty-mux-ctl: {reason}")?;
                    false
                }
            };
            if !ran {
                writeln!(err, "rusty-mux-ctl: on[{i}] failed for event {seq} {kind}")?;
            }
        }
        Ok(())
    }

    /// The text `method` answers for the event's block, such as its working
    /// directory; `None`, with the reason on `err`, if it can't be read.
    fn read(
        &self,
        method: &str,
        fields: &[(String, String)],
        err: &mut dyn Write,
    ) -> Option<String> {
        let (_, block) = fields.iter().find(|(name, _)| name == "block")?;
        let mut text = Vec::new();
        let request = [method.to_owned(), format!("--block={block}")];
        if self.run(&request, false, &mut text, err) != Exit::Ok {
            return None;
        }
        let mut text = String::from_utf8(text).ok()?;
        if text.ends_with('\n') {
            text.pop();
        }
        Some(text)
    }

    /// Runs `chain` as `rusty-mux-ctl` would, at the watched socket.
    fn run(&self, chain: &[String], json: bool, out: &mut dyn Write, err: &mut dyn Write) -> Exit {
        let args: Vec<String> = json
            .then(|| "--json".to_owned())
            .into_iter()
            .chain(chain.iter().cloned())
            .collect();
        run_until(&args, self.env, self.stop, out, err)
    }
}

/// The feature a server must have negotiated to send events of `kind`.
pub(crate) fn needs(kind: &str) -> Features {
    match kind {
        "block-status" => Features::STATUS,
        "pwd-changed" | "title-changed" => Features::TERMINAL_EVENTS,
        "program-exited" | "bell" | "progress-changed" | "notification" => Features::PROGRAM_EVENTS,
        "process-changed" => Features::PROCESS,
        "session-renamed" | "window-renamed" | "block-renamed" | "client-renamed" => {
            Features::LABELS
        }
        "layout-changed" => Features::LAYOUT,
        _ => Features::NONE,
    }
}

/// The field an event of `kind` gets from its block, and the method that
/// reads it.
fn read_field(kind: &str) -> Option<(&'static str, &'static str)> {
    match kind {
        "pwd-changed" => Some(("pwd", "terminal.pwd")),
        "title-changed" => Some(("title", "terminal.title")),
        _ => None,
    }
}
