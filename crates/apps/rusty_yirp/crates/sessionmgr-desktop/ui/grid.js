// Pure grid layout math -- ported from `sessionmgr-tui/src/grid.rs`'s
// own `Grid`, weight-for-weight, so both front ends arrange N panes the
// same way: as close to square as `count` allows, row-major, weights as
// percentages of the available space rather than fixed pixel counts.
// The one real difference from the Rust original is *how* a weight
// changes -- `grid.rs` nudges by a fixed step per keypress; this grid is
// resized by dragging a divider, so `setColWeight`/`setRowWeight` take
// an absolute target percentage (computed from the pointer's position)
// and redistribute the difference across the immediate neighbor being
// dragged against, not every other row/column at once -- correct for a
// drag gesture, which only ever moves the boundary between two cells.

export const MIN_WEIGHT = 10;

export function evenWeights(n) {
  if (n <= 0) return [];
  const base = Math.floor(100 / n);
  const weights = new Array(n).fill(base);
  weights[n - 1] += 100 - base * n;
  return weights;
}

export function gridShape(count) {
  if (count <= 0) return { rows: 0, cols: 0 };
  const cols = Math.ceil(Math.sqrt(count));
  const rows = Math.ceil(count / cols);
  return { rows, cols };
}

export class Grid {
  constructor(count) {
    const { rows, cols } = gridShape(count);
    this.rowWeights = evenWeights(rows);
    this.colWeights = evenWeights(cols);
  }

  get rows() {
    return this.rowWeights.length;
  }

  get cols() {
    return this.colWeights.length;
  }

  // Moves the boundary between `i` and `i+1` in `weights` so `i]`'s
  // share becomes `target` (clamped so neither side crosses MIN_WEIGHT),
  // taking/giving the difference to `i+1` alone -- the drag only ever
  // touches the one boundary the pointer is on.
  static _dragBoundary(weights, i, target) {
    if (i < 0 || i + 1 >= weights.length) return;
    const pairSum = weights[i] + weights[i + 1];
    const clamped = Math.max(MIN_WEIGHT, Math.min(pairSum - MIN_WEIGHT, target));
    weights[i] = clamped;
    weights[i + 1] = pairSum - clamped;
  }

  dragColBoundary(i, target) {
    Grid._dragBoundary(this.colWeights, i, target);
  }

  dragRowBoundary(i, target) {
    Grid._dragBoundary(this.rowWeights, i, target);
  }
}
