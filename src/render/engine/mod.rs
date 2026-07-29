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
//! # Public surface
//!
//! The streaming writer is primary (R4.1): `render_with` on both IR
//! types feeds any `core::fmt::Write`; `render_string` is the owned
//! convenience; `render_to_bytes` is the zero-allocation byte surface
//! with caller arena + buffer (R4.2/R4.3). `RenderPlan` is the public
//! introspection type (R5); `Renderer` hosts external backends (M8).

pub(crate) mod api;
pub(crate) mod cell;
pub(crate) mod charset;
pub(crate) mod color;
pub(crate) mod compose;
pub(crate) mod config;
pub(crate) mod emit;
pub(crate) mod mem;
#[cfg(all(test, feature = "std", feature = "arena"))]
mod parity;
pub(crate) mod plan;
pub(crate) mod presets;
pub(crate) mod style;
pub(crate) mod view;

/// Stream a rendered view into any writer — the one alloc-backed
/// band loop behind every std entry point. `options` decides the mode:
/// colors when `color_mode != None` (plus the legend when `legend`),
/// plain glyphs otherwise. One band-sized buffer set is reused across
/// bands (N3.4): memory is `width × min(band_cap, height)` cells
/// regardless of graph height.
#[cfg(feature = "alloc")]
pub(crate) fn render_into<V: view::LayoutView, W: core::fmt::Write>(
    view_ref: &V,
    options: &config::RenderOptions,
    out: &mut W,
) -> core::fmt::Result {
    let colored = !matches!(options.color_mode, color::ColorMode::None);
    let plan = plan::RenderPlan::build(view_ref, options);
    let band_rows = plan.max_band_rows().max(1);
    let area = plan.width() * band_rows;
    let mut scratch = compose::PaintScratch::heap_backed(view_ref, &plan, colored, band_rows);
    let mut cells = alloc::vec![cell::Cell::EMPTY; area];
    let mut colors_plane = if colored {
        alloc::vec![color::CellColor::DEFAULT; area]
    } else {
        alloc::vec::Vec::new()
    };
    for &(y0, rows) in plan.band_ranges() {
        let plane = colored.then(|| &mut colors_plane[..]);
        let mut canvas = compose::BandCanvas::new(&mut cells, plane, plan.width(), y0, rows);
        compose::composite_band(view_ref, &plan, options, &mut canvas, &mut scratch);
        if colored {
            emit::emit_colored_band(&canvas, options.charset, options.color_mode, out)?;
        } else {
            emit::emit_plain_band(&canvas, options.charset, out)?;
        }
    }
    if colored && options.legend {
        emit::emit_legend(view_ref, &plan, options.charset, out)?;
    }
    Ok(())
}

/// Owned-`String` render of a plain-mode `options` (parity-suite
/// helper; the public surface is `render_with`/`render_string`).
#[cfg(all(test, feature = "alloc"))]
pub(crate) fn render_plain<V: view::LayoutView>(
    view_ref: &V,
    options: &config::RenderOptions,
) -> alloc::string::String {
    let mut out = alloc::string::String::new();
    let _ = render_into(view_ref, options, &mut out);
    out
}

/// Owned-`String` render of colored-mode `options` (parity-suite
/// helper; the legacy `render_scanline_colored_with_legend` shape when
/// `options.legend` is set).
#[cfg(all(test, feature = "alloc"))]
pub(crate) fn render_colored<V: view::LayoutView>(
    view_ref: &V,
    options: &config::RenderOptions,
) -> alloc::string::String {
    let mut out = alloc::string::String::new();
    let _ = render_into(view_ref, options, &mut out);
    out
}

/// Render into a caller byte buffer with all working memory carved from
/// `arena` — the zero-allocation surface (N2, R4.3). Returns the byte
/// count written. Failure domains map to the WDP `Render` component:
/// plan storage → `E.Render.Plan.026`, band canvas + compositing
/// scratch → `E.Render.Canvas.026`, output buffer →
/// `E.Render.Sink.026`. Size `arena` with [`estimate_render_arena_size`]
/// and `out` with [`estimate_render_output_size`].
pub(crate) fn render_to_bytes<V: view::LayoutView>(
    view_ref: &V,
    options: &config::RenderOptions,
    arena: &crate::graph::arena::Arena<'_>,
    out: &mut [u8],
) -> Result<usize, crate::GraphError> {
    let colored = !matches!(options.color_mode, color::ColorMode::None);
    let plan = plan::RenderPlan::build_in(view_ref, options, arena)?;
    let band_rows = plan.max_band_rows().max(1);
    let area = plan.width() * band_rows;
    let canvas_oom = || crate::GraphError::RenderCanvasTooSmall {
        needed: area,
        got: arena.remaining() / core::mem::size_of::<cell::Cell>(),
    };
    let mut scratch = compose::PaintScratch::carve(view_ref, &plan, colored, band_rows, arena)?;
    let cells = arena
        .alloc_slice_default::<cell::Cell>(area)
        .ok_or_else(canvas_oom)?;
    let colors = if colored {
        Some(
            arena
                .alloc_slice_default::<color::CellColor>(area)
                .ok_or_else(canvas_oom)?,
        )
    } else {
        None
    };
    let mut colors = colors;

    let mut sink = emit::ByteSink::new(out);
    let mut write = || -> core::fmt::Result {
        for &(y0, rows) in plan.band_ranges() {
            let mut canvas = compose::BandCanvas::new(
                cells,
                colors.as_deref_mut(),
                plan.width(),
                y0,
                rows,
            );
            compose::composite_band(view_ref, &plan, options, &mut canvas, &mut scratch);
            if colored {
                emit::emit_colored_band(&canvas, options.charset, options.color_mode, &mut sink)?;
            } else {
                emit::emit_plain_band(&canvas, options.charset, &mut sink)?;
            }
        }
        if colored && options.legend {
            emit::emit_legend(view_ref, &plan, options.charset, &mut sink)?;
        }
        Ok(())
    };
    match write() {
        Ok(()) => Ok(sink.written()),
        // The sink is the only fallible writer here; a fmt error that
        // isn't the sink overflowing would be an engine bug.
        Err(_) => {
            debug_assert!(sink.overflowed());
            Err(crate::GraphError::RenderOutputTooSmall)
        }
    }
}

/// Bytes of arena an [`render_to_bytes`] call needs for this view and
/// options: plan storage, compositing scratch, and the band canvas
/// planes, plus per-allocation alignment slack.
pub(crate) fn estimate_render_arena_size<V: view::LayoutView>(
    view_ref: &V,
    options: &config::RenderOptions,
) -> usize {
    plan::estimate_plan_bytes(view_ref, options)
}

/// Upper bound on the bytes [`render_to_bytes`] can write for this view
/// and options.
pub(crate) fn estimate_render_output_size<V: view::LayoutView>(
    view_ref: &V,
    options: &config::RenderOptions,
) -> usize {
    let colored = !matches!(options.color_mode, color::ColorMode::None);
    emit::estimate_output_size(view_ref, colored, colored && options.legend)
}

pub use api::{EngineRenderer, Renderer};
pub use charset::Charset;
pub use color::{CellColor, ColorMode};
pub use config::{DEFAULT_BAND_ROWS, RenderOptions};
pub use plan::{HitResult, RenderPlan};
pub use style::{
    EdgeLabelStyle, EdgeStyle, EdgeStyleCtx, LabelPlacement, LabelPosition, LineWeight,
    MarkerShape, NodeBorder, NodeStyle, NodeStyleCtx, SubgraphBorder, SubgraphStyle,
    SubgraphStyleCtx,
};
