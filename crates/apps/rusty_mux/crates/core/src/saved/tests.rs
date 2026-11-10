use super::*;
use crate::{Side, Size};

fn model() -> (Mux, BlockId) {
    let mut mux = Mux::new();
    let s = mux.create_session("work").unwrap();
    let w = mux.create_window(s, "main").unwrap();
    let a = mux.create_block(w, Size::new(80, 24).unwrap()).unwrap();
    let b = BlockId::random();
    mux.split_block(b, a, Side::Right, Size::new(40, 24).unwrap())
        .unwrap();
    let _detached = {
        let c = mux.create_block(w, Size::new(20, 10).unwrap()).unwrap();
        mux.detach_block(c).unwrap();
        c
    };
    mux.rename(crate::EntityId::Block(a), "editor").unwrap();
    mux.zoom(b).unwrap();
    mux.create_generated_session(7).unwrap();
    (mux, a)
}

#[test]
fn a_restored_model_saves_the_same_tree() {
    let (mux, _) = model();
    let saved = mux.save();
    let mut restored = Mux::restore(saved.clone()).unwrap();
    assert_eq!(restored.save(), saved);
    assert!(restored.take_events().is_empty());
    assert_eq!(restored.sessions().count(), 2);
}

#[test]
fn a_restored_model_keeps_working() {
    let (mux, a) = model();
    let mut restored = Mux::restore(mux.save()).unwrap();
    let window = restored.block(a).unwrap().window();
    let block = restored
        .create_block(window, Size::new(10, 5).unwrap())
        .unwrap();
    assert!(restored.block(block).is_some());
    restored.close_block(a).unwrap();
    assert!(restored.block(a).is_none());
}

fn broken(edit: impl FnOnce(&mut Saved)) -> RestoreError {
    let mut saved = model().0.save();
    edit(&mut saved);
    Mux::restore(saved).unwrap_err()
}

#[test]
fn refuses_trees_that_break_the_rules() {
    broken(|s| {
        let again = s.sessions[0].clone();
        s.sessions.push(again);
    });
    broken(|s| s.sessions[0].active = None);
    broken(|s| s.sessions[0].active = Some(WindowId::random()));
    broken(|s| s.sessions[0].windows[0].focused = Some(BlockId::random()));
    broken(|s| {
        let w = &mut s.sessions[0].windows[0];
        w.zoomed = Some(w.blocks[0].id);
    });
    broken(|s| {
        let w = &mut s.sessions[0].windows[0];
        w.layout = None;
    });
    broken(|s| drop(s.sessions[0].windows[0].blocks.remove(0)));
    broken(|s| s.sessions[0].windows[0].layout = Some(Layout::Block(BlockId::random())));
    broken(|s| s.sessions[0].name = "x".repeat(MAX_NAME + 1));
}

/// D164: a creator is not saved, so a restored session has none and
/// belongs to the server's own account.
#[test]
fn a_restored_session_has_no_creator() {
    let (mut mux, a) = model();
    let session = mux.sessions().next().unwrap().id();
    let them = crate::Principal::unix(1001, Some("them".into())).unwrap();
    mux.set_creator(session, them).unwrap();
    assert!(mux.sessions().any(|s| s.creator().is_some()));
    let restored = Mux::restore(mux.save()).unwrap();
    assert!(restored.sessions().all(|s| s.creator().is_none()));
    assert!(restored.block(a).is_some());
}
