//! The data model: maturity ladder, domains, sources, outcomes,
//! requirements, and the trace-report types a renderer consumes.

use std::fmt;
use std::time::Duration;

use rusty_err::Error;

/// Errors from constructing a model value out of raw scalars (a maturity
/// level outside `0..=4`, an unknown enum name, ...).
#[derive(Debug, Error, PartialEq, Eq, Clone)]
pub enum ModelError {
    /// A maturity level outside `0..=4`.
    #[error("maturity level must be 0..=4, got {0}")]
    InvalidMaturity(i64),
    /// A source rating outside `0..=4`.
    #[error("rating must be 0..=4, got {0}")]
    InvalidRating(i64),
    /// An unrecognised [`SourceKind`] name.
    #[error("unknown source kind {0:?} (expected system|file|person|process)")]
    UnknownSourceKind(String),
    /// An unrecognised [`Criticality`] name.
    #[error("unknown criticality {0:?} (expected blocking|degrading|optional)")]
    UnknownCriticality(String),
    /// An unrecognised [`Maturity`] name.
    #[error("unknown maturity {0:?} (expected tribal|documented|structured|accessible|integrated or 0..=4)")]
    UnknownMaturity(String),
}

// ── Maturity ladder ──────────────────────────────────────────────────────

/// Ordinal domain maturity level, `0..=4`.
///
/// Levels are ordinal: a domain has exactly one current level, and the
/// level determines how (and whether) the domain can feed an outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum Maturity {
    /// Data exists only in people's heads or undocumented process. No
    /// artifact. ("Ask the PM lead.")
    Tribal = 0,
    /// Static artifacts: slide decks, PDFs, Word docs, a personal
    /// spreadsheet. (Monthly PMR slides.)
    Documented = 1,
    /// In a system with a schema, but siloed; extraction is manual or ad
    /// hoc. (Excel tracker on SharePoint, legacy DB with no API.)
    Structured = 2,
    /// Programmatically queryable: API, DB connection, scheduled export.
    /// (IMS via Project Server export.)
    Accessible = 3,
    /// On the mesh: governed, data contract, flows automatically,
    /// observable. (Any domain already feeding Rusty Meshed.)
    Integrated = 4,
}

impl Maturity {
    /// Every level, lowest first.
    pub const ALL: [Maturity; 5] = [
        Maturity::Tribal,
        Maturity::Documented,
        Maturity::Structured,
        Maturity::Accessible,
        Maturity::Integrated,
    ];

    /// The level as its ordinal, `0..=4`.
    pub fn level(self) -> u8 {
        self as u8
    }

    /// Builds a level from its ordinal; anything outside `0..=4` is an
    /// error rather than a clamp, so a typo in a scenario file fails
    /// loudly.
    pub fn from_level(level: i64) -> Result<Self, ModelError> {
        match level {
            0 => Ok(Maturity::Tribal),
            1 => Ok(Maturity::Documented),
            2 => Ok(Maturity::Structured),
            3 => Ok(Maturity::Accessible),
            4 => Ok(Maturity::Integrated),
            other => Err(ModelError::InvalidMaturity(other)),
        }
    }

    /// The level's short name, as used in the org's shorthand ("that
    /// domain's a Level 1 -- Documented").
    pub fn name(self) -> &'static str {
        match self {
            Maturity::Tribal => "Tribal",
            Maturity::Documented => "Documented",
            Maturity::Structured => "Structured",
            Maturity::Accessible => "Accessible",
            Maturity::Integrated => "Integrated",
        }
    }

    /// Parses either a level name (case-insensitive) or an ordinal.
    pub fn parse(text: &str) -> Result<Self, ModelError> {
        if let Ok(n) = text.trim().parse::<i64>() {
            return Self::from_level(n);
        }
        match text.trim().to_ascii_lowercase().as_str() {
            "tribal" => Ok(Maturity::Tribal),
            "documented" => Ok(Maturity::Documented),
            "structured" => Ok(Maturity::Structured),
            "accessible" => Ok(Maturity::Accessible),
            "integrated" => Ok(Maturity::Integrated),
            _ => Err(ModelError::UnknownMaturity(text.to_string())),
        }
    }

    /// How many levels `self` falls short of `required`, clamped at zero.
    pub fn gap_to(self, required: Maturity) -> u8 {
        required.level().saturating_sub(self.level())
    }
}

