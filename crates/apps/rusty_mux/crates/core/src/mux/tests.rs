use super::*;

fn size(cols: u16, rows: u16) -> Size {
    Size::new(cols, rows).expect("non-zero size")
}

/// A session with one window and one block, events drained.
fn fixture() -> (Mux, SessionId, WindowId, BlockId) {
    let mut mux = Mux::new();
    let session = mux.create_session("s").expect("session");
    let window = mux.create_window(session, "w").expect("window");
    let block = mux.create_block(window, size(80, 24)).expect("block");
    mux.take_events();
    (mux, session, window, block)
}

fn leased_client(mux: &mut Mux, block: BlockId) -> ClientId {
    let client = mux.connect();
    mux.attach(client, block).expect("attach");
    mux.acquire_lease(client, block).expect("lease");
    mux.take_events();
    client
}

#[test]
fn create_links_children_and_emits_in_order() {
    let mut mux = Mux::new();
    let session = mux.create_session("work").unwrap();
    let window = mux.create_window(session, "main").unwrap();
    let block = mux.create_block(window, size(80, 24)).unwrap();

    assert_eq!(mux.session(session).unwrap().windows(), &[window]);
    assert_eq!(mux.session(session).unwrap().name(), "work");
    assert_eq!(mux.window(window).unwrap().blocks(), &[block]);
    assert_eq!(mux.block(block).unwrap().size(), size(80, 24));
    assert_eq!(
        mux.take_events(),
        vec![
            Event::SessionCreated { session },
            Event::WindowCreated { window, session },
            Event::BlockCreated {
                block,
                window,
                size: size(80, 24)
            },
            Event::LayoutChanged {
                window,
                layout: Some(Layout::Block(block)),
                zoomed: None,
            },
        ]
    );
    assert!(mux.take_events().is_empty());
}

#[test]
fn zero_size_is_rejected() {
    assert_eq!(Size::new(0, 24), Err(Error::ZeroSize { cols: 0, rows: 24 }));
    assert_eq!(Size::new(80, 0), Err(Error::ZeroSize { cols: 80, rows: 0 }));
}

#[test]
fn unknown_parents_are_rejected_without_events() {
    let (mut mux, session, window, _) = fixture();
    mux.close_session(session).unwrap();
    mux.take_events();

    assert_eq!(
        mux.create_window(session, "w"),
        Err(Error::UnknownSession(session))
    );
    assert_eq!(
        mux.create_block(window, size(1, 1)),
        Err(Error::UnknownWindow(window))
    );
    assert!(mux.take_events().is_empty());
}

#[test]
fn lease_is_single_writer() {
    let (mut mux, _, _, block) = fixture();
    let a = leased_client(&mut mux, block);
    let b = mux.connect();
    mux.attach(b, block).unwrap();

    assert_eq!(
        mux.acquire_lease(b, block),
        Err(Error::LeaseHeld { block, holder: a })
    );
    assert_eq!(
        mux.authorize_input(b, block),
        Err(Error::NoLease { client: b, block })
    );
    assert_eq!(mux.authorize_input(a, block), Ok(()));
    assert_eq!(mux.block(block).unwrap().lease(), Some(a));
}

#[test]
fn lease_requires_attachment() {
    let (mut mux, _, _, block) = fixture();
    let client = mux.connect();
    assert_eq!(
        mux.acquire_lease(client, block),
        Err(Error::NotAttached { client, block })
    );
    assert_eq!(
        mux.authorize_input(client, block),
        Err(Error::NotAttached { client, block })
    );
}

#[test]
fn acquiring_own_lease_twice_is_a_no_op() {
    let (mut mux, _, _, block) = fixture();
    let client = leased_client(&mut mux, block);
    mux.acquire_lease(client, block).unwrap();
    assert!(mux.take_events().is_empty());
}

#[test]
fn release_frees_the_lease_for_another_client() {
    let (mut mux, _, _, block) = fixture();
    let a = leased_client(&mut mux, block);
    let b = mux.connect();
    mux.attach(b, block).unwrap();
    mux.take_events();

    mux.release_lease(a, block).unwrap();
    assert_eq!(
        mux.take_events(),
        vec![Event::LeaseChanged {
            block,
            holder: None
        }]
    );
    assert_eq!(
        mux.release_lease(a, block),
        Err(Error::NoLease { client: a, block })
    );
    mux.acquire_lease(b, block).unwrap();
    assert_eq!(mux.block(block).unwrap().lease(), Some(b));
}

