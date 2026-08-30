//! `SceneComposer` — the cell answers.
//!
//! A composer turns a [`Scene`] into a stream of [`CellView`]s: one
//! callback per canvas cell, row-major, each carrying the cell's
//! meaning, resolved color, and hit/pick owner. It is always
//! **color-complete** and **owner-complete** — the same scene serves a
//! terminal, an SVG writer, and an interactive picker without
//! replanning.
//!
//! The composer owns ONE retained workspace chunk; every per-compose
//! buffer (band canvas, color plane, paint scratch, the ownership
//! plane and its scratch) is carved from it by a fresh bump arena per
//! visit. Carving is pointer arithmetic — at steady state a repaint
//! allocates nothing, and there is no reset step and no `unsafe`.
//! Sizing comes from [`Scene::composition_requirements`]: scene
//! cardinalities folded through the exact carve sequence with checked
//! arithmetic. A composer accepts ANY scene whose requirements fit its
//! chunk: the heap composer grows to a new high-water mark (the only
//! allocating event); a fixed-workspace composer reports
//! [`GraphError::RenderWorkspaceTooSmall`] at preflight — byte units,
//! nothing carved.
//!
//! Composition is banded internally (the band cap comes from
//! [`ComposeBudget`], memory behavior only): band boundaries are
//! deliberately UNOBSERVABLE — the callback sees a seamless row-major
//! stream, and workspace size never affects the values.

use core::ops::ControlFlow;

use super::cell::Cell;
use super::cells::{CellKind, CellView};
use super::color::CellColor;
use super::compose::{BandCanvas, PaintScratch, composite_band};
use super::config::ComposeBudget;
use super::owner::{
    OwnerScratch, OwnerSweep, owner_incidence_capacity, owner_prepare, owner_rasterize_band,
    owner_to_hit,
};
use super::plan::RenderPlan;
use super::scene::{Scene, ViewRef};
use super::view::LayoutView;
use crate::GraphError;
use crate::graph::arena::Arena;

/// Dispatch one expression over both lens backends.
macro_rules! with_view {
    ($scene:expr, $v:ident => $e:expr) => {
        match *$scene.view() {
            #[cfg(feature = "alloc")]
            ViewRef::Heap($v) => $e,
            ViewRef::Arena($v) => $e,
        }
    };
}

/// Workspace descriptor for one scene under one [`ComposeBudget`]:
/// scene cardinalities (exact run capacity, element counts, the
/// ownership tables' sizes), not a width×height formula. Opaque;
/// sizes a [`SceneComposer`].
#[derive(Debug, Clone, Copy)]
pub struct CompositionRequirements {
    pub(crate) band_rows_cap: usize,
    pub(crate) band_rows: usize,
    pub(crate) width: usize,
    pub(crate) height: usize,
    pub(crate) run_capacity: usize,
    pub(crate) nodes: usize,
    pub(crate) edges: usize,
    pub(crate) subgraphs: usize,
    pub(crate) elements: usize,
    pub(crate) incidence: usize,
    pub(crate) bytes: Option<usize>,
}

impl CompositionRequirements {
    /// Bytes of workspace one composition of this scene needs —
    /// checked arithmetic over the exact carve sequence. `None` means
    /// the requirement itself overflows (the scene cannot fit any
    /// workspace).
    pub fn workspace_bytes(&self) -> Option<usize> {
        self.bytes
    }

    fn compute<V: LayoutView>(view: &V, plan: &RenderPlan<'_>, budget: &ComposeBudget) -> Self {
        let height = plan.height().max(1);
        let band_rows_cap = budget.band_rows_cap.max(1);
        let band_rows = plan.max_band_rows(band_rows_cap).max(1);
        let mut req = Self {
            band_rows_cap,
            band_rows,
            width: plan.width(),
            height,
            run_capacity: plan.run_capacity(),
            nodes: view.node_count(),
            edges: view.edge_count(),
            subgraphs: view.subgraph_count(),
            elements: plan.elements().len(),
            // `usize::MAX` marks an overflowed incidence sum; the
            // checked fold below then reports the whole requirement
            // unfittable.
            incidence: owner_incidence_capacity(plan, band_rows).unwrap_or(usize::MAX),
            bytes: None,
        };
        req.bytes = req.layout_bytes();
        req
    }

