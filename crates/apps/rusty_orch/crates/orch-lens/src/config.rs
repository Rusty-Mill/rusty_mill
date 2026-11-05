//! The lens configuration, kept in Factory: providers, data classes,
//! ceilings and lenses. Parsed and checked in one place, so a bad config is
//! refused at load with the field named, before any call.

use std::collections::{BTreeMap, BTreeSet};

use orch_core::AgentId;
use rusty_json::{Map, Value};

use crate::class::DataClass;
use crate::meter::{Ceilings, Price};
use crate::post::DirectionKey;

/// One micro-dollar per dollar, for ceilings given in dollars.
const USD: u64 = 1_000_000;

/// How a provider is reached. CLI providers are cloud models; the HTTP
/// adapter only ever connects to loopback or private addresses, so it is
/// local.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderKind {
    ClaudeCli,
    CodexCli,
    OpenAiHttp,
}

impl ProviderKind {
    fn parse(word: &str) -> Option<Self> {
        match word {
            "claude-cli" => Some(Self::ClaudeCli),
            "codex-cli" => Some(Self::CodexCli),
            "openai-http" => Some(Self::OpenAiHttp),
            _ => None,
        }
    }

    /// True for providers whose model runs off the LAN.
    pub fn is_cloud(self) -> bool {
        !matches!(self, Self::OpenAiHttp)
    }

    /// Only CLI providers can run a lens's read-only tools.
    pub fn has_tools(self) -> bool {
        self.is_cloud()
    }
}

/// One configured provider.
#[derive(Debug, Clone, PartialEq)]
pub struct Provider {
    pub kind: ProviderKind,
    pub max_class: DataClass,
    pub context_tokens: u32,
    pub price: Price,
    /// The adapter's own fields (model, base_url, ...), for the shell.
    pub settings: Map,
}

/// A lens's tool access.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tools {
    None,
    ReadOnly,
}

/// One lens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lens {
    /// Also its BBP role id and principal.
    pub id: String,
    pub prompt: String,
    /// `*` means every domain.
    pub domains: Vec<String>,
    pub evidence: BTreeSet<DirectionKey>,
    pub claims: bool,
    pub routine: Vec<AgentId>,
    pub contested: Vec<AgentId>,
    /// Proposers for questions, in order, and the rebel's provider.
    pub proposers: Vec<AgentId>,
    pub rebel: Option<AgentId>,
    pub tools: Tools,
    pub diversity_group: Option<String>,
}

impl Lens {
    /// Does this lens follow `domain`?
    pub fn follows(&self, domain: &str) -> bool {
        self.domains.iter().any(|d| d == "*" || d == domain)
    }
}

/// The #708 alien lens (D75, D99): a shadow lens that hunts defects and
/// posts nothing, so it has no domains and no `may_post`. Its calls use
/// read-only tools (design §3), so each provider must be a CLI one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Alien {
    /// Its BBP role id and principal; no ordinary lens may share it.
    pub id: String,
    pub prompt: String,
    /// In fallback order.
    pub providers: Vec<AgentId>,
}

/// The whole configuration.
#[derive(Debug, Clone, PartialEq)]
pub struct LensConfig {
    pub providers: BTreeMap<AgentId, Provider>,
    pub domain_classes: BTreeMap<String, DataClass>,
    pub ceilings: Ceilings,
    pub lenses: Vec<Lens>,
    /// The alien lens, when configured; `lens run --alien` needs it.
    pub alien: Option<Alien>,
}

impl LensConfig {
    /// Parse and check the JSON config.
    pub fn parse(text: &str) -> Result<Self, String> {
        let v = Value::parse(text).map_err(|e| format!("config is not JSON: {e}"))?;
        let top = obj(&v, "config")?;
        only(
            top,
            &["providers", "domain_classes", "ceilings", "lenses", "alien"],
            "config",
        )?;
        let providers = providers(field(top, "providers")?)?;
        let domain_classes = domain_classes(top.get("domain_classes"))?;
        let ceilings = ceilings(field(top, "ceilings")?)?;
        let lenses = arr(field(top, "lenses")?, "lenses")?
            .iter()
            .enumerate()
            .map(|(i, l)| lens(l, &providers).map_err(|e| format!("lenses[{i}].{e}")))
            .collect::<Result<Vec<_>, _>>()?;
        check_unique_lenses(&lenses)?;
        check_one_proposer(&lenses)?;
        check_diversity(&lenses)?;
        let alien = match top.get("alien") {
            None => None,
            Some(a) => Some(alien(a, &providers, &lenses).map_err(|e| format!("alien.{e}"))?),
        };
        Ok(Self {
            providers,
            domain_classes,
            ceilings,
            lenses,
            alien,
        })
    }

