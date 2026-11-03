//! The provider registry: one [`Model`] per configured provider, built from
//! the lens config's `providers` block (#594, design §2).
//!
//! Each provider's adapter-specific `settings` are checked here, at load,
//! with the field named, so a typo never waits for the first call. The
//! operator's billing and host assertions ([`super::assertions`]) are passed
//! per provider to the CLI adapters, which fail closed without them:
//! building a registry never makes a call, and an unasserted provider is
//! `Unavailable` when asked.

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

use super::assertions::Grants;
use super::container::ContainerSpec;

/// A local model server's deadline when the config names none. Large
/// local models answer a 32k prompt in minutes, not seconds.
pub const HTTP_TIMEOUT: Duration = Duration::from_secs(300);

/// The longest `timeout_secs` a provider may set: one day. Larger values
/// mean a typo, and an unbounded one could overflow a deadline.
pub const MAX_TIMEOUT_SECS: u64 = 86_400;

/// What the operator asserts about one CLI provider's account and host.
/// The adapters cannot check either, so both default to off and the
/// providers that need them stay `Unavailable`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Assertions {
    /// Credits and extra usage are disabled on this account, so a call can
    /// never be billed per token.
    pub no_overage: Option<NoOverage>,
    /// Everything the provider can read where it runs is cleared for it.
    pub host_cleared: Option<Clearance>,
}

/// Where a host-cleared provider may run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Clearance {
    /// On the runner's own host, whose machine id the operator named.
    Host,
    /// Only inside this container, checked before every call.
    Container(ContainerSpec),
}

