//! `const fn` option presets (temp/06 §1, R3.1).
//!
//! Presets are the second rung of the progressive-override ladder
//! (defaults → presets → per-render options → per-element style fns):
//! named, `const`-constructible starting points that reproduce a known
//! output shape. Each maps a 0.10 look onto the sorted option homes —
//! the old implicit `colored && legend` label gate becomes the
//! explicit `AvoidNodeRows`/`Legend` pair. Style-bundle presets join
//! when demand appears — the style vocabulary is public, so callers
//! can build their own today.

use super::charset::Charset;
use super::color::ColorMode;
use super::config::{LabelOverflow, LabelPlacementPolicy, LabelPolicy, RenderOptions};
use crate::render::colors::Palette;

impl RenderOptions {
    /// Plain text output — the 0.9 `render_scanline()` family's look.
    /// Geometric label placement, overflow omitted, no legend block.
    pub const fn plain() -> Self {
        Self::defaults()
    }

    /// ANSI-colored output with a legend — matches the legacy
    /// 0.9's `render_scanline_colored_with_legend(palette)` look:
    /// labels avoid node rows, overflow goes to the legend, and the
    /// legend block is printed.
    pub const fn colored(palette: Palette) -> Self {
        let mut o = Self::defaults();
        o.plan.palette = palette;
        o.plan.label_policy = LabelPolicy::new()
            .with_placement(LabelPlacementPolicy::AvoidNodeRows)
            .with_overflow(LabelOverflow::Legend);
        o.emit.color_mode = ColorMode::Ansi256;
        o.emit.render_legend = true;
        o
    }

    /// ASCII-charset plain output: the same semantic canvas projected
    /// through the ASCII decode table (`| - = + v ^ > < @ o`). Note
    /// user-provided label text passes through unchanged — the charset
    /// governs the engine's own glyphs, not your content.
    pub const fn ascii() -> Self {
        let mut o = Self::defaults();
        o.emit.charset = Charset::Ascii;
        o
    }

    /// ASCII-charset colored output with a legend (the legend arrow
    /// follows the charset: `->`).
    pub const fn ascii_colored(palette: Palette) -> Self {
        let mut o = Self::colored(palette);
        o.emit.charset = Charset::Ascii;
        o
    }
}