    /// Bytes of workspace a
    /// [`TerminalRenderer::new_in`](super::terminal::TerminalRenderer::new_in)
    /// needs for this scene under `emit` — the LEAN terminal profile:
    /// the same checked carve-mirrored fold as
    /// [`workspace_bytes`](Self::workspace_bytes), but with no
    /// ownership plane and a color plane only for colored emission.
    /// The semantic composer's introspection cost never becomes an
    /// unavoidable part of the terminal surface. `None` means the
    /// requirement itself overflows.
    pub fn terminal_workspace_bytes(&self, emit: &super::config::EmitOptions) -> Option<usize> {
        self.terminal_bytes(!matches!(emit.color_mode, super::color::ColorMode::None))
    }

    /// Internal bool-flag form of
    /// [`terminal_workspace_bytes`](Self::terminal_workspace_bytes).
    pub(crate) fn terminal_bytes(&self, colored: bool) -> Option<usize> {
        use core::mem::{align_of, size_of};
        let area = self.width.checked_mul(self.band_rows)?;
        let scratch_terms = PaintScratch::carve_layout(
            self.run_capacity,
            self.subgraphs,
            self.edges,
            self.nodes,
            colored,
            self.width,
            self.band_rows,
        )?;
        let extra_terms: [(Option<usize>, usize); 2] = [
            (area.checked_mul(size_of::<Cell>()), align_of::<Cell>()),
            if colored {
                (
                    area.checked_mul(size_of::<CellColor>()),
                    align_of::<CellColor>(),
                )
            } else {
                (Some(0), 1)
            },
        ];
        let mut cursor = 0usize;
        let mut max_align = 1usize;
        for &(bytes, align) in scratch_terms.iter() {
            max_align = max_align.max(align);
            cursor = cursor.checked_add(align - 1)? & !(align - 1);
            cursor = cursor.checked_add(bytes)?;
        }
        for (bytes, align) in extra_terms {
            let bytes = bytes?;
            max_align = max_align.max(align);
            cursor = cursor.checked_add(align - 1)? & !(align - 1);
            cursor = cursor.checked_add(bytes)?;
        }
        cursor.checked_add(max_align - 1)
    }

    /// Fold the exact `(bytes, align)` carve sequence — the
    /// `PaintScratch` carves in their real order, then the band
    /// canvas, the color plane, and the ownership carves — plus
    /// max-align headroom for the chunk's base pointer. Runs on every
    /// visit preflight, so: fixed-size term list, no allocation.
    fn layout_bytes(&self) -> Option<usize> {
        use core::mem::{align_of, size_of};
        let area = self.width.checked_mul(self.band_rows)?;
        let u32_term = |count: usize| (count.checked_mul(size_of::<u32>()), align_of::<u32>());

        let scratch_terms = PaintScratch::carve_layout(
            self.run_capacity,
            self.subgraphs,
            self.edges,
            self.nodes,
            true, // the composer is always color-complete
            self.width,
            self.band_rows,
        )?;
        let extra_terms: [(Option<usize>, usize); 10] = [
            (area.checked_mul(size_of::<Cell>()), align_of::<Cell>()),
            (
                area.checked_mul(size_of::<CellColor>()),
                align_of::<CellColor>(),
            ),
            u32_term(area),                           // owner plane
            u32_term(self.width.checked_add(1)?),     // claim scratch
            u32_term(self.edges),                     // edge_slot
            u32_term(self.elements),                  // by_y_min
            u32_term(self.elements),                  // active set
            u32_term(self.band_rows.checked_add(1)?), // row offsets
            u32_term(self.band_rows),                 // row cursors
            u32_term(self.incidence),                 // row incidence
        ];

        let mut cursor = 0usize;
        let mut max_align = 1usize;
        for &(bytes, align) in scratch_terms.iter() {
            max_align = max_align.max(align);
            cursor = cursor.checked_add(align - 1)? & !(align - 1);
            cursor = cursor.checked_add(bytes)?;
        }
        for (bytes, align) in extra_terms {
            let bytes = bytes?;
            max_align = max_align.max(align);
            cursor = cursor.checked_add(align - 1)? & !(align - 1);
            cursor = cursor.checked_add(bytes)?;
        }
        cursor.checked_add(max_align - 1)
    }
}

impl Scene<'_, '_> {
    /// Workspace descriptor for composing this scene under `budget` —
    /// hand it to [`SceneComposer::new`] or
    /// [`SceneComposer::new_in`].
    pub fn composition_requirements(&self, budget: &ComposeBudget) -> CompositionRequirements {
        with_view!(self, v => CompositionRequirements::compute(v, self.plan(), budget))
    }
}

enum Workspace<'ws> {
    /// Growable retained chunk: grows to each new high-water mark,
    /// never shrinks, allocates nothing at steady state.
    #[cfg(feature = "alloc")]
    Heap(alloc::vec::Vec<u8>),
    /// Caller-provided fixed chunk: a scene that does not fit is a
    /// documented error, never a grow.
    Fixed(&'ws mut [u8]),
}