/// A client giving back a lease it no longer holds is refused, and the
/// holder keeps it: a TUI releasing after a takeover cannot disturb it.
#[test]
fn releasing_a_lease_held_by_another_client_leaves_it() {
    let (mut mux, _, _, block) = fixture();
    let a = mux.connect();
    mux.attach(a, block).unwrap();
    let b = leased_client(&mut mux, block);
    mux.take_events();

    assert_eq!(
        mux.release_lease(a, block),
        Err(Error::NoLease { client: a, block })
    );
    assert_eq!(mux.block(block).unwrap().lease(), Some(b));
    assert!(mux.take_events().is_empty());
}

#[test]
fn reattach_to_another_block_drops_the_old_lease() {
    let (mut mux, _, window, first) = fixture();
    let second = mux.create_block(window, size(80, 24)).unwrap();
    let client = leased_client(&mut mux, first);
    mux.take_events();

    mux.attach(client, second).unwrap();
    assert_eq!(
        mux.take_events(),
        vec![
            Event::LeaseChanged {
                block: first,
                holder: None
            },
            Event::Detached {
                client,
                block: first
            },
            Event::Attached {
                client,
                block: second
            },
        ]
    );
    assert_eq!(mux.block(first).unwrap().lease(), None);
    assert_eq!(mux.client(client).unwrap().attached(), Some(second));
}

#[test]
fn attach_to_same_block_is_a_no_op() {
    let (mut mux, _, _, block) = fixture();
    let client = leased_client(&mut mux, block);
    mux.attach(client, block).unwrap();
    assert!(mux.take_events().is_empty());
    assert_eq!(mux.block(block).unwrap().lease(), Some(client));
}

#[test]
fn attach_to_unknown_block_keeps_the_current_attachment() {
    let (mut mux, _, window, block) = fixture();
    let gone = mux.create_block(window, size(80, 24)).unwrap();
    mux.close_block(gone).unwrap();
    let client = leased_client(&mut mux, block);

    assert_eq!(mux.attach(client, gone), Err(Error::UnknownBlock(gone)));
    assert_eq!(mux.client(client).unwrap().attached(), Some(block));
    assert_eq!(mux.block(block).unwrap().lease(), Some(client));
    assert!(mux.take_events().is_empty());
}

#[test]
fn disconnect_releases_lease_and_forgets_client() {
    let (mut mux, _, _, block) = fixture();
    let client = leased_client(&mut mux, block);

    mux.disconnect(client).unwrap();
    assert_eq!(
        mux.take_events(),
        vec![
            Event::LeaseChanged {
                block,
                holder: None
            },
            Event::Detached { client, block },
            Event::ClientDisconnected { client },
        ]
    );
    assert!(mux.client(client).is_none());
    assert_eq!(mux.disconnect(client), Err(Error::UnknownClient(client)));
}

#[test]
fn resize_needs_the_lease_and_skips_same_size() {
    let (mut mux, _, _, block) = fixture();
    let viewer = mux.connect();
    mux.attach(viewer, block).unwrap();
    assert_eq!(
        mux.resize_block(viewer, block, size(100, 30)),
        Err(Error::NoLease {
            client: viewer,
            block
        })
    );

    let writer = leased_client(&mut mux, block);
    mux.resize_block(writer, block, size(80, 24)).unwrap();
    assert!(mux.take_events().is_empty());
    mux.resize_block(writer, block, size(100, 30)).unwrap();
    assert_eq!(
        mux.take_events(),
        vec![Event::BlockResized {
            block,
            size: size(100, 30)
        }]
    );
    assert_eq!(mux.block(block).unwrap().size(), size(100, 30));
}

#[test]
fn close_session_cascades_children_first() {
    let (mut mux, session, window, block) = fixture();
    let client = leased_client(&mut mux, block);

    mux.close_session(session).unwrap();
    assert_eq!(
        mux.take_events(),
        vec![
            Event::LeaseChanged {
                block,
                holder: None
            },
            Event::Detached { client, block },
            Event::BlockClosed { block },
            Event::WindowClosed { window },
            Event::SessionClosed { session },
        ]
    );
    assert!(mux.session(session).is_none());
    assert!(mux.window(window).is_none());
    assert!(mux.block(block).is_none());
    assert_eq!(mux.client(client).unwrap().attached(), None);
    assert_eq!(mux.sessions().count(), 0);
}

#[test]
fn close_window_and_block_unlink_from_parents() {
    let (mut mux, session, window, block) = fixture();
    let sibling = mux.create_window(session, "w2").unwrap();

    mux.close_block(block).unwrap();
    assert!(mux.window(window).unwrap().blocks().is_empty());
    mux.close_window(window).unwrap();
    assert_eq!(mux.session(session).unwrap().windows(), &[sibling]);

    assert_eq!(mux.close_block(block), Err(Error::UnknownBlock(block)));
    assert_eq!(mux.close_window(window), Err(Error::UnknownWindow(window)));
}

