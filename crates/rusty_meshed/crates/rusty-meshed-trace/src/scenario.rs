//! Loading a [`Scenario`] from its TOML file format, plus the shared
//! assembly/validation step the JSON loader reuses.
//!
//! The TOML shape (see `scenarios/acquisition_status.toml` for the full
//! shipped example):
//!
//! ```toml
//! name = "Acquisition Status Dashboard"
//! description = "..."
//!
//! [domains.schedule]
//! name = "Schedule (IMS)"
//! owner = "Program Control"
//! maturity = 4          # 0..=4, or a level name
//! order = 1             # optional; display order, ties broken by id
//!
//! [sources.ims_export]
//! domain = "schedule"
//! name = "MS Project / IMS export"
//! kind = "system"       # system | file | person | process
//! structure = 4         # 0..=4
//! availability = 3      # 0..=4
//! latency_secs = 86400  # 0 = no cadence, "whenever you ask"
//!
//! [outcomes.acq_status]
//! name = "Acquisition Status Dashboard"
//! description = "..."
//! requires = [
//!   { domain = "schedule", min_maturity = 3, criticality = "blocking" },
//! ]
//! ```
//!
//! Domains, sources and outcomes are keyed tables rather than
//! `[[arrays-of-tables]]` because the workspace's own TOML parser
//! (`rusty_codec::toml`) deliberately doesn't implement the latter; the
//! optional `order` key recovers display order, since keyed tables come
//! back sorted by id. A domain's `sources` list is derived from each
//! source's `domain` field, never written by hand.

use std::collections::BTreeSet;
use std::time::Duration;

use rusty_codec::TomlValue;
use rusty_err::Error;

use crate::model::{
    Criticality, Domain, DomainId, Maturity, ModelError, Outcome, OutcomeId, Rating, Requirement,
    Scenario, Source, SourceId, SourceKind,
};

/// Errors from loading a scenario file.
#[derive(Debug, Error, PartialEq, Eq, Clone)]
pub enum ScenarioError {
    /// The TOML text didn't parse; `{0}` is the parser's message.
    #[error("scenario TOML did not parse: {0}")]
    Toml(String),
    /// The JSON text didn't parse; `{0}` is the parser's message.
    #[error("scenario JSON did not parse: {0}")]
    Json(String),
    /// A required field is absent; `{0}` is its dotted path.
    #[error("missing field {0}")]
    MissingField(String),
    /// A field is present but the wrong type or an invalid value; `{0}`
    /// is its dotted path, `{1}` what was wrong with it.
    #[error("invalid field {0}: {1}")]
    InvalidField(String, String),
    /// Two domains, sources, or outcomes share an id.
    #[error("duplicate id {0:?}")]
    DuplicateId(String),
    /// Source `{0}` names domain `{1}`, which the scenario doesn't define.
    #[error("source {0:?} belongs to unknown domain {1:?}")]
    DanglingDomain(String, String),
    /// Outcome `{0}` requires domain `{1}`, which the scenario doesn't
    /// define.
    #[error("outcome {0:?} requires unknown domain {1:?}")]
    DanglingRequirement(String, String),
}

fn invalid(path: &str, err: ModelError) -> ScenarioError {
    ScenarioError::InvalidField(path.to_string(), err.to_string())
}

// ── Raw (pre-validation) records shared by the TOML and JSON loaders ────

pub(crate) struct RawDomain {
    pub id: String,
    pub name: String,
    pub owner: String,
    pub maturity: MaturityText,
    pub order: i64,
}

pub(crate) struct RawSource {
    pub id: String,
    pub domain: String,
    pub name: String,
    pub kind: String,
    pub structure: i64,
    pub availability: i64,
    pub latency_secs: i64,
    pub order: i64,
}

pub(crate) struct RawRequirement {
    pub domain: String,
    pub min_maturity: MaturityText,
    pub criticality: String,
}

pub(crate) struct RawOutcome {
    pub id: String,
    pub name: String,
    pub description: String,
    pub requires: Vec<RawRequirement>,
    pub order: i64,
}

/// A maturity as it appears in a file: an integer level or a name.
pub(crate) enum MaturityText {
    Level(i64),
    Name(String),
}

impl MaturityText {
    fn resolve(&self, path: &str) -> Result<Maturity, ScenarioError> {
        match self {
            MaturityText::Level(n) => Maturity::from_level(*n),
            MaturityText::Name(s) => Maturity::parse(s),
        }
        .map_err(|e| invalid(path, e))
    }
}