/// Composes [`Scene`]s into per-cell answers, retaining its workspace
/// across visits and scenes.
///
/// Construct from a scene's [`CompositionRequirements`]; the composer
/// then accepts any scene (and any number of repaints) whose
/// requirements fit — growing on the heap when they don't, erroring
/// in fixed-workspace mode. Steady-state repaint through a fitting
/// composer performs zero allocations (pinned by test).
pub struct SceneComposer<'ws> {
    ws: Workspace<'ws>,
    band_rows_cap: usize,
}

#[cfg(feature = "alloc")]
impl SceneComposer<'static> {
    /// Heap composer, presized for `requirements`; visits for larger
    /// scenes grow the workspace to their high-water mark.
    ///
    /// Unfittable requirements (`workspace_bytes()` returned `None`)
    /// presize nothing; every visit of such a scene then reports
    /// [`GraphError::RenderWorkspaceTooSmall`] instead of attempting
    /// an absurd allocation.
    pub fn new(requirements: CompositionRequirements) -> Self {
        Self {
            ws: Workspace::Heap(alloc::vec![
                0u8;
                requirements.workspace_bytes().unwrap_or(0)
            ]),
            band_rows_cap: requirements.band_rows_cap,
        }
    }
}

impl<'ws> SceneComposer<'ws> {
    /// No-alloc composer over a caller-provided workspace. Fails at
    /// construction — with BYTE units, nothing carved — when
    /// `requirements` do not fit.
    pub fn new_in(
        requirements: CompositionRequirements,
        workspace: &'ws mut [u8],
    ) -> Result<Self, GraphError> {
        let needed = requirements.workspace_bytes().unwrap_or(usize::MAX);
        if needed > workspace.len() {
            return Err(GraphError::RenderWorkspaceTooSmall {
                needed_bytes: needed,
                got_bytes: workspace.len(),
            });
        }
        Ok(Self {
            ws: Workspace::Fixed(workspace),
            band_rows_cap: requirements.band_rows_cap,
        })
    }

