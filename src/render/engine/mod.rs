//! Unified render engine.
//!
//! One paint path serving both IRs — the render layer has no "backends".
//! The engine composes **semantic cells** (what a cell means, not which
//! glyph shows it) and projects them through a charset decode table at
//! emission, so Unicode and ASCII are equal outputs of one canvas.
//!
//! ```text
//! Layout IR (heap or arena)
//!       ↓
//! RenderPlan  — styles, spatial index, labels
//!       ↓
//! Band compositor — semantic cells, Z-order
//!       ↓
//! Emission — charset decode, colors, writer
//! ```
//!
//! # Public surface
//!
//! The streaming writer is primary: `render_with` on both IR types
//! feeds any `core::fmt::Write`; `render_string` is the owned
//! convenience; `render_to_bytes` is the zero-allocation byte surface
//! with a caller arena + buffer. `ScenePlanner`/`Scene` are the
//! public introspection surface (dimensions, legend, hit-testing).

pub(crate) mod api;

pub(crate) mod cell;
pub(crate) mod cells;
pub(crate) mod charset;
pub(crate) mod color;
pub(crate) mod compose;
pub(crate) mod composer;
pub(crate) mod config;
pub(crate) mod emit;
pub(crate) mod mem;
pub(crate) mod node_content;
pub(crate) mod owner;
#[cfg(all(test, feature = "std", feature = "arena"))]
mod parity;
pub(crate) mod plan;
pub(crate) mod presets;
pub(crate) mod region;
pub(crate) mod scene;
pub(crate) mod style;
pub(crate) mod terminal;
#[cfg(all(test, feature = "std", feature = "layout-vertical"))]
mod test_alloc;
pub(crate) mod view;
pub(crate) mod views;

/// Stream a rendered view into any writer — the one alloc-backed
/// band loop behind every std entry point. `options.emit` decides the
/// mode: colors when `color_mode != None` (plus the legend block when
/// `render_legend`), plain glyphs otherwise. One band-sized buffer set is reused across
/// bands (N3.4): memory is `width × min(band_cap, height)` cells
/// regardless of graph height.
#[cfg(feature = "alloc")]
pub(crate) fn render_into<V: view::LayoutView, W: core::fmt::Write>(
    view_ref: &V,
    options: &config::RenderOptions,
    out: &mut W,
) -> core::fmt::Result {
    let colored = !matches!(options.emit.color_mode, color::ColorMode::None);
    let plan = plan::RenderPlan::build(view_ref, &options.plan);
    let cap = options.compose.cap();
    let band_rows = plan.max_band_rows(cap).max(1);
    let area = plan.width() * band_rows;
    let mut scratch = compose::PaintScratch::heap_backed(view_ref, &plan, colored, band_rows);
    let mut cells = alloc::vec![cell::Cell::EMPTY; area];
    let mut colors_plane = if colored {
        alloc::vec![color::CellColor::DEFAULT; area]
    } else {
        alloc::vec::Vec::new()
    };
    emit_bands(
        view_ref,
        &plan,
        &options.emit,
        cap,
        &mut scratch,
        &mut cells,
        colored.then(|| &mut colors_plane[..]),
        out,
    )
}

/// The ONE band-emission loop behind every terminal surface — the
/// one-step wrappers and [`TerminalRenderer`](terminal::TerminalRenderer)
/// compose and emit through exactly this path (plan → compose → emit).
///
/// D4 (temp/08): the legend works in plain mode too — labels that
/// don't fit go to the legend regardless of color mode; a plain
/// legend is self-keying (`from -> to: label`). `ColorMode::None`
/// emits no escapes.
#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_bands<V: view::LayoutView, W: core::fmt::Write>(
    view_ref: &V,
    plan: &plan::RenderPlan<'_>,
    emit_options: &config::EmitOptions,
    cap: usize,
    scratch: &mut compose::PaintScratch<'_>,
    cells: &mut [cell::Cell],
    mut colors: Option<&mut [color::CellColor]>,
    out: &mut W,
) -> core::fmt::Result {
    let colored = !matches!(emit_options.color_mode, color::ColorMode::None);
    for (y0, rows) in plan.bands(cap) {
        let plane = if colored { colors.as_deref_mut() } else { None };
        let mut canvas = compose::BandCanvas::new(cells, plane, plan.width(), y0, rows);
        compose::composite_band(view_ref, plan, &mut canvas, scratch);
        if colored {
            emit::emit_colored_band(&canvas, emit_options.charset, emit_options.color_mode, out)?;
        } else {
            emit::emit_plain_band(&canvas, emit_options.charset, out)?;
        }
    }
    if emit_options.render_legend {
        emit::emit_legend(
            view_ref,
            plan,
            emit_options.charset,
            emit_options.color_mode,
            out,
        )?;
    }
    Ok(())
}

/// Owned-`String` render of a plain-mode `options` (parity-suite
/// helper; the public surface is `render_with`/`render_string`).
#[cfg(all(test, feature = "std", feature = "arena"))]
pub(crate) fn render_plain<V: view::LayoutView>(
    view_ref: &V,
    options: &config::RenderOptions,
) -> alloc::string::String {
    let mut out = alloc::string::String::new();
    let _ = render_into(view_ref, options, &mut out);
    out
}

/// Owned-`String` render of colored-mode `options` (parity-suite
/// helper; the 0.9 `render_scanline_colored_with_legend` shape when
/// `options.emit.render_legend` is set).
#[cfg(all(test, feature = "std"))]
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
    let colored = !matches!(options.emit.color_mode, color::ColorMode::None);
    let plan = plan::RenderPlan::build_in(view_ref, &options.plan, arena)?;
    let cap = options.compose.cap();
    let band_rows = plan.max_band_rows(cap).max(1);
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

    let mut sink = emit::ByteSink::new(out);
    let write = emit_bands(
        view_ref,
        &plan,
        &options.emit,
        cap,
        &mut scratch,
        cells,
        colors,
        &mut sink,
    );
    match write {
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
    let colored = !matches!(options.emit.color_mode, color::ColorMode::None);
    emit::estimate_output_size(view_ref, colored, options.emit.render_legend)
}

pub use cells::{ArmWeight, ArmWeights, CellKind, CellMarker, CellView, MarkerDirection};
pub use charset::Charset;
pub use color::{CellColor, ColorMode};
pub use composer::{CompositionRequirements, SceneComposer};
pub use config::{
    ComposeBudget, DEFAULT_BAND_ROWS, EmitOptions, LabelOverflow, LabelPlacementPolicy,
    LabelPolicy, PlanOptions, RenderOptions,
};
pub use node_content::{BoxedNode, CustomNode, NodeContent, NodeKindTag, SimpleNode};
pub use plan::HitResult;
pub use region::{NodePaintCtx, NodeRegion};
pub use scene::{LayoutSource, PlanRun, Scene, ScenePlanner};
pub use style::{
    EdgeLabelStyle, EdgeStyle, EdgeStyleCtx, LabelPlacement, LabelPosition, LineWeight,
    MarkerShape, NodePaintFn, SubgraphBorder, SubgraphStyle, SubgraphStyleCtx,
};
pub use terminal::TerminalRenderer;
pub use views::{
    EdgePathView, EdgeView, LabelSlot, LabelView, NodeKind, NodeOrigin, NodeView, SubgraphView,
};
