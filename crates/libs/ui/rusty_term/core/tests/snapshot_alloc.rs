//! Resource bounds of `Grid::decode_snapshot` on hostile input, measured
//! with a counting allocator. In its own test binary so the global allocator
//! counts only this test.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

use rusty_term_core::Grid;

struct Counting;

static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

// SAFETY: every call forwards to `System` with the caller's own pointer and
// layout, and only adds byte counting around it.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller upholds `GlobalAlloc::alloc`'s contract.
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            let live = LIVE.fetch_add(layout.size(), Ordering::SeqCst) + layout.size();
            PEAK.fetch_max(live, Ordering::SeqCst);
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        LIVE.fetch_sub(layout.size(), Ordering::SeqCst);
        // SAFETY: `ptr` came from `alloc` above with this `layout`.
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOC: Counting = Counting;

/// Bytes allocated at the peak of `f` above what was live before it.
fn peak_during(f: impl FnOnce()) -> usize {
    let base = LIVE.load(Ordering::SeqCst);
    PEAK.store(base, Ordering::SeqCst);
    f();
    PEAK.load(Ordering::SeqCst) - base
}

/// One test, so no other test's allocations land in the counts.
#[test]
fn hostile_snapshots_allocate_little_before_failing() {
    const LIMIT: usize = 256 * 1024;
    // 4096x4096 declared, then the input ends: nothing but the reader.
    let truncated = b"RTSN\x01\x80\x20\x80\x20".to_vec();
    // A complete 4096x4096 header with no history, then the screen ends
    // after one line: at most that line (4096 cells) is allocated.
    let mut one_row = truncated.clone();
    one_row.extend([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    one_row.extend([
        0x80, 0x20, 0x00, 0x82, 0x80, 0x01, 0x00, 0, 0, 0, 0, 0, b' ',
    ]);
    // More history lines declared than the line budget allows.
    let mut many_lines = b"RTSN\x01\x04\x01".to_vec();
    many_lines.extend([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x81, 0x80, 0x40]);
    for (name, input) in [
        ("truncated header", truncated),
        ("one screen row of 4096x4096", one_row),
        ("history over the line budget", many_lines),
    ] {
        let mut result = None;
        let peak = peak_during(|| result = Some(Grid::decode_snapshot(&input).is_err()));
        assert_eq!(result, Some(true), "{name}: must fail");
        assert!(peak < LIMIT, "{name}: {peak} bytes allocated");
    }
}
