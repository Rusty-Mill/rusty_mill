#![allow(clippy::expect_used, clippy::unwrap_used)]
//! #447: `bbp` accepts the receipt bytes the board signs. The fixture is pinned on the board side
//! by `rusty_mill_board` `receipts::format`, so a change to either side's format fails a test.

use rusty_bbp_host::approval;

#[test]
fn the_boards_receipt_verifies_and_parses() {
    let file = include_bytes!("fixtures/board-receipt.txt");
    let board = rusty_pk::ed25519::SigningKey::from_seed(&[5; 32]).expect("key");
    let v = approval::verify(board.public_key(), file).expect("verifies");
    assert_eq!(
        (
            v.receipt.task.as_str(),
            v.receipt.gate.as_str(),
            v.receipt.subject
        ),
        ("orch-abc", "merge", 4)
    );
    assert_eq!((v.receipt.run, v.receipt.fence), (Some(1), 17));
    assert_eq!(v.receipt.approver, "nano");
    assert!(approval::op(&v).0.starts_with("by:nano/rcpt:"));
}
