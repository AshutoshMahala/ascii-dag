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
        compose::BandCanvas::new(&mut cells, plan.width(), 0, plan.height());
    compose::composite_band(view_ref, &plan, options, &mut canvas);
    let mut out = String::with_capacity(plan.width() * plan.height());
    let _ = emit::emit_plain_band(&canvas, options.charset, &mut out);
    out
}

pub use charset::Charset;
pub use color::{CellColor, ColorMode};
pub use config::{DEFAULT_BAND_ROWS, RenderOptions};
pub use style::{
    EdgeLabelStyle, EdgeStyle, EdgeStyleCtx, LabelPlacement, LabelPosition, LineWeight,
    MarkerShape, NodeBorder, NodeStyle, NodeStyleCtx, SubgraphStyle, SubgraphStyleCtx,
};
