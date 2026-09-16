//! JSON round-tripping for [`Scenario`] and JSON export for
//! [`TraceReport`], via `rusty_json` -- the wire shape the
//! `data-mesh-monitor` reverse-trace view consumes.
//!
//! Scenario shape (a domain's `sources` is written for readers' benefit
//! and re-derived from the sources' own `domain` fields on load):
//!
//! ```json
//! {
//!   "name": "...", "description": "...",
//!   "domains":  [{ "id": "schedule", "name": "...", "owner": "...", "maturity": 4, "sources": ["ims_export"] }],
//!   "sources":  [{ "id": "ims_export", "domain": "schedule", "name": "...", "kind": "system",
//!                  "structure": 4, "availability": 3, "latency_secs": 86400 }],
//!   "outcomes": [{ "id": "acq_status", "name": "...", "description": "...",
//!                  "requires": [{ "domain": "schedule", "min_maturity": 3, "criticality": "blocking" }] }]
//! }
//! ```
//!
//! Report shape:
//!
//! ```json
//! {
//!   "outcome": "acq_status", "outcome_name": "...",
//!   "achievable": { "kind": "partial", "fraction": 0.6 },
//!   "bottlenecks": [{ "domain": "contracts", "domain_name": "...", "owner": "...", "current": 1,
//!                     "required": 2, "gap": 1, "criticality": "blocking", "state": "blocked",
//!                     "sources": ["contract_mods_pdf"] }],
//!   "edges": [{ "from": { "kind": "domain", "id": "schedule" },
//!               "to": { "kind": "outcome", "id": "acq_status" }, "state": "satisfied" }]
//! }
//! ```

use rusty_json::{json, Value};

use crate::model::{Fidelity, NodeRef, Scenario, TraceReport};
use crate::scenario::{
    assemble, MaturityText, RawDomain, RawOutcome, RawRequirement, RawSource, ScenarioError,
};

fn json_str(v: &Value, path: &str, key: &str) -> Result<String, ScenarioError> {
    match v.get(key) {
        None => Err(ScenarioError::MissingField(format!("{path}.{key}"))),
        Some(Value::String(s)) => Ok(s.clone()),
        Some(_) => Err(ScenarioError::InvalidField(
            format!("{path}.{key}"),
            "expected a string".into(),
        )),
    }
}

fn json_str_or(v: &Value, key: &str, default: &str) -> Result<String, ScenarioError> {
    match v.get(key) {
        None | Some(Value::Null) => Ok(default.to_string()),
        Some(Value::String(s)) => Ok(s.clone()),
        Some(_) => Err(ScenarioError::InvalidField(
            key.to_string(),
            "expected a string".into(),
        )),
    }
}

fn json_int(v: &Value, path: &str, key: &str) -> Result<i64, ScenarioError> {
    match v.get(key) {
        None => Err(ScenarioError::MissingField(format!("{path}.{key}"))),
        Some(n) => n.as_i64().ok_or_else(|| {
            ScenarioError::InvalidField(format!("{path}.{key}"), "expected an integer".into())
        }),
    }
}

fn json_int_or(v: &Value, path: &str, key: &str, default: i64) -> Result<i64, ScenarioError> {
    match v.get(key) {
        None | Some(Value::Null) => Ok(default),
        Some(n) => n.as_i64().ok_or_else(|| {
            ScenarioError::InvalidField(format!("{path}.{key}"), "expected an integer".into())
        }),
    }
}

fn json_maturity(v: &Value, path: &str, key: &str) -> Result<MaturityText, ScenarioError> {
    match v.get(key) {
        None => Err(ScenarioError::MissingField(format!("{path}.{key}"))),
        Some(Value::String(s)) => Ok(MaturityText::Name(s.clone())),
        Some(n) => n.as_i64().map(MaturityText::Level).ok_or_else(|| {
            ScenarioError::InvalidField(
                format!("{path}.{key}"),
                "expected a maturity level 0..=4 or a level name".into(),
            )
        }),
    }
}

fn json_array<'a>(v: &'a Value, path: &str, key: &str) -> Result<&'a [Value], ScenarioError> {
    match v.get(key) {
        None => Err(ScenarioError::MissingField(format!("{path}.{key}"))),
        Some(Value::Array(items)) => Ok(items.as_slice()),
        Some(_) => Err(ScenarioError::InvalidField(
            format!("{path}.{key}"),
            "expected an array".into(),
        )),
    }
}

impl Scenario {
    /// Serialises the scenario to the JSON shape documented on this
    /// module.
    pub fn to_json(&self) -> Value {
        json!({
            "name": self.name.as_str(),
            "description": self.description.as_str(),
            "domains": self.domains.iter().map(|d| json!({
                "id": d.id.as_str(),
                "name": d.name.as_str(),
                "owner": d.owner.as_str(),
                "maturity": d.maturity.level(),
                "sources": d.sources.iter().map(|s| Value::from(s.as_str())).collect::<Vec<Value>>(),
            })).collect::<Vec<_>>(),
            "sources": self.sources.iter().map(|s| json!({
                "id": s.id.as_str(),
                "domain": s.domain.as_str(),
                "name": s.name.as_str(),
                "kind": s.kind.as_str(),
                "structure": s.structure.value(),
                "availability": s.availability.value(),
                "latency_secs": s.latency.as_secs(),
            })).collect::<Vec<_>>(),
            "outcomes": self.outcomes.iter().map(|o| json!({
                "id": o.id.as_str(),
                "name": o.name.as_str(),
                "description": o.description.as_str(),
                "requires": o.requires.iter().map(|r| json!({
                    "domain": r.domain.as_str(),
                    "min_maturity": r.min_maturity.level(),
                    "criticality": r.criticality.as_str(),
                })).collect::<Vec<_>>(),
            })).collect::<Vec<_>>(),
        })
    }

