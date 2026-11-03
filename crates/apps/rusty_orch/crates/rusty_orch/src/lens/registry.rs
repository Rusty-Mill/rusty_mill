//! The provider registry: one [`Model`] per configured provider, built from
//! the lens config's `providers` block (#594, design §2).
//!
//! Each provider's adapter-specific `settings` are checked here, at load,
//! with the field named, so a typo never waits for the first call. The
//! operator's billing and host assertions are passed through to the CLI
//! adapters, which fail closed without them: building a registry never
//! makes a call, and an unasserted provider is `Unavailable` when asked.

use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

use orch_claude::ClaudeAgent;
use orch_cli::{CommandRunner, StdCommand};
use orch_codex::CodexAgent;
use orch_core::AgentId;
use orch_lens::config::{ProviderKind, Tools};
use orch_lens::{LensConfig, Model};
use orch_openai::{Config as HttpConfig, OpenAiHttp};
use rusty_json::Map;

/// A local model server's deadline when the config names none. Large
/// local models answer a 32k prompt in minutes, not seconds.
pub const HTTP_TIMEOUT: Duration = Duration::from_secs(300);

/// The longest `timeout_secs` a provider may set: one day. Larger values
/// mean a typo, and an unbounded one could overflow a deadline.
pub const MAX_TIMEOUT_SECS: u64 = 86_400;

/// What the operator asserts about the hosts and accounts the CLI
/// providers run on. The adapters cannot check either, so both default to
/// false and the providers that need them stay `Unavailable`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Assertions {
    /// Credits and extra usage are disabled on every subscription a CLI
    /// provider logs in with, so a call can never be billed per token.
    pub no_overage: bool,
    /// Everything the Codex host lets Codex read is cleared for it.
    pub host_cleared: bool,
}

/// One provider's adapters. Claude runs tool-less unless a lens asks for
/// read-only tools, so it has one adapter for each; Codex has no tool-less
/// mode and the HTTP adapter has no tools, so one serves both.
struct Entry {
    plain: Box<dyn Model>,
    read_only: Option<Box<dyn Model>>,
}

/// Every configured provider's adapter, by id.
pub struct Registry {
    entries: BTreeMap<AgentId, Entry>,
}

impl Registry {
    /// Adapters over the real CLIs and network, working in `repo`.
    pub fn build(config: &LensConfig, repo: &Path, a: Assertions) -> Result<Self, String> {
        Self::build_with(config, repo, a, || StdCommand)
    }

    /// As [`Registry::build`], with each CLI adapter's process runner made
    /// by `runner` (a fake in tests).
    pub fn build_with<C, F>(
        config: &LensConfig,
        repo: &Path,
        a: Assertions,
        runner: F,
    ) -> Result<Self, String>
    where
        C: CommandRunner + 'static,
        F: Fn() -> C,
    {
        let mut entries = BTreeMap::new();
        for (id, p) in &config.providers {
            let at = |e: String| format!("providers.{id}.{e}");
            let entry = match p.kind {
                ProviderKind::ClaudeCli => claude(&p.settings, repo, a, &runner).map_err(at)?,
                ProviderKind::CodexCli => codex(&p.settings, repo, a, &runner).map_err(at)?,
                ProviderKind::OpenAiHttp => http(&p.settings).map_err(at)?,
            };
            entries.insert(*id, entry);
        }
        Ok(Self { entries })
    }

    /// The adapter for provider `id` with a lens's `tools`, or `None` when
    /// `id` is not configured.
    pub fn get(&mut self, id: AgentId, tools: Tools) -> Option<&mut dyn Model> {
        let e = self.entries.get_mut(&id)?;
        match (tools, e.read_only.as_deref_mut()) {
            (Tools::ReadOnly, Some(m)) => Some(m),
            _ => Some(e.plain.as_mut()),
        }
    }

    /// The configured provider ids.
    pub fn ids(&self) -> impl Iterator<Item = AgentId> + '_ {
        self.entries.keys().copied()
    }
}

fn claude<C: CommandRunner + 'static>(
    s: &Map,
    repo: &Path,
    a: Assertions,
    runner: &impl Fn() -> C,
) -> Result<Entry, String> {
    only(s, &["model", "timeout_secs"])?;
    let (model, timeout) = (opt_str(s, "model")?, opt_secs(s)?);
    let make = || {
        let mut agent = ClaudeAgent::with_runner(repo, runner());
        if let Some(m) = model {
            agent = agent.model(m);
        }
        if let Some(t) = timeout {
            agent = agent.timeout(t);
        }
        if a.no_overage {
            agent = agent.lens_no_overage();
        }
        agent
    };
    Ok(Entry {
        plain: Box::new(make()),
        read_only: Some(Box::new(make().lens_read_only())),
    })
}