#[test]
fn ids_round_trip_through_uuid() {
    let (_, _, _, block) = fixture();
    assert_eq!(BlockId::from_uuid(block.uuid()), block);
    assert_eq!(block.to_string(), block.uuid().to_string());
}

#[test]
fn errors_display_their_ids() {
    let (_, _, _, block) = fixture();
    let text = Error::UnknownBlock(block).to_string();
    assert!(text.contains(&block.to_string()), "{text}");
}

#[test]
fn sessions_list_in_creation_order() {
    let mut mux = Mux::new();
    let ids: Vec<SessionId> = (0..8)
        .map(|n| mux.create_session(n.to_string()).unwrap())
        .collect();
    mux.close_session(ids[3]).unwrap();
    let listed: Vec<SessionId> = mux.sessions().map(Session::id).collect();
    let expected: Vec<SessionId> = ids.iter().copied().filter(|id| *id != ids[3]).collect();
    assert_eq!(listed, expected);
}

#[test]
fn a_block_can_take_a_chosen_id_once() {
    let (mut mux, _, window, _) = fixture();
    mux.take_events();
    let id = BlockId::from_uuid(rusty_uuid::Uuid::new_v4());
    mux.add_block(id, window, size(80, 24)).unwrap();
    assert_eq!(mux.block(id).map(Block::window), Some(window));
    assert_eq!(
        mux.add_block(id, window, size(80, 24)),
        Err(Error::BlockExists(id))
    );
    assert_eq!(
        mux.take_events().len(),
        2,
        "a create and a layout; the refused add emits nothing"
    );
}

#[test]
fn status_notes_need_a_live_block() {
    let (mut mux, _, _, block) = fixture();
    assert_eq!(mux.note_status(block), Ok(()));
    assert_eq!(mux.take_events(), vec![Event::BlockStatus { block }]);
    mux.close_block(block).expect("close");
    mux.take_events();
    assert_eq!(mux.note_status(block), Err(Error::UnknownBlock(block)));
    assert_eq!(mux.note_pwd(block), Err(Error::UnknownBlock(block)));
    assert_eq!(mux.note_title(block), Err(Error::UnknownBlock(block)));
    let gone = Err(Error::UnknownBlock(block));
    assert_eq!(mux.note_exit(block, ExitStatus::Code(0)), gone);
    assert_eq!(mux.note_bell(block), gone);
    assert_eq!(mux.note_progress(block, Progress::Cleared), gone);
    assert_eq!(
        mux.note_notification(block, String::new(), "b".into()),
        gone
    );
    assert_eq!(mux.note_process(block, 7, "vim".into()), gone);
    assert!(mux.take_events().is_empty());
}

#[test]
fn pwd_and_title_notes_emit_their_events() {
    let (mut mux, _, _, block) = fixture();
    mux.take_events();
    assert_eq!(mux.note_pwd(block), Ok(()));
    assert_eq!(mux.note_title(block), Ok(()));
    assert_eq!(
        mux.take_events(),
        vec![Event::PwdChanged { block }, Event::TitleChanged { block }]
    );
}

#[test]
fn program_notes_emit_their_events_in_order() {
    let (mut mux, _, _, block) = fixture();
    mux.take_events();
    assert_eq!(mux.note_bell(block), Ok(()));
    assert_eq!(mux.note_progress(block, Progress::Normal(40)), Ok(()));
    assert_eq!(mux.note_notification(block, "t".into(), "b".into()), Ok(()));
    assert_eq!(mux.note_exit(block, ExitStatus::Signal(9)), Ok(()));
    assert_eq!(
        mux.take_events(),
        vec![
            Event::Bell { block },
            Event::ProgressChanged {
                block,
                progress: Progress::Normal(40)
            },
            Event::Notification {
                block,
                title: "t".into(),
                body: "b".into()
            },
            Event::ProgramExited {
                block,
                status: ExitStatus::Signal(9)
            },
        ]
    );
}

#[test]
fn process_notes_emit_their_event() {
    let (mut mux, _, _, block) = fixture();
    mux.take_events();
    assert_eq!(mux.note_process(block, 42, "vim".into()), Ok(()));
    assert_eq!(
        mux.take_events(),
        vec![Event::ProcessChanged {
            block,
            pid: 42,
            name: "vim".into()
        }]
    );
}

