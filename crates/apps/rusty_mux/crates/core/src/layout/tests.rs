use rusty_uuid::Uuid;

use super::*;

fn id(n: u128) -> BlockId {
    BlockId::from_uuid(Uuid::from_bytes(n.to_be_bytes()))
}

fn size(cols: u16, rows: u16) -> Size {
    Size::new(cols, rows).expect("non-zero size")
}

/// `1` left; `2` over `3` on the right.
fn three() -> Layout {
    Layout::Block(id(1))
        .split(id(1), id(2), Side::Right)
        .split(id(2), id(3), Side::Down)
}

#[test]
fn splits_put_the_new_block_on_its_side() {
    let layout = three();
    assert_eq!(layout.blocks(), vec![id(1), id(2), id(3)]);
    let left = Layout::Block(id(1)).split(id(1), id(2), Side::Left);
    assert_eq!(left.blocks(), vec![id(2), id(1)]);
    let up = Layout::Block(id(1)).split(id(1), id(2), Side::Up);
    assert!(matches!(
        up,
        Layout::Split {
            axis: Axis::Rows,
            ..
        }
    ));
    assert_eq!(up.blocks(), vec![id(2), id(1)]);
}

#[test]
fn removing_a_block_promotes_its_sibling() {
    let layout = three().remove(id(2)).expect("two left");
    assert_eq!(layout.blocks(), vec![id(1), id(3)]);
    assert!(matches!(
        layout,
        Layout::Split {
            axis: Axis::Columns,
            ..
        }
    ));
    let one = layout.remove(id(1)).expect("one left");
    assert_eq!(one, Layout::Block(id(3)));
    assert_eq!(one.remove(id(3)), None);
}

#[test]
fn depth_counts_enclosing_splits() {
    let layout = three();
    assert_eq!(layout.depth_of(id(1)), Some(1));
    assert_eq!(layout.depth_of(id(3)), Some(2));
    assert_eq!(layout.depth_of(id(9)), None);
}

#[test]
fn the_largest_block_is_the_first_of_the_biggest_share() {
    assert_eq!(three().largest(), Some(id(1)));
    let even = Layout::Block(id(1)).split(id(1), id(2), Side::Down);
    assert_eq!(even.largest(), Some(id(1)));
}

#[test]
fn rects_tile_the_area_around_one_cell_dividers() {
    let rects = three().rects(size(81, 25));
    let at = |x, y, cols, rows| Rect {
        x,
        y,
        size: size(cols, rows),
    };
    assert_eq!(
        rects,
        vec![
            (id(1), at(0, 0, 40, 25)),
            (id(2), at(41, 0, 40, 12)),
            (id(3), at(41, 13, 40, 12)),
        ]
    );
}

#[test]
fn a_split_without_room_shows_only_its_first_child() {
    let rects = three().rects(size(81, 2));
    assert_eq!(rects.len(), 2);
    assert_eq!(rects[1].0, id(2));
    assert_eq!(rects[1].1.size, size(40, 2));
    assert_eq!(three().rects(size(2, 2)).len(), 1);
}

#[test]
fn shares_divide_lengths_around_the_divider() {
    assert_eq!(Share::HALF.lengths(81), Some((40, 40)));
    assert_eq!(Share::HALF.lengths(80), Some((40, 39)));
    assert_eq!(Share::HALF.lengths(3), Some((1, 1)));
    assert_eq!(Share::HALF.lengths(2), None);
    assert_eq!(Share::new(1).and_then(|s| s.lengths(10)), Some((1, 8)));
    assert_eq!(Share::new(99).and_then(|s| s.lengths(10)), Some((8, 1)));
    assert_eq!(Share::new(0), None);
    assert_eq!(Share::new(100), None);
}

#[test]
fn text_round_trips() {
    let layout = three();
    let text = layout.to_string();
    assert!(text.starts_with("cols50(00000000-0000-0000-0000-000000000001,rows50("));
    assert_eq!(text.parse(), Ok(layout));
    assert_eq!(id(7).to_string().parse(), Ok(Layout::Block(id(7))));
}

#[test]
fn malformed_text_is_refused() {
    let a = id(1).to_string();
    let b = id(2).to_string();
    for bad in [
        String::new(),
        "cols50()".into(),
        format!("cols50({a})"),
        format!("cols50({a},{b}"),
        format!("cols50({a},{b}))"),
        format!("cols0({a},{b})"),
        format!("cols100({a},{b})"),
        format!("cols+5({a},{b})"),
        format!("cols({a},{b})"),
        format!("diag50({a},{b})"),
        format!("cols50({a},{a})"),
        format!("{a} "),
    ] {
        assert_eq!(bad.parse::<Layout>(), Err(ParseError), "{bad:?}");
    }
}

#[test]
fn text_deeper_than_the_limit_is_refused() {
    let nest = |depth: usize| {
        let mut text = id(0).to_string();
        for n in 1..=depth {
            text = format!("rows50({},{text})", id(n as u128));
        }
        text
    };
    assert!(nest(MAX_DEPTH).parse::<Layout>().is_ok());
    assert_eq!(nest(MAX_DEPTH + 1).parse::<Layout>(), Err(ParseError));
}

