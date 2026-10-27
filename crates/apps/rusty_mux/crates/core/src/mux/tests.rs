use super::*;

fn size(cols: u16, rows: u16) -> Size {
    Size::new(cols, rows).expect("non-zero size")
}

/// A session with one window and one block, events drained.
fn fixture() -> (Mux, SessionId, WindowId, BlockId) {
    let mut mux = Mux::new();
    let session = mux.create_session("s");
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
    let session = mux.create_session("work");
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
    let ids: Vec<SessionId> = (0..8).map(|n| mux.create_session(n.to_string())).collect();
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
    assert_eq!(mux.take_events().len(), 1, "the refused add emits nothing");
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
