//! Color representation.

/// RGBA 32-bit color.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Color {
    /// Red component (0-255).
    pub r: u8,
    /// Green component (0-255).
    pub g: u8,
    /// Blue component (0-255).
    pub b: u8,
    /// Alpha component (0-255).
    pub a: u8,
}

impl Color {
    /// Creates a new Color.
    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    /// Creates an opaque Color.
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    /// Packs color into a 32-bit ARGB/BGRA pixel format.
    pub fn to_u32(&self) -> u32 {
        ((self.a as u32) << 24) | ((self.r as u32) << 16) | ((self.g as u32) << 8) | (self.b as u32)
    }

    /// Unpacks a 32-bit ARGB pixel (as produced by [`Color::to_u32`]) back
    /// into a `Color`.
    pub const fn from_u32(pixel: u32) -> Self {
        Self {
            a: ((pixel >> 24) & 0xFF) as u8,
            r: ((pixel >> 16) & 0xFF) as u8,
            g: ((pixel >> 8) & 0xFF) as u8,
            b: (pixel & 0xFF) as u8,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_u32_from_u32_roundtrips() {
        let color = Color::rgba(0x11, 0x22, 0x33, 0x44);
        assert_eq!(Color::from_u32(color.to_u32()), color);
    }
}