    /// A domain's class: configured, else `internal`.
    pub fn domain_class(&self, domain: &str) -> DataClass {
        self.domain_classes
            .get(domain)
            .copied()
            .unwrap_or(DataClass::Internal)
    }

    /// The lens with this id.
    pub fn lens(&self, id: &str) -> Option<&Lens> {
        self.lenses.iter().find(|l| l.id == id)
    }
}

fn providers(v: &Value) -> Result<BTreeMap<AgentId, Provider>, String> {
    let m = obj(v, "providers")?;
    if m.is_empty() {
        return Err("providers: at least one required".to_owned());
    }
    m.iter()
        .map(|(id, p)| {
            let at = format!("providers.{id}");
            let id = AgentId::new(id).map_err(|e| format!("{at}: {e}"))?;
            Ok((id, provider(p).map_err(|e| format!("{at}.{e}"))?))
        })
        .collect()
}

fn provider(v: &Value) -> Result<Provider, String> {
    let m = obj(v, "provider")?;
    let kind_word = string(m, "kind")?;
    let kind = ProviderKind::parse(kind_word).ok_or_else(|| {
        format!("kind: {kind_word:?} is not claude-cli, codex-cli or openai-http")
    })?;
    let max_class = match m.get("max_class") {
        None if kind.is_cloud() => DataClass::Internal,
        None => DataClass::Sensitive,
        Some(c) => class(c).map_err(|e| format!("max_class: {e}"))?,
    };
    if kind.is_cloud() && max_class == DataClass::Sensitive {
        return Err("max_class: a cloud provider cannot be cleared for sensitive".to_owned());
    }
    let context_tokens = u32::try_from(uint(m, "context_tokens")?)
        .ok()
        .filter(|n| *n > 0)
        .ok_or("context_tokens: 1 to 4294967295 required")?;
    let price = price(
        m.get("price")
            .ok_or("price: required (0 rates are allowed)")?,
    )
    .map_err(|e| format!("price.{e}"))?;
    // No adapter bounds a call's cost before it is made (agentic CLI
    // turns, unbounded chat-template tokens), so v0 admits only flat-rate
    // and local providers. The meter stays for a bounded paid adapter.
    if !price.is_free() {
        return Err(
            "price: v0 admits only zero rates; no adapter bounds a call's cost before it is made"
                .to_owned(),
        );
    }
    let mut settings = m.clone();
    for k in ["kind", "max_class", "context_tokens", "price"] {
        settings.remove(k);
    }
    Ok(Provider {
        kind,
        max_class,
        context_tokens,
        price,
        settings,
    })
}

fn price(v: &Value) -> Result<Price, String> {
    let m = obj(v, "price")?;
    only(m, &["input", "output", "cached_input"], "price")?;
    Ok(Price {
        input: uint(m, "input")?,
        output: uint(m, "output")?,
        cached_input: uint(m, "cached_input")?,
    })
}

fn domain_classes(v: Option<&Value>) -> Result<BTreeMap<String, DataClass>, String> {
    let Some(v) = v else {
        return Ok(BTreeMap::new());
    };
    obj(v, "domain_classes")?
        .iter()
        .map(|(d, c)| {
            Ok((
                d.clone(),
                class(c).map_err(|e| format!("domain_classes.{d}: {e}"))?,
            ))
        })
        .collect()
}

fn ceilings(v: &Value) -> Result<Ceilings, String> {
    let m = obj(v, "ceilings")?;
    only(m, &["experiment_usd", "task_usd", "task_calls"], "ceilings")?;
    let usd = |k: &str| {
        uint(m, k)
            .map_err(|e| format!("ceilings.{e}"))?
            .checked_mul(USD)
            .ok_or_else(|| format!("ceilings.{k}: too large"))
    };
    Ok(Ceilings {
        experiment: usd("experiment_usd")?,
        task: usd("task_usd")?,
        task_calls: u32::try_from(uint(m, "task_calls").map_err(|e| format!("ceilings.{e}"))?)
            .map_err(|_| "ceilings.task_calls: too large".to_owned())?,
    })
}

const LENS_KEYS: &[&str] = &[
    "id",
    "prompt",
    "domains",
    "may_post",
    "models",
    "proposers",
    "rebel",
    "tools",
    "diversity_group",
];