impl fmt::Display for Maturity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "L{} {}", self.level(), self.name())
    }
}

/// A finer-grained `0..=4` rating carried by an individual source (how
/// well-structured its data is, how readily it can be pulled). Independent
/// of each other on purpose: a thing can be highly structured but locked
/// away, or freely available but chaos.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Rating(u8);

impl Rating {
    /// Builds a rating; anything outside `0..=4` is an error.
    pub fn new(value: i64) -> Result<Self, ModelError> {
        match value {
            0..=4 => Ok(Rating(value as u8)),
            other => Err(ModelError::InvalidRating(other)),
        }
    }

    /// The rating's value, `0..=4`.
    pub fn value(self) -> u8 {
        self.0
    }
}

// ── Identifiers ──────────────────────────────────────────────────────────

macro_rules! id_newtype {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(String);

        impl $name {
            /// Wraps a raw identifier string.
            pub fn new(id: impl Into<String>) -> Self {
                $name(id.into())
            }

            /// The raw identifier.
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl From<&str> for $name {
            fn from(s: &str) -> Self {
                $name(s.to_string())
            }
        }

        impl From<String> for $name {
            fn from(s: String) -> Self {
                $name(s)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

id_newtype!(
    /// Identifies a [`Domain`] within a scenario (e.g. `"schedule"`).
    DomainId
);
id_newtype!(
    /// Identifies a [`Source`] within a scenario (e.g. `"ims_export"`).
    SourceId
);
id_newtype!(
    /// Identifies an [`Outcome`] within a scenario (e.g. `"acq_status"`).
    OutcomeId
);

// ── Inventory ────────────────────────────────────────────────────────────

/// A business/data domain (Schedule, Cost, Risk, Contracts, ...).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Domain {
    /// Scenario-unique identifier.
    pub id: DomainId,
    /// Display name.
    pub name: String,
    /// PM office / functional lead responsible for the domain.
    pub owner: String,
    /// The domain's single current maturity level.
    pub maturity: Maturity,
    /// The sources holding this domain's data, in scenario order.
    pub sources: Vec<SourceId>,
}

/// What kind of thing a [`Source`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SourceKind {
    /// A queryable system (EVM system, risk register app, PLM).
    System,
    /// A static artifact (spreadsheet on a share, PDF contract mods).
    File,
    /// A person ("the program lead's head").
    Person,
    /// An undocumented or manual process.
    Process,
}

impl SourceKind {
    /// The kind's lowercase wire name.
    pub fn as_str(self) -> &'static str {
        match self {
            SourceKind::System => "system",
            SourceKind::File => "file",
            SourceKind::Person => "person",
            SourceKind::Process => "process",
        }
    }

    /// Parses a wire name (case-insensitive).
    pub fn parse(text: &str) -> Result<Self, ModelError> {
        match text.trim().to_ascii_lowercase().as_str() {
            "system" => Ok(SourceKind::System),
            "file" => Ok(SourceKind::File),
            "person" => Ok(SourceKind::Person),
            "process" => Ok(SourceKind::Process),
            _ => Err(ModelError::UnknownSourceKind(text.to_string())),
        }
    }
}

/// A concrete system, file, person, or process that holds data for a
/// domain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    /// Scenario-unique identifier.
    pub id: SourceId,
    /// The domain this source belongs to.
    pub domain: DomainId,
    /// Display name.
    pub name: String,
    /// What kind of thing it is.
    pub kind: SourceKind,
    /// How well-structured the data is, `0..=4`.
    pub structure: Rating,
    /// How readily it can be pulled, `0..=4`.
    pub availability: Rating,
    /// Refresh cadence. `Duration::ZERO` means "no cadence -- whenever you
    /// ask", which is what a Level 0 source amounts to.
    pub latency: Duration,
}

