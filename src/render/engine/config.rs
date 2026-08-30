//! Render options, sorted into their three homes.
//!
//! The normative rule: **an option that could invalidate a scene is a
//! planning option.** [`PlanOptions`] holds everything that affects
//! resolved semantics; [`EmitOptions`] holds how those semantics are
//! written out (never invalidates a scene); [`ComposeBudget`] holds
//! memory behavior only (may never affect output). [`RenderOptions`]
//! is the composite convenience carrying one of each — it keeps the
//! 0.10 name and presets, so one-step call sites keep compiling with a
//! field-path change (`opts.charset` → `opts.emit.charset`).
//!
//! Everything here is `Copy`, `const`-constructible, `no_std`-safe.
//! The named presets (`plain`, `colored`, `ascii`, `ascii_colored`)
//! live in `presets.rs` per the growth-by-addition rule. The
//! extensible structs are `#[non_exhaustive]`; each implements
//! `Default` plus complete `with_*` builders, which together with the
//! presets are the only downstream construction paths.

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

/// Where an edge label may be placed inline.
///
/// This is a whole-render **policy** — distinct from the per-edge
/// [`LabelPlacement`](super::style::LabelPlacement) style, which picks
/// a position along one edge. Until 0.11 this policy was implicit:
/// the node-row veto silently switched on when color AND the legend
/// were both enabled. It is now an explicit, freely combinable choice.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LabelPlacementPolicy {
    /// Standard geometric placement (the 0.10 plain/ascii semantics).
    #[default]
    Geometric,
    /// Additionally veto rows that host nodes (the 0.10
    /// colored-with-legend semantics). Permanent first-class policy —
    /// 0.10 preset parity requires it indefinitely.
    AvoidNodeRows,
}

/// What happens to a label that found no inline position.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LabelOverflow {
    /// Unplaced labels appear nowhere (a diagnostic still fires — the
    /// output is never silently lossy).
    #[default]
    Omit,
    /// Unplaced labels are recorded in the plan's legend list. Whether
    /// that list is *printed* is emission
    /// ([`EmitOptions::render_legend`]).
    Legend,
}

/// The label-handling pair: inline placement policy + overflow
/// destination.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LabelPolicy {
    /// Where labels may sit inline.
    pub placement: LabelPlacementPolicy,
    /// Where unplaceable labels go.
    pub overflow: LabelOverflow,
}

impl LabelPolicy {
    /// The const baseline — identical to `Default::default()`, but
    /// usable in const contexts (embedded callers build options at
    /// compile time; `#[non_exhaustive]` rules out literals).
    pub const fn new() -> Self {
        Self {
            placement: LabelPlacementPolicy::Geometric,
            overflow: LabelOverflow::Omit,
        }
    }

    /// Builder: set the inline placement policy.
    pub const fn with_placement(mut self, placement: LabelPlacementPolicy) -> Self {
        self.placement = placement;
        self
    }

    /// Builder: set the overflow destination.
    pub const fn with_overflow(mut self, overflow: LabelOverflow) -> Self {
        self.overflow = overflow;
        self
    }
}

/// Planning options: everything that affects **resolved semantics**.
/// Changing any field here invalidates a plan/scene.
///
/// There is deliberately no color on/off flag: planning always
/// resolves colors (cheap), and plain emission simply ignores them —
/// that is what lets one plan emit colored and plain output alike.
#[non_exhaustive]
#[derive(Clone, Copy)]
pub struct PlanOptions {
    /// Edge color palette used by the default style (modulo
    /// assignment, legacy behavior). Ignored when an edge style fn
    /// returns an explicit color.
    pub palette: Palette,
    /// Per-edge style callback.
    pub edge_style_fn: EdgeStyleFn,
    /// Per-subgraph style callback.
    pub subgraph_style_fn: SubgraphStyleFn,
    /// Per-edge-label style callback.
    pub edge_label_style_fn: EdgeLabelStyleFn,
    /// Label placement + overflow handling.
    pub label_policy: LabelPolicy,
    /// Paint dummy nodes (`◍`) when the IR contains them.
    pub show_dummy_nodes: bool,
}