fn lens(v: &Value, providers: &BTreeMap<AgentId, Provider>) -> Result<Lens, String> {
    let m = obj(v, "lens")?;
    only(m, LENS_KEYS, "lens")?;
    let id = string(m, "id")?;
    AgentId::new(id).map_err(|e| format!("id: {e}"))?;
    let domains = strings(m, "domains")?;
    if domains.is_empty() {
        return Err("domains: at least one required (\"*\" for all)".to_owned());
    }
    let (evidence, claims) = may_post(field(m, "may_post")?)?;
    let models = obj(field(m, "models")?, "models")?;
    only(models, &["routine", "contested"], "models")?;
    let named = |list: &Map, k: &str, at: &str| -> Result<Vec<AgentId>, String> {
        let ids = ids(list, k).map_err(|e| format!("{at}{e}"))?;
        match ids.iter().find(|id| !providers.contains_key(*id)) {
            Some(missing) => Err(format!("{at}{k}: provider {missing} is not configured")),
            None => Ok(ids),
        }
    };
    let routine = named(models, "routine", "models.")?;
    let contested = named(models, "contested", "models.")?;
    if routine.is_empty() || contested.is_empty() {
        return Err("models: routine and contested each need a provider".to_owned());
    }
    let proposers = named(m, "proposers", "")?;
    let rebel = match m.get("rebel") {
        None => None,
        Some(r) => Some(provider_id(r, providers).map_err(|e| format!("rebel: {e}"))?),
    };
    check_proposers(&proposers, rebel, claims)?;
    let tools = match m.get("tools").map(|t| t.as_str()) {
        None | Some(Some("none")) => Tools::None,
        Some(Some("read-only")) => Tools::ReadOnly,
        _ => return Err("tools: none or read-only".to_owned()),
    };
    if tools == Tools::ReadOnly {
        let all = routine
            .iter()
            .chain(&contested)
            .chain(&proposers)
            .chain(rebel.as_ref());
        if let Some(p) = all.into_iter().find(|p| !providers[*p].kind.has_tools()) {
            return Err(format!(
                "tools: read-only needs a CLI provider, and {p} is not one"
            ));
        }
    }
    let diversity_group = match m.get("diversity_group") {
        None => None,
        Some(g) => Some(
            g.as_str()
                .ok_or("diversity_group: string required")?
                .to_owned(),
        ),
    };
    Ok(Lens {
        id: id.to_owned(),
        prompt: string(m, "prompt")?.to_owned(),
        domains,
        evidence,
        claims,
        routine,
        contested,
        proposers,
        rebel,
        tools,
        diversity_group,
    })
}

fn alien(
    v: &Value,
    providers: &BTreeMap<AgentId, Provider>,
    lenses: &[Lens],
) -> Result<Alien, String> {
    let m = obj(v, "alien")?;
    only(m, &["id", "prompt", "providers"], "alien")?;
    let id = string(m, "id")?;
    AgentId::new(id).map_err(|e| format!("id: {e}"))?;
    if lenses.iter().any(|l| l.id == id) {
        return Err(format!("id: {id:?} is already a lens"));
    }
    let list = ids(m, "providers")?;
    if list.is_empty() {
        return Err("providers: at least one required".to_owned());
    }
    for p in &list {
        let Some(provider) = providers.get(p) else {
            return Err(format!("providers: provider {p} is not configured"));
        };
        if !provider.kind.has_tools() {
            return Err(format!(
                "providers: the alien lens uses read-only tools, and {p} is not a CLI provider"
            ));
        }
    }
    Ok(Alien {
        id: id.to_owned(),
        prompt: string(m, "prompt")?.to_owned(),
        providers: list,
    })
}

fn may_post(v: &Value) -> Result<(BTreeSet<DirectionKey>, bool), String> {
    let m = obj(v, "may_post")?;
    only(m, &["evidence", "claim"], "may_post")?;
    let evidence = strings(m, "evidence")
        .map_err(|e| format!("may_post.{e}"))?
        .iter()
        .map(|d| match d.as_str() {
            "support" => Ok(DirectionKey::Support),
            "counter" => Ok(DirectionKey::Counter),
            other => Err(format!(
                "may_post.evidence: {other:?} is not support or counter"
            )),
        })
        .collect::<Result<BTreeSet<_>, _>>()?;
    let claims = match m.get("claim") {
        None => false,
        Some(c) => c.as_bool().ok_or("may_post.claim: true or false")?,
    };
    if evidence.is_empty() && !claims {
        return Err("may_post: the lens may post nothing".to_owned());
    }
    Ok((evidence, claims))
}

