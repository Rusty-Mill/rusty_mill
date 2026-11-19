//! The session overview of an attached window (gui-design D97, handoff
//! D336): the session's blocks as tiles, one row per card, so one look
//! shows who works what.
//!
//! Pure: [`Overview`] keeps the cards and the selected tile, answers keys
//! and clicks with an [`Out`] for the window to carry out, and keeps the
//! selection by the tile's pane key when the cards change.
//! [`super::overview_view`] draws it.

use super::blocks::{Glyph, Ink};
use super::source::OverviewTile;

/// A tile with what the window knows of its block besides the source's
/// tell: the lead glyph and state word of its header (D48), and whether
/// it has the focus.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Tile {
    pub src: OverviewTile,
    pub lead: Option<(Glyph, Ink)>,
    pub state: &'static str,
    pub focused: bool,
}

/// A row: the blocks worked for one card, or with none.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Card {
    pub card: Option<String>,
    pub tiles: Vec<Tile>,
}

/// A key the overview answers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Key {
    Up,
    Down,
    Left,
    Right,
    Enter,
    Esc,
}

/// What the window does next.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Out {
    Stay,
    Close,
    /// Close, then show the block: focus it when it is in its window's
    /// layout, and show its window first (D97).
    Open {
        key: u64,
        window: u64,
        detached: bool,
    },
}

/// An open overview.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Overview {
    cards: Vec<Card>,
    selected: Option<u64>,
    /// The link is up: with it down the tiles can't be opened (D62).
    online: bool,
}

impl Overview {
    /// Open on the focused block's tile, else the first.
    pub(crate) fn open(cards: Vec<Card>, online: bool) -> Self {
        let selected = cards
            .iter()
            .flat_map(|c| &c.tiles)
            .find(|t| t.focused)
            .or_else(|| cards.iter().flat_map(|c| &c.tiles).next())
            .map(|t| t.src.key);
        Self {
            cards,
            selected,
            online,
        }
    }

    /// The cards' new state; the selection stays on its block, else falls
    /// to the first tile (D60: kept by id).
    pub(crate) fn update(&mut self, cards: Vec<Card>, online: bool) {
        self.cards = cards;
        self.online = online;
        let has = |k: u64| self.tiles().any(|t| t.src.key == k);
        if !self.selected.is_some_and(has) {
            let first = self.tiles().next().map(|t| t.src.key);
            self.selected = first;
        }
    }

    pub(crate) fn cards(&self) -> &[Card] {
        &self.cards
    }

    pub(crate) fn selected(&self) -> Option<u64> {
        self.selected
    }

    pub(crate) fn online(&self) -> bool {
        self.online
    }

    fn tiles(&self) -> impl Iterator<Item = &Tile> {
        self.cards.iter().flat_map(|c| &c.tiles)
    }

    /// The selected tile's row and place in it.
    fn at(&self) -> Option<(usize, usize)> {
        let key = self.selected?;
        self.cards.iter().enumerate().find_map(|(r, c)| {
            c.tiles
                .iter()
                .position(|t| t.src.key == key)
                .map(|i| (r, i))
        })
    }

    pub(crate) fn key(&mut self, key: Key) -> Out {
        match key {
            Key::Esc => Out::Close,
            Key::Enter => self.selected.map_or(Out::Stay, |k| self.click(k)),
            Key::Left | Key::Right => {
                self.step_tile(key == Key::Right);
                Out::Stay
            }
            Key::Up | Key::Down => {
                self.step_row(key == Key::Down);
                Out::Stay
            }
        }
    }

    /// A click on, or Enter over, the tile of pane `key`.
    pub(crate) fn click(&mut self, key: u64) -> Out {
        let Some(tile) = self.tiles().find(|t| t.src.key == key) else {
            return Out::Stay;
        };
        let (window, detached) = (tile.src.window, tile.src.detached);
        self.selected = Some(key);
        if !self.online {
            return Out::Stay;
        }
        Out::Open {
            key,
            window,
            detached,
        }
    }