impl PlanOptions {
    /// The const baseline — identical to `Default::default()`, but
    /// usable in const contexts (embedded callers build options at
    /// compile time; `#[non_exhaustive]` rules out literals).
    pub const fn new() -> Self {
        Self {
            palette: Palette::Ansi,
            edge_style_fn: default_edge_style,
            subgraph_style_fn: default_subgraph_style,
            edge_label_style_fn: default_edge_label_style,
            label_policy: LabelPolicy::new(),
            show_dummy_nodes: false,
        }
    }

    /// Builder: set the default-style palette.
    pub const fn with_palette(mut self, palette: Palette) -> Self {
        self.palette = palette;
        self
    }

    /// Builder: set the per-edge style callback.
    pub const fn with_edge_style_fn(mut self, f: EdgeStyleFn) -> Self {
        self.edge_style_fn = f;
        self
    }

    /// Builder: set the per-subgraph style callback.
    pub const fn with_subgraph_style_fn(mut self, f: SubgraphStyleFn) -> Self {
        self.subgraph_style_fn = f;
        self
    }

    /// Builder: set the per-edge-label style callback.
    pub const fn with_edge_label_style_fn(mut self, f: EdgeLabelStyleFn) -> Self {
        self.edge_label_style_fn = f;
        self
    }

    /// Builder: set the label placement + overflow policy.
    pub const fn with_label_policy(mut self, policy: LabelPolicy) -> Self {
        self.label_policy = policy;
        self
    }

    /// Builder: paint dummy nodes when the IR contains them.
    pub const fn with_show_dummy_nodes(mut self, show: bool) -> Self {
        self.show_dummy_nodes = show;
        self
    }
}

impl Default for PlanOptions {
    fn default() -> Self {
        Self::new()
    }
}

/// Emission options: how resolved semantics are **written**. Never
/// invalidates a plan/scene — the same plan serves every combination
/// here.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EmitOptions {
    /// Output character set (decode table applied at emission).
    pub charset: Charset,
    /// Color output mode. `None` allocates no color planes at all.
    pub color_mode: ColorMode,
    /// Print the legend block after the diagram. The legend's CONTENT
    /// is planning ([`LabelOverflow::Legend`]); whether the block is
    /// printed is emission — that split is what makes "same plan,
    /// legend on/off" possible.
    pub render_legend: bool,
}

impl EmitOptions {
    /// The const baseline — identical to `Default::default()`, but
    /// usable in const contexts (embedded callers build options at
    /// compile time; `#[non_exhaustive]` rules out literals).
    pub const fn new() -> Self {
        Self {
            charset: Charset::Unicode,
            color_mode: ColorMode::None,
            render_legend: false,
        }
    }

    /// Builder: set the output character set.
    pub const fn with_charset(mut self, charset: Charset) -> Self {
        self.charset = charset;
        self
    }

    /// Builder: set the color output mode.
    pub const fn with_color_mode(mut self, mode: ColorMode) -> Self {
        self.color_mode = mode;
        self
    }

    /// Builder: print (or suppress) the legend block.
    pub const fn with_render_legend(mut self, render: bool) -> Self {
        self.render_legend = render;
        self
    }
}

impl Default for EmitOptions {
    fn default() -> Self {
        Self::new()
    }
}

/// Composition resources: **memory behavior only**. May never affect
/// output; not part of plan/scene identity.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ComposeBudget {
    /// Band height cap in rows (clamped to ≥ 1; see
    /// [`DEFAULT_BAND_ROWS`]). Canvas memory is `width × cap` cells.
    pub band_rows_cap: usize,
}

impl ComposeBudget {
    /// The const baseline — identical to `Default::default()`, but
    /// usable in const contexts (embedded callers build options at
    /// compile time; `#[non_exhaustive]` rules out literals).
    pub const fn new() -> Self {
        Self {
            band_rows_cap: DEFAULT_BAND_ROWS,
        }
    }

    /// Builder: cap band height in rows.
    pub const fn with_band_rows_cap(mut self, cap: usize) -> Self {
        self.band_rows_cap = cap;
        self
    }

    /// The effective band cap (degenerate configs clamp, never error).
    pub(crate) fn cap(&self) -> usize {
        self.band_rows_cap.max(1)
    }
}

impl Default for ComposeBudget {
    fn default() -> Self {
        Self::new()
    }
}

