//! The shared fixture tree.

/// Contents of `/hello.txt`.
pub const HELLO: &[u8] = b"hello, world\n";

/// Length of `/dir/big.bin`: several read buffers long and not a power of two,
/// so ranged and short reads are both exercised.
pub const BIG_LEN: usize = 70_001;

/// One item of the fixture, by path relative to the root with `/` separators.
/// Parents always come before children.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FixtureItem {
    /// A directory.
    Dir(&'static str),
    /// A file and its contents.
    File(&'static str, Vec<u8>),
}

/// The deterministic contents of `/dir/big.bin`.
#[must_use]
pub fn big_contents() -> Vec<u8> {
    (0..BIG_LEN).map(|i| (i % 251) as u8).collect()
}

/// The fixture every backend is loaded with:
///
/// ```text
/// /hello.txt        HELLO
/// /dir/             directory
/// /dir/a.txt        "a"
/// /dir/big.bin      big_contents()
/// /dir/empty/       empty directory
/// ```
#[must_use]
pub fn fixture() -> Vec<FixtureItem> {
    vec![
        FixtureItem::File("hello.txt", HELLO.to_vec()),
        FixtureItem::Dir("dir"),
        FixtureItem::File("dir/a.txt", b"a".to_vec()),
        FixtureItem::File("dir/big.bin", big_contents()),
        FixtureItem::Dir("dir/empty"),
    ]
}
