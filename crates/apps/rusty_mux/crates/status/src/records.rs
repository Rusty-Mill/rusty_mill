//! A block's status records, one per [`Id`].

use std::collections::BTreeMap;

use crate::{Id, Kind, Record, Report, State};

/// The most records a block keeps; a new one past it evicts the least
/// recently updated. The protocol allows 64 to 256.
pub const MAX_RECORDS: usize = 64;

/// The records of one terminal, as its program reported them.
#[derive(Debug, Default)]
pub struct Records {
    /// Each record with the clock value of its last update.
    map: BTreeMap<Id, (Record, u64)>,
    clock: u64,
}

impl Records {
    /// Applies a report: a set replaces its record whole, a clear removes the
    /// record and those beneath it.
    pub fn apply(&mut self, report: Report) {
        match report {
            Report::Clear(id) => self.map.retain(|k, _| !k.within(&id)),
            Report::Set(id, record) => {
                if !self.map.contains_key(&id) && self.map.len() >= MAX_RECORDS {
                    self.evict_oldest();
                }
                self.clock += 1;
                self.map.insert(id, (record, self.clock));
            }
        }
    }

    /// A new shell prompt: working and blocked records end.
    pub fn prompt(&mut self) {
        self.map
            .retain(|_, (r, _)| !matches!(r.state, State::Working | State::Blocked));
    }

    /// A full reset: every record ends.
    pub fn reset(&mut self) {
        self.map.clear();
    }

    /// The records in id order, the root first.
    pub fn iter(&self) -> impl Iterator<Item = (&Id, &Record)> {
        self.map.iter().map(|(id, (record, _))| (id, record))
    }

    /// Each record's id, state and kind: what a status indicator shows.
    /// Compare two to tell whether a change is worth announcing.
    pub fn states(&self) -> Vec<(Id, State, Option<Kind>)> {
        self.iter()
            .map(|(id, r)| (id.clone(), r.state, r.kind))
            .collect()
    }

    /// Each record as an OSC 7501 body, in id order. A record without `app`
    /// takes its nearest ancestor's, as the protocol says to show it.
    pub fn bodies(&self) -> Vec<String> {
        self.iter()
            .map(|(id, record)| body(id, record, self.inherited_app(id, record)))
            .collect()
    }

    fn inherited_app<'a>(&'a self, id: &Id, record: &'a Record) -> Option<&'a str> {
        record.app.as_deref().or_else(|| {
            id.ancestors()
                .find_map(|a| self.map.get(&a).and_then(|(r, _)| r.app.as_deref()))
        })
    }

    fn evict_oldest(&mut self) {
        let oldest = self
            .map
            .iter()
            .min_by_key(|(_, (_, at))| *at)
            .map(|(id, _)| id.clone());
        if let Some(id) = oldest {
            self.map.remove(&id);
        }
    }
}

fn body(id: &Id, record: &Record, app: Option<&str>) -> String {
    let mut pairs = vec![format!("state={}", state_name(record.state))];
    if !id.is_root() {
        pairs.push(format!("id={}", id.as_str()));
    }
    if let Some(kind) = record.kind {
        pairs.push(format!("kind={}", kind_name(kind)));
    }
    if let Some(progress) = record.progress {
        pairs.push(format!("progress={progress}"));
    }
    if let Some(app) = app {
        pairs.push(format!("app={app}"));
    }
    for (key, text) in [("title", &record.title), ("msg", &record.msg)] {
        if let Some(text) = text {
            pairs.push(format!(
                "{key}={}",
                rusty_base64::encode_standard(text.as_bytes())
            ));
        }
    }
    pairs.join(":")
}

const fn state_name(state: State) -> &'static str {
    match state {
        State::Idle => "idle",
        State::Working => "working",
        State::Blocked => "blocked",
        State::Done => "done",
        State::Error => "error",
    }
}

const fn kind_name(kind: Kind) -> &'static str {
    match kind {
        Kind::Permission => "permission",
        Kind::Question => "question",
        Kind::Auth => "auth",
    }
}

#[cfg(test)]
mod tests;