#[test]
fn rename_names_every_kind_and_emits_once() {
    let (mut mux, session, window, block) = fixture();
    let client = mux.connect();
    mux.take_events();
    mux.rename(EntityId::Session(session), "work").unwrap();
    mux.rename(EntityId::Window(window), "logs").unwrap();
    mux.rename(EntityId::Block(block), "build").unwrap();
    mux.rename(EntityId::Client(client), "laptop").unwrap();
    // The same name again changes nothing.
    mux.rename(EntityId::Block(block), "build").unwrap();

    assert_eq!(mux.session(session).unwrap().name(), "work");
    assert_eq!(mux.window(window).unwrap().name(), "logs");
    assert_eq!(mux.block(block).unwrap().name(), "build");
    assert_eq!(mux.client(client).unwrap().name(), "laptop");
    assert_eq!(
        mux.take_events(),
        vec![
            Event::SessionRenamed {
                session,
                name: "work".into()
            },
            Event::WindowRenamed {
                window,
                name: "logs".into()
            },
            Event::BlockRenamed {
                block,
                name: "build".into()
            },
            Event::ClientRenamed {
                client,
                name: "laptop".into()
            },
        ]
    );
}

#[test]
fn new_blocks_and_clients_have_empty_names() {
    let (mut mux, _, _, block) = fixture();
    let client = mux.connect();
    assert_eq!(mux.block(block).unwrap().name(), "");
    assert_eq!(mux.client(client).unwrap().name(), "");
    assert_eq!(mux.clients().map(Client::id).collect::<Vec<_>>(), [client]);
}

#[test]
fn renaming_an_unknown_entity_changes_nothing() {
    let (mut mux, _, window, block) = fixture();
    let client = mux.connect();
    mux.close_window(window).unwrap();
    mux.disconnect(client).unwrap();
    mux.take_events();
    assert_eq!(
        mux.rename(EntityId::Block(block), "x"),
        Err(Error::UnknownBlock(block))
    );
    assert_eq!(
        mux.rename(EntityId::Client(client), "x"),
        Err(Error::UnknownClient(client))
    );
    assert!(mux.take_events().is_empty());
}

#[test]
fn names_over_the_limit_are_refused_without_a_change() {
    let (mut mux, session, window, block) = fixture();
    let client = mux.connect();
    mux.rename(EntityId::Block(block), "build").unwrap();
    mux.take_events();
    let at_limit = "a".repeat(MAX_NAME);
    let over = "a".repeat(MAX_NAME + 1);
    let error = Error::NameTooLong { len: MAX_NAME + 1 };
    let too_long = Err(error.clone());
    for id in [
        EntityId::Session(session),
        EntityId::Window(window),
        EntityId::Block(block),
        EntityId::Client(client),
    ] {
        assert_eq!(mux.rename(id, over.clone()), too_long, "{id:?}");
    }
    assert_eq!(mux.create_session(over.clone()), Err(error.clone()));
    assert_eq!(mux.create_window(session, over), Err(error));
    assert_eq!(mux.block(block).unwrap().name(), "build");
    assert_eq!(mux.sessions().count(), 1);
    assert!(mux.take_events().is_empty());
    // Exactly the limit is a name like any other.
    mux.rename(EntityId::Block(block), at_limit.clone())
        .unwrap();
    assert_eq!(mux.block(block).unwrap().name(), at_limit);
}

/// The layout of `window`.
fn layout_of(mux: &Mux, window: WindowId) -> Option<Layout> {
    mux.window(window).and_then(|w| w.layout().cloned())
}

#[test]
fn new_blocks_split_the_largest_block_by_its_shape() {
    let (mut mux, _, window, a) = fixture();
    // 80x24 is wider than tall: side by side.
    let b = mux.create_block(window, size(80, 24)).unwrap();
    // `a` and `b` are equal, so `a` splits; at 20x24 it is taller than wide.
    let client = leased_client(&mut mux, a);
    mux.resize_block(client, a, size(20, 24)).unwrap();
    let c = mux.create_block(window, size(20, 12)).unwrap();
    let expected = Layout::Block(a)
        .split(a, b, Side::Right)
        .split(a, c, Side::Down);
    assert_eq!(layout_of(&mux, window), Some(expected));
    assert_eq!(mux.window(window).unwrap().blocks(), &[a, b, c]);
}

