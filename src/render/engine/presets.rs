//! `const fn` option presets (temp/06 §1, R3.1).
//!
//! Presets are the second rung of the progressive-override ladder
//! (defaults → presets → per-render options → per-element style fns):
//! named, `const`-constructible starting points that reproduce a known
//! output shape. Style-bundle presets join when demand appears — the
//! style vocabulary is public, so callers can build their own today.

use super::charset::Charset;
use super::color::ColorMode;
use super::config::RenderOptions;
use crate::render::colors::Palette;

impl RenderOptions {
    /// Plain text output — matches the legacy `render_scanline()` family.
    pub const fn plain() -> Self {
        Self::defaults()
    }

    /// ANSI-colored output with a legend — matches the legacy
    /// `render_scanline_colored_with_legend(palette)`.
    pub const fn colored(palette: Palette) -> Self {
        Self {
            color_mode: ColorMode::Ansi256,
            legend: true,
            palette,
            ..Self::defaults()
        }
    }

    /// Pure-ASCII plain output: the same semantic canvas projected
    /// through the ASCII decode table (`| - = + v ^ > < @ o`).
    pub const fn ascii() -> Self {
        Self {
            charset: Charset::Ascii,
            ..Self::defaults()
        }
    }

    /// Pure-ASCII colored output with a legend (the legend arrow
    /// follows the charset: `->`).
    pub const fn ascii_colored(palette: Palette) -> Self {
        Self {
            charset: Charset::Ascii,
            ..Self::colored(palette)
        }
    }
}