    /// [`Scenario::to_json`], pretty-printed.
    pub fn to_json_string(&self) -> String {
        rusty_json::to_string_pretty(&self.to_json()).expect("a JSON Value always serialises")
    }

    /// Parses a scenario from the JSON shape documented on this module,
    /// with the same validation as [`Scenario::from_toml`].
    pub fn from_json(v: &Value) -> Result<Self, ScenarioError> {
        let name = json_str(v, "", "name").map_err(|e| match e {
            ScenarioError::MissingField(_) => ScenarioError::MissingField("name".into()),
            other => other,
        })?;
        let description = json_str_or(v, "description", "")?;

        let domains = json_array(v, "", "domains")?
            .iter()
            .enumerate()
            .map(|(i, d)| {
                let path = format!("domains[{i}]");
                Ok(RawDomain {
                    id: json_str(d, &path, "id")?,
                    name: json_str(d, &path, "name")?,
                    owner: json_str_or(d, "owner", "")?,
                    maturity: json_maturity(d, &path, "maturity")?,
                    order: i as i64,
                })
            })
            .collect::<Result<Vec<_>, ScenarioError>>()?;

        let sources = json_array(v, "", "sources")?
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let path = format!("sources[{i}]");
                Ok(RawSource {
                    id: json_str(s, &path, "id")?,
                    domain: json_str(s, &path, "domain")?,
                    name: json_str(s, &path, "name")?,
                    kind: json_str(s, &path, "kind")?,
                    structure: json_int(s, &path, "structure")?,
                    availability: json_int(s, &path, "availability")?,
                    latency_secs: json_int_or(s, &path, "latency_secs", 0)?,
                    order: i as i64,
                })
            })
            .collect::<Result<Vec<_>, ScenarioError>>()?;

        let outcomes = json_array(v, "", "outcomes")?
            .iter()
            .enumerate()
            .map(|(i, o)| {
                let path = format!("outcomes[{i}]");
                let requires = json_array(o, &path, "requires")?
                    .iter()
                    .enumerate()
                    .map(|(j, r)| {
                        let rpath = format!("{path}.requires[{j}]");
                        Ok(RawRequirement {
                            domain: json_str(r, &rpath, "domain")?,
                            min_maturity: json_maturity(r, &rpath, "min_maturity")?,
                            criticality: json_str(r, &rpath, "criticality")?,
                        })
                    })
                    .collect::<Result<Vec<_>, ScenarioError>>()?;
                Ok(RawOutcome {
                    id: json_str(o, &path, "id")?,
                    name: json_str(o, &path, "name")?,
                    description: json_str_or(o, "description", "")?,
                    requires,
                    order: i as i64,
                })
            })
            .collect::<Result<Vec<_>, ScenarioError>>()?;

        assemble(name, description, domains, sources, outcomes)
    }

    /// Parses a scenario from JSON text.
    pub fn from_json_str(text: &str) -> Result<Self, ScenarioError> {
        let v: Value =
            rusty_json::from_str(text).map_err(|e| ScenarioError::Json(e.to_string()))?;
        Self::from_json(&v)
    }
}

fn node_json(n: &NodeRef) -> Value {
    let (kind, id) = match n {
        NodeRef::Outcome(id) => ("outcome", id.as_str()),
        NodeRef::Domain(id) => ("domain", id.as_str()),
        NodeRef::Source(id) => ("source", id.as_str()),
    };
    json!({ "kind": kind, "id": id })
}

impl TraceReport {
    /// Serialises the report to the JSON shape documented on this module.
    pub fn to_json(&self) -> Value {
        let achievable = match self.achievable {
            Fidelity::Full => json!({ "kind": "full" }),
            Fidelity::Partial(fraction) => {
                json!({ "kind": "partial", "fraction": fraction as f64 })
            }
            Fidelity::NotAchievable => json!({ "kind": "not_achievable" }),
        };
        json!({
            "outcome": self.outcome.as_str(),
            "outcome_name": self.outcome_name.as_str(),
            "achievable": achievable,
            "bottlenecks": self.bottlenecks.iter().map(|b| json!({
                "domain": b.domain.as_str(),
                "domain_name": b.domain_name.as_str(),
                "owner": b.owner.as_str(),
                "current": b.current.level(),
                "required": b.required.level(),
                "gap": b.gap,
                "criticality": b.criticality.as_str(),
                "state": b.state.as_str(),
                "sources": b.sources.iter().map(|s| Value::from(s.as_str())).collect::<Vec<Value>>(),
            })).collect::<Vec<_>>(),
            "edges": self.edges.iter().map(|e| json!({
                "from": node_json(&e.from),
                "to": node_json(&e.to),
                "state": e.state.as_str(),
            })).collect::<Vec<_>>(),
        })
    }

    /// [`TraceReport::to_json`], pretty-printed.
    pub fn to_json_string(&self) -> String {
        rusty_json::to_string_pretty(&self.to_json()).expect("a JSON Value always serialises")
    }
}