fn codex<C: CommandRunner + 'static>(
    s: &Map,
    repo: &Path,
    a: Assertions,
    runner: &impl Fn() -> C,
) -> Result<Entry, String> {
    only(s, &["model", "timeout_secs"])?;
    let mut agent = CodexAgent::with_runner(repo, runner());
    if let Some(m) = opt_str(s, "model")? {
        agent = agent.model(m);
    }
    if let Some(t) = opt_secs(s)? {
        agent = agent.timeout(t);
    }
    if a.no_overage {
        agent = agent.lens_no_overage();
    }
    if a.host_cleared {
        agent = agent.lens_host_cleared();
    }
    Ok(Entry {
        plain: Box::new(agent),
        read_only: None,
    })
}

fn http(s: &Map) -> Result<Entry, String> {
    only(
        s,
        &[
            "base_url",
            "model",
            "api_key_env",
            "timeout_secs",
            "json_mode",
        ],
    )?;
    let required = |k: &str| opt_str(s, k)?.ok_or_else(|| format!("{k}: required"));
    let json_mode = match s.get("json_mode") {
        None => false,
        Some(v) => v.as_bool().ok_or("json_mode: true or false")?,
    };
    let config = HttpConfig::new(
        required("base_url")?,
        required("model")?,
        opt_str(s, "api_key_env")?,
        opt_secs(s)?.unwrap_or(HTTP_TIMEOUT),
        json_mode,
    )
    .map_err(|e| e.to_string())?;
    Ok(Entry {
        plain: Box::new(OpenAiHttp::new(config)),
        read_only: None,
    })
}

/// Refuse any setting this adapter does not read.
fn only(s: &Map, keys: &[&str]) -> Result<(), String> {
    match s.keys().find(|k| !keys.contains(&k.as_str())) {
        Some(k) => Err(format!("{k}: not a setting of this provider kind")),
        None => Ok(()),
    }
}

fn opt_str<'a>(s: &'a Map, k: &str) -> Result<Option<&'a str>, String> {
    match s.get(k) {
        None => Ok(None),
        Some(v) => v
            .as_str()
            .filter(|t| !t.trim().is_empty())
            .map(Some)
            .ok_or_else(|| format!("{k}: non-empty string required")),
    }
}