/// Validates the raw records and assembles a [`Scenario`]: ids unique,
/// every reference resolves, every scalar in range, display order
/// applied, each domain's `sources` derived.
pub(crate) fn assemble(
    name: String,
    description: String,
    mut domains: Vec<RawDomain>,
    mut sources: Vec<RawSource>,
    mut outcomes: Vec<RawOutcome>,
) -> Result<Scenario, ScenarioError> {
    domains.sort_by(|a, b| a.order.cmp(&b.order).then(a.id.cmp(&b.id)));
    sources.sort_by(|a, b| a.order.cmp(&b.order).then(a.id.cmp(&b.id)));
    outcomes.sort_by(|a, b| a.order.cmp(&b.order).then(a.id.cmp(&b.id)));

    for (label, ids) in [
        (
            "domains",
            domains.iter().map(|d| d.id.as_str()).collect::<Vec<_>>(),
        ),
        ("sources", sources.iter().map(|s| s.id.as_str()).collect()),
        ("outcomes", outcomes.iter().map(|o| o.id.as_str()).collect()),
    ] {
        let mut seen = BTreeSet::new();
        for id in ids {
            if !seen.insert(id) {
                return Err(ScenarioError::DuplicateId(format!("{label}.{id}")));
            }
        }
    }

    let domain_ids: BTreeSet<String> = domains.iter().map(|d| d.id.clone()).collect();

    let sources: Vec<Source> = sources
        .into_iter()
        .map(|s| {
            if !domain_ids.contains(s.domain.as_str()) {
                return Err(ScenarioError::DanglingDomain(
                    s.id.clone(),
                    s.domain.clone(),
                ));
            }
            let path = format!("sources.{}", s.id);
            if s.latency_secs < 0 {
                return Err(ScenarioError::InvalidField(
                    format!("{path}.latency_secs"),
                    format!("must be >= 0, got {}", s.latency_secs),
                ));
            }
            Ok(Source {
                id: SourceId::new(s.id),
                domain: DomainId::new(s.domain),
                name: s.name,
                kind: SourceKind::parse(&s.kind)
                    .map_err(|e| invalid(&format!("{path}.kind"), e))?,
                structure: Rating::new(s.structure)
                    .map_err(|e| invalid(&format!("{path}.structure"), e))?,
                availability: Rating::new(s.availability)
                    .map_err(|e| invalid(&format!("{path}.availability"), e))?,
                latency: Duration::from_secs(s.latency_secs as u64),
            })
        })
        .collect::<Result<_, _>>()?;

    let domains: Vec<Domain> = domains
        .into_iter()
        .map(|d| {
            let path = format!("domains.{}.maturity", d.id);
            let id = DomainId::new(d.id);
            Ok(Domain {
                sources: sources
                    .iter()
                    .filter(|s| s.domain == id)
                    .map(|s| s.id.clone())
                    .collect(),
                id,
                name: d.name,
                owner: d.owner,
                maturity: d.maturity.resolve(&path)?,
            })
        })
        .collect::<Result<_, _>>()?;

    let outcomes: Vec<Outcome> = outcomes
        .into_iter()
        .map(|o| {
            let requires = o
                .requires
                .into_iter()
                .enumerate()
                .map(|(i, r)| {
                    if !domain_ids.contains(r.domain.as_str()) {
                        return Err(ScenarioError::DanglingRequirement(o.id.clone(), r.domain));
                    }
                    let path = format!("outcomes.{}.requires[{i}]", o.id);
                    Ok(Requirement {
                        domain: DomainId::new(r.domain),
                        min_maturity: r.min_maturity.resolve(&format!("{path}.min_maturity"))?,
                        criticality: Criticality::parse(&r.criticality)
                            .map_err(|e| invalid(&format!("{path}.criticality"), e))?,
                    })
                })
                .collect::<Result<_, _>>()?;
            Ok(Outcome {
                id: OutcomeId::new(o.id),
                name: o.name,
                description: o.description,
                requires,
            })
        })
        .collect::<Result<_, _>>()?;

    Ok(Scenario {
        name,
        description,
        domains,
        sources,
        outcomes,
    })
}

// ── TOML ─────────────────────────────────────────────────────────────────

fn toml_str(table: &TomlValue, path: &str, key: &str) -> Result<String, ScenarioError> {
    match table.get(key) {
        None => Err(ScenarioError::MissingField(format!("{path}.{key}"))),
        Some(TomlValue::String(s)) => Ok(s.clone()),
        Some(_) => Err(ScenarioError::InvalidField(
            format!("{path}.{key}"),
            "expected a string".into(),
        )),
    }
}

fn toml_str_or(table: &TomlValue, key: &str, default: &str) -> Result<String, ScenarioError> {
    match table.get(key) {
        None => Ok(default.to_string()),
        Some(TomlValue::String(s)) => Ok(s.clone()),
        Some(_) => Err(ScenarioError::InvalidField(
            key.to_string(),
            "expected a string".into(),
        )),
    }
}

fn toml_int(table: &TomlValue, path: &str, key: &str) -> Result<i64, ScenarioError> {
    match table.get(key) {
        None => Err(ScenarioError::MissingField(format!("{path}.{key}"))),
        Some(TomlValue::Integer(n)) => Ok(*n),
        Some(_) => Err(ScenarioError::InvalidField(
            format!("{path}.{key}"),
            "expected an integer".into(),
        )),
    }
}