#[test]
fn split_places_the_block_beside_its_target_and_unzooms() {
    let (mut mux, _, window, a) = fixture();
    mux.zoom(a).unwrap();
    mux.take_events();
    let new = BlockId::from_uuid(rusty_uuid::Uuid::new_v4());
    let size = mux.split_size(a, Side::Left).unwrap();
    assert_eq!(size, Size::new(40, 24).unwrap());
    mux.split_block(new, a, Side::Left, size).unwrap();
    let layout = Layout::Block(a).split(a, new, Side::Left);
    assert_eq!(
        mux.take_events(),
        vec![
            Event::BlockCreated {
                block: new,
                window,
                size
            },
            Event::LayoutChanged {
                window,
                layout: Some(layout),
                zoomed: None
            },
        ]
    );
    assert_eq!(
        mux.split_size(a, Side::Down),
        Ok(Size::new(80, 11).unwrap())
    );
}

#[test]
fn a_refused_split_changes_nothing() {
    let (mut mux, _, window, a) = fixture();
    let client = leased_client(&mut mux, a);
    mux.resize_block(client, a, size(2, 24)).unwrap();
    mux.take_events();
    let no_room = Err(Error::NoRoom {
        block: a,
        side: Side::Right,
    });
    assert_eq!(mux.split_size(a, Side::Right), no_room);
    let gone = BlockId::from_uuid(rusty_uuid::Uuid::new_v4());
    assert_eq!(
        mux.split_size(gone, Side::Down),
        Err(Error::UnknownBlock(gone))
    );
    assert_eq!(
        mux.split_block(gone, gone, Side::Down, size(1, 1)),
        Err(Error::UnknownBlock(gone))
    );
    assert_eq!(
        mux.split_block(a, a, Side::Down, size(1, 1)),
        Err(Error::BlockExists(a))
    );
    assert!(mux.take_events().is_empty());
    assert_eq!(layout_of(&mux, window), Some(Layout::Block(a)));
}

#[test]
fn splits_stop_at_the_depth_limit() {
    let (mut mux, _, _, mut target) = fixture();
    for _ in 0..MAX_DEPTH {
        let new = BlockId::from_uuid(rusty_uuid::Uuid::new_v4());
        mux.split_block(new, target, Side::Down, size(80, 24))
            .unwrap();
        target = new;
    }
    assert_eq!(
        mux.split_size(target, Side::Down),
        Err(Error::TooDeep(target))
    );
    let new = BlockId::from_uuid(rusty_uuid::Uuid::new_v4());
    assert_eq!(
        mux.split_block(new, target, Side::Down, size(80, 24)),
        Err(Error::TooDeep(target))
    );
}

#[test]
fn closing_a_block_promotes_its_sibling_and_ends_its_zoom() {
    let (mut mux, _, window, a) = fixture();
    let b = mux.create_block(window, size(80, 24)).unwrap();
    mux.zoom(b).unwrap();
    mux.take_events();
    mux.close_block(b).unwrap();
    assert_eq!(
        mux.take_events(),
        vec![
            Event::BlockClosed { block: b },
            Event::LayoutChanged {
                window,
                layout: Some(Layout::Block(a)),
                zoomed: None
            },
        ]
    );
    mux.close_block(a).unwrap();
    assert_eq!(
        mux.take_events().last(),
        Some(&Event::LayoutChanged {
            window,
            layout: None,
            zoomed: None
        })
    );
    assert_eq!(layout_of(&mux, window), None);
}

#[test]
fn closing_a_window_sends_no_layout() {
    let (mut mux, _, window, _) = fixture();
    mux.close_window(window).unwrap();
    let events = mux.take_events();
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, Event::LayoutChanged { .. })),
        "{events:?}"
    );
}

#[test]
fn zoom_and_unzoom_emit_only_on_a_change() {
    let (mut mux, _, window, a) = fixture();
    mux.zoom(a).unwrap();
    mux.zoom(a).unwrap();
    let zoomed = Event::LayoutChanged {
        window,
        layout: Some(Layout::Block(a)),
        zoomed: Some(a),
    };
    assert_eq!(mux.take_events(), vec![zoomed]);
    assert_eq!(mux.window(window).unwrap().zoomed(), Some(a));
    // A new block joins the layout behind the zoom.
    let b = mux.create_block(window, size(80, 24)).unwrap();
    assert_eq!(mux.window(window).unwrap().zoomed(), Some(a));
    mux.take_events();
    mux.unzoom(window).unwrap();
    mux.unzoom(window).unwrap();
    assert_eq!(mux.take_events().len(), 1);
    let gone = BlockId::from_uuid(rusty_uuid::Uuid::new_v4());
    assert_eq!(mux.zoom(gone), Err(Error::UnknownBlock(gone)));
    mux.close_window(window).unwrap();
    assert_eq!(mux.unzoom(window), Err(Error::UnknownWindow(window)));
    assert!(mux.block(b).is_none());
}