#[test]
fn sides_parse_by_name() {
    for side in [Side::Left, Side::Right, Side::Up, Side::Down] {
        assert_eq!(side.name().parse(), Ok(side));
    }
    assert_eq!("sideways".parse::<Side>(), Err(ParseError));
}

#[test]
fn resizing_sets_the_innermost_split_along_the_axis() {
    // cols50(1, rows50(2, 3))
    let mut layout = three();
    let share = |p| Share::new(p).expect("in range");
    // 3 is the second child of the rows split: it gets 30, 2 gets 70.
    assert!(layout.resize(id(3), Axis::Rows, share(30)));
    assert_eq!(
        layout.to_string(),
        format!("cols50({},rows70({},{}))", id(1), id(2), id(3))
    );
    // Along columns the innermost split holding 3 is the outer one.
    assert!(layout.resize(id(3), Axis::Columns, share(75)));
    assert_eq!(
        layout.to_string(),
        format!("cols25({},rows70({},{}))", id(1), id(2), id(3))
    );
    assert!(layout.resize(id(1), Axis::Columns, share(60)));
    assert_eq!(
        layout.to_string(),
        format!("cols60({},rows70({},{}))", id(1), id(2), id(3))
    );
}

#[test]
fn resizing_without_a_split_along_the_axis_changes_nothing() {
    let mut layout = three();
    let before = layout.clone();
    assert!(!layout.resize(id(1), Axis::Rows, Share::HALF));
    assert!(!layout.resize(id(9), Axis::Columns, Share::HALF));
    let mut alone = Layout::Block(id(1));
    assert!(!alone.resize(id(1), Axis::Columns, Share::HALF));
    assert_eq!(layout, before);
}

#[test]
fn swapping_trades_places_or_takes_an_absent_block_in() {
    let mut layout = three();
    layout.swap(id(1), id(3));
    assert_eq!(layout.blocks(), vec![id(3), id(2), id(1)]);
    // 4 is not in the layout: it takes 2's place, and 2 leaves.
    layout.swap(id(2), id(4));
    assert_eq!(layout.blocks(), vec![id(3), id(4), id(1)]);
    assert!(!layout.contains(id(2)));
}

#[test]
fn axes_parse_by_name() {
    assert_eq!("cols".parse(), Ok(Axis::Columns));
    assert_eq!("rows".parse(), Ok(Axis::Rows));
    assert_eq!("left".parse::<Axis>(), Err(ParseError));
    assert_eq!(Axis::Rows.name(), "rows");
}

#[test]
fn the_largest_block_is_one_that_may_still_be_split() {
    // A chain where each split gives its second child 99%: the deepest
    // leaf is the largest, but it sits MAX_DEPTH splits deep.
    let mut layout = Layout::Block(id(0));
    for n in 1..=MAX_DEPTH as u128 {
        layout = layout.split(id(n - 1), id(n), Side::Right);
        assert!(layout.resize(id(n), Axis::Columns, Share::new(99).expect("in range")));
    }
    let deepest = id(MAX_DEPTH as u128);
    assert_eq!(layout.depth_of(deepest), Some(MAX_DEPTH));
    let largest = layout.largest().expect("a shallower block");
    assert_ne!(largest, deepest);
    assert!(layout.depth_of(largest).is_some_and(|d| d < MAX_DEPTH));
}

#[test]
fn neighbours_border_the_block_on_their_side() {
    // cols50(1, rows50(2, 3))
    let layout = three();
    let side = |b, s| layout.neighbour(id(b), Toward::Side(s));
    assert_eq!(side(1, Side::Right), Some(id(2)));
    assert_eq!(side(3, Side::Left), Some(id(1)));
    assert_eq!(side(2, Side::Down), Some(id(3)));
    assert_eq!(side(3, Side::Up), Some(id(2)));
    assert_eq!(side(1, Side::Up), None);
    assert_eq!(side(9, Side::Up), None);
    assert_eq!(layout.neighbour(id(3), Toward::Next), Some(id(1)));
    assert_eq!(layout.neighbour(id(1), Toward::Previous), Some(id(3)));
    assert_eq!(Layout::Block(id(1)).neighbour(id(1), Toward::Next), None);
}

#[test]
fn a_sibling_is_the_nearest_block_of_the_other_child() {
    let layout = three();
    assert_eq!(layout.sibling(id(1)), Some(id(2)));
    assert_eq!(layout.sibling(id(2)), Some(id(3)));
    assert_eq!(layout.sibling(id(3)), Some(id(2)));
    assert_eq!(Layout::Block(id(1)).sibling(id(1)), None);
}

#[test]
fn ways_to_move_focus_parse_by_name() {
    for toward in [
        Toward::Side(Side::Left),
        Toward::Side(Side::Down),
        Toward::Next,
        Toward::Previous,
    ] {
        assert_eq!(toward.name().parse(), Ok(toward));
    }
    assert_eq!("north".parse::<Toward>(), Err(ParseError));
}

