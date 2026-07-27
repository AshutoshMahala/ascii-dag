//! `RenderOptions` — the engine's render configuration (temp/06 §8, M5).
//!
//! `Copy`, `const`-constructible, `no_std`-safe. Presets reproduce the
//! legacy entry points' behavior: [`RenderOptions::plain`] matches
//! `render_scanline()`, [`RenderOptions::colored`] matches
//! `render_scanline_colored(palette)`.

use super::charset::Charset;
use super::color::ColorMode;
use super::style::{
    EdgeLabelStyleFn, EdgeStyleFn, NodeStyleFn, SubgraphStyleFn, default_edge_label_style,
    default_edge_style, default_node_style, default_subgraph_style,
};
use crate::render::colors::Palette;

/// Default cap on band height in rows (Q1: level-aligned bands split
/// when they would exceed this). Typical bands are 3–15 rows, so the
/// default never splits a level except pathological ones; embedded
/// callers lower it to bound canvas memory (`width × cap` cells).
pub const DEFAULT_BAND_ROWS: usize = 64;

/// Render configuration for the unified engine.
#[derive(Clone, Copy)]
pub struct RenderOptions {
    /// Color output mode. `None` allocates no color planes at all.
    pub color_mode: ColorMode,
    /// Output character set (decode table applied at emission).
    pub charset: Charset,
    /// Paint dummy nodes (`◍`) when the IR contains them.
    pub show_dummy_nodes: bool,
    /// Band height cap in rows (clamped to ≥ 1; see [`DEFAULT_BAND_ROWS`]).
    pub band_rows_cap: usize,
    /// Append the skipped-label legend (colored path behavior).
    pub legend: bool,
    /// Edge color palette used by the default style (modulo assignment,
    /// legacy behavior). Ignored when `color_mode` is `None` or an edge
    /// style fn returns an explicit color.
    pub palette: Palette,
    /// Per-edge style callback.
    pub edge_style_fn: EdgeStyleFn,
    /// Per-node style callback.
    pub node_style_fn: NodeStyleFn,
    /// Per-subgraph style callback.
    pub subgraph_style_fn: SubgraphStyleFn,
    /// Per-edge-label style callback.
    pub edge_label_style_fn: EdgeLabelStyleFn,
}

impl RenderOptions {
    /// Plain text output — matches the legacy `render_scanline()` family.
    pub const fn plain() -> Self {
        Self {
            color_mode: ColorMode::None,
            charset: Charset::Unicode,
            show_dummy_nodes: false,
            band_rows_cap: DEFAULT_BAND_ROWS,
            legend: false,
            palette: Palette::Ansi,
            edge_style_fn: default_edge_style,
            node_style_fn: default_node_style,
            subgraph_style_fn: default_subgraph_style,
            edge_label_style_fn: default_edge_label_style,
        }
    }

    /// ANSI-colored output with a legend — matches the legacy
    /// `render_scanline_colored_with_legend(palette)`.
    pub const fn colored(palette: Palette) -> Self {
        Self {
            color_mode: ColorMode::Ansi256,
            legend: true,
            palette,
            ..Self::plain()
        }
    }

    /// The effective band cap (degenerate configs clamp, never error).
    pub(crate) fn band_cap(&self) -> usize {
        self.band_rows_cap.max(1)
    }
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self::plain()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Presets must stay const-constructible.
    const _PLAIN: RenderOptions = RenderOptions::plain();
    const _COLORED: RenderOptions = RenderOptions::colored(Palette::Ansi);

    #[test]
    fn presets_and_clamping() {
        let plain = RenderOptions::plain();
        let colored = RenderOptions::colored(Palette::Ansi);
        assert_eq!(plain.color_mode, ColorMode::None);
        assert_eq!(plain.legend, colored.color_mode == ColorMode::None);
        assert_eq!(colored.color_mode, ColorMode::Ansi256);
        assert!(colored.legend);

        let mut o = RenderOptions::plain();
        o.band_rows_cap = 0;
        assert_eq!(o.band_cap(), 1);
    }
}
