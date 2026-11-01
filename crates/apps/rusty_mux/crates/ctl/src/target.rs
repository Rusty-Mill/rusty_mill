//! Resolving a target to an id against a listing, the way `--block 2` or
//! `--window main/logs` names an entity without its full id.
//!
//! A target is a full id, a case-insensitive name (an empty one names
//! nothing), a position counting from 1, or the last four or more
//! characters of an id, tried in that order. Clients are named only by
//! themselves, never by a path. A path `SESSION/WINDOW/BLOCK` names the parents too; a shorter path drops
//! leading parts. Without a session part, windows and blocks are looked up
//! in the default session when there is one.

use rusty_mux_proto::Entry;
use rusty_mux_proto::schema::Entity;
use rusty_uuid::Uuid;

/// Why a target named no single entity.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Miss {
    NotFound(String),
    /// What was looked up, and the ids that matched it.
    Ambiguous(String, Vec<Uuid>),
}

/// One entity of a listing, flattened.
struct Node<'a> {
    entity: Entity,
    id: Uuid,
    parent: Option<Uuid>,
    name: Option<&'a str>,
}

/// The id of the `entity` that `spec` names among `entries`, looking in
/// `session` when the spec names no session.
pub(crate) fn resolve(
    entries: &[Entry],
    entity: Entity,
    spec: &str,
    session: Option<Uuid>,
) -> Result<Uuid, Miss> {
    let nodes: Vec<Node<'_>> = entries.iter().map(node).collect();
    let chain = chain(entity);
    let segments: Vec<&str> = spec.split('/').collect();
    let Some(start) = chain.len().checked_sub(segments.len()) else {
        return Err(Miss::NotFound(format!("`{spec}` has too many parts")));
    };
    let mut within = if start > 0 { session } else { None };
    for (kind, segment) in chain[start..].iter().zip(segments) {
        let candidates: Vec<&Node<'_>> = nodes
            .iter()
            .filter(|n| n.entity == *kind && within.is_none_or(|w| descends(&nodes, n, w)))
            .collect();
        within = Some(pick(&candidates, *kind, segment)?);
    }
    within.ok_or_else(|| Miss::NotFound(format!("`{spec}` is empty")))
}

fn chain(entity: Entity) -> &'static [Entity] {
    match entity {
        Entity::Session => &[Entity::Session],
        Entity::Window => &[Entity::Session, Entity::Window],
        Entity::Block => &[Entity::Session, Entity::Window, Entity::Block],
        Entity::Client => &[Entity::Client],
    }
}

fn node(entry: &Entry) -> Node<'_> {
    match entry {
        Entry::Session { session, name, .. } => Node {
            entity: Entity::Session,
            id: session.uuid(),
            parent: None,
            name: Some(name),
        },
        Entry::Window {
            window,
            session,
            name,
            ..
        } => Node {
            entity: Entity::Window,
            id: window.uuid(),
            parent: Some(session.uuid()),
            name: Some(name),
        },
        Entry::Block {
            block,
            window,
            name,
            ..
        } => Node {
            entity: Entity::Block,
            id: block.uuid(),
            parent: Some(window.uuid()),
            name: Some(name),
        },
        Entry::Client { client, name, .. } => Node {
            entity: Entity::Client,
            id: client.uuid(),
            parent: None,
            name: Some(name),
        },
    }
}

/// Whether `node` is `ancestor` or below it.
fn descends(nodes: &[Node<'_>], node: &Node<'_>, ancestor: Uuid) -> bool {
    let mut current = Some(node.id);
    while let Some(id) = current {
        if id == ancestor {
            return true;
        }
        current = nodes.iter().find(|n| n.id == id).and_then(|n| n.parent);
    }
    false
}

/// The one candidate `segment` names, by the first rule that matches any.
fn pick(candidates: &[&Node<'_>], entity: Entity, segment: &str) -> Result<Uuid, Miss> {
    let lower = segment.to_lowercase();
    let position = segment
        .parse::<usize>()
        .ok()
        .and_then(|n| n.checked_sub(1))
        .and_then(|i| candidates.get(i));
    let rules: [Vec<Uuid>; 4] = [
        ids(candidates, |n| segment.parse() == Ok(n.id)),
        ids(candidates, |n| {
            n.name
                .is_some_and(|name| !name.is_empty() && name.to_lowercase() == lower)
        }),
        position.map(|n| vec![n.id]).unwrap_or_default(),
        ids(candidates, |n| {
            segment.len() >= 4 && n.id.to_string().ends_with(&lower)
        }),
    ];
    let what = format!("{} `{segment}`", entity.name());
    match rules.into_iter().find(|ids| !ids.is_empty()) {
        Some(ids) if ids.len() == 1 => Ok(ids[0]),
        Some(ids) => Err(Miss::Ambiguous(what, ids)),
        None => Err(Miss::NotFound(format!("no {what}"))),
    }
}

fn ids(candidates: &[&Node<'_>], matches: impl Fn(&Node<'_>) -> bool) -> Vec<Uuid> {
    candidates
        .iter()
        .filter(|n| matches(n))
        .map(|n| n.id)
        .collect()
}

#[cfg(test)]
mod tests;