/// The composite render configuration: one option set per home.
///
/// Start from a preset and adjust — every field is public.
///
/// ```
/// use ascii_dag::{Charset, Graph, RenderOptions};
///
/// let g = Graph::from_edges(&[(1, "A"), (2, "B")], &[(1, 2)]);
/// let ir = g.compute_layout();
///
/// use ascii_dag::LabelOverflow;
///
/// let mut options = RenderOptions::plain();
/// options.emit.charset = Charset::Ascii;   // no box-drawing glyphs
/// options.plan.label_policy.overflow = LabelOverflow::Legend; // collect overflow…
/// options.emit.render_legend = true;       // …and print the legend block
/// options.compose.band_rows_cap = 16;      // cap canvas memory
///
/// let text = ir.render_string(&options);
/// assert!(text.contains("[A]"));
/// ```
///
/// Presets: [`plain`](Self::plain), [`colored`](Self::colored),
/// [`ascii`](Self::ascii), [`ascii_colored`](Self::ascii_colored).
#[derive(Clone, Copy)]
pub struct RenderOptions {
    /// Resolved-semantics options (invalidate a plan when changed).
    pub plan: PlanOptions,
    /// Output-writing options (never invalidate a plan).
    pub emit: EmitOptions,
    /// Memory-behavior knobs (never affect output).
    pub compose: ComposeBudget,
}

impl RenderOptions {
    /// Every-field default: plain Unicode, no colors, geometric labels
    /// that omit on overflow, default band cap, default style fns.
    /// Named presets live in `presets.rs`.
    pub(crate) const fn defaults() -> Self {
        Self {
            plan: PlanOptions::new(),
            emit: EmitOptions::new(),
            compose: ComposeBudget::new(),
        }
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
    fn presets_map_the_0_10_semantics() {
        let plain = RenderOptions::plain();
        let colored = RenderOptions::colored(Palette::Ansi);
        assert_eq!(plain.emit.color_mode, ColorMode::None);
        assert_eq!(colored.emit.color_mode, ColorMode::Ansi256);
        // The 0.10 implicit `colored && legend` pair is now explicit:
        // plain places geometrically and omits overflow; the colored
        // preset avoids node rows, records overflow in the legend, and
        // prints the legend block.
        assert_eq!(
            plain.plan.label_policy.placement,
            LabelPlacementPolicy::Geometric
        );
        assert_eq!(plain.plan.label_policy.overflow, LabelOverflow::Omit);
        assert!(!plain.emit.render_legend);
        assert_eq!(
            colored.plan.label_policy.placement,
            LabelPlacementPolicy::AvoidNodeRows
        );
        assert_eq!(colored.plan.label_policy.overflow, LabelOverflow::Legend);
        assert!(colored.emit.render_legend);

        let mut o = RenderOptions::plain();
        o.compose.band_rows_cap = 0;
        assert_eq!(o.compose.cap(), 1);
    }

    // The public const baselines must stay const-constructible and
    // equal to Default.
    const _PLAN: PlanOptions = PlanOptions::new();
    const _EMIT: EmitOptions = EmitOptions::new();
    const _COMPOSE: ComposeBudget = ComposeBudget::new();
    const _POLICY: LabelPolicy = LabelPolicy::new();

    #[test]
    fn builders_cover_every_field() {
        let plan = PlanOptions::default()
            .with_palette(Palette::Ansi)
            .with_edge_style_fn(super::default_edge_style)
            .with_subgraph_style_fn(super::default_subgraph_style)
            .with_edge_label_style_fn(super::default_edge_label_style)
            .with_label_policy(
                LabelPolicy::default()
                    .with_placement(LabelPlacementPolicy::AvoidNodeRows)
                    .with_overflow(LabelOverflow::Legend),
            )
            .with_show_dummy_nodes(true);
        assert!(plan.show_dummy_nodes);
        assert_eq!(
            plan.label_policy.placement,
            LabelPlacementPolicy::AvoidNodeRows
        );

        let emit = EmitOptions::default()
            .with_charset(Charset::Ascii)
            .with_color_mode(ColorMode::Ansi256)
            .with_render_legend(true);
        assert_eq!(emit.charset, Charset::Ascii);
        assert!(emit.render_legend);

        let compose = ComposeBudget::default().with_band_rows_cap(4);
        assert_eq!(compose.band_rows_cap, 4);
    }
}
