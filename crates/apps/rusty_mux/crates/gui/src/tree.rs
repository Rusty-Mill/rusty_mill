//! A server window's layout as the GUI window's pane tree.

use rusty_mux_core::{Axis, Layout};
use rusty_term::gui::source::{Dir, Tree};

use crate::keys::Keys;

/// `layout` with its blocks named by `keys`.
pub fn tree(layout: &Layout, keys: &mut Keys) -> Tree {
    match layout {
        Layout::Block(block) => Tree::Pane(keys.key(*block)),
        Layout::Split {
            axis,
            share,
            first,
            second,
        } => Tree::Split {
            dir: match axis {
                Axis::Columns => Dir::Vertical,
                Axis::Rows => Dir::Horizontal,
            },
            ratio: f32::from(share.percent()) / 100.0,
            a: Box::new(tree(first, keys)),
            b: Box::new(tree(second, keys)),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::{Keys, tree};
    use rusty_mux_core::{Axis, BlockId, Layout, Share};
    use rusty_term::gui::source::{Dir, Tree};
    use rusty_uuid::Uuid;

    #[test]
    fn splits_keep_their_axis_share_and_order() {
        let [a, b, c] = [(); 3].map(|_| BlockId::from_uuid(Uuid::new_v4()));
        let layout = Layout::Split {
            axis: Axis::Columns,
            share: Share::new(30).expect("a share"),
            first: Box::new(Layout::Block(a)),
            second: Box::new(Layout::Split {
                axis: Axis::Rows,
                share: Share::HALF,
                first: Box::new(Layout::Block(b)),
                second: Box::new(Layout::Block(c)),
            }),
        };
        let mut keys = Keys::default();
        let got = tree(&layout, &mut keys);
        let [ka, kb, kc] = [a, b, c].map(|x| keys.key(x));
        assert_eq!(
            got,
            Tree::Split {
                dir: Dir::Vertical,
                ratio: 0.3,
                a: Box::new(Tree::Pane(ka)),
                b: Box::new(Tree::Split {
                    dir: Dir::Horizontal,
                    ratio: 0.5,
                    a: Box::new(Tree::Pane(kb)),
                    b: Box::new(Tree::Pane(kc)),
                }),
            }
        );
    }
}
