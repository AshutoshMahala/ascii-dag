//! Packed cell colors (temp/06 §4, ruling Q8).
//!
//! [`CellColor`] stores color **semantically** in one tagged `u32`
//! (default / ANSI-256 index / 24-bit RGB) — storage never knows the
//! output mode. Conversion happens once per emitted cell, per the active
//! [`ColorMode`]: RGB quantizes to the xterm 256 cube, ANSI expands to
//! RGB for truecolor sinks.
//!
//! Planes built from this type are gated (no plane when colors are off)
//! and band-sized, so color memory is bounded by `width × band_rows`
//! regardless of graph size.

/// Output color encoding mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ColorMode {
    /// No color output (no color plane is allocated at all).
    None,
    /// ANSI 256-color escapes (default; today's palette behavior).
    #[default]
    Ansi256,
    /// 24-bit truecolor escapes.
    TrueColor,
}

const TAG_SHIFT: u32 = 30;
const TAG_ANSI: u32 = 1;
const TAG_RGB: u32 = 2;

/// One packed color value. All-zero = terminal default (no escape).
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct CellColor(u32);

impl CellColor {
    /// Terminal default — no escape emitted.
    pub const DEFAULT: CellColor = CellColor(0);

    /// An ANSI-256 palette color.
    pub const fn ansi256(index: u8) -> CellColor {
        CellColor((TAG_ANSI << TAG_SHIFT) | index as u32)
    }

    /// A 24-bit RGB color.
    pub const fn rgb(r: u8, g: u8, b: u8) -> CellColor {
        CellColor((TAG_RGB << TAG_SHIFT) | ((r as u32) << 16) | ((g as u32) << 8) | b as u32)
    }

    /// Whether this is a real color (an escape will be emitted).
    #[inline]
    pub fn is_set(self) -> bool {
        self.0 != 0
    }

    #[inline]
    pub(crate) fn is_ansi(self) -> bool {
        self.0 >> TAG_SHIFT == TAG_ANSI
    }

    #[inline]
    pub(crate) fn is_rgb(self) -> bool {
        self.0 >> TAG_SHIFT == TAG_RGB
    }

    /// The ANSI index of an ANSI-tagged color (undefined otherwise).
    #[inline]
    pub(crate) fn ansi_index(self) -> u8 {
        (self.0 & 0xFF) as u8
    }

    /// The RGB components of an RGB-tagged color (undefined otherwise).
    #[inline]
    pub(crate) fn rgb_parts(self) -> (u8, u8, u8) {
        (
            ((self.0 >> 16) & 0xFF) as u8,
            ((self.0 >> 8) & 0xFF) as u8,
            (self.0 & 0xFF) as u8,
        )
    }

    /// This color as an ANSI-256 index, quantizing RGB if needed.
    /// Returns `None` for [`CellColor::DEFAULT`].
    pub(crate) fn as_ansi256(self) -> Option<u8> {
        if self.is_ansi() {
            Some(self.ansi_index())
        } else if self.is_rgb() {
            let (r, g, b) = self.rgb_parts();
            Some(rgb_to_ansi256(r, g, b))
        } else {
            None
        }
    }

    /// This color as RGB, expanding an ANSI index if needed.
    /// Returns `None` for [`CellColor::DEFAULT`].
    pub(crate) fn as_rgb(self) -> Option<(u8, u8, u8)> {
        if self.is_rgb() {
            Some(self.rgb_parts())
        } else if self.is_ansi() {
            Some(ansi256_to_rgb(self.ansi_index()))
        } else {
            None
        }
    }
}

impl core::fmt::Debug for CellColor {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        if !self.is_set() {
            write!(f, "CellColor::DEFAULT")
        } else if self.is_ansi() {
            write!(f, "CellColor::ansi256({})", self.ansi_index())
        } else {
            let (r, g, b) = self.rgb_parts();
            write!(f, "CellColor::rgb({r}, {g}, {b})")
        }
    }
}

