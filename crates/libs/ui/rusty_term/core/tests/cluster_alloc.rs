//! Under mode 2027 every non-ASCII scalar asks whether it continues the glyph
//! on its left. Once a cluster's text is interned, writing it again must not
//! allocate (#1191). In its own test binary so the global allocator counts
//! only this test.

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
fn mode_2027_joining_does_not_allocate() {
    let mut grid = Grid::new(120, 4);
    let mut parser = AnsiParser::new();
    // Combining marks, a ZWJ family, a skin tone, a flag, VS16 and a keycap.
    let line =
        "\x1b[Hcafe\u{301} 👨\u{200d}👩\u{200d}👧 👍\u{1f3fd} 🇯🇵 ❤\u{fe0f} 1\u{fe0f}\u{20e3} 漢字";
    parser.advance(&mut grid, b"\x1b[?2027h");
    parser.advance(&mut grid, line.as_bytes()); // interns each cluster once
    let before = ALLOCS.load(Ordering::SeqCst);
    for _ in 0..8 {
        parser.advance(&mut grid, line.as_bytes());
    }
    let allocs = ALLOCS.load(Ordering::SeqCst) - before;
    assert_eq!(grid.cursor.0, 24, "fixture lays out as clusters");
    assert_eq!(allocs, 0, "joining under mode 2027 allocated");
}
