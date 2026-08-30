//! Spike: composer capacity and reuse for the 0.11 scene work
//! (prototype stage — see temp/scene-api-sketch.md §9 and
//! temp/spike-4.0e-findings.md).
//!
//! **THROWAWAY CODE.** Test-only, deleted when the real
//! `SceneComposer` lands. Questions:
//!
//! 1. What is the `CompositionRequirements` shape — scene-derived
//!    workspace terms for two explicit profiles: the PUBLIC semantic
//!    composer (always cells + colors + owners, spike 4.0b's
//!    rasterizer EXECUTED in every band) and a private lean terminal
//!    profile (cells only; colors only for colored output; no
//!    ownership) — and is its layout calculation exact (checked
//!    arithmetic, mirroring the real carve sequence)?
//! 2. Reuse across scenes: does one retained workspace serve any scene
//!    whose requirements fit — growing on the heap when they don't,
//!    and failing preflight with a byte-unit workspace error in arena
//!    mode?
//! 3. Does the R6 allocation gate hold — zero allocations at steady
//!    state, across repaints AND scene switches, with ownership
//!    rasterization running — growth being the only counted event?
//! 4. Is the band partition a pure `ComposeBudget` choice — same plan,
//!    different budgets, byte-identical output and owner agreement?
//!
//! The composer model: ONE retained byte chunk; every per-compose
//! buffer is carved from it by a fresh bump-`Arena` each call. Carving
//! is pointer arithmetic — the global allocator is never touched
//! unless the chunk itself must grow.

use super::cell::Cell;
use super::color::CellColor;
use super::compose::{BandCanvas, PaintScratch, composite_band};
use super::ownership_spike::{
    OwnerScratch, OwnerSweep, owner_incidence_capacity, owner_prepare, owner_rasterize_band,
    owner_to_hit,
};
use super::plan::RenderPlan;
use super::view::LayoutView;
use crate::RenderOptions;
use crate::graph::Graph;
use crate::graph::arena::Arena;
use crate::render::colors::Palette;

use super::test_alloc::allocations_on_this_thread;

// ── CompositionRequirements prototype ────────────────────────────────────

/// Which workspace profile a composer provisions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Profile {
    /// The public `SceneComposer`: always cells + colors + ownership
    /// (color-complete `CellView`s and `owner` answers are the
    /// contract, whatever the emission mode).
    Semantic,
    /// Private terminal fast path (`render_to_bytes` today): cells
    /// only; a color plane only when the output is colored; no
    /// ownership. This cost never becomes an unavoidable part of the
    /// byte surface.
    Lean { colored: bool },
}

/// Composition-resource choices — NOT scene identity. The band
/// partition lives here (sequencing 4.2's `ComposeBudget`), never in
/// the plan.
#[derive(Debug, Clone, Copy)]
struct BudgetSpike {
    band_rows: usize,
}

/// Scene-derived workspace descriptor. Cardinalities plus plan
/// geometry: run scratch by the plan's exact `run_capacity`, paint
/// scratch by element counts, ownership tables by width/elements, and
/// the row-incidence bound by an O(elements) sweep of element spans.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ReqSpike {
    width: usize,
    height: usize,
    band_rows: usize,
    run_capacity: usize,
    nodes: usize,
    edges: usize,
    subgraphs: usize,
    elements: usize,
    incidence: usize,
    profile: Profile,
}

impl ReqSpike {
    fn of<V: LayoutView>(
        view: &V,
        plan: &RenderPlan<'_>,
        budget: &BudgetSpike,
        profile: Profile,
    ) -> Self {
        let height = plan.height().max(1);
        let band_rows = budget.band_rows.clamp(1, height);
        Self {
            width: plan.width(),
            height,
            band_rows,
            run_capacity: plan.run_capacity(),
            nodes: view.node_count(),
            edges: view.edge_count(),
            subgraphs: view.subgraph_count(),
            elements: plan.elements().len(),
            incidence: owner_incidence_capacity(plan, band_rows),
            profile,
        }
    }

    /// Whether the paint scratch and canvas carry the color machinery.
    fn scratch_colored(&self) -> bool {
        match self.profile {
            Profile::Semantic => true, // always color-complete
            Profile::Lean { colored } => colored,
        }
    }