/// Quantize 24-bit RGB to the nearest xterm-256 index (standard cube +
/// grayscale-ramp mapping).
pub(crate) fn rgb_to_ansi256(r: u8, g: u8, b: u8) -> u8 {
    // Gray: pick the closer of the 24-step gray ramp (232–255) and the
    // cube's gray diagonal (0, 95, 135, 175, 215, 255) — the ramp is
    // denser but the cube holds the exact mid-gray levels.
    if r == g && g == b {
        let v = r as i32;
        let ramp_i = ((v - 8 + 5) / 10).clamp(0, 23);
        let ramp_v = 8 + 10 * ramp_i;
        let cube_q = if v < 48 {
            0
        } else if v < 115 {
            1
        } else {
            ((v - 35) / 40).min(5)
        };
        let cube_v = if cube_q == 0 { 0 } else { 55 + 40 * cube_q };
        return if (v - cube_v).abs() < (v - ramp_v).abs() {
            (16 + 43 * cube_q) as u8 // gray diagonal: 16 + 36q + 6q + q
        } else {
            (232 + ramp_i) as u8
        };
    }
    let q = |v: u8| -> u16 {
        // Standard xterm cube breakpoints: 0, 95, 135, 175, 215, 255.
        if v < 48 {
            0
        } else if v < 115 {
            1
        } else {
            ((v as u16) - 35) / 40
        }
    };
    (16 + 36 * q(r) + 6 * q(g) + q(b)) as u8
}

/// Expand an xterm-256 index to 24-bit RGB.
pub(crate) fn ansi256_to_rgb(index: u8) -> (u8, u8, u8) {
    match index {
        // 16 system colors (standard xterm values).
        0 => (0, 0, 0),
        1 => (205, 0, 0),
        2 => (0, 205, 0),
        3 => (205, 205, 0),
        4 => (0, 0, 238),
        5 => (205, 0, 205),
        6 => (0, 205, 205),
        7 => (229, 229, 229),
        8 => (127, 127, 127),
        9 => (255, 0, 0),
        10 => (0, 255, 0),
        11 => (255, 255, 0),
        12 => (92, 92, 255),
        13 => (255, 0, 255),
        14 => (0, 255, 255),
        15 => (255, 255, 255),
        16..=231 => {
            // 6×6×6 color cube.
            let i = index as u16 - 16;
            let level = |c: u16| -> u8 {
                if c == 0 { 0 } else { (55 + 40 * c) as u8 }
            };
            (
                level(i / 36),
                level((i / 6) % 6),
                level(i % 6),
            )
        }
        232..=255 => {
            // Grayscale ramp: 8, 18, …, 238.
            let v = (8 + 10 * (index as u16 - 232)) as u8;
            (v, v, v)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packing_roundtrips() {
        assert!(!CellColor::DEFAULT.is_set());
        let a = CellColor::ansi256(196);
        assert!(a.is_set() && a.is_ansi());
        assert_eq!(a.ansi_index(), 196);
        let c = CellColor::rgb(12, 34, 56);
        assert!(c.is_set() && c.is_rgb());
        assert_eq!(c.rgb_parts(), (12, 34, 56));
    }

    #[test]
    fn cube_expansion_matches_xterm() {
        assert_eq!(ansi256_to_rgb(196), (255, 0, 0)); // cube pure red
        assert_eq!(ansi256_to_rgb(16), (0, 0, 0)); // cube black
        assert_eq!(ansi256_to_rgb(231), (255, 255, 255)); // cube white
        assert_eq!(ansi256_to_rgb(244), (128, 128, 128)); // gray ramp middle
        assert_eq!(ansi256_to_rgb(51), (0, 255, 255)); // cube cyan
    }

    #[test]
    fn quantization_hits_cube_corners() {
        assert_eq!(rgb_to_ansi256(255, 0, 0), 196);
        assert_eq!(rgb_to_ansi256(0, 255, 255), 51);
        assert_eq!(rgb_to_ansi256(0, 0, 0), 16);
        assert_eq!(rgb_to_ansi256(255, 255, 255), 231);
        assert_eq!(rgb_to_ansi256(128, 128, 128), 244);
    }

    #[test]
    fn quantize_expand_is_stable() {
        // Expanding an index and re-quantizing returns the same index
        // for the cube and gray ramp (16..=255).
        for index in 16u16..=255 {
            let (r, g, b) = ansi256_to_rgb(index as u8);
            assert_eq!(
                rgb_to_ansi256(r, g, b),
                index as u8,
                "index {index} not stable"
            );
        }
    }

    #[test]
    fn mode_conversions() {
        let red = CellColor::rgb(255, 0, 0);
        assert_eq!(red.as_ansi256(), Some(196));
        let ansi = CellColor::ansi256(196);
        assert_eq!(ansi.as_rgb(), Some((255, 0, 0)));
        assert_eq!(CellColor::DEFAULT.as_ansi256(), None);
        assert_eq!(CellColor::DEFAULT.as_rgb(), None);
        assert_eq!(ColorMode::default(), ColorMode::Ansi256);
    }
}
