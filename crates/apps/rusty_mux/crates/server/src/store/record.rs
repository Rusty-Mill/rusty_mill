//! The saved model as `rusty_multimodal_db_engine` records (rusty_term D73,
//! D76): one record per session, holding its windows and their blocks, so
//! a window can only be in one session and a block in one window, and a
//! session changes in one write.

use std::collections::BTreeMap;

use rusty_multimodal_db_engine::generic::traits::{
    IndexedField, Record, ScannableField, SchemaTag,
};
use rusty_mux_core::{
    BlockId, Saved, SavedBlock, SavedSession, SavedWindow, SessionId, Size, WindowId,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// How a block's program was started, so a later run knows what to do with it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Started {
    /// Started with the server's configured program, not one the request named.
    pub(crate) shell: bool,
    /// Restored read-only already: its command was not run again (D72).
    pub(crate) read_only: bool,
    pub(crate) program: String,
    pub(crate) args: Vec<String>,
    /// Its last known working directory.
    pub(crate) cwd: String,
}

/// The saved model with each block's [`Started`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Manifest {
    pub(crate) saved: Saved,
    pub(crate) started: BTreeMap<BlockId, Started>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SessionRecord {
    pub(crate) id: Uuid,
    /// Its place in creation order.
    pub(crate) order: i64,
    name: String,
    generated: bool,
    active: Option<Uuid>,
    windows: Vec<WindowRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct WindowRecord {
    id: Uuid,
    name: String,
    /// In the model's text form, as `list` shows it.
    layout: Option<String>,
    zoomed: Option<Uuid>,
    focused: Option<Uuid>,
    blocks: Vec<BlockRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct BlockRecord {
    id: Uuid,
    name: String,
    cols: u16,
    rows: u16,
    started: Started,
}

impl Record for SessionRecord {
    type Id = Uuid;
    fn id(&self) -> Uuid {
        self.id
    }
}

impl SchemaTag for SessionRecord {
    const SCHEMA_TAG: &'static str = "rusty_mux::Session@1";
}

/// Index marker: sessions by whether the server named them.
pub(crate) struct ByGenerated;
impl IndexedField<ByGenerated> for SessionRecord {
    type IndexValue = bool;
    fn indexed_value(&self) -> &bool {
        &self.generated
    }
}

/// Slot marker: creation order.
pub(crate) struct ByOrder;
impl ScannableField<ByOrder> for SessionRecord {
    type ScanValue = i64;
    fn scannable_value(&self) -> i64 {
        self.order
    }
    fn set_scannable_value(&mut self, value: i64) {
        self.order = value;
    }
}

fn to_uuid(id: rusty_uuid::Uuid) -> Uuid {
    Uuid::from_bytes(*id.as_bytes())
}

fn from_uuid(id: Uuid) -> rusty_uuid::Uuid {
    rusty_uuid::Uuid::from_bytes(*id.as_bytes())
}

/// One record per session, in creation order. A block with no
/// [`Started`] (none should lack one) is saved as a shell.
pub(crate) fn records(manifest: &Manifest) -> Vec<SessionRecord> {
    let shell = Started {
        shell: true,
        read_only: false,
        program: String::new(),
        args: Vec::new(),
        cwd: String::new(),
    };
    let block = |b: &SavedBlock| BlockRecord {
        id: to_uuid(b.id.uuid()),
        name: b.name.clone(),
        cols: b.size.cols(),
        rows: b.size.rows(),
        started: manifest.started.get(&b.id).unwrap_or(&shell).clone(),
    };
    let window = |w: &SavedWindow| WindowRecord {
        id: to_uuid(w.id.uuid()),
        name: w.name.clone(),
        layout: w.layout.as_ref().map(ToString::to_string),
        zoomed: w.zoomed.map(|b| to_uuid(b.uuid())),
        focused: w.focused.map(|b| to_uuid(b.uuid())),
        blocks: w.blocks.iter().map(block).collect(),
    };
    manifest
        .saved
        .sessions
        .iter()
        .zip(0..)
        .map(|(s, order)| SessionRecord {
            id: to_uuid(s.id.uuid()),
            order,
            name: s.name.clone(),
            generated: s.generated,
            active: s.active.map(|w| to_uuid(w.uuid())),
            windows: s.windows.iter().map(window).collect(),
        })
        .collect()
}

/// The manifest `records` hold, sessions in creation order; refuses a
/// layout or size that does not parse. [`rusty_mux_core::Mux::restore`]
/// checks the rest.
pub(crate) fn manifest(mut records: Vec<SessionRecord>) -> Result<Manifest, String> {
    records.sort_by_key(|r| (r.order, r.id));
    let mut manifest = Manifest::default();
    for r in records {
        let mut windows = Vec::with_capacity(r.windows.len());
        for w in r.windows {
            windows.push(window(w, &mut manifest.started)?);
        }
        manifest.saved.sessions.push(SavedSession {
            id: SessionId::from_uuid(from_uuid(r.id)),
            name: r.name,
            generated: r.generated,
            active: r.active.map(|w| WindowId::from_uuid(from_uuid(w))),
            windows,
        });
    }
    Ok(manifest)
}

fn window(
    w: WindowRecord,
    started: &mut BTreeMap<BlockId, Started>,
) -> Result<SavedWindow, String> {
    let block_id = |id: Uuid| BlockId::from_uuid(from_uuid(id));
    let layout = match w.layout {
        Some(text) => Some(
            text.parse()
                .map_err(|_| format!("window {}: bad layout", w.id))?,
        ),
        None => None,
    };
    let mut blocks = Vec::with_capacity(w.blocks.len());
    for b in w.blocks {
        let size = Size::new(b.cols, b.rows).map_err(|e| format!("block {}: {e}", b.id))?;
        started.insert(block_id(b.id), b.started);
        blocks.push(SavedBlock {
            id: block_id(b.id),
            name: b.name,
            size,
        });
    }
    Ok(SavedWindow {
        id: WindowId::from_uuid(from_uuid(w.id)),
        name: w.name,
        layout,
        zoomed: w.zoomed.map(block_id),
        focused: w.focused.map(block_id),
        blocks,
    })
}

#[cfg(test)]
mod tests {
    use rusty_mux_core::{Mux, Side};

    use super::*;

    pub(crate) fn manifest_of_two_sessions() -> Manifest {
        let mut mux = Mux::new();
        let s = mux.create_session("my work").unwrap();
        let w = mux.create_window(s, "").unwrap();
        let a = mux.create_block(w, Size::new(80, 24).unwrap()).unwrap();
        let b = BlockId::from_uuid(rusty_uuid::Uuid::new_v4());
        mux.split_block(b, a, Side::Down, Size::new(80, 11).unwrap())
            .unwrap();
        mux.rename(rusty_mux_core::EntityId::Block(b), "100% =\n\u{e9}")
            .unwrap();
        mux.create_session("empty").unwrap();
        let started = |shell, args: &[&str]| Started {
            shell,
            read_only: false,
            program: "/bin/sh".into(),
            args: args.iter().map(|a| a.to_string()).collect(),
            cwd: "/home/a b".into(),
        };
        Manifest {
            saved: mux.save(),
            started: BTreeMap::from([(a, started(true, &[])), (b, started(false, &["-c", "x"]))]),
        }
    }

    #[test]
    fn round_trips_in_creation_order() {
        let manifest = manifest_of_two_sessions();
        let mut records = records(&manifest);
        assert_eq!(records.iter().map(|r| r.order).collect::<Vec<_>>(), [0, 1]);
        records.reverse();
        assert_eq!(super::manifest(records), Ok(manifest));
    }

    #[test]
    fn refuses_a_layout_or_size_that_does_not_parse() {
        let manifest = manifest_of_two_sessions();
        let mut bad = records(&manifest);
        bad[0].windows[0].layout = Some("rows(".into());
        assert!(super::manifest(bad).is_err());
        let mut bad = records(&manifest);
        bad[0].windows[0].blocks[0].cols = 0;
        assert!(super::manifest(bad).is_err());
    }
}