/// A no-overage assertion: the account it names, and whether it is bound
/// to this machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoOverage {
    pub account: String,
    /// The entry names this host's machine id. Codex cannot report which
    /// account it is logged into, so its assertion counts only when bound
    /// to the dedicated machine whose login the operator set up.
    ///
    /// This is a host binding, not proof of the account: a login changed
    /// on that same machine keeps the grant. Never treat it as account
    /// identity. Pending the owner's acceptance of that limit (#594).
    pub machine_bound: bool,
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
    pub fn build(config: &LensConfig, repo: &Path, grants: &Grants) -> Result<Self, String> {
        Self::build_with(config, repo, grants, || StdCommand)
    }

    /// As [`Registry::build`], with each CLI adapter's process runner made
    /// by `runner` (a fake in tests).
    pub fn build_with<C, F>(
        config: &LensConfig,
        repo: &Path,
        grants: &Grants,
        runner: F,
    ) -> Result<Self, String>
    where
        C: CommandRunner + 'static,
        F: Fn() -> C,
    {
        let mut entries = BTreeMap::new();
        for (id, p) in &config.providers {
            let at = |e: String| format!("providers.{id}.{e}");
            let a = &grants.for_provider(*id);
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
    a: &Assertions,
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
        // Claude runs on the runner's host. A container clearance naming
        // it would authorise a boundary this adapter cannot enter, so it
        // withholds the grant and Claude stays `Unavailable`.
        let contained = matches!(a.host_cleared, Some(Clearance::Container(_)));
        if let (Some(n), false) = (&a.no_overage, contained) {
            agent = agent.lens_no_overage(n.account.clone());
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
    a: &Assertions,
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
    // Codex cannot report its account, so this is a machine binding, not
    // an account check: switching the login on that same machine keeps the
    // grant. It must never be read as account proof (pending owner
    // acceptance, #594).
    if a.no_overage.as_ref().is_some_and(|n| n.machine_bound) {
        agent = agent.lens_no_overage();
    }
    // A container clearance is not honoured until the container check and
    // `podman exec` are wired in (next slice): it fails closed.
    if a.host_cleared == Some(Clearance::Host) {
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

    const ACCOUNT: &str = "nano@example.com";

    fn no_overage(account: &str, machine_bound: bool) -> Option<NoOverage> {
        Some(NoOverage {
            account: account.to_owned(),
            machine_bound,
        })
    }

    fn build(c: &LensConfig, a: Assertions) -> (Result<Registry, String>, Arc<FakeCommand>) {
        let grants: Grants = ["claude", "codex", "local"]
            .into_iter()
            .map(|p| (id(p), a.clone()))
            .collect();
        // Passes the subscription probe for ACCOUNT, then is not a valid reply.
        let fake = Arc::new(FakeCommand::ok(
            r#"{"authMethod":"claude.ai","email":"nano@example.com"}"#,
        ));
        let shared = Shared(Arc::clone(&fake));
        let r = Registry::build_with(c, &PathBuf::from("/repo"), &grants, || shared.clone());
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
            no_overage: no_overage(ACCOUNT, true),
            host_cleared: None,
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
    fn codex_no_overage_counts_only_when_bound_to_this_machine() {
        let unbound = Assertions {
            no_overage: no_overage(ACCOUNT, false),
            host_cleared: Some(Clearance::Host),
        };
        let (r, fake) = build(&config("", "", LOCAL), unbound);
        let e = r
            .expect("builds")
            .get(id("codex"), Tools::None)
            .expect("configured")
            .ask(&question())
            .expect_err("codex cannot report its account");
        assert!(matches!(e, ClassifiedError::Unavailable(_)), "{e:?}");
        assert!(calls(&fake).is_empty());
    }

    #[test]
    fn a_container_clearance_is_not_honoured_yet_and_nothing_runs() {
        let spec = ContainerSpec {
            name: "lel-codex".to_owned(),
            machine_id: "lel-1".to_owned(),
            mounts: Vec::new(),
        };
        let contained = Assertions {
            no_overage: no_overage(ACCOUNT, true),
            host_cleared: Some(Clearance::Container(spec)),
        };
        let (r, fake) = build(&config("", "", LOCAL), contained);
        let e = r
            .expect("builds")
            .get(id("codex"), Tools::None)
            .expect("configured")
            .ask(&question())
            .expect_err("no container check yet");
        assert!(matches!(e, ClassifiedError::Unavailable(_)), "{e:?}");
        assert!(calls(&fake).is_empty());
    }

    #[test]
    fn a_container_clearance_never_grants_claude_on_the_host() {
        let spec = ContainerSpec {
            name: "lel-codex".to_owned(),
            machine_id: "lel-1".to_owned(),
            mounts: Vec::new(),
        };
        let contained = Assertions {
            no_overage: no_overage(ACCOUNT, true),
            host_cleared: Some(Clearance::Container(spec)),
        };
        let (r, fake) = build(&config("", "", LOCAL), contained);
        let mut r = r.expect("builds");
        for tools in [Tools::None, Tools::ReadOnly] {
            let e = r
                .get(id("claude"), tools)
                .expect("configured")
                .ask(&question())
                .expect_err("container clearance on a host adapter");
            assert!(matches!(e, ClassifiedError::Unavailable(_)), "{e:?}");
        }
        assert!(calls(&fake).is_empty());
    }

    #[test]
    fn claude_logged_into_another_account_gets_no_grant() {
        let other = Assertions {
            no_overage: no_overage("someone@example.com", false),
            host_cleared: None,
        };
        let (r, fake) = build(&config("", "", LOCAL), other);
        let e = r
            .expect("builds")
            .get(id("claude"), Tools::None)
            .expect("configured")
            .ask(&question())
            .expect_err("another account");
        assert!(matches!(e, ClassifiedError::Unavailable(_)), "{e:?}");
        let calls = calls(&fake);
        assert_eq!(calls.len(), 1, "only the login probe ran: {calls:?}");
        assert!(!calls[0].iter().any(|a| a == "--system-prompt"));
    }

    #[test]
    fn asserted_claude_runs_with_settings_and_the_lens_tools() {
        let a = Assertions {
            no_overage: no_overage(ACCOUNT, true),
            host_cleared: Some(Clearance::Host),
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
    fn the_longest_timeout_is_accepted_without_a_call() {
        let day = r#", "timeout_secs": 86400"#;
        let local = format!("{LOCAL}{day}");
        let (r, fake) = build(&config(day, day, &local), Assertions::default());
        assert!(r.is_ok(), "{:?}", r.err());
        assert!(calls(&fake).is_empty());
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
            r#", "timeout_secs": 86401"#,
            "",
            LOCAL,
            "providers.claude.timeout_secs: a whole number from 1 to 86400",
        );
        refused(
            "",
            "",
            r#", "base_url": "http://127.0.0.1/v1", "model": "q", "timeout_secs": 18446744073709551615"#,
            "providers.local.timeout_secs: a whole number from 1 to 86400",
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
