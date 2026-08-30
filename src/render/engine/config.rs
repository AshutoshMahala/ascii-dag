//! `RenderOptions` — the engine's render configuration (temp/06 §8, M5).
//!
//! `Copy`, `const`-constructible, `no_std`-safe. The named presets
//! (`plain`, `colored`, `ascii`, `ascii_colored`) live in `presets.rs`
//! per the growth-by-addition rule (a new preset is a new entry there,
//! never an edit here).

use super::charset::Charset;
use super::color::ColorMode;
use super::style::{
    EdgeLabelStyleFn, EdgeStyleFn, SubgraphStyleFn, default_edge_label_style, default_edge_style,
    default_subgraph_style,
};
use crate::render::colors::Palette;

/// Default cap on band height in rows (level-aligned bands split when
/// they would exceed this). Typical bands are 3–15 rows, so the
/// default never splits a level except pathological ones; embedded
/// callers lower it to bound canvas memory (`width × cap` cells).
pub const DEFAULT_BAND_ROWS: usize = 64;

/// Render configuration for the unified engine.
///
/// Start from a preset and adjust — every field is public.
///
/// ```
/// use ascii_dag::{Charset, Graph, RenderOptions};
///
/// let g = Graph::from_edges(&[(1, "A"), (2, "B")], &[(1, 2)]);
/// let ir = g.compute_layout();
///
/// let mut options = RenderOptions::plain();
/// options.charset = Charset::Ascii;   // no box-drawing glyphs
/// options.legend = true;              // list labels that did not fit
/// options.band_rows_cap = 16;         // cap canvas memory
///
/// let text = ir.render_string(&options);
/// assert!(text.contains("[A]"));
/// ```
///
/// Presets: [`plain`](Self::plain), [`colored`](Self::colored),
/// [`ascii`](Self::ascii), [`ascii_colored`](Self::ascii_colored).
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
    /// Append the legend listing labels that found no inline position.
    /// OFF by default (the colored preset turns it on): with it off,
    /// an unplaced label appears NOWHERE in the output — silently,
    /// unless the `warnings` feature is enabled (`W.Render.Label.031`).
    pub legend: bool,
    /// Edge color palette used by the default style (modulo assignment,
    /// legacy behavior). Ignored when `color_mode` is `None` or an edge
    /// style fn returns an explicit color.
    pub palette: Palette,
    /// Per-edge style callback.
    pub edge_style_fn: EdgeStyleFn,
    /// Per-subgraph style callback.
    pub subgraph_style_fn: SubgraphStyleFn,
    /// Per-edge-label style callback.
    pub edge_label_style_fn: EdgeLabelStyleFn,
}

impl RenderOptions {
    /// Every-field default: plain Unicode, no colors, default band cap,
    /// default style fns. Named presets live in `presets.rs`.
    pub(crate) const fn defaults() -> Self {
        Self {
            color_mode: ColorMode::None,
            charset: Charset::Unicode,
            show_dummy_nodes: false,
            band_rows_cap: DEFAULT_BAND_ROWS,
            legend: false,
            palette: Palette::Ansi,
            edge_style_fn: default_edge_style,
            subgraph_style_fn: default_subgraph_style,
            edge_label_style_fn: default_edge_label_style,
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
        assert_eq!(colored.color_mode, ColorMode::Ansi256);
        // Legend: off for plain (unplaced labels then warn under the
        // `warnings` feature), on for the colored preset (legacy).
        assert!(!plain.legend);
        assert!(colored.legend);

        let mut o = RenderOptions::plain();
        o.band_rows_cap = 0;
        assert_eq!(o.band_cap(), 1);
    }
}