fn toml_int_or(
    table: &TomlValue,
    path: &str,
    key: &str,
    default: i64,
) -> Result<i64, ScenarioError> {
    match table.get(key) {
        None => Ok(default),
        Some(TomlValue::Integer(n)) => Ok(*n),
        Some(_) => Err(ScenarioError::InvalidField(
            format!("{path}.{key}"),
            "expected an integer".into(),
        )),
    }
}

fn toml_maturity(table: &TomlValue, path: &str, key: &str) -> Result<MaturityText, ScenarioError> {
    match table.get(key) {
        None => Err(ScenarioError::MissingField(format!("{path}.{key}"))),
        Some(TomlValue::Integer(n)) => Ok(MaturityText::Level(*n)),
        Some(TomlValue::String(s)) => Ok(MaturityText::Name(s.clone())),
        Some(_) => Err(ScenarioError::InvalidField(
            format!("{path}.{key}"),
            "expected a maturity level 0..=4 or a level name".into(),
        )),
    }
}

fn toml_section<'a>(
    root: &'a TomlValue,
    key: &str,
) -> Result<Vec<(&'a String, &'a TomlValue)>, ScenarioError> {
    match root.get(key) {
        None => Err(ScenarioError::MissingField(key.to_string())),
        Some(TomlValue::Table(t)) => t
            .iter()
            .map(|(id, v)| match v {
                TomlValue::Table(_) => Ok((id, v)),
                _ => Err(ScenarioError::InvalidField(
                    format!("{key}.{id}"),
                    "expected a table".into(),
                )),
            })
            .collect(),
        Some(_) => Err(ScenarioError::InvalidField(
            key.to_string(),
            "expected a table of tables".into(),
        )),
    }
}

impl Scenario {
    /// Parses a scenario from its TOML text (format documented on this
    /// module), validating every reference and scalar.
    pub fn from_toml(text: &str) -> Result<Self, ScenarioError> {
        let root = TomlValue::parse_str(text).map_err(|e| ScenarioError::Toml(e.to_string()))?;
        let name = toml_str(&root, "", "name").map_err(|e| match e {
            ScenarioError::MissingField(_) => ScenarioError::MissingField("name".into()),
            other => other,
        })?;
        let description = toml_str_or(&root, "description", "")?;

        let domains = toml_section(&root, "domains")?
            .into_iter()
            .map(|(id, t)| {
                let path = format!("domains.{id}");
                Ok(RawDomain {
                    id: id.clone(),
                    name: toml_str(t, &path, "name")?,
                    owner: toml_str_or(t, "owner", "")?,
                    maturity: toml_maturity(t, &path, "maturity")?,
                    order: toml_int_or(t, &path, "order", i64::MAX)?,
                })
            })
            .collect::<Result<Vec<_>, ScenarioError>>()?;

        let sources = toml_section(&root, "sources")?
            .into_iter()
            .map(|(id, t)| {
                let path = format!("sources.{id}");
                Ok(RawSource {
                    id: id.clone(),
                    domain: toml_str(t, &path, "domain")?,
                    name: toml_str(t, &path, "name")?,
                    kind: toml_str(t, &path, "kind")?,
                    structure: toml_int(t, &path, "structure")?,
                    availability: toml_int(t, &path, "availability")?,
                    latency_secs: toml_int_or(t, &path, "latency_secs", 0)?,
                    order: toml_int_or(t, &path, "order", i64::MAX)?,
                })
            })
            .collect::<Result<Vec<_>, ScenarioError>>()?;

        let outcomes = toml_section(&root, "outcomes")?
            .into_iter()
            .map(|(id, t)| {
                let path = format!("outcomes.{id}");
                let requires = match t.get("requires") {
                    None => Err(ScenarioError::MissingField(format!("{path}.requires"))),
                    Some(TomlValue::Array(items)) => items
                        .iter()
                        .enumerate()
                        .map(|(i, item)| {
                            let rpath = format!("{path}.requires[{i}]");
                            if item.as_table().is_none() {
                                return Err(ScenarioError::InvalidField(
                                    rpath,
                                    "expected an inline table".into(),
                                ));
                            }
                            Ok(RawRequirement {
                                domain: toml_str(item, &rpath, "domain")?,
                                min_maturity: toml_maturity(item, &rpath, "min_maturity")?,
                                criticality: toml_str(item, &rpath, "criticality")?,
                            })
                        })
                        .collect(),
                    Some(_) => Err(ScenarioError::InvalidField(
                        format!("{path}.requires"),
                        "expected an array of inline tables".into(),
                    )),
                }?;
                Ok(RawOutcome {
                    id: id.clone(),
                    name: toml_str(t, &path, "name")?,
                    description: toml_str_or(t, "description", "")?,
                    requires,
                    order: toml_int_or(t, &path, "order", i64::MAX)?,
                })
            })
            .collect::<Result<Vec<_>, ScenarioError>>()?;

        assemble(name, description, domains, sources, outcomes)
    }
}
