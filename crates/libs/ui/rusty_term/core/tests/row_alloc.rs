//! `Grid::row_cells` is the per-frame live-row reader (the accessibility
//! tree walks it on every redraw), so iterating it must not allocate. In its
//! own test binary so the global allocator counts only this test.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

use rusty_term_core::{AnsiParser, Grid};

struct Counting;

static ALLOCS: AtomicUsize = AtomicUsize::new(0);

// SAFETY: every call forwards to `System` with the caller's own pointer and
// layout, and only counts allocations around it.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::SeqCst);
        // SAFETY: the caller upholds `GlobalAlloc::alloc`'s contract.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: `ptr` came from `alloc` above with this `layout`.
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOC: Counting = Counting;

/// One test, so no other test's allocations land in the count.
#[test]
fn row_cells_iterates_without_allocating() {
    let mut grid = Grid::new(80, 24);
    let mut parser = AnsiParser::new();
    parser.advance(
        &mut grid,
        "\x1b[31mred \x1b[1mbold 日本 e\u{301}\r\n"
            .repeat(40)
            .as_bytes(),
    );
    let before = ALLOCS.load(Ordering::SeqCst);
    let mut cells = 0;
    for y in 0..=grid.rows {
        cells += grid.row_cells(y).filter(|c| c.ch != ' ').count();
    }
    let allocs = ALLOCS.load(Ordering::SeqCst) - before;
    assert!(cells > 0, "fixture draws something");
    assert_eq!(allocs, 0, "row_cells allocated");
}