    /// Final cells, row-major, one callback per cell: `(x, y,
    /// CellView)`. Composed in bands internally; band boundaries are
    /// unspecified and UNOBSERVABLE — the callback sees a seamless
    /// row-major stream, and workspace size never affects the values.
    ///
    /// The `CellView` is callback-scoped (a lending visitor) — copy
    /// its fields to retain cells; the view itself cannot escape the
    /// callback.
    pub fn visit_cells<F>(&mut self, scene: &Scene<'_, '_>, mut f: F) -> Result<(), GraphError>
    where
        F: FnMut(usize, usize, CellView<'_>),
    {
        self.try_visit_cells(scene, |x, y, cell| {
            f(x, y, cell);
            ControlFlow::<core::convert::Infallible>::Continue(())
        })
        .map(|_| ())
    }

    /// Fallible/early-exit form for real sinks (an SVG writer, a TUI
    /// buffer with damage tracking): the callback can stop the visit
    /// with `ControlFlow::Break(B)` — a consumer failure or an
    /// intentional early exit — and the composer distinguishes its own
    /// errors (`Err(GraphError)`) from the consumer's break value.
    pub fn try_visit_cells<B, F>(
        &mut self,
        scene: &Scene<'_, '_>,
        f: F,
    ) -> Result<ControlFlow<B>, GraphError>
    where
        F: FnMut(usize, usize, CellView<'_>) -> ControlFlow<B>,
    {
        let cap = self.band_rows_cap;
        with_view!(scene, v => {
            let req = CompositionRequirements::compute(v, scene.plan(), &ComposeBudget::new().with_band_rows_cap(cap));
            // An overflowed requirement is an ERROR in both modes —
            // never a `usize::MAX` heap grow.
            let Some(needed) = req.workspace_bytes() else {
                let got_bytes = match &self.ws {
                    #[cfg(feature = "alloc")]
                    Workspace::Heap(buf) => buf.len(),
                    Workspace::Fixed(buf) => buf.len(),
                };
                return Err(GraphError::RenderWorkspaceTooSmall {
                    needed_bytes: usize::MAX,
                    got_bytes,
                });
            };
            let chunk: &mut [u8] = match &mut self.ws {
                #[cfg(feature = "alloc")]
                Workspace::Heap(buf) => {
                    if needed > buf.len() {
                        buf.resize(needed, 0); // the only allocating event
                    }
                    buf.as_mut_slice()
                }
                Workspace::Fixed(buf) => {
                    if needed > buf.len() {
                        return Err(GraphError::RenderWorkspaceTooSmall {
                            needed_bytes: needed,
                            got_bytes: buf.len(),
                        });
                    }
                    &mut buf[..]
                }
            };
            let got_bytes = chunk.len();
            // A fresh bump arena over the retained chunk each visit:
            // carves cannot outlive the call — no reset, no `unsafe`.
            let arena = Arena::new(chunk);
            visit_core(v, scene.plan(), &req, &arena, got_bytes, f)
        })
    }
}

/// One generic visit over the private lens — monomorphized per
/// backend; the enum dispatch above keeps the public types
/// non-generic.
fn visit_core<V: LayoutView, B, F>(
    view: &V,
    plan: &RenderPlan<'_>,
    req: &CompositionRequirements,
    arena: &Arena<'_>,
    got_bytes: usize,
    mut f: F,
) -> Result<ControlFlow<B>, GraphError>
where
    F: FnMut(usize, usize, CellView<'_>) -> ControlFlow<B>,
{
    let width = req.width;
    let area = width * req.band_rows;
    let oom = || GraphError::RenderWorkspaceTooSmall {
        needed_bytes: req.bytes.unwrap_or(usize::MAX),
        got_bytes,
    };

    let mut scratch = PaintScratch::carve(view, plan, true, req.band_rows, arena)?;
    let cells = arena.alloc_slice_default::<Cell>(area).ok_or_else(oom)?;
    let colors = arena
        .alloc_slice_default::<CellColor>(area)
        .ok_or_else(oom)?;
    let owner_plane = arena.alloc_slice_default::<u32>(area).ok_or_else(oom)?;
    let carve_u32 = |n: usize| arena.alloc_slice_default::<u32>(n).ok_or_else(oom);
    let mut owner_scratch = OwnerScratch {
        claim_next: carve_u32(width + 1)?,
        edge_slot: carve_u32(req.edges)?,
        by_y_min: carve_u32(req.elements)?,
        active: carve_u32(req.elements)?,
        row_off: carve_u32(req.band_rows + 1)?,
        row_cur: carve_u32(req.band_rows)?,
        row_inc: carve_u32(req.incidence)?,
    };
    let mut sweep = OwnerSweep::default();
    owner_prepare(plan, &mut owner_scratch, &mut sweep);

    if plan.height() == 0 {
        return Ok(ControlFlow::Continue(()));
    }
    let mut y0 = 0usize;
    while y0 < req.height {
        let rows = req.band_rows.min(req.height - y0);
        let mut canvas = BandCanvas::new(cells, Some(colors), width, y0, rows);
        composite_band(view, plan, &mut canvas, &mut scratch);
        let band_plane = &mut owner_plane[..width * rows];
        owner_rasterize_band(
            plan,
            view,
            y0,
            y0 + rows,
            width,
            band_plane,
            &mut owner_scratch,
            &mut sweep,
        );
        for row in 0..rows {
            let y = y0 + row;
            let cell_row = canvas.row(row);
            let color_row = canvas.color_row(row).expect("composer is color-complete");
            for x in 0..width {
                let cell = CellView {
                    kind: CellKind::from_cell(cell_row[x]),
                    color: color_row[x],
                    owner: owner_to_hit(plan, view, band_plane[row * width + x]),
                    _reserved: core::marker::PhantomData,
                };
                if let ControlFlow::Break(b) = f(x, y, cell) {
                    return Ok(ControlFlow::Break(b));
                }
            }
        }
        y0 += rows;
    }
    Ok(ControlFlow::Continue(()))
}

#[cfg(all(test, feature = "std", feature = "layout-vertical"))]
mod tests {
    use super::super::cell::{Cell, Dir, MarkerKind, Weight};
    use super::super::cells::{ArmWeight, CellMarker, MarkerDirection};
    use super::super::charset::Charset;
    use super::super::config::{PlanOptions, RenderOptions};
    use super::super::scene::ScenePlanner;
    use super::super::test_alloc::allocations_on_this_thread;
    use super::*;
    use crate::graph::Graph;
    use crate::render::engine::HitResult;

    use crate as ascii_dag;
    include!("../../../examples/shared/hero_graph.rs");

    // ── Fixtures (the ownership-agreement corpus) ────────────────────

    fn stage_graph() -> Graph<'static> {
        let mut g = Graph::new();
        g.add_node(1usize, "Start");
        g.add_node(2usize, "Middle");
        g.add_node(3usize, "End");
        g.add_edge(1usize, 2usize, Some("go"));
        g.add_edge(2usize, 3usize, None);
        let sg = g.add_subgraph("Stage");
        g.put_nodes(&[2]).inside(sg).unwrap();
        g
    }

    fn fan(n: usize) -> Graph<'static> {
        let mut g = Graph::new();
        g.add_node(0usize, "R");
        for i in 1..=n {
            g.add_node(i, "c");
            g.add_edge(0usize, i, None);
        }
        g
    }

    fn clusters_graph() -> Graph<'static> {
        let mut g = Graph::new();
        g.add_node(1usize, "a");
        g.add_node(2usize, "b");
        g.add_node(3usize, "c");
        g.add_node(4usize, "d");
        let s1 = g.add_subgraph("S1");
        let s2 = g.add_subgraph("S2");
        g.put_nodes(&[1, 2]).inside(s1).unwrap();
        g.put_nodes(&[3, 4]).inside(s2).unwrap();
        g.add_edge(1usize, 4usize, None);
        g
    }