fn split(axis: Axis, percent: u8, first: Layout, second: Layout) -> Layout {
    Layout::Split {
        axis,
        share: Share::new(percent).expect("valid share"),
        first: Box::new(first),
        second: Box::new(second),
    }
}

#[test]
fn blocks_too_small_to_show_still_have_neighbours() {
    // cols1(cols1(cols1(cols1(1, 2), 3), 4), 5): on any screen, even the
    // largest, block 2 gets no room, yet moves from and to it work.
    let b = |n| Layout::Block(id(n));
    let mut layout = b(1);
    for n in 2..=5 {
        layout = split(Axis::Columns, 1, layout, b(n));
    }
    let largest = size(u16::MAX, u16::MAX);
    assert!(!layout.rects(largest).iter().any(|(b, _)| *b == id(2)));
    let side = |b, s| layout.neighbour(id(b), Toward::Side(s));
    assert_eq!(side(2, Side::Left), Some(id(1)));
    assert_eq!(side(2, Side::Right), Some(id(3)));
    assert_eq!(side(1, Side::Right), Some(id(2)));
    assert_eq!(side(3, Side::Left), Some(id(2)));
    assert_eq!(side(5, Side::Left), Some(id(4)));
    assert_eq!(side(2, Side::Up), None);
}

#[test]
fn a_move_picks_the_block_sharing_the_longest_edge() {
    // Left: 1 over cols1(cols1(cols1(cols1(5, 6), 7), 8), 9), where 6
    // gets no room on any screen. Right: 3 (top 30%) over 4.
    let b = |n| Layout::Block(id(n));
    let mut low = b(5);
    for n in 6..=9 {
        low = split(Axis::Columns, 1, low, b(n));
    }
    let left = split(Axis::Rows, 50, b(1), low);
    let right = split(Axis::Rows, 30, b(3), b(4));
    let layout = split(Axis::Columns, 50, left, right);
    let largest = size(u16::MAX, u16::MAX);
    assert!(!layout.rects(largest).iter().any(|(b, _)| *b == id(6)));
    let side = |b, s| layout.neighbour(id(b), Toward::Side(s));
    // 1 borders 3 for 30% of the height and 4 for 20%.
    assert_eq!(side(1, Side::Right), Some(id(3)));
    // 4 borders 1 for 20% and 9 for 50%.
    assert_eq!(side(4, Side::Left), Some(id(9)));
    // Below 1 sit 5 to 9; 9 is the widest.
    assert_eq!(side(1, Side::Down), Some(id(9)));
    assert_eq!(side(6, Side::Up), Some(id(1)));
    assert_eq!(side(6, Side::Right), Some(id(7)));
    assert_eq!(side(9, Side::Right), Some(id(4)));
    assert_eq!(side(3, Side::Left), Some(id(1)));
}

#[test]
fn moves_stay_exact_in_slivers_far_below_a_float_cell() {
    // Left: ten rows99 splits, 1 at the bottom, 1e-20 of the height.
    // Right: nine rows99 splits ending in rows30(2, 3) at the bottom,
    // 1e-18 of the height. 1 lies beside 3 only; 3 shares more with 100,
    // just above 1, than with 1.
    let b = |n| Layout::Block(id(n));
    let chain = |bottom, from: u128, levels: u128| {
        (from..from + levels).fold(bottom, |low, n| split(Axis::Rows, 99, b(n), low))
    };
    let left = chain(b(1), 100, 10);
    let right = chain(split(Axis::Rows, 30, b(2), b(3)), 200, 9);
    let layout = split(Axis::Columns, 50, left, right);
    assert_eq!(
        layout.neighbour(id(1), Toward::Side(Side::Right)),
        Some(id(3))
    );
    assert_eq!(
        layout.neighbour(id(3), Toward::Side(Side::Left)),
        Some(id(100))
    );
    assert_eq!(
        layout.neighbour(id(2), Toward::Side(Side::Left)),
        Some(id(100))
    );
}

#[test]
fn moves_reach_blocks_at_the_depth_limit() {
    // Left: 30 rows1 splits, each giving its second child 99%, down to
    // cols1(1, 2) at depth 32. Right: 3.
    let b = |n| Layout::Block(id(n));
    let left = (100..130).fold(split(Axis::Columns, 1, b(1), b(2)), |low, n| {
        split(Axis::Rows, 1, b(n), low)
    });
    let layout = split(Axis::Columns, 1, left, b(3));
    assert_eq!(layout.depth_of(id(1)), Some(MAX_DEPTH));
    let side = |b, s| layout.neighbour(id(b), Toward::Side(s));
    assert_eq!(side(1, Side::Right), Some(id(2)));
    assert_eq!(side(2, Side::Left), Some(id(1)));
    assert_eq!(side(2, Side::Right), Some(id(3)));
    assert_eq!(side(1, Side::Up), Some(id(100)));
    // 2 is 99% of the deepest row, wider than 1, and the tallest left.
    assert_eq!(side(3, Side::Left), Some(id(2)));
}