/// How much an outcome depends on one of its requirements.
///
/// Ordered so that `Blocking > Degrading > Optional`; a worst-first sort
/// is a descending sort on this.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Criticality {
    /// Nice-to-have: an unmet optional requirement never changes the
    /// verdict, it just shows up grey in the trace.
    Optional = 0,
    /// The outcome still ships without it, at reduced fidelity (that
    /// panel blank or manual).
    Degrading = 1,
    /// The outcome cannot be delivered as asked without it.
    Blocking = 2,
}

impl Criticality {
    /// The criticality's lowercase wire name.
    pub fn as_str(self) -> &'static str {
        match self {
            Criticality::Blocking => "blocking",
            Criticality::Degrading => "degrading",
            Criticality::Optional => "optional",
        }
    }

    /// Parses a wire name (case-insensitive).
    pub fn parse(text: &str) -> Result<Self, ModelError> {
        match text.trim().to_ascii_lowercase().as_str() {
            "blocking" => Ok(Criticality::Blocking),
            "degrading" => Ok(Criticality::Degrading),
            "optional" => Ok(Criticality::Optional),
            _ => Err(ModelError::UnknownCriticality(text.to_string())),
        }
    }

    /// The requirement's weight in a `Partial` fidelity fraction: blocking
    /// requirements count three times an optional one, degrading twice.
    pub fn weight(self) -> f32 {
        match self {
            Criticality::Blocking => 3.0,
            Criticality::Degrading => 2.0,
            Criticality::Optional => 1.0,
        }
    }
}

/// What an outcome needs from a domain, and at what minimum maturity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Requirement {
    /// The domain the outcome draws on.
    pub domain: DomainId,
    /// Fidelity floor for this panel/metric: the domain must be at or
    /// above this level to satisfy the requirement.
    pub min_maturity: Maturity,
    /// How much the outcome depends on it.
    pub criticality: Criticality,
}

/// A leadership-facing question or dashboard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    /// Scenario-unique identifier.
    pub id: OutcomeId,
    /// Display name.
    pub name: String,
    /// What leadership is actually asking for, in their words.
    pub description: String,
    /// Everything the outcome needs, one entry per domain it draws on.
    pub requires: Vec<Requirement>,
}

/// A complete inventory: domains, their sources, and the outcomes that
/// depend on them. Loaded from TOML or JSON; the unit a trace runs over.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Scenario {
    /// Scenario title.
    pub name: String,
    /// What this scenario represents and any caveats about its numbers.
    pub description: String,
    /// All domains, in file order.
    pub domains: Vec<Domain>,
    /// All sources, in file order.
    pub sources: Vec<Source>,
    /// All outcomes, in file order.
    pub outcomes: Vec<Outcome>,
}

impl Scenario {
    /// Looks up a domain by id.
    pub fn domain(&self, id: &DomainId) -> Option<&Domain> {
        self.domains.iter().find(|d| &d.id == id)
    }

    /// Looks up a source by id.
    pub fn source(&self, id: &SourceId) -> Option<&Source> {
        self.sources.iter().find(|s| &s.id == id)
    }

    /// Looks up an outcome by id.
    pub fn outcome(&self, id: &OutcomeId) -> Option<&Outcome> {
        self.outcomes.iter().find(|o| &o.id == id)
    }

    /// The sources belonging to `domain`, in scenario order.
    pub fn sources_of<'a>(&'a self, domain: &'a DomainId) -> impl Iterator<Item = &'a Source> + 'a {
        self.sources.iter().filter(move |s| &s.domain == domain)
    }

    /// What-if support: sets one domain's maturity in place. Returns
    /// `false` (and changes nothing) if the domain is unknown. Callers
    /// wanting scenario-local changes clone first; nothing here persists.
    pub fn set_maturity(&mut self, domain: &DomainId, maturity: Maturity) -> bool {
        match self.domains.iter_mut().find(|d| &d.id == domain) {
            Some(d) => {
                d.maturity = maturity;
                true
            }
            None => false,
        }
    }
}

// ── Trace output ─────────────────────────────────────────────────────────

/// How one requirement (and the edges behind it) came out of a trace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EdgeState {
    /// `gap == 0`: the domain meets the floor. Green, animated flow.
    Satisfied,
    /// `gap > 0` on a `Blocking` requirement. Red, dashed, no flow.
    Blocked,
    /// `gap > 0` on a `Degrading` requirement. Amber, throttled flow.
    Degraded,
    /// `gap > 0` on an `Optional` requirement. Grey, dotted.
    Missing,
}