    fn nested_graph() -> Graph<'static> {
        let mut g = Graph::new();
        g.add_node(1usize, "outer");
        g.add_node(2usize, "inner");
        g.add_node(3usize, "leaf");
        g.add_edge(1usize, 2usize, None);
        g.add_edge(2usize, 3usize, None);
        let outer = g.add_subgraph("Outer");
        let inner = g.add_subgraph("Inner");
        g.put_nodes(&[1, 2, 3]).inside(inner).unwrap();
        g.put_subgraphs(&[inner]).inside(outer).unwrap();
        g
    }

    fn self_loop_graph() -> Graph<'static> {
        let mut g = Graph::new();
        g.add_node(1usize, "Gate");
        g.add_node(2usize, "Next");
        g.add_edge(1usize, 1usize, Some("retry"));
        g.add_edge(1usize, 2usize, None);
        g
    }

    fn budget(cap: usize) -> ComposeBudget {
        ComposeBudget::new().with_band_rows_cap(cap)
    }

    // ── Re-encode a CellView for glyph comparison ────────────────────

    fn to_weight(w: ArmWeight) -> Weight {
        match w {
            ArmWeight::None => Weight::None,
            ArmWeight::Dashed => Weight::Dashed,
            ArmWeight::Light => Weight::Light,
            ArmWeight::Double => Weight::Double,
        }
    }

    fn to_dir(d: MarkerDirection) -> Dir {
        match d {
            MarkerDirection::Up => Dir::Up,
            MarkerDirection::Down => Dir::Down,
            MarkerDirection::Left => Dir::Left,
            MarkerDirection::Right => Dir::Right,
        }
    }

    /// Decode one CellView's kind to the glyph the plain Unicode
    /// terminal emitter would print.
    fn glyph(kind: &CellKind) -> char {
        match *kind {
            CellKind::Empty => ' ',
            CellKind::Text { ch } => ch,
            CellKind::Stroke { arms } => Charset::Unicode.decode(Cell::stroke(
                to_weight(arms.up),
                to_weight(arms.down),
                to_weight(arms.left),
                to_weight(arms.right),
            )),
            CellKind::Marker { marker } => Charset::Unicode.decode(match marker {
                CellMarker::SelfLoop => Cell::marker(MarkerKind::SelfLoop, Dir::Up, false),
                CellMarker::Dummy => Cell::marker(MarkerKind::Dummy, Dir::Up, false),
                CellMarker::Arrow { direction, dashed } => {
                    Cell::marker(MarkerKind::Arrow, to_dir(direction), dashed)
                }
            }),
        }
    }

    // ── THE permanent gate: cells and hit-testing agree ──────────────

    /// For every canvas cell, across the corpus × both backends ×
    /// every enabled direction × full-height and 3-row band budgets:
    /// `CellView.owner` answers exactly what `Scene::hit_test`
    /// answers, and the decoded glyphs reproduce the plain terminal
    /// output byte-for-byte.
    #[test]
    fn cells_and_hit_testing_agree() {
        type Fixture = (&'static str, fn() -> Graph<'static>);
        let corpus: Vec<Fixture> = vec![
            ("stage", stage_graph),
            ("hero", hero_graph as fn() -> Graph<'static>),
            ("fan-40", || fan(40)),
            ("clusters", clusters_graph),
            ("nested", nested_graph),
            ("self-loop (legacy node-owned rule)", self_loop_graph),
        ];
        #[cfg_attr(not(feature = "layout-horizontal"), allow(unused_mut))]
        let mut directions = vec![
            crate::graph::Direction::TopDown,
            crate::graph::Direction::BottomUp,
        ];
        #[cfg(feature = "layout-horizontal")]
        directions.extend([
            crate::graph::Direction::LeftRight,
            crate::graph::Direction::RightLeft,
        ]);

        for (what, build) in corpus {
            for &dir in &directions {
                let mut cfg = crate::LayoutConfig::standard();
                cfg.direction = dir;
                let g = build();
                let ir = g.compute_layout_with_config(&cfg);
                check_scene(&ir, &PlanOptions::new(), dir, what);

                // Arena backend.
                let g = build();
                let mut csr_buf = vec![0u8; g.estimate_csr_arena_size() * 2];
                let mut csr_arena = Arena::new(&mut csr_buf);
                let csr = g.to_csr(&mut csr_arena).unwrap();
                let size = (g.estimate_layout_arena_size() * 2).max(256 * 1024);
                let mut temp_buf = vec![0u8; size];
                let mut out_buf = vec![0u8; size];
                let mut temp_arena = Arena::new(&mut temp_buf);
                let mut out_arena = Arena::new(&mut out_buf);
                let arena_ir = csr
                    .compute_layout_arena(&cfg, &mut temp_arena, &mut out_arena)
                    .unwrap();
                check_scene(&arena_ir, &PlanOptions::new(), dir, what);
            }
        }
    }

    pub(super) fn check_scene<L: super::super::scene::LayoutSource>(
        ir: &L,
        options: &PlanOptions,
        dir: crate::graph::Direction,
        what: &str,
    ) {
        let mut planner = ScenePlanner::new();
        let scene = planner.plan(ir, options).unwrap();
        for cap in [usize::MAX, 3] {
            let req = scene.composition_requirements(&budget(cap));
            let mut composer = SceneComposer::new(req);
            let mut rows: Vec<String> = vec![String::new(); scene.height()];
            composer
                .visit_cells(&scene, |x, y, cell| {
                    assert_eq!(
                        cell.owner,
                        scene.hit_test(x, y),
                        "{what} {dir:?} cap={cap}: owner vs hit_test at ({x},{y})"
                    );
                    rows[y].push(glyph(&cell.kind));
                })
                .unwrap();
            let assembled: Vec<&str> = rows.iter().map(|r| r.trim_end()).collect();
            let mut opts = RenderOptions::plain();
            opts.plan = *options;
            let rendered = render_plain_text(ir, &opts);
            let rendered_rows: Vec<&str> = rendered.lines().collect();
            assert_eq!(
                assembled, rendered_rows,
                "{what} {dir:?} cap={cap}: glyphs vs plain render"
            );
        }
    }

    fn render_plain_text<L: super::super::scene::LayoutSource>(
        ir: &L,
        opts: &RenderOptions,
    ) -> String {
        // Both IRs carry render_string; dispatch through the sealed
        // source the same way the planner does.
        use super::super::scene::ViewRef;
        let mut planner = ScenePlanner::new();
        let scene = planner.plan(ir, &opts.plan).unwrap();
        match *scene.view() {
            ViewRef::Heap(v) => v.render_string(opts),
            ViewRef::Arena(v) => {
                let mut out = String::new();
                v.render_with(opts, &mut out).unwrap();
                out
            }
        }
    }

    // ── Capacity / reuse / allocation gates ──────────────────────────

    /// Steady-state repaint through a fitting composer — ownership
    /// rasterization included — performs ZERO allocations.
    #[test]
    fn steady_state_repaint_allocates_nothing() {
        let ir = clusters_graph().compute_layout();
        let mut planner = ScenePlanner::new();
        let scene = planner.plan(&ir, &PlanOptions::new()).unwrap();
        let mut composer = SceneComposer::new(scene.composition_requirements(&budget(64)));
        let mut acc = 0u64;
        composer.visit_cells(&scene, |_, _, _| {}).unwrap(); // warm-up

        let before = allocations_on_this_thread();
        for _ in 0..50 {
            composer
                .visit_cells(&scene, |x, y, c| {
                    acc = acc
                        .wrapping_add(x as u64)
                        .wrapping_add(y as u64)
                        .wrapping_add(matches!(c.kind, CellKind::Empty) as u64);
                })
                .unwrap();
        }
        assert_eq!(
            allocations_on_this_thread() - before,
            0,
            "steady-state repaint allocated"
        );
        std::hint::black_box(acc);
    }

    /// One composer across a scene switch: smaller scenes reuse the
    /// chunk allocation-free; a larger scene grows it; after growth
    /// the gate holds again.
    #[test]
    fn scene_switch_reuses_workspace() {
        let small_ir = stage_graph().compute_layout();
        let big_ir = fan(60).compute_layout();
        let options = PlanOptions::new();
        let mut planner = ScenePlanner::new();

        let mut composer = {
            let scene = planner.plan(&small_ir, &options).unwrap();
            let mut c = SceneComposer::new(scene.composition_requirements(&budget(64)));
            c.visit_cells(&scene, |_, _, _| {}).unwrap(); // warm-up
            c
        };

        // Repaints of the fitting scene: zero allocations.
        let scene = planner.plan(&small_ir, &options).unwrap();
        let before = allocations_on_this_thread();
        for _ in 0..10 {
            composer.visit_cells(&scene, |_, _, _| {}).unwrap();
        }
        assert_eq!(allocations_on_this_thread() - before, 0);
        drop(scene);

        // Larger scene grows the chunk; repaints hold the gate again.
        let scene = planner.plan(&big_ir, &options).unwrap();
        composer.visit_cells(&scene, |_, _, _| {}).unwrap(); // grow
        let before = allocations_on_this_thread();
        for _ in 0..10 {
            composer.visit_cells(&scene, |_, _, _| {}).unwrap();
        }
        assert_eq!(
            allocations_on_this_thread() - before,
            0,
            "post-growth repaint allocated"
        );
    }

    /// A fixed workspace refuses a misfit scene at preflight — byte
    /// units, nothing carved — and keeps serving fitting scenes.
    #[test]
    fn fixed_workspace_errors_on_misfit_and_survives() {
        let small_ir = stage_graph().compute_layout();
        let big_ir = fan(60).compute_layout();
        let options = PlanOptions::new();
        let mut planner = ScenePlanner::new();

        let small_req = {
            let scene = planner.plan(&small_ir, &options).unwrap();
            scene.composition_requirements(&budget(64))
        };
        let mut ws = vec![0u8; small_req.workspace_bytes().unwrap()];
        let ws_len = ws.len();
        let mut composer = SceneComposer::new_in(small_req, &mut ws).unwrap();

        let scene = planner.plan(&big_ir, &options).unwrap();
        match composer.visit_cells(&scene, |_, _, _| panic!("must not visit")) {
            Err(GraphError::RenderWorkspaceTooSmall {
                needed_bytes,
                got_bytes,
            }) => {
                assert_eq!(got_bytes, ws_len);
                assert!(needed_bytes > got_bytes);
            }
            other => panic!("expected byte-unit workspace error, got {other:?}"),
        }
        drop(scene);

        let scene = planner.plan(&small_ir, &options).unwrap();
        let mut cells = 0usize;
        composer.visit_cells(&scene, |_, _, _| cells += 1).unwrap();
        assert_eq!(cells, scene.width() * scene.height());
    }

    /// Band budgets are memory behavior only: budgets 2 and 64 yield
    /// identical cell streams.
    #[test]
    fn band_budget_never_affects_the_stream() {
        let ir = hero_graph().compute_layout();
        let mut planner = ScenePlanner::new();
        let scene = planner.plan(&ir, &PlanOptions::new()).unwrap();

        type CellRecord = (usize, usize, String, CellColor, HitResult);
        let mut streams: Vec<Vec<CellRecord>> = Vec::new();
        for cap in [64usize, 2] {
            let mut composer = SceneComposer::new(scene.composition_requirements(&budget(cap)));
            let mut stream = Vec::new();
            composer
                .visit_cells(&scene, |x, y, c| {
                    stream.push((x, y, format!("{:?}", c.kind), c.color, c.owner));
                })
                .unwrap();
            streams.push(stream);
        }
        assert_eq!(streams[0], streams[1], "band budget changed the stream");
    }

    /// The early-exit form stops mid-stream and reports the consumer's
    /// break value, distinct from composer errors.
    #[test]
    fn try_visit_cells_breaks_early() {
        let ir = stage_graph().compute_layout();
        let mut planner = ScenePlanner::new();
        let scene = planner.plan(&ir, &PlanOptions::new()).unwrap();
        let mut composer = SceneComposer::new(scene.composition_requirements(&budget(64)));

        let mut seen = 0usize;
        let flow = composer
            .try_visit_cells(&scene, |x, y, _| {
                seen += 1;
                if (x, y) == (3, 1) {
                    ControlFlow::Break("stopped")
                } else {
                    ControlFlow::Continue(())
                }
            })
            .unwrap();
        assert_eq!(flow, ControlFlow::Break("stopped"));
        assert!(seen < scene.width() * scene.height());
    }
}