    /// Next or previous tile in its row, wrapping.
    fn step_tile(&mut self, forward: bool) {
        let Some((r, i)) = self.at() else {
            return;
        };
        let n = self.cards[r].tiles.len();
        let i = if forward {
            (i + 1) % n
        } else {
            (i + n - 1) % n
        };
        self.selected = Some(self.cards[r].tiles[i].src.key);
    }

    /// The next or previous row, wrapping, at the same place or its last.
    fn step_row(&mut self, forward: bool) {
        let Some((r, i)) = self.at() else {
            return;
        };
        let n = self.cards.len();
        let r = if forward {
            (r + 1) % n
        } else {
            (r + n - 1) % n
        };
        // A card always has a tile today; one with none keeps the selection.
        let Some(last) = self.cards[r].tiles.len().checked_sub(1) else {
            return;
        };
        self.selected = Some(self.cards[r].tiles[i.min(last)].src.key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tile(key: u64, window: u64) -> Tile {
        Tile {
            src: OverviewTile {
                key,
                window,
                name: format!("agent{key}"),
                role: "builder".into(),
                exited: false,
                detached: false,
                holder: None,
            },
            lead: None,
            state: "Idle",
            focused: false,
        }
    }

    fn cards() -> Vec<Card> {
        vec![
            Card {
                card: Some("#1".into()),
                tiles: vec![tile(1, 10), tile(2, 10), tile(3, 11)],
            },
            Card {
                card: None,
                tiles: vec![tile(4, 10)],
            },
        ]
    }

    #[test]
    fn it_opens_on_the_focused_tile_else_the_first() {
        assert_eq!(Overview::open(cards(), true).selected(), Some(1));
        let mut c = cards();
        c[1].tiles[0].focused = true;
        assert_eq!(Overview::open(c, true).selected(), Some(4));
        assert_eq!(Overview::open(Vec::new(), true).selected(), None);
    }

    #[test]
    fn a_row_with_no_tiles_keeps_the_selection() {
        let mut c = cards();
        c.push(Card {
            card: Some("#2".into()),
            tiles: Vec::new(),
        });
        let mut o = Overview::open(c, true);
        o.key(Key::Up);
        assert_eq!(o.selected(), Some(1), "an empty row is not stepped into");
    }

    #[test]
    fn arrows_move_between_tiles_and_rows_and_wrap() {
        let mut o = Overview::open(cards(), true);
        o.key(Key::Right);
        o.key(Key::Right);
        assert_eq!(o.selected(), Some(3));
        o.key(Key::Right);
        assert_eq!(o.selected(), Some(1), "a row wraps");
        o.key(Key::Left);
        o.key(Key::Down);
        assert_eq!(o.selected(), Some(4), "a shorter row keeps its last tile");
        o.key(Key::Down);
        assert_eq!(o.selected(), Some(1), "rows wrap, at the same place");
        o.key(Key::Up);
        assert_eq!(o.selected(), Some(4));
    }

    #[test]
    fn enter_opens_the_tile_and_esc_closes() {
        let mut o = Overview::open(cards(), true);
        o.key(Key::Right);
        o.key(Key::Right);
        assert_eq!(
            o.key(Key::Enter),
            Out::Open {
                key: 3,
                window: 11,
                detached: false
            }
        );
        assert_eq!(o.key(Key::Esc), Out::Close);
    }

    #[test]
    fn a_tile_does_not_open_while_the_link_is_down() {
        let mut o = Overview::open(cards(), false);
        assert_eq!(o.click(2), Out::Stay);
        assert_eq!(o.selected(), Some(2));
        assert_eq!(o.key(Key::Enter), Out::Stay);
    }

    #[test]
    fn the_selection_follows_its_block_through_an_update() {
        let mut o = Overview::open(cards(), true);
        o.click(2);
        let mut c = cards();
        c[0].tiles.remove(0);
        o.update(c.clone(), true);
        assert_eq!(o.selected(), Some(2));
        c[0].tiles.remove(0);
        o.update(c, true);
        assert_eq!(o.selected(), Some(3), "a tile that left falls to the first");
        o.update(Vec::new(), true);
        assert_eq!(o.selected(), None);
        assert_eq!(o.key(Key::Enter), Out::Stay);
        o.key(Key::Down);
    }
}