fn opt_secs(s: &Map) -> Result<Option<Duration>, String> {
    match s.get("timeout_secs") {
        None => Ok(None),
        Some(v) => v
            .as_u64()
            .filter(|n| (1..=MAX_TIMEOUT_SECS).contains(n))
            .map(|n| Some(Duration::from_secs(n)))
            .ok_or_else(|| format!("timeout_secs: a whole number from 1 to {MAX_TIMEOUT_SECS}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::path::PathBuf;
    use std::sync::Arc;

    use orch_cli::fake::FakeCommand;
    use orch_cli::{ExecError, Exit};
    use orch_dispatch::ClassifiedError;
    use orch_lens::{Allowed, Question};

    /// One fake shared by every adapter, so a test sees all their calls.
    #[derive(Clone)]
    struct Shared(Arc<FakeCommand>);

    impl CommandRunner for Shared {
        fn run_in(
            &self,
            cwd: Option<&Path>,
            argv: &[String],
            stdin: &[u8],
            timeout: Duration,
            remove_env: &[&str],
        ) -> Result<Exit, ExecError> {
            self.0.run_in(cwd, argv, stdin, timeout, remove_env)
        }
    }

    const ZERO: &str =
        r#""context_tokens": 32768, "price": {"input": 0, "output": 0, "cached_input": 0}"#;

    fn config(claude: &str, codex: &str, local: &str) -> LensConfig {
        let text = format!(
            r#"{{
              "providers": {{
                "claude": {{"kind": "claude-cli", {ZERO}{claude}}},
                "codex": {{"kind": "codex-cli", {ZERO}{codex}}},
                "local": {{"kind": "openai-http", {ZERO}{local}}}
              }},
              "ceilings": {{"experiment_usd": 0, "task_usd": 0, "task_calls": 10}},
              "lenses": [
                {{"id": "research", "prompt": "p", "domains": ["*"],
                  "may_post": {{"evidence": ["counter"]}}, "tools": "read-only",
                  "models": {{"routine": ["claude", "codex"], "contested": ["claude"]}}}},
                {{"id": "operator", "prompt": "p", "domains": ["*"],
                  "may_post": {{"evidence": ["counter"]}},
                  "models": {{"routine": ["local"], "contested": ["local"]}}}}
              ]
            }}"#
        );
        LensConfig::parse(&text).expect("config loads")
    }

    const LOCAL: &str = r#", "base_url": "http://127.0.0.1:8080/v1", "model": "qwen""#;

    fn id(s: &str) -> AgentId {
        AgentId::new(s).expect("id")
    }

    fn build(c: &LensConfig, a: Assertions) -> (Result<Registry, String>, Arc<FakeCommand>) {
        // Passes the subscription probe, then is not a valid reply.
        let fake = Arc::new(FakeCommand::ok(r#"{"authMethod":"claude.ai"}"#));
        let shared = Shared(Arc::clone(&fake));
        let r = Registry::build_with(c, &PathBuf::from("/repo"), a, || shared.clone());
        (r, fake)
    }

    fn question() -> Question {
        Question {
            prefix: "rules".to_owned(),
            body: "claim".to_owned(),
            allowed: Allowed {
                evidence: None,
                claims: None,
            },
            max_output_tokens: 256,
        }
    }

    fn calls(fake: &FakeCommand) -> Vec<Vec<String>> {
        let calls = fake.calls.lock().expect("lock");
        calls.iter().map(|(argv, _, _)| argv.clone()).collect()
    }

    #[test]
    fn every_provider_gets_an_adapter_and_unknown_ids_none() {
        let (r, _) = build(&config("", "", LOCAL), Assertions::default());
        let mut r = r.expect("builds");
        let mut ids: Vec<String> = r.ids().map(|i| i.to_string()).collect();
        ids.sort();
        assert_eq!(ids, ["claude", "codex", "local"]);
        assert!(r.get(id("gemini"), Tools::None).is_none());
    }

    #[test]
    fn without_assertions_cli_providers_are_unavailable_and_nothing_runs() {
        let (r, fake) = build(&config("", "", LOCAL), Assertions::default());
        let mut r = r.expect("builds");
        for (p, tools) in [
            ("claude", Tools::None),
            ("claude", Tools::ReadOnly),
            ("codex", Tools::None),
        ] {
            let m = r.get(id(p), tools).expect("configured");
            let e = m.ask(&question()).expect_err("unasserted");
            assert!(matches!(e, ClassifiedError::Unavailable(_)), "{p}: {e:?}");
        }
        assert!(calls(&fake).is_empty());
    }

    #[test]
    fn codex_needs_both_assertions() {
        let only_overage = Assertions {
            no_overage: true,
            host_cleared: false,
        };
        let (r, fake) = build(&config("", "", LOCAL), only_overage);
        let mut r = r.expect("builds");
        let e = r
            .get(id("codex"), Tools::None)
            .expect("configured")
            .ask(&question())
            .expect_err("host not cleared");
        assert!(matches!(e, ClassifiedError::Unavailable(_)), "{e:?}");
        assert!(calls(&fake).is_empty());
    }

    #[test]
    fn asserted_claude_runs_with_settings_and_the_lens_tools() {
        let a = Assertions {
            no_overage: true,
            host_cleared: true,
        };
        let (r, fake) = build(
            &config(r#", "model": "opus", "timeout_secs": 90"#, "", LOCAL),
            a,
        );
        let mut r = r.expect("builds");
        for tools in [Tools::None, Tools::ReadOnly] {
            let e = r
                .get(id("claude"), tools)
                .expect("configured")
                .ask(&question())
                .expect_err("the fake's reply is not a lens answer");
            assert!(matches!(e, ClassifiedError::Permanent(_)), "{e:?}");
        }
        let main: Vec<_> = calls(&fake)
            .into_iter()
            .filter(|argv| argv.iter().any(|a| a == "--system-prompt"))
            .collect();
        assert_eq!(main.len(), 2, "one probe and one ask per question");
        let tools_of = |argv: &Vec<String>| {
            let at = argv.iter().position(|a| a == "--tools").expect("--tools");
            argv[at + 1].clone()
        };
        assert_eq!(tools_of(&main[0]), "");
        assert_eq!(tools_of(&main[1]), orch_claude::TOOLS);
        assert!(main[0].windows(2).any(|w| w == ["--model", "opus"]));
        let timeouts: Vec<_> = fake
            .calls
            .lock()
            .expect("lock")
            .iter()
            .filter(|(argv, _, _)| argv.iter().any(|a| a == "--system-prompt"))
            .map(|(_, _, t)| *t)
            .collect();
        assert_eq!(timeouts, [Duration::from_secs(90); 2]);
    }

    #[test]
    fn settings_are_checked_at_load_with_the_field_named() {
        let refused = |claude: &str, codex: &str, local: &str, expected: &str| {
            let (r, fake) = build(&config(claude, codex, local), Assertions::default());
            let e = r.err().expect(expected);
            assert!(e.contains(expected), "{expected:?} not in {e:?}");
            assert!(calls(&fake).is_empty());
        };
        refused(
            r#", "modle": "opus""#,
            "",
            LOCAL,
            "providers.claude.modle: not a setting",
        );
        refused(
            "",
            r#", "timeout_secs": 0"#,
            LOCAL,
            "providers.codex.timeout_secs",
        );
        refused(
            "",
            r#", "model": """#,
            LOCAL,
            "providers.codex.model: non-empty",
        );
        refused(
            "",
            "",
            r#", "model": "qwen""#,
            "providers.local.base_url: required",
        );
        refused(
            "",
            "",
            r#", "base_url": "https://baileyai/v1", "model": "qwen""#,
            "providers.local.base_url: https is not supported",
        );
        refused(
            "",
            "",
            r#", "base_url": "http://baileyai/v1", "model": "q", "json_mode": "yes""#,
            "providers.local.json_mode",
        );
        refused(
            "",
            "",
            r#", "base_url": "http://baileyai/v1", "model": "q", "api_key_env": "a b""#,
            "providers.local.api_key_env",
        );
    }
}