#[cfg(all(test, feature = "std", feature = "layout-vertical"))]
mod review_tests {
    use super::super::config::PlanOptions;
    use super::super::scene::ScenePlanner;
    use super::super::style::{SubgraphBorder, SubgraphStyle, SubgraphStyleCtx};
    use super::*;
    use crate::graph::Graph;
    use crate::render::engine::HitResult;

    fn budget(cap: usize) -> ComposeBudget {
        ComposeBudget::new().with_band_rows_cap(cap)
    }

    /// Borderless clusters and shown dummies hold the agreement gate
    /// too — and shown dummies surface as `HitResult::Dummy` with
    /// their SEMANTIC identity (input edge + level), never a
    /// synthetic backend id.
    #[test]
    fn agreement_covers_borderless_and_shown_dummies() {
        fn borderless(_ctx: SubgraphStyleCtx<'_>) -> SubgraphStyle {
            SubgraphStyle {
                border: SubgraphBorder::None,
                ..Default::default()
            }
        }
        let mut g = Graph::new();
        g.add_node(1usize, "outer");
        g.add_node(2usize, "inner");
        g.add_node(3usize, "leaf");
        g.add_edge(1usize, 2usize, None);
        g.add_edge(2usize, 3usize, None);
        let outer = g.add_subgraph("Outer");
        let inner = g.add_subgraph("Inner");
        g.put_nodes(&[1, 2, 3]).inside(inner).unwrap();
        g.put_subgraphs(&[inner]).inside(outer).unwrap();
        let ir = g.compute_layout();
        super::tests::check_scene(
            &ir,
            &PlanOptions::new().with_subgraph_style_fn(borderless),
            crate::graph::Direction::TopDown,
            "borderless nested",
        );

        // A self-loop BEFORE a skip edge: input and scene edge
        // indices diverge, so this pins that dummy identity follows
        // the INPUT convention.
        let mut g = Graph::new();
        g.add_node(1usize, "A");
        g.add_node(2usize, "B");
        g.add_node(3usize, "C");
        g.add_edge(1usize, 1usize, None); // self-loop: input 0, no scene index
        g.add_edge(1usize, 2usize, None); // input 1
        g.add_edge(2usize, 3usize, None); // input 2
        g.add_edge(1usize, 3usize, None); // input 3: skip edge → dummy
        let mut cfg = crate::LayoutConfig::standard();
        cfg.include_dummy_nodes = true;
        let ir = g.compute_layout_with_config(&cfg);
        let options = PlanOptions::new().with_show_dummy_nodes(true);
        super::tests::check_scene(
            &ir,
            &options,
            crate::graph::Direction::TopDown,
            "shown dummies",
        );

        let mut planner = ScenePlanner::new();
        let scene = planner.plan(&ir, &options).unwrap();
        let mut composer = SceneComposer::new(scene.composition_requirements(&budget(64)));
        let mut dummy_owners = Vec::new();
        composer
            .visit_cells(&scene, |_, _, cell| {
                if let HitResult::Dummy { edge, level } = cell.owner {
                    dummy_owners.push((edge, level));
                }
            })
            .unwrap();
        assert!(!dummy_owners.is_empty(), "skip edge must surface a dummy");
        for &(edge, _) in &dummy_owners {
            assert_eq!(edge, 3, "dummy identity uses the INPUT edge index");
        }
        // The views agree: same identity pair on the NodeView side.
        let dummies: Vec<(usize, usize)> = scene.nodes().filter_map(|n| n.dummy_of).collect();
        assert!(dummy_owners.iter().all(|d| dummies.contains(d)));
    }

