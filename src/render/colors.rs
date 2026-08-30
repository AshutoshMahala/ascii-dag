//! Color Palettes for Graph Visualization
//!
//! Provides pre-defined color palettes for edge and node coloring.
//! These can be used by any renderer (ASCII, ANSI, etc.)
//!
//! # Usage
//!
//! ```rust
//! use ascii_dag::render::colors::Palette;
//!
//! // Index the palette table directly (cycle with `%` if needed)
//! let colors = Palette::Ansi.colors();
//! assert_eq!(colors[0], 196); // Bright Red
//! ```

/// ANSI 256-color codes for terminal edge coloring.
/// Pre-selected to look good on both light and dark terminals.
/// Colors are ordered to maximize contrast between adjacent indices
/// (alternating warm/cool, light/dark).
pub const ANSI: &[u8] = &[
    // Set 1: High Contrast Primaries (Vivid)
    196, // Bright Red
    39,  // Bright Blue
    46,  // Neon Green
    226, // Bright Yellow
    201, // Magenta
    51,  // Cyan
    // Set 2: Deep/Rich Tones (Darker)
    160, // Deep Red
    21,  // Deep Blue
    28,  // Forest Green
    208, // Orange
    93,  // Purple
    30,  // Teal
    // Set 3: Light/Pastel (High Luma)
    203, // Salmon
    75,  // Sky Blue
    154, // Lime
    215, // Gold
    213, // Pink
    159, // Pale Cyan
    // Set 4: Earth & Electric (Mixed)
    166, // Burnt Orange
    57,  // Indigo Blue
    82,  // Electric Green
    190, // Chartreuse
    129, // Violet
    37,  // Dark Cyan
    // Set 5: Extended Range
    220, // Goldenrod
    33,  // Dodger Blue
    198, // Deep Pink
    88,  // Dark Maroon
    71,  // Moss Green
    17,  // Navy Blue
    202, // Red-Orange
    45,  // Turquoise
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

/// Get an ANSI color code from a custom palette by index (cycles through)
#[inline]
pub fn get_custom(palette: &[u8], index: usize) -> u8 {
    palette[index % palette.len()]
}

/// ANSI escape sequence constants
pub mod escape {
    /// Reset all formatting
    pub const RESET: &str = "\x1b[0m";
}

// Re-export for convenience
pub use escape::RESET;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_palettes_not_empty() {
        assert!(!Palette::Ansi.colors().is_empty());
        assert!(!Palette::AnsiDark.colors().is_empty());
        assert!(!Palette::AnsiLight.colors().is_empty());
    }
}