/// Two distinct proposers and a rebel that is neither of them; only a lens
/// that may post claims proposes.
fn check_proposers(
    proposers: &[AgentId],
    rebel: Option<AgentId>,
    claims: bool,
) -> Result<(), String> {
    if proposers.is_empty() && rebel.is_none() {
        return Ok(());
    }
    if !claims {
        return Err("proposers: only a lens that may post claims proposes".to_owned());
    }
    let first_two: BTreeSet<_> = proposers.iter().take(2).collect();
    if first_two.len() < 2 {
        return Err("proposers: at least two different providers required".to_owned());
    }
    match rebel {
        None => Err("rebel: required beside proposers".to_owned()),
        Some(r) if first_two.contains(&r) => Err(format!(
            "rebel: {r} is already one of the first two proposers"
        )),
        Some(_) => Ok(()),
    }
}

fn check_unique_lenses(lenses: &[Lens]) -> Result<(), String> {
    let mut seen = BTreeSet::new();
    match lenses.iter().find(|l| !seen.insert(l.id.as_str())) {
        Some(dup) => Err(format!("lenses: id {:?} appears twice", dup.id)),
        None => Ok(()),
    }
}

/// Proposal state is kept per question, so at most one lens proposes.
fn check_one_proposer(lenses: &[Lens]) -> Result<(), String> {
    let mut proposing = lenses
        .iter()
        .filter(|l| !l.proposers.is_empty() || l.rebel.is_some());
    match (proposing.next(), proposing.next()) {
        (Some(a), Some(b)) => Err(format!(
            "lenses: only one lens may propose, and both {:?} and {:?} do",
            a.id, b.id
        )),
        _ => Ok(()),
    }
}

/// Lenses in one group must lead with different providers.
fn check_diversity(lenses: &[Lens]) -> Result<(), String> {
    let mut first: BTreeMap<(&str, AgentId), &str> = BTreeMap::new();
    for l in lenses {
        let (Some(group), Some(lead)) = (&l.diversity_group, l.routine.first()) else {
            continue;
        };
        if let Some(other) = first.insert((group.as_str(), *lead), &l.id) {
            return Err(format!(
                "lenses: {other:?} and {:?} share diversity_group {group:?} and both lead with {lead}",
                l.id
            ));
        }
    }
    Ok(())
}

fn provider_id(v: &Value, providers: &BTreeMap<AgentId, Provider>) -> Result<AgentId, String> {
    let s = v.as_str().ok_or("provider id required")?;
    let id = AgentId::new(s).map_err(|e| e.to_string())?;
    if providers.contains_key(&id) {
        Ok(id)
    } else {
        Err(format!("provider {id} is not configured"))
    }
}

fn class(v: &Value) -> Result<DataClass, String> {
    v.as_str()
        .and_then(DataClass::parse)
        .ok_or_else(|| "public, internal or sensitive required".to_owned())
}

fn obj<'a>(v: &'a Value, what: &str) -> Result<&'a Map, String> {
    v.as_object()
        .ok_or_else(|| format!("{what}: object required"))
}

fn arr<'a>(v: &'a Value, what: &str) -> Result<&'a Vec<Value>, String> {
    v.as_array()
        .ok_or_else(|| format!("{what}: array required"))
}

fn field<'a>(m: &'a Map, k: &str) -> Result<&'a Value, String> {
    m.get(k).ok_or_else(|| format!("{k}: required"))
}

fn only(m: &Map, keys: &[&str], what: &str) -> Result<(), String> {
    match m.keys().find(|k| !keys.contains(&k.as_str())) {
        Some(k) => Err(format!("{what}: unknown field {k:?}")),
        None => Ok(()),
    }
}

fn string<'a>(m: &'a Map, k: &str) -> Result<&'a str, String> {
    m.get(k)
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| format!("{k}: non-empty string required"))
}

fn uint(m: &Map, k: &str) -> Result<u64, String> {
    m.get(k)
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("{k}: whole number required"))
}

fn strings(m: &Map, k: &str) -> Result<Vec<String>, String> {
    match m.get(k) {
        None => Ok(vec![]),
        Some(v) => arr(v, k)?
            .iter()
            .map(|s| {
                s.as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| format!("{k}: strings required"))
            })
            .collect(),
    }
}

