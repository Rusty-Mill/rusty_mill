//! A block's status from its two producers: the program, by OSC 7501 on
//! its output, and the agent's hooks, through the server. Hooks know more
//! (the tool in use, a permission wait), so they win; OSC 7501 fills in
//! where they are silent.

use std::time::{Duration, Instant};

use crate::{Records, Report};

/// How long hooks must be silent before a program report that disagrees
/// with them takes the block. A finished turn is silent by design, so only
/// new program work, not the silence itself, ends their hold.
pub const HOOK_HOLD: Duration = Duration::from_secs(120);

/// The records of both producers and which one is shown.
#[derive(Debug, Default)]
pub struct Status {
    program: Records,
    hook: Records,
    /// When the last hook report arrived, while hooks hold the block.
    held_since: Option<Instant>,
}

impl Status {
    /// A report the program sent with OSC 7501.
    pub fn program(&mut self, report: Report, now: Instant) {
        self.program.apply(report);
        let silent = self
            .held_since
            .is_some_and(|at| now.saturating_duration_since(at) >= HOOK_HOLD);
        if silent && self.program.states() != self.hook.states() {
            self.held_since = None;
        }
    }

    /// A report from the agent's hooks: they hold the block from now on.
    pub fn hook(&mut self, report: Report, now: Instant) {
        self.hook.apply(report);
        self.held_since = Some(now);
    }

    /// A new shell prompt ends working and blocked records of both.
    pub fn prompt(&mut self) {
        self.program.prompt();
        self.hook.prompt();
    }

    /// A full reset ends every record, and the hooks' hold.
    pub fn reset(&mut self) {
        self.program.reset();
        self.hook.reset();
        self.held_since = None;
    }

    /// The records shown: the hooks' while they hold the block and have
    /// any, else the program's.
    pub fn shown(&self) -> &Records {
        match self.held_since {
            Some(_) if !self.hook.is_empty() => &self.hook,
            _ => &self.program,
        }
    }
}

#[cfg(test)]
mod tests;
