//! Glyph outline definitions.

use alloc::vec::Vec;

/// A 2D coordinate point on a glyph contour.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point {
    /// X coordinate.
    pub x: f32,
    /// Y coordinate.
    pub y: f32,
    /// Flag indicating whether point is on curve or off curve (control point).
    pub on_curve: bool,
}

impl Point {
    /// Creates a new Point.
    pub fn new(x: f32, y: f32, on_curve: bool) -> Self {
        Self { x, y, on_curve }
    }
}

/// A vector outline of a font glyph contour.
#[derive(Debug, Clone, Default)]
pub struct GlyphOutline {
    /// Collection of 2D points forming the contour paths.
    pub points: Vec<Point>,
    /// End indices of contours.
    pub contour_ends: Vec<usize>,
    /// Minimum bounding box X.
    pub min_x: i16,
    /// Minimum bounding box Y.
    pub min_y: i16,
    /// Maximum bounding box X.
    pub max_x: i16,
    /// Maximum bounding box Y.
    pub max_y: i16,
}