fn ids(m: &Map, k: &str) -> Result<Vec<AgentId>, String> {
    strings(m, k)?
        .iter()
        .map(|s| AgentId::new(s).map_err(|e| format!("{k}: {e}")))
        .collect()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// The starter config of the design (rev 3), with #598's classes,
    /// prices and ceilings.
    pub(crate) const STARTER: &str = r#"{
      "providers": {
        "claude": {"kind": "claude-cli", "model": "opus", "context_tokens": 200000,
                   "price": {"input": 0, "output": 0, "cached_input": 0}},
        "codex": {"kind": "codex-cli", "context_tokens": 200000,
                  "price": {"input": 0, "output": 0, "cached_input": 0}},
        "local": {"kind": "openai-http", "base_url": "http://baileyai:8080/v1",
                  "model": "Qwen3.6-35B-A3B", "context_tokens": 32768,
                  "price": {"input": 0, "output": 0, "cached_input": 0}},
        "local-high": {"kind": "openai-http", "base_url": "http://baileyai:8080/v1",
                  "model": "gpt-oss-120b", "context_tokens": 32768,
                  "price": {"input": 0, "output": 0, "cached_input": 0}}
      },
      "domain_classes": {"real-data": "sensitive"},
      "ceilings": {"experiment_usd": 200, "task_usd": 25, "task_calls": 400},
      "lenses": [
        {"id": "architect", "prompt": "lenses/architect.md", "domains": ["*"],
         "may_post": {"claim": true},
         "proposers": ["claude", "local"], "rebel": "codex",
         "models": {"routine": ["claude", "local", "codex"], "contested": ["claude", "local-high"]}},
        {"id": "research", "prompt": "lenses/research.md", "domains": ["*"],
         "may_post": {"evidence": ["support", "counter"], "claim": true}, "tools": "read-only",
         "models": {"routine": ["claude", "codex"], "contested": ["claude"]}},
        {"id": "adversary", "prompt": "lenses/adversary.md", "domains": ["*"],
         "may_post": {"evidence": ["counter", "support"]}, "diversity_group": "attack",
         "models": {"routine": ["codex", "local"], "contested": ["claude", "local-high"]}},
        {"id": "operator", "prompt": "lenses/operator.md", "domains": ["*"],
         "may_post": {"evidence": ["counter", "support"]}, "diversity_group": "attack",
         "models": {"routine": ["local", "codex"], "contested": ["local-high", "claude"]}}
      ]
    }"#;

    #[test]
    fn the_starter_config_loads() {
        let c = LensConfig::parse(STARTER).expect("loads");
        assert_eq!(c.lenses.len(), 4);
        let local = &c.providers[&AgentId::new("local").expect("id")];
        assert_eq!(local.max_class, DataClass::Sensitive);
        assert_eq!(
            local.settings.get("model").and_then(Value::as_str),
            Some("Qwen3.6-35B-A3B")
        );
        let claude = &c.providers[&AgentId::new("claude").expect("id")];
        assert_eq!(claude.max_class, DataClass::Internal);
        assert_eq!(c.ceilings.task, 25 * USD);
        assert_eq!(c.domain_class("real-data"), DataClass::Sensitive);
        assert_eq!(c.domain_class("dedup"), DataClass::Internal);
        let arch = c.lens("architect").expect("architect");
        assert_eq!(arch.rebel, AgentId::new("codex").ok());
        assert!(arch.follows("anything"));
    }

    fn refused(from: &str, to: &str, expected: &str) {
        let text = STARTER.replacen(from, to, 1);
        assert_ne!(text, STARTER, "fixture edit {from:?} did not apply");
        let e = LensConfig::parse(&text).expect_err(expected);
        assert!(e.contains(expected), "{expected:?} not in {e:?}");
    }

    #[test]
    fn provider_refusals_name_the_field() {
        refused(
            r#""kind": "codex-cli""#,
            r#""kind": "gemini-cli""#,
            "providers.codex.kind",
        );
        refused(
            r#""kind": "codex-cli","#,
            r#""kind": "codex-cli", "max_class": "sensitive","#,
            "providers.codex.max_class: a cloud provider cannot be cleared for sensitive",
        );
        refused(
            r#", "context_tokens": 200000,
                  "price": {"input": 0, "output": 0, "cached_input": 0}}"#,
            r#", "context_tokens": 200000}"#,
            "providers.codex.price: required",
        );
        refused(
            r#""context_tokens": 200000,
                  "price": {"input": 0, "output": 0, "cached_input": 0}}"#,
            r#""context_tokens": 200000,
                  "price": {"input": 0, "output": 1, "cached_input": 0}}"#,
            "providers.codex.price: v0 admits only zero rates",
        );
        refused(
            r#""claude": {"kind""#,
            r#""Claude": {"kind""#,
            "providers.Claude",
        );
    }

    #[test]
    fn a_second_proposing_lens_is_refused() {
        // The starter Architect again under another id, following every
        // domain: its proposal jobs would collide with the first one's.
        refused(
            r#"{"id": "research","#,
            r#"{"id": "architect-2", "prompt": "lenses/architect.md", "domains": ["*"],
         "may_post": {"claim": true},
         "proposers": ["claude", "local"], "rebel": "codex",
         "models": {"routine": ["claude", "local", "codex"], "contested": ["claude"]}},
        {"id": "research","#,
            "lenses: only one lens may propose, and both \"architect\" and \"architect-2\" do",
        );
    }

    #[test]
    fn lens_refusals_name_the_field() {
        refused(
            r#""routine": ["codex", "local"]"#,
            r#""routine": ["codex", "gemini"]"#,
            "lenses[2].models.routine: provider gemini is not configured",
        );
        refused(
            r#""rebel": "codex""#,
            r#""rebel": "local""#,
            "lenses[0].rebel: local is already one of the first two proposers",
        );
        refused(
            r#""rebel": "codex""#,
            r#""rebel": "nobody""#,
            "lenses[0].rebel: provider nobody is not configured",
        );
        refused(
            r#""proposers": ["claude", "local"]"#,
            r#""proposers": ["claude", "claude"]"#,
            "lenses[0].proposers: at least two different providers",
        );
        refused(
            r#""routine": ["claude", "codex"], "contested": ["claude"]"#,
            r#""routine": ["claude", "local"], "contested": ["claude"]"#,
            "lenses[1].tools: read-only needs a CLI provider, and local is not one",
        );
        refused(
            r#""routine": ["local", "codex"]"#,
            r#""routine": ["codex", "local"]"#,
            "share diversity_group \"attack\" and both lead with codex",
        );
        refused(
            r#""id": "operator""#,
            r#""id": "adversary""#,
            "appears twice",
        );
        refused(
            r#""may_post": {"evidence": ["counter", "support"]}, "diversity_group""#,
            r#""may_post": {"evidence": ["maybe"]}, "diversity_group""#,
            "is not support or counter",
        );
        refused(
            r#""task_calls": 400"#,
            r#""task_cals": 400"#,
            "unknown field \"task_cals\"",
        );
    }

    /// STARTER with an alien lens added.
    fn with_alien(alien: &str) -> String {
        let at = STARTER.rfind(']').expect("lenses end");
        format!("{}],\n  \"alien\": {alien}\n}}", &STARTER[..at])
    }

    #[test]
    fn the_alien_lens_is_optional_and_checked() {
        assert_eq!(LensConfig::parse(STARTER).expect("loads").alien, None);
        let good =
            r#"{"id": "alien", "prompt": "lenses/alien.md", "providers": ["codex", "claude"]}"#;
        let c = LensConfig::parse(&with_alien(good)).expect("loads");
        let a = c.alien.expect("alien");
        assert_eq!(a.id, "alien");
        assert_eq!(a.prompt, "lenses/alien.md");
        let ids: Vec<&str> = a.providers.iter().map(AgentId::as_str).collect();
        assert_eq!(ids, ["codex", "claude"]);
        for (alien, expected) in [
            (
                r#"{"id": "adversary", "prompt": "p", "providers": ["codex"]}"#,
                "alien.id: \"adversary\" is already a lens",
            ),
            (
                r#"{"id": "alien", "prompt": "p", "providers": []}"#,
                "alien.providers: at least one required",
            ),
            (
                r#"{"id": "alien", "prompt": "p", "providers": ["gemini"]}"#,
                "alien.providers: provider gemini is not configured",
            ),
            (
                r#"{"id": "alien", "prompt": "p", "providers": ["local"]}"#,
                "local is not a CLI provider",
            ),
            (
                r#"{"id": "alien", "providers": ["codex"]}"#,
                "alien.prompt: non-empty string required",
            ),
            (
                r#"{"id": "alien", "prompt": "p", "providers": ["codex"], "domains": ["*"]}"#,
                "alien: unknown field \"domains\"",
            ),
        ] {
            let e = LensConfig::parse(&with_alien(alien)).expect_err(expected);
            assert!(e.contains(expected), "{expected:?} not in {e:?}");
        }
    }
}