impl EdgeState {
    /// The state's lowercase wire name.
    pub fn as_str(self) -> &'static str {
        match self {
            EdgeState::Satisfied => "satisfied",
            EdgeState::Blocked => "blocked",
            EdgeState::Degraded => "degraded",
            EdgeState::Missing => "missing",
        }
    }
}

/// How much of an outcome is achievable today.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Fidelity {
    /// Every requirement is satisfied.
    Full,
    /// No blocking gaps, but at least one degrading one: the fraction is
    /// `satisfied_weight / total_weight` over all requirements, weighted
    /// by [`Criticality::weight`].
    Partial(f32),
    /// At least one blocking requirement is unmet.
    NotAchievable,
}

impl Fidelity {
    /// The fidelity's lowercase wire name.
    pub fn as_str(self) -> &'static str {
        match self {
            Fidelity::Full => "full",
            Fidelity::Partial(_) => "partial",
            Fidelity::NotAchievable => "not_achievable",
        }
    }
}

impl fmt::Display for Fidelity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Fidelity::Full => f.write_str("Full"),
            Fidelity::Partial(fraction) => write!(f, "Partial ({:.0}%)", fraction * 100.0),
            Fidelity::NotAchievable => f.write_str("Not achievable"),
        }
    }
}

/// One unmet requirement, with everything a "what to fund first" list
/// needs to say about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bottleneck {
    /// The short-falling domain.
    pub domain: DomainId,
    /// Its display name.
    pub domain_name: String,
    /// Who owns closing the gap.
    pub owner: String,
    /// Where the domain is today.
    pub current: Maturity,
    /// Where the outcome needs it to be.
    pub required: Maturity,
    /// `required - current`, always `>= 1` here.
    pub gap: u8,
    /// How much the outcome depends on it.
    pub criticality: Criticality,
    /// The edge state this bottleneck produced (never `Satisfied`).
    pub state: EdgeState,
    /// The domain's sources -- where the data lives today.
    pub sources: Vec<SourceId>,
}

/// A node in the rendered trace graph.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum NodeRef {
    /// The outcome being traced (rightmost column).
    Outcome(OutcomeId),
    /// A domain the outcome draws on (middle column).
    Domain(DomainId),
    /// A source behind a domain (leftmost column).
    Source(SourceId),
}

/// One edge of the trace, in data-flow direction (source -> domain,
/// domain -> outcome). The renderer animates the trace right-to-left --
/// the outcome "requests", particles attempt to flow back from sources
/// along these edges -- but the edge itself is stored the way the data
/// moves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraceEdge {
    /// Upstream end.
    pub from: NodeRef,
    /// Downstream end.
    pub to: NodeRef,
    /// The classification of the requirement this edge serves. A
    /// source -> domain edge carries the same state as its domain's edge
    /// to the outcome: the whole branch stalls at an immature domain.
    pub state: EdgeState,
}

/// Output of a reverse trace.
#[derive(Debug, Clone, PartialEq)]
pub struct TraceReport {
    /// The outcome that was traced.
    pub outcome: OutcomeId,
    /// Its display name.
    pub outcome_name: String,
    /// The verdict.
    pub achievable: Fidelity,
    /// Unmet requirements, sorted worst-first: by criticality, then by gap
    /// size, then by domain id for determinism.
    pub bottlenecks: Vec<Bottleneck>,
    /// Every edge of the trace, outcome edges first (in requirement
    /// order), then each domain's source edges.
    pub edges: Vec<TraceEdge>,
}

impl TraceReport {
    /// The state of the edge from `domain` to the traced outcome, if the
    /// outcome draws on that domain.
    pub fn domain_state(&self, domain: &DomainId) -> Option<EdgeState> {
        self.edges
            .iter()
            .find(|e| matches!((&e.from, &e.to), (NodeRef::Domain(d), NodeRef::Outcome(_)) if d == domain))
            .map(|e| e.state)
    }
}
