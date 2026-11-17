//! A server window's floats (C42) as the GUI window's: panes by key, each
//! placed in whole percent of the body.

use rusty_mux_core::{Float, FloatRect};
use rusty_term::gui::source::{Float as Pane, FloatAt};

use crate::keys::Keys;

/// Where a block is floated at first: the middle of the window, as the
/// TUI does (handoff D325).
pub fn first() -> Option<FloatRect> {
    FloatRect::new(20, 20, 60, 60).ok()
}

/// `rect` as the window's place.
pub fn place(rect: FloatRect) -> FloatAt {
    FloatAt {
        x: rect.x(),
        y: rect.y(),
        w: rect.w(),
        h: rect.h(),
    }
}

/// The server's rect for the window's `at`; `None` when it is under the
/// least size or outside the window, which the server refuses.
pub fn rect(at: FloatAt) -> Option<FloatRect> {
    FloatRect::new(at.x, at.y, at.w, at.h).ok()
}

/// `floats`, back to front, named by `keys`.
pub fn panes(floats: &[Float], keys: &mut Keys) -> Vec<Pane> {
    floats
        .iter()
        .map(|f| Pane {
            key: keys.key(f.block),
            at: place(f.at),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusty_mux_core::BlockId;
    use rusty_uuid::Uuid;

    fn float(block: BlockId, x: u8, y: u8, w: u8, h: u8) -> Float {
        Float {
            block,
            at: FloatRect::new(x, y, w, h).expect("a float"),
        }
    }

    #[test]
    fn a_place_goes_both_ways() {
        let at = place(FloatRect::new(5, 10, 40, 30).expect("a float"));
        assert_eq!(
            at,
            FloatAt {
                x: 5,
                y: 10,
                w: 40,
                h: 30
            }
        );
        assert_eq!(rect(at), FloatRect::new(5, 10, 40, 30).ok());
    }

    #[test]
    fn a_place_the_server_would_refuse_is_none() {
        assert_eq!(
            rect(FloatAt {
                x: 0,
                y: 0,
                w: 9,
                h: 50
            }),
            None,
            "too narrow"
        );
        assert_eq!(
            rect(FloatAt {
                x: 60,
                y: 0,
                w: 50,
                h: 50
            }),
            None,
            "outside"
        );
    }

    #[test]
    fn the_first_place_is_the_middle() {
        let first = first().expect("a float");
        assert_eq!(
            (first.x(), first.y(), first.w(), first.h()),
            (20, 20, 60, 60)
        );
    }

    #[test]
    fn floats_keep_their_order_and_a_block_its_key() {
        let [a, b] = [(); 2].map(|_| BlockId::from_uuid(Uuid::new_v4()));
        let mut keys = Keys::default();
        let ka = keys.key(a);
        let got = panes(
            &[float(b, 0, 0, 50, 50), float(a, 10, 10, 20, 20)],
            &mut keys,
        );
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].key, keys.key(b));
        assert_eq!(got[1].key, ka);
        assert_eq!(
            got[1].at,
            FloatAt {
                x: 10,
                y: 10,
                w: 20,
                h: 20
            }
        );
    }
}
