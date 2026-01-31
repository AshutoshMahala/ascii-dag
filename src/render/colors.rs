//! Color Palettes for Graph Visualization
//!
//! Provides pre-defined color palettes for edge and node coloring.
//! These can be used by any renderer (ASCII, ANSI, etc.)
//!
//! # Usage
//!
//! ```rust
//! use ascii_dag::render::colors::{self, Palette};
//!
//! // Get color by index (cycles through palette)
//! let color = colors::get(Palette::Ansi, 0);
//! assert_eq!(color, 39); // Blue
//! ```

/// ANSI 256-color codes for terminal edge coloring.
/// Pre-selected to look good on both light and dark terminals.
/// Colors are ordered to maximize contrast between adjacent indices
/// (alternating warm/cool, light/dark).
pub const ANSI: &[u8] = &[
    39,  // Blue (cool)
    203, // Red/Tomato (warm)
    37,  // Teal/Cyan (cool)
    208, // Orange (warm)
    134, // Purple (cool)
    35,  // Green (cool)
    220, // Yellow (warm)
    81,  // Sky blue (cool)
    196, // Bright red (warm)
    123, // Light cyan (cool)
    214, // Amber (warm)
    99,  // Violet (cool)
    71,  // Grass green (cool)
    205, // Pink (warm)
    33,  // Bright blue (cool)
    170, // Plum (warm)
];

/// ANSI palette optimized for dark terminals (brighter colors)
pub const ANSI_DARK: &[u8] = &[
    81,  // Bright cyan
    156, // Lime green
    222, // Peach/light orange
    183, // Lavender
    210, // Salmon/light red
    117, // Sky blue
    121, // Mint
    221, // Amber
    216, // Apricot
    189, // Mauve
    87,  // Turquoise
    147, // Light purple
];

/// ANSI palette optimized for light terminals (darker colors)
pub const ANSI_LIGHT: &[u8] = &[
    27,  // Dark blue
    124, // Dark red
    22,  // Dark green
    166, // Dark orange
    91,  // Dark purple
    30,  // Dark teal
    125, // Dark pink
    136, // Dark yellow
    24,  // Dark steel blue
    130, // Brown
    23,  // Dark cyan
    54,  // Dark violet
];

/// Available color palettes
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Palette {
    /// Default ANSI palette - works on both light and dark terminals
    Ansi,
    /// Optimized for dark terminal backgrounds
    AnsiDark,
    /// Optimized for light terminal backgrounds
    AnsiLight,
}

impl Palette {
    /// Get the color codes for this palette
    #[inline]
    pub fn colors(self) -> &'static [u8] {
        match self {
            Palette::Ansi => ANSI,
            Palette::AnsiDark => ANSI_DARK,
            Palette::AnsiLight => ANSI_LIGHT,
        }
    }
}

/// Get an ANSI color code from a palette by index (cycles through)
#[inline]
pub fn get(palette: Palette, index: usize) -> u8 {
    let colors = palette.colors();
    colors[index % colors.len()]
}

/// Get an ANSI color code from a custom palette by index (cycles through)
#[inline]
pub fn get_custom(palette: &[u8], index: usize) -> u8 {
    palette[index % palette.len()]
}

/// ANSI escape sequence constants
pub mod escape {
    /// Reset all formatting
    pub const RESET: &str = "\x1b[0m";

    /// Format a foreground color using 256-color palette
    /// Returns the escape sequence as a fixed-size array
    #[inline]
    pub fn fg256(color: u8) -> [u8; 11] {
        let mut buf = [0u8; 11];
        buf[0] = 0x1b; // ESC
        buf[1] = b'[';
        buf[2] = b'3';
        buf[3] = b'8';
        buf[4] = b';';
        buf[5] = b'5';
        buf[6] = b';';
        // Write color as 3 digits with leading zeros
        buf[7] = b'0' + (color / 100);
        buf[8] = b'0' + ((color / 10) % 10);
        buf[9] = b'0' + (color % 10);
        buf[10] = b'm';
        buf
    }

    /// Format a foreground color and write to a string
    #[inline]
    pub fn write_fg256(output: &mut alloc::string::String, color: u8) {
        use core::fmt::Write;
        let _ = write!(output, "\x1b[38;5;{}m", color);
    }
}

// Re-export for convenience
pub use escape::RESET;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_cycles_through_palette() {
        assert_eq!(get(Palette::Ansi, 0), 39);
        assert_eq!(get(Palette::Ansi, 1), 203);
        assert_eq!(get(Palette::Ansi, 16), 39); // cycles back
    }

    #[test]
    fn test_fg256_format() {
        let seq = escape::fg256(39);
        assert_eq!(seq[0], 0x1b); // ESC
        assert_eq!(seq[1], b'[');
    }

    #[test]
    fn test_palettes_not_empty() {
        assert!(!Palette::Ansi.colors().is_empty());
        assert!(!Palette::AnsiDark.colors().is_empty());
        assert!(!Palette::AnsiLight.colors().is_empty());
    }
}
