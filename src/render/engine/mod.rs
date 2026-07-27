//! Unified render engine (temp/05 requirements, temp/06 design).
//!
//! One paint path serving both IRs — the render layer has no "backends".
//! The engine composes **semantic cells** (what a cell means, not which
//! glyph shows it) and projects them through a charset decode table at
//! emission, so Unicode and ASCII are equal outputs of one canvas.
//!
//! ```text
//! LayoutView (both IRs)                            [RW1]
//!       ↓
//! RenderPlan  — styles, spatial index, labels      [RW2]
//!       ↓
//! Band compositor — semantic cells, Z-order        [RW3+]
//!       ↓
//! Emission — charset decode, colors, writer        [RW3+]
//! ```
//!
//! # Organization rules (temp/05 N6b)
//!
//! One concern per file; growth by addition (a new charset is a new file
//! in `charset/`); internals are `pub(crate)`; the public surface is
//! exported only from this module; soft guardrail ~600 lines per file.
//!
//! **Staging note:** the engine integrates with the public API at RW3.
//! Until then it is exercised by its unit tests only; the module-level
//! `dead_code` allowance below is removed at integration.

#![allow(dead_code)] // staged: removed at RW3 integration

pub(crate) mod cell;
pub(crate) mod charset;
pub(crate) mod color;
#[cfg(feature = "alloc")]
pub(crate) mod compose;
pub(crate) mod config;
#[cfg(feature = "alloc")]
pub(crate) mod emit;
#[cfg(all(test, feature = "std", feature = "arena"))]
mod parity;
#[cfg(feature = "alloc")]
pub(crate) mod plan;
pub(crate) mod style;
pub(crate) mod view;

/// Render a laid-out view as plain text (single full-height band —
/// banding and the public streaming surface arrive at RW6).
#[cfg(feature = "alloc")]
pub(crate) fn render_plain<V: view::LayoutView>(
    view_ref: &V,
    options: &config::RenderOptions,
) -> alloc::string::String {
    use alloc::string::String;
    let plan = plan::RenderPlan::build(view_ref, options);
    let mut cells = alloc::vec![cell::Cell::EMPTY; plan.width() * plan.height().max(1)];
    let mut canvas =
        compose::BandCanvas::new(&mut cells, None, plan.width(), 0, plan.height());
    compose::composite_band(view_ref, &plan, options, &mut canvas);
    let mut out = String::with_capacity(plan.width() * plan.height());
    let _ = emit::emit_plain_band(&canvas, options.charset, &mut out);
    out
}

/// Render a laid-out view with ANSI colors (and, when `options.legend`
/// is set, the skipped-label legend — the legacy
/// `render_scanline_colored_with_legend` shape).
#[cfg(feature = "alloc")]
pub(crate) fn render_colored<V: view::LayoutView>(
    view_ref: &V,
    options: &config::RenderOptions,
) -> alloc::string::String {
    use alloc::string::String;
    use core::fmt::Write as _;
    let plan = plan::RenderPlan::build(view_ref, options);
    let area = plan.width() * plan.height().max(1);
    let mut cells = alloc::vec![cell::Cell::EMPTY; area];
    let mut color_cells = alloc::vec![color::CellColor::DEFAULT; area];
    let mut canvas = compose::BandCanvas::new(
        &mut cells,
        Some(&mut color_cells),
        plan.width(),
        0,
        plan.height(),
    );
    compose::composite_band(view_ref, &plan, options, &mut canvas);
    let mut out = String::with_capacity(area * 2);
    let _ = emit::emit_colored_band(&canvas, options.charset, options.color_mode, &mut out);

    // Legend for labels that could not be placed (legacy format).
    if options.legend && !plan.legend_entries().is_empty() {
        out.push_str("\nEdge labels:\n");
        for &ei in plan.legend_entries() {
            let e = view_ref.edge(ei);
            let Some(label) = e.label else { continue };
            let find_label = |id: usize| -> Option<alloc::string::String> {
                (0..view_ref.node_count())
                    .map(|i| view_ref.node(i))
                    .find(|n| n.id == id && !matches!(n.kind, crate::ir::NodeKind::Dummy))
                    .map(|n| alloc::string::String::from(n.label))
            };
            // Legacy lists an entry only when both endpoints resolve.
            let (Some(from), Some(to)) = (find_label(e.from_id), find_label(e.to_id)) else {
                continue;
            };
            let color = plan
                .edge_plan(ei)
                .color
                .as_ansi256()
                .unwrap_or(0);
            let _ = writeln!(
                out,
                "  \x1b[38;5;{color}m{from} \u{2192} {to}: \"{label}\"\x1b[0m"
            );
        }
    }
    out
}

pub use charset::Charset;
pub use color::{CellColor, ColorMode};
pub use config::{DEFAULT_BAND_ROWS, RenderOptions};
pub use style::{
    EdgeLabelStyle, EdgeStyle, EdgeStyleCtx, LabelPlacement, LabelPosition, LineWeight,
    MarkerShape, NodeBorder, NodeStyle, NodeStyleCtx, SubgraphStyle, SubgraphStyleCtx,
};