    /// Exact workspace byte budget: checked arithmetic over the SAME
    /// `(bytes, align)` sequence the carves perform, in carve order,
    /// plus max-align headroom for the chunk's base pointer. `None`
    /// means unfittable (overflow).
    fn workspace_bytes(&self) -> Option<usize> {
        use core::mem::{align_of, size_of};
        let area = self.width.checked_mul(self.band_rows)?;
        let u32_layout = |count: usize| (count.checked_mul(size_of::<u32>()), align_of::<u32>());

        // Fixed-size term list (no allocation — this runs on every
        // compose preflight): the PaintScratch carves in their real
        // order, then canvas, color plane, and the ownership carves.
        let semantic = matches!(self.profile, Profile::Semantic);
        let scratch_terms = PaintScratch::carve_layout(
            self.run_capacity,
            self.subgraphs,
            self.edges,
            self.nodes,
            self.scratch_colored(),
            self.width,
            self.band_rows,
        );
        let extra_terms: [(Option<usize>, usize); 10] = [
            (area.checked_mul(size_of::<Cell>()), align_of::<Cell>()),
            if self.scratch_colored() {
                (
                    area.checked_mul(size_of::<CellColor>()),
                    align_of::<CellColor>(),
                )
            } else {
                (Some(0), 1)
            },
            if semantic {
                u32_layout(area)
            } else {
                (Some(0), 1)
            }, // owner plane
            if semantic {
                u32_layout(self.width.checked_add(1)?)
            } else {
                (Some(0), 1)
            }, // claim
            if semantic {
                u32_layout(self.edges)
            } else {
                (Some(0), 1)
            }, // edge_slot
            if semantic {
                u32_layout(self.elements)
            } else {
                (Some(0), 1)
            }, // by_y_min
            if semantic {
                u32_layout(self.elements)
            } else {
                (Some(0), 1)
            }, // active
            if semantic {
                u32_layout(self.band_rows.checked_add(1)?)
            } else {
                (Some(0), 1)
            }, // row_off
            if semantic {
                u32_layout(self.band_rows)
            } else {
                (Some(0), 1)
            }, // row_cur
            if semantic {
                u32_layout(self.incidence)
            } else {
                (Some(0), 1)
            }, // row_inc
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

// ── Errors ───────────────────────────────────────────────────────────────

/// Composer-level failure. Workspace exhaustion carries BYTE units
/// explicitly — the existing `RenderCanvasTooSmall` documents cell
/// counts, and callers must never infer units from the operation (the
/// real 0.11 surface adds a workspace error kind with these fields).
#[derive(Debug)]
enum ComposeError {
    WorkspaceTooSmall {
        needed_bytes: usize,
        got_bytes: usize,
    },
    Graph(#[allow(dead_code)] crate::GraphError), // read via Debug in test failures
}

impl From<crate::GraphError> for ComposeError {
    fn from(e: crate::GraphError) -> Self {
        ComposeError::Graph(e)
    }
}

// ── The compose core ─────────────────────────────────────────────────────

/// Everything one compose call carves and runs. The band partition
/// comes from the requirements (budget), NOT the plan: bands are
/// generated here, ascending, tiling `0..height`. `out` must have
/// capacity reserved by the caller.
fn compose_carved<V: LayoutView>(
    view: &V,
    plan: &RenderPlan<'_>,
    options: &RenderOptions,
    req: &ReqSpike,
    arena: &Arena<'_>,
    out: &mut String,
    verify_owners: bool,
) -> Result<(), ComposeError> {
    let area = req.width * req.band_rows;
    let oom = || crate::GraphError::RenderCanvasTooSmall {
        needed: area,
        got: 0,
    };
    let scratch_colored = req.scratch_colored();
    let emit_colored = !matches!(options.emit.color_mode, super::color::ColorMode::None);

    let mut scratch = PaintScratch::carve(view, plan, scratch_colored, req.band_rows, arena)?;
    let cells = arena
        .alloc_slice_default::<Cell>(area)
        .ok_or_else(oom)
        .map_err(ComposeError::Graph)?;
    let mut colors = if scratch_colored {
        Some(
            arena
                .alloc_slice_default::<CellColor>(area)
                .ok_or_else(oom)
                .map_err(ComposeError::Graph)?,
        )
    } else {
        None
    };

    // Semantic profile: the 4.0b ownership rasterizer runs in every
    // band, over the SAME arena-carved scratch shape the spike's
    // agreement corpus exercises.
    let mut ownership = if matches!(req.profile, Profile::Semantic) {
        let carve_u32 = |n: usize| {
            arena
                .alloc_slice_default::<u32>(n)
                .ok_or_else(oom)
                .map_err(ComposeError::Graph)
        };
        let plane = carve_u32(area)?;
        let mut scratch = OwnerScratch {
            claim_next: carve_u32(req.width + 1)?,
            edge_slot: carve_u32(req.edges)?,
            by_y_min: carve_u32(req.elements)?,
            active: carve_u32(req.elements)?,
            row_off: carve_u32(req.band_rows + 1)?,
            row_cur: carve_u32(req.band_rows)?,
            row_inc: carve_u32(req.incidence)?,
        };
        let mut sweep = OwnerSweep::default();
        owner_prepare(plan, &mut scratch, &mut sweep);
        Some((plane, scratch, sweep))
    } else {
        None
    };

    let mut y0 = 0;
    while y0 < req.height {
        let rows = req.band_rows.min(req.height - y0);
        let mut canvas = BandCanvas::new(cells, colors.as_deref_mut(), req.width, y0, rows);
        composite_band(view, plan, options, &mut canvas, &mut scratch);
        if let Some((plane, owner_scratch, sweep)) = ownership.as_mut() {
            let band_plane = &mut plane[..req.width * rows];
            owner_rasterize_band(
                plan,
                view,
                y0,
                y0 + rows,
                req.width,
                band_plane,
                owner_scratch,
                sweep,
            );
            if verify_owners {
                for y in y0..y0 + rows {
                    for x in 0..req.width {
                        let got = owner_to_hit(plan, view, band_plane[(y - y0) * req.width + x]);
                        let want = plan.element_at(view, x, y);
                        assert_eq!(got, want, "owner plane disagreement at ({x},{y})");
                    }
                }
            }
        }
        let written = if emit_colored {
            super::emit::emit_colored_band(
                &canvas,
                options.emit.charset,
                options.emit.color_mode,
                out,
            )
        } else {
            super::emit::emit_plain_band(&canvas, options.emit.charset, out)
        };
        written.expect("string sink never fails");
        y0 += rows;
    }
    if options.emit.render_legend {
        super::emit::emit_legend(
            view,
            plan,
            options.emit.charset,
            options.emit.color_mode,
            out,
        )
        .expect("string sink never fails");
    }
    Ok(())
}

/// Heap composer: retained chunk, grows to fit any scene (preflight,
/// checked), then serves it allocation-free.
struct HeapComposerSpike {
    ws: Vec<u8>,
    grows: usize,
}

impl HeapComposerSpike {
    fn new(req: &ReqSpike) -> Self {
        Self {
            ws: vec![0u8; req.workspace_bytes().expect("fittable requirements")],
            grows: 0,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn compose<V: LayoutView>(
        &mut self,
        view: &V,
        plan: &RenderPlan<'_>,
        options: &RenderOptions,
        budget: &BudgetSpike,
        profile: Profile,
        out: &mut String,
        verify_owners: bool,
    ) -> Result<(), ComposeError> {
        let req = ReqSpike::of(view, plan, budget, profile);
        let need = req
            .workspace_bytes()
            .ok_or(ComposeError::WorkspaceTooSmall {
                needed_bytes: usize::MAX,
                got_bytes: self.ws.len(),
            })?;
        if need > self.ws.len() {
            self.ws.resize(need, 0); // the ONLY allocating event
            self.grows += 1;
        }
        let arena = Arena::new(&mut self.ws);
        compose_carved(view, plan, options, &req, &arena, out, verify_owners)
    }
}

/// Arena-mode composer: caller-provided fixed workspace; a scene that
/// does not fit fails preflight — byte units, nothing carved.
struct ArenaComposerSpike<'ws> {
    ws: &'ws mut [u8],
}

impl ArenaComposerSpike<'_> {
    fn compose<V: LayoutView>(
        &mut self,
        view: &V,
        plan: &RenderPlan<'_>,
        options: &RenderOptions,
        budget: &BudgetSpike,
        profile: Profile,
        out: &mut String,
    ) -> Result<(), ComposeError> {
        let req = ReqSpike::of(view, plan, budget, profile);
        let needed_bytes = req.workspace_bytes().unwrap_or(usize::MAX);
        if needed_bytes > self.ws.len() {
            return Err(ComposeError::WorkspaceTooSmall {
                needed_bytes,
                got_bytes: self.ws.len(),
            });
        }
        let arena = Arena::new(self.ws);
        compose_carved(view, plan, options, &req, &arena, out, false)
    }
}

// ── Fixtures ─────────────────────────────────────────────────────────────

fn small_graph() -> Graph<'static> {
    let mut g = Graph::new();
    g.add_node(1usize, "Start");
    g.add_node(2usize, "Middle");
    g.add_node(3usize, "End");
    g.add_edge(1usize, 2usize, Some("go"));
    g.add_edge(2usize, 3usize, None);
    g
}

fn cluster_graph() -> Graph<'static> {
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
    g.add_edge(2usize, 3usize, Some("cross"));
    g
}

fn fan_graph(n: usize) -> Graph<'static> {
    let mut g = Graph::new();
    g.add_node(0usize, "Root");
    for i in 1..=n {
        g.add_node(i, "leaf");
        g.add_edge(0usize, i, None);
    }
    g
}

fn plain() -> RenderOptions {
    RenderOptions::plain()
}

fn colored_legend() -> RenderOptions {
    // The colored preset now carries the whole legacy pair explicitly:
    // AvoidNodeRows + Legend overflow + printed legend block.
    RenderOptions::colored(Palette::Ansi)
}

/// The engine's own band partition, as a budget — makes byte
/// comparisons against `render_string` band-structure-identical.
fn default_budget(plan: &RenderPlan<'_>) -> BudgetSpike {
    BudgetSpike {
        band_rows: plan.max_band_rows(super::config::DEFAULT_BAND_ROWS).max(1),
    }
}

fn reserve_for<V: LayoutView>(view: &V, options: &RenderOptions) -> String {
    let colored = !matches!(options.emit.color_mode, super::color::ColorMode::None);
    String::with_capacity(
        super::emit::estimate_output_size(view, colored, options.emit.render_legend) * 2,
    )
}

// ── The proofs ───────────────────────────────────────────────────────────

/// Question 1: the checked, carve-mirrored `workspace_bytes()` is
/// sufficient — a workspace of EXACTLY that size composes every corpus
/// scene in BOTH profiles and both emission modes, byte-identical to
/// the shipping render path, with the semantic profile's owner plane
/// EXECUTED and validated against `element_at` cell for cell. The
/// arena backend runs the same composer (parity discipline).
#[test]
fn requirements_bytes_are_sufficient_and_output_correct() {
    for (graph, what) in [
        (small_graph(), "small"),
        (cluster_graph(), "clusters"),
        (fan_graph(40), "fan-40"),
    ] {
        for options in [plain(), colored_legend()] {
            let colored = !matches!(options.emit.color_mode, super::color::ColorMode::None);
            for profile in [Profile::Semantic, Profile::Lean { colored }] {
                let ir = graph.compute_layout();
                let plan = RenderPlan::build(&ir, &options.plan);
                let budget = default_budget(&plan);
                let req = ReqSpike::of(&ir, &plan, &budget, profile);
                let mut ws = vec![0u8; req.workspace_bytes().unwrap()];
                let mut out = reserve_for(&ir, &options);
                if matches!(profile, Profile::Semantic) {
                    // Exercise verification through the heap path (the
                    // arena path shares compose_carved).
                    let arena = Arena::new(&mut ws);
                    compose_carved(&ir, &plan, &options, &req, &arena, &mut out, true)
                        .unwrap_or_else(|e| panic!("{what}: exact-size semantic failed: {e:?}"));
                } else {
                    let mut composer = ArenaComposerSpike { ws: &mut ws };
                    composer
                        .compose(&ir, &plan, &options, &budget, profile, &mut out)
                        .unwrap_or_else(|e| panic!("{what}: exact-size lean failed: {e:?}"));
                }
                assert_eq!(
                    out,
                    ir.render_string(&options),
                    "{what} bytes diverged ({profile:?})"
                );
            }
        }
    }

    // Arena backend through the same composer, semantic profile.
    let g = small_graph();
    let mut csr_buf = vec![0u8; g.estimate_csr_arena_size() * 2];
    let mut csr_arena = Arena::new(&mut csr_buf);
    let csr = g.to_csr(&mut csr_arena).unwrap();
    let size = (g.estimate_layout_arena_size() * 2).max(256 * 1024);
    let mut temp_buf = vec![0u8; size];
    let mut out_buf = vec![0u8; size];
    let mut temp_arena = Arena::new(&mut temp_buf);
    let mut out_arena = Arena::new(&mut out_buf);
    let cfg = crate::LayoutConfig::standard();
    let arena_ir = csr
        .compute_layout_arena(&cfg, &mut temp_arena, &mut out_arena)
        .unwrap();
    let options = plain();
    let plan = RenderPlan::build(&arena_ir, &options.plan);
    let budget = default_budget(&plan);
    let req = ReqSpike::of(&arena_ir, &plan, &budget, Profile::Semantic);
    let mut ws = vec![0u8; req.workspace_bytes().unwrap()];
    let arena = Arena::new(&mut ws);
    let mut out = reserve_for(&arena_ir, &options);
    compose_carved(&arena_ir, &plan, &options, &req, &arena, &mut out, true).unwrap();
    assert_eq!(out, small_graph().compute_layout().render_string(&options));
}

/// Question 3, first half: steady-state repaint through a fitting
/// SEMANTIC composer — ownership rasterization running every band —
/// performs ZERO allocations, plain and colored+legend both.
#[test]
fn steady_state_repaint_allocates_nothing() {
    for options in [plain(), colored_legend()] {
        let ir = cluster_graph().compute_layout();
        let plan = RenderPlan::build(&ir, &options.plan);
        let budget = default_budget(&plan);
        let req = ReqSpike::of(&ir, &plan, &budget, Profile::Semantic);
        let mut composer = HeapComposerSpike::new(&req);
        let mut out = reserve_for(&ir, &options);

        composer
            .compose(
                &ir,
                &plan,
                &options,
                &budget,
                Profile::Semantic,
                &mut out,
                false,
            )
            .unwrap(); // warm-up

        let before = allocations_on_this_thread();
        for _ in 0..50 {
            out.clear(); // keeps capacity
            composer
                .compose(
                    &ir,
                    &plan,
                    &options,
                    &budget,
                    Profile::Semantic,
                    &mut out,
                    false,
                )
                .unwrap();
        }
        let after = allocations_on_this_thread();
        assert_eq!(after - before, 0, "steady-state repaint allocated");
        assert_eq!(composer.grows, 0);
    }
}

/// Questions 2 and 3, second half: one semantic composer across a
/// scene-switch sequence — smaller scenes reuse the chunk
/// allocation-free, a larger scene grows it exactly once, and repaints
/// after the growth are allocation-free again.
#[test]
fn scene_switch_reuses_grows_once_then_holds_gate() {
    let options = plain();
    let profile = Profile::Semantic;
    let big_ir = fan_graph(60).compute_layout();
    let big_plan = RenderPlan::build(&big_ir, &options.plan);
    let small_ir = small_graph().compute_layout();
    let small_plan = RenderPlan::build(&small_ir, &options.plan);
    let mid_ir = cluster_graph().compute_layout();
    let mid_plan = RenderPlan::build(&mid_ir, &options.plan);

    let budget = BudgetSpike { band_rows: 8 };
    let mid_req = ReqSpike::of(&mid_ir, &mid_plan, &budget, profile);
    let small_need = ReqSpike::of(&small_ir, &small_plan, &budget, profile)
        .workspace_bytes()
        .unwrap();
    let big_need = ReqSpike::of(&big_ir, &big_plan, &budget, profile)
        .workspace_bytes()
        .unwrap();
    let mid_need = mid_req.workspace_bytes().unwrap();
    assert!(small_need <= mid_need, "corpus ordering");
    assert!(big_need > mid_need, "corpus ordering");

    let mut composer = HeapComposerSpike::new(&mid_req);
    let mut out = String::with_capacity(64 * 1024);
    composer
        .compose(
            &mid_ir, &mid_plan, &options, &budget, profile, &mut out, false,
        )
        .unwrap(); // warm-up

    // Smaller scene, then back: pure reuse, zero allocations.
    let before = allocations_on_this_thread();
    for _ in 0..10 {
        out.clear();
        composer
            .compose(
                &small_ir,
                &small_plan,
                &options,
                &budget,
                profile,
                &mut out,
                false,
            )
            .unwrap();
        out.clear();
        composer
            .compose(
                &mid_ir, &mid_plan, &options, &budget, profile, &mut out, false,
            )
            .unwrap();
    }
    assert_eq!(
        allocations_on_this_thread() - before,
        0,
        "switching between fitting scenes allocated"
    );
    assert_eq!(composer.grows, 0);

    // Larger scene: grows exactly once...
    out.clear();
    composer
        .compose(
            &big_ir, &big_plan, &options, &budget, profile, &mut out, false,
        )
        .unwrap();
    assert_eq!(composer.grows, 1);

    // ...and the gate holds again at the new high-water mark.
    let before = allocations_on_this_thread();
    for _ in 0..10 {
        out.clear();
        composer
            .compose(
                &big_ir, &big_plan, &options, &budget, profile, &mut out, false,
            )
            .unwrap();
        out.clear();
        composer
            .compose(
                &small_ir,
                &small_plan,
                &options,
                &budget,
                profile,
                &mut out,
                false,
            )
            .unwrap();
    }
    assert_eq!(
        allocations_on_this_thread() - before,
        0,
        "post-growth repaint allocated"
    );
    assert_eq!(composer.grows, 1);
}

/// Question 2, arena half: a fixed workspace refuses a misfit scene at
/// preflight with BYTE units — nothing carved, nothing written — and
/// keeps serving fitting scenes afterwards.
#[test]
fn arena_composer_errors_on_misfit_and_survives() {
    let options = plain();
    let profile = Profile::Semantic;
    let small_ir = small_graph().compute_layout();
    let small_plan = RenderPlan::build(&small_ir, &options.plan);
    let budget = default_budget(&small_plan);
    let small_req = ReqSpike::of(&small_ir, &small_plan, &budget, profile);
    let big_ir = fan_graph(60).compute_layout();
    let big_plan = RenderPlan::build(&big_ir, &options.plan);

    let ws_len = small_req.workspace_bytes().unwrap();
    let mut ws = vec![0u8; ws_len];
    let mut composer = ArenaComposerSpike { ws: &mut ws };
    let mut out = String::with_capacity(64 * 1024);

    let big_budget = default_budget(&big_plan);
    let err = composer
        .compose(&big_ir, &big_plan, &options, &big_budget, profile, &mut out)
        .unwrap_err();
    match err {
        ComposeError::WorkspaceTooSmall {
            needed_bytes,
            got_bytes,
        } => {
            assert_eq!(got_bytes, ws_len);
            assert!(needed_bytes > got_bytes);
        }
        other => panic!("expected byte-unit workspace error, got {other:?}"),
    }
    assert!(out.is_empty(), "misfit must fail before writing");

    composer
        .compose(&small_ir, &small_plan, &options, &budget, profile, &mut out)
        .unwrap();
    assert_eq!(out, small_ir.render_string(&options));
}

/// Question 4: the band partition is a pure budget choice — SAME plan,
/// budget of 2 rows vs the engine default, byte-identical output in
/// both emission modes, with the tiny-budget owner plane still
/// agreeing with `element_at` (multi-band rasterization).
#[test]
fn band_budget_never_affects_composed_bytes() {
    for options in [plain(), colored_legend()] {
        let ir = cluster_graph().compute_layout();
        let plan = RenderPlan::build(&ir, &options.plan);
        let tiny = BudgetSpike { band_rows: 2 };
        let full = default_budget(&plan);
        assert!(full.band_rows > tiny.band_rows, "budget must differ");

        let req = ReqSpike::of(&ir, &plan, &full, Profile::Semantic);
        let mut composer = HeapComposerSpike::new(&req);

        let mut a = String::new();
        composer
            .compose(&ir, &plan, &options, &full, Profile::Semantic, &mut a, true)
            .unwrap();
        let mut b = String::new();
        composer
            .compose(&ir, &plan, &options, &tiny, Profile::Semantic, &mut b, true)
            .unwrap();
        assert_eq!(a, b, "band budget changed the composed bytes");
        assert_eq!(a, ir.render_string(&options));
    }
}
