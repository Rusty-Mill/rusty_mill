//! The reverse-trace algorithm: outcome -> domains -> sources.

use rusty_err::Error;

use crate::model::{
    Bottleneck, Criticality, EdgeState, Fidelity, NodeRef, OutcomeId, Scenario, TraceEdge,
    TraceReport,
};

/// Errors from tracing an outcome. A scenario that passed
/// [`Scenario::from_toml`](crate::Scenario::from_toml)'s reference checks
/// can only produce `UnknownOutcome`; `UnknownDomain` guards a scenario
/// assembled by hand.
#[derive(Debug, Error, PartialEq, Eq, Clone)]
pub enum TraceError {
    /// No outcome with this id in the scenario.
    #[error("unknown outcome {0:?}")]
    UnknownOutcome(String),
    /// Outcome `{0}` requires domain `{1}`, which the scenario does not
    /// define.
    #[error("outcome {0:?} requires unknown domain {1:?}")]
    UnknownDomain(String, String),
}

/// Traces one outcome back through the domains it requires and the
/// sources behind them.
///
/// For each requirement: `gap = min_maturity - domain.maturity` (clamped
/// at zero); `gap == 0` is [`EdgeState::Satisfied`], otherwise the
/// requirement's criticality decides `Blocked` / `Degraded` / `Missing`.
/// Any blocked requirement makes the outcome [`Fidelity::NotAchievable`];
/// else any degraded one makes it `Partial(satisfied_weight /
/// total_weight)`; else it is `Full`. Unmet requirements come back as
/// bottlenecks sorted worst-first. Pure: no I/O, no clock.
pub fn trace(scenario: &Scenario, outcome_id: &OutcomeId) -> Result<TraceReport, TraceError> {
    let outcome = scenario
        .outcome(outcome_id)
        .ok_or_else(|| TraceError::UnknownOutcome(outcome_id.to_string()))?;

    let mut edges = Vec::new();
    let mut source_edges = Vec::new();
    let mut bottlenecks = Vec::new();
    let mut any_blocked = false;
    let mut any_degraded = false;
    let mut satisfied_weight = 0.0f32;
    let mut total_weight = 0.0f32;

    for req in &outcome.requires {
        let domain = scenario.domain(&req.domain).ok_or_else(|| {
            TraceError::UnknownDomain(outcome_id.to_string(), req.domain.to_string())
        })?;
        let gap = domain.maturity.gap_to(req.min_maturity);
        let state = if gap == 0 {
            EdgeState::Satisfied
        } else {
            match req.criticality {
                Criticality::Blocking => EdgeState::Blocked,
                Criticality::Degrading => EdgeState::Degraded,
                Criticality::Optional => EdgeState::Missing,
            }
        };

        let weight = req.criticality.weight();
        total_weight += weight;
        match state {
            EdgeState::Satisfied => satisfied_weight += weight,
            EdgeState::Blocked => any_blocked = true,
            EdgeState::Degraded => any_degraded = true,
            EdgeState::Missing => {}
        }

        edges.push(TraceEdge {
            from: NodeRef::Domain(domain.id.clone()),
            to: NodeRef::Outcome(outcome.id.clone()),
            state,
        });
        for source in scenario.sources_of(&domain.id) {
            source_edges.push(TraceEdge {
                from: NodeRef::Source(source.id.clone()),
                to: NodeRef::Domain(domain.id.clone()),
                state,
            });
        }

        if state != EdgeState::Satisfied {
            bottlenecks.push(Bottleneck {
                domain: domain.id.clone(),
                domain_name: domain.name.clone(),
                owner: domain.owner.clone(),
                current: domain.maturity,
                required: req.min_maturity,
                gap,
                criticality: req.criticality,
                state,
                sources: scenario
                    .sources_of(&domain.id)
                    .map(|s| s.id.clone())
                    .collect(),
            });
        }
    }
    edges.extend(source_edges);

    let achievable = if any_blocked {
        Fidelity::NotAchievable
    } else if any_degraded {
        let fraction = if total_weight > 0.0 {
            satisfied_weight / total_weight
        } else {
            1.0
        };
        Fidelity::Partial(fraction)
    } else {
        Fidelity::Full
    };

    // Worst first: highest criticality, then widest gap, then a stable
    // tiebreak on domain id so the "what to fund first" list is
    // deterministic.
    bottlenecks.sort_by(|a, b| {
        b.criticality
            .cmp(&a.criticality)
            .then(b.gap.cmp(&a.gap))
            .then(a.domain.cmp(&b.domain))
    });

    Ok(TraceReport {
        outcome: outcome.id.clone(),
        outcome_name: outcome.name.clone(),
        achievable,
        bottlenecks,
        edges,
    })
}

/// Traces every outcome in the scenario, in scenario order.
pub fn trace_all(scenario: &Scenario) -> Result<Vec<TraceReport>, TraceError> {
    scenario
        .outcomes
        .iter()
        .map(|o| trace(scenario, &o.id))
        .collect()
}