    /// Absurd hand-built dimensions: the requirement reports
    /// unfittable, and BOTH composer modes error instead of panicking
    /// or attempting a `usize::MAX` allocation.
    #[test]
    fn absurd_dimensions_error_instead_of_panicking() {
        let mut b = crate::ir::LayoutIRBuilder::new().with_levels(1);
        b.add_node(crate::ir::LayoutNode {
            id: 0,
            label: "a",
            x: 0,
            y: 0,
            width: 3,
            height: 1,
            center_x: 1,
            center_y: 0,
            level: 0,
            level_position: 0,
            kind: crate::ir::NodeKind::Explicit,
            has_self_loop: false,
            self_loop_at: None,
            edge_index: None,
            content_tag: 0,
        });
        b.set_dimensions(usize::MAX, 1);
        let ir = b.build();
        let mut planner = ScenePlanner::new();
        let scene = planner.plan(&ir, &PlanOptions::new()).unwrap();
        let req = scene.composition_requirements(&budget(64));
        assert_eq!(req.workspace_bytes(), None, "requirement must overflow");

        let mut composer = SceneComposer::new(req);
        assert!(matches!(
            composer.visit_cells(&scene, |_, _, _| {}),
            Err(GraphError::RenderWorkspaceTooSmall {
                needed_bytes: usize::MAX,
                ..
            })
        ));
        let mut ws = [0u8; 64];
        assert!(matches!(
            SceneComposer::new_in(req, &mut ws),
            Err(GraphError::RenderWorkspaceTooSmall { .. })
        ));
    }
}
