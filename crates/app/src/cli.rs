//! Extended CLI rendering (PRD 06 / Phase 13). Pure render functions for the
//! REPL commands that the snapshot tests pin (`/help`, `/stats`, banner,
//! `/config`, `/doctor`); the REPL in `main.rs` wires them to `Session` data.

use rk_config::Config;

/// The full slash-command list (`/help`).
pub fn help_text() -> &'static str {
    "commands:\n\
     turn:     <text> to send · /plan [goal] · /explore <task>\n\
     verify:   /verify · /mhir · /entropy · /evidence\n\
     memory:   /memory · /reflect · /sleep · /groom · /task [goal | criteria]\n\
     context:  /cost · /compact · /stats · /model [name]\n\
     git:      /diff · /commit [msg] · /branch [name] · /review\n\
     env:      /config · /env · /permissions [mode] · /doctor · /init · /mcp\n\
     session:  /help · /quit"
}

/// A combined-stats snapshot (`/stats`).
#[derive(Debug, Clone, PartialEq)]
pub struct Stats {
    /// Line-item tokens used on the last turn.
    pub tokens_used: usize,
    /// The context-window limit.
    pub tokens_limit: usize,
    /// Turns completed this session.
    pub turns: usize,
    /// Cumulative tool calls this session.
    pub tool_calls: usize,
    /// M-HIR rate.
    pub mhir_rate: f64,
    /// Cumulative entropy delta (negative = burden introduced).
    pub entropy_delta: i64,
    /// Compactions this session.
    pub compactions: usize,
}

/// Render `/stats`.
pub fn render_stats(s: &Stats) -> String {
    let pct = if s.tokens_limit > 0 {
        (s.tokens_used as f64 / s.tokens_limit as f64) * 100.0
    } else {
        0.0
    };
    format!(
        "stats :: turns={} tool_calls={} compactions={}\n\
         tokens :: {} / {} ({:.0}%)\n\
         m-hir  :: {:.3}\n\
         entropy:: delta={}",
        s.turns,
        s.tool_calls,
        s.compactions,
        s.tokens_used,
        s.tokens_limit,
        pct,
        s.mhir_rate,
        s.entropy_delta
    )
}

/// Render `/config` — the resolved `RUSTYKEYS_*` settings (configuration SSOT).
pub fn render_config(c: &Config) -> String {
    let mut lines = vec![format!("RUSTYKEYS_MODEL              = {}", c.model)];
    lines.push(format!(
        "RUSTYKEYS_WORKSPACE          = {}",
        c.workspace.display()
    ));
    lines.push(format!(
        "RUSTYKEYS_HARNESS_LEVEL      = {:?}",
        c.harness_level
    ));
    lines.push(format!(
        "RUSTYKEYS_EMBED_MODEL        = {}",
        c.embed_model.as_deref().unwrap_or("(unset)")
    ));
    lines.push(format!("RUSTYKEYS_ALLOW_WEB          = {}", c.allow_web));
    lines.push(format!(
        "RUSTYKEYS_PERMISSION_MODE    = {}",
        c.permission_mode
    ));
    lines.push(format!("RUSTYKEYS_ISOLATION          = {}", c.isolation));
    lines.push(format!(
        "RUSTYKEYS_CONTEXT_LIMIT      = {}",
        c.context_limit
    ));
    lines.push(format!(
        "RUSTYKEYS_COMPACT_*          = {} / {} / {}",
        c.compact_micro, c.compact_session, c.compact_full
    ));
    lines.push(format!("RUSTYKEYS_EXPLORE            = {}", c.explore));
    lines.join("\n")
}

/// One `/doctor` subsystem check.
#[derive(Debug, Clone, PartialEq)]
pub struct Subsystem {
    /// Subsystem name.
    pub name: String,
    /// Whether it passed.
    pub ok: bool,
    /// Detail / reason.
    pub detail: String,
}

/// Render `/doctor`.
pub fn render_doctor(checks: &[Subsystem]) -> String {
    let mut out = String::from("doctor:\n");
    for c in checks {
        out.push_str(&format!(
            "  [{}] {}: {}\n",
            if c.ok { "ok" } else { "FAIL" },
            c.name,
            c.detail
        ));
    }
    let all = checks.iter().all(|c| c.ok);
    out.push_str(if all {
        "all subsystems OK"
    } else {
        "one or more subsystems FAILED"
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn help_lists_the_command_groups() {
        let h = help_text();
        for needle in ["/plan", "/stats", "/diff", "/doctor", "/mcp", "/quit"] {
            assert!(h.contains(needle), "help missing {needle}");
        }
    }

    #[test]
    fn stats_renders_all_lines() {
        let s = Stats {
            tokens_used: 50,
            tokens_limit: 200,
            turns: 3,
            tool_calls: 7,
            mhir_rate: 0.25,
            entropy_delta: -2,
            compactions: 1,
        };
        let out = render_stats(&s);
        assert!(out.contains("turns=3 tool_calls=7 compactions=1"));
        assert!(out.contains("50 / 200 (25%)"));
        assert!(out.contains("m-hir  :: 0.250"));
        assert!(out.contains("delta=-2"));
    }

    #[test]
    fn doctor_reports_pass_fail_per_subsystem() {
        let checks = vec![
            Subsystem {
                name: "model".into(),
                ok: true,
                detail: "fake".into(),
            },
            Subsystem {
                name: "sqlite".into(),
                ok: false,
                detail: "locked".into(),
            },
        ];
        let out = render_doctor(&checks);
        assert!(out.contains("[ok] model"));
        assert!(out.contains("[FAIL] sqlite"));
        assert!(out.contains("one or more subsystems FAILED"));
    }
}
