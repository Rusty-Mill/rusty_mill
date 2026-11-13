use super::*;

fn lookups(text: &str) -> Result<Allow, String> {
    Allow::parse(
        text,
        |user| match user {
            "alice" => Some(1001),
            "root" => Some(0),
            _ => None,
        },
        |uid| match uid {
            0 => Some("root".into()),
            1001 => Some("alice".into()),
            _ => None,
        },
    )
}

fn alice() -> Principal {
    Principal::unix(1001, Some("alice".into())).expect("principal")
}

#[test]
fn an_empty_list_names_nobody() {
    let allow = lookups("# nobody yet\n\n").expect("parses");
    assert_eq!(allow, Allow::default());
    assert!(matches!(admit(1000, 1001, &allow), Peer::Refused(_)));
}

#[test]
fn a_listed_user_is_served_as_its_own_principal() {
    let allow = lookups("uid:1001  # by number\nuid:7\n").expect("parses");
    match admit(1000, 1001, &allow) {
        Peer::Guest(principal) => assert_eq!(principal, alice()),
        _ => panic!("not a guest"),
    }
    let named = lookups("user:alice\n").expect("parses");
    assert!(matches!(admit(1000, 1001, &named), Peer::Guest(p) if p == alice()));
    match admit(1000, 7, &allow) {
        Peer::Guest(principal) => assert_eq!(principal.id(), "uid:7"),
        _ => panic!("not a guest"),
    }
}

#[test]
fn the_servers_own_user_is_always_served_and_others_are_refused() {
    let allow = lookups("uid:1001\n").expect("parses");
    assert!(matches!(admit(1000, 1000, &allow), Peer::Allowed));
    assert!(matches!(admit(1000, 1002, &allow), Peer::Refused(who) if who == "uid 1002"));
}

#[test]
fn root_is_served_only_when_an_entry_names_it() {
    let allow = lookups("uid:1001\n").expect("parses");
    assert!(matches!(admit(1000, 0, &allow), Peer::Refused(_)));
    let named = lookups("user:root\n").expect("parses");
    assert!(matches!(admit(1000, 0, &named), Peer::Guest(_)));
}

#[test]
fn bad_lines_refuse_the_whole_list() {
    for (text, why) in [
        ("*\n", "no wildcard"),
        ("uid:1001 extra\n", "not a uid"),
        ("uid:x\n", "not a uid"),
        ("user:nobody-here\n", "no account"),
        ("uid:1001\nuser:alice\n", "listed already"),
        ("everyone\n", "no wildcard"),
    ] {
        let err = lookups(text).expect_err(text);
        assert!(err.contains(why), "{text:?}: {err}");
    }
    assert!(
        lookups("uid:1\nuid:x\n")
            .expect_err("line")
            .starts_with("line 2")
    );
}

#[test]
fn a_reload_closes_only_the_connections_of_principals_it_no_longer_names() {
    let guests = Guests::default();
    let kept = Rc::new(Notify::new());
    let revoked = Rc::new(Notify::new());
    let bob = Principal::unix(1002, Some("bob".into())).expect("principal");
    guests.add(alice(), &kept);
    guests.add(bob, &revoked);
    guests.revoke_unlisted(&lookups("uid:1001\n").expect("parses"));
    let runtime = rusty_tokio::Builder::new_current_thread()
        .build_local(&rusty_tokio::LocalOptions::default())
        .expect("runtime");
    runtime.block_on(async {
        let closed =
            rusty_tokio::time::timeout(std::time::Duration::from_secs(5), revoked.notified()).await;
        assert!(closed.is_ok(), "bob was not closed");
        let open =
            rusty_tokio::time::timeout(std::time::Duration::from_millis(100), kept.notified())
                .await;
        assert!(open.is_err(), "alice was closed");
    });
}

#[test]
fn a_connection_that_is_gone_is_forgotten() {
    let guests = Guests::default();
    let gone = Rc::new(Notify::new());
    guests.add(alice(), &gone);
    drop(gone);
    guests.add(alice(), &Rc::new(Notify::new()));
    assert_eq!(guests.open.borrow().len(), 1);
}

#[test]
fn a_guest_revoked_before_it_registers_is_closed_when_it_does() {
    // It connected, held back `Hello`, and was revoked meanwhile.
    let guests = Guests::default();
    guests.revoke_unlisted(&lookups("uid:1002\n").expect("parses"));
    let late = Rc::new(Notify::new());
    guests.add(alice(), &late);
    let runtime = rusty_tokio::Builder::new_current_thread()
        .build_local(&rusty_tokio::LocalOptions::default())
        .expect("runtime");
    runtime.block_on(async {
        let closed =
            rusty_tokio::time::timeout(std::time::Duration::from_secs(5), late.notified()).await;
        assert!(closed.is_ok(), "alice was served after her revoke");
    });
    assert!(guests.open.borrow().is_empty());
}
