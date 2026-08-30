//! Spike: the `ScenePlanner` → `Scene` borrow-and-capacity model
//! (0.11 scene work, prototype stage — see temp/scene-api-sketch.md §9).
//!
//! **THROWAWAY CODE.** This module is test-only, ships in no build, and
//! is deleted when the real planner lands. Its only product is proof:
//! each test pins one architectural claim the public API design relies
//! on, against the real `LayoutView` / `PlanBuf` / `Arena` machinery.
//!
//! Claims proven here:
//!
//! 1. A heap planner can retain resolved-storage capacity across plans,
//!    with `plan(&mut self)` enforcing exactly one live scene — and
//!    re-planning after the scene drops reuses the allocation
//!    (`heap_planner_retains_capacity_across_plans`).
//! 2. The plan-once/compose-many consumption shape works as nested
//!    loops without storing the scene in a struct
//!    (`event_loop_shape_composes_many_frames_per_plan`).
//! 3. One `plan` generic over `LayoutView` serves both IRs — no
//!    `plan`/`plan_arena` split needed
//!    (`one_generic_plan_serves_both_backends`).
//! 4. The diagnostics-context borrow interleaves correctly: the run can
//!    absorb events during planning, stay usable while the scene lives,
//!    and finish after the scene drops — on success and error paths
//!    (`diagnostic_run_outlives_scene_on_success_and_error`).
//! 5. An arena planner can rebuild into caller storage by resetting its
//!    arena between plans — BUT soundness is entirely API discipline:
//!    `Arena::reset` is `unsafe fn(&self)`, and carved slices carry the
//!    arena's *buffer* lifetime, not the planner borrow. The scene must
//!    deliberately shorten those slices to the `&mut self` borrow so
//!    the compiler forbids a stale scene across a reset
//!    (`arena_planner_resets_between_plans`). A scene holding the raw
//!    `'buf` lifetime instead would compile while allowing
//!    use-after-reset — the real implementation must encapsulate this
//!    shortening and never leak `'buf`. (Negative case checked by hand
//!    during the spike: extending the slice lifetime to `'buf` and
//!    keeping the old scene across `plan()` is accepted by the borrow
//!    checker — the hazard is real.)
//!
//! Re-plan cost (the framework-callback fallback where a scene cannot
//! be stored in `self`): `replan_cost_report` (`#[ignore]`, run
//! manually) measures real `RenderPlan::build` on the hero graph.

use super::plan::{EdgePlan, LabelPlan, RenderPlan};
use super::view::LayoutView;
use crate::LayoutConfig;
use crate::graph::Graph;
use crate::graph::arena::Arena;
use crate::{GraphError, RenderOptions};

use crate as ascii_dag;

include!("../../../examples/shared/hero_graph.rs");

// ── Diagnostics stand-ins (the proposal's run/context split) ─────────────

struct DiagnosticRun {
    events: Vec<u32>,
}

struct DiagnosticContext<'a> {
    events: &'a mut Vec<u32>,
}

impl DiagnosticRun {
    fn new() -> Self {
        Self { events: Vec::new() }
    }
    fn context(&mut self) -> DiagnosticContext<'_> {
        DiagnosticContext {
            events: &mut self.events,
        }
    }
    fn finish<T, E>(self, outcome: Result<T, E>) -> (Result<T, E>, Vec<u32>) {
        (outcome, self.events)
    }
}

// ── Heap planner ─────────────────────────────────────────────────────────

/// Retained resolved-scene storage. The real planner retains the full
/// `RenderPlan` buffer set; the spike retains two representative ones —
/// the borrow shapes are identical.
#[derive(Default)]
struct ScenePlanner {
    edge_styles: Vec<EdgePlan>,
    labels: Vec<LabelPlan>,
}

/// The scene: resolved data borrowed from the planner (`'p`), geometry
/// borrowed from the input IR (`'ir`). Two lifetimes by design — the
/// borrow checker is the compatibility fingerprint.
struct Scene<'p, 'ir, V: LayoutView> {
    edges: &'p [EdgePlan],
    #[allow(dead_code)]
    labels: &'p [LabelPlan],
    view: &'ir V,
}

impl ScenePlanner {
    fn new() -> Self {
        Self::default()
    }

    /// `&mut self` IS the ownership model: exactly one live scene per
    /// planner, enforced at compile time. Style resolution runs here
    /// once; the spike fills defaults (resolution correctness is the
    /// engine's, not this prototype's).
    fn plan<'p, 'ir, V: LayoutView>(
        &'p mut self,
        view: &'ir V,
        cx: &mut DiagnosticContext<'_>,
    ) -> Result<Scene<'p, 'ir, V>, GraphError> {
        self.edge_styles.clear(); // capacity retained
        self.labels.clear();
        for _ in 0..view.edge_count() {
            self.edge_styles.push(EdgePlan::default());
        }
        cx.events.push(1); // a planning diagnostic
        Ok(Scene {
            edges: &self.edge_styles,
            labels: &self.labels,
            view,
        })
    }
}

impl<V: LayoutView> Scene<'_, '_, V> {
    fn edge_count(&self) -> usize {
        self.edges.len()
    }
    /// Stands in for composition: touches both borrows.
    fn compose_frame(&self) -> usize {
        self.view.node_count() + self.edges.len()
    }
}

// ── Arena planner ────────────────────────────────────────────────────────

/// Planner over caller storage. Owns the `Arena` (which borrows the
/// caller's buffer), resets it on every plan.
struct ArenaScenePlanner<'buf> {
    arena: Arena<'buf>,
}

/// NOTE the deliberate lifetime shortening: `edges` is `&'p [EdgePlan]`
/// — the planner-borrow lifetime — even though the carve produced a
/// `'buf`-lifetime slice. This is what makes `plan(&mut self)` after a
/// previous scene drops the ONLY way to reset: a scene carrying `'buf`
/// would survive the reset and read recycled memory.
struct ArenaScene<'p, 'ir, V: LayoutView> {
    edges: &'p [EdgePlan],
    view: &'ir V,
}

impl<V: LayoutView> ArenaScene<'_, '_, V> {
    fn compose_frame(&self) -> usize {
        self.view.node_count() + self.edges.len()
    }
}

impl<'buf> ArenaScenePlanner<'buf> {
    fn new_in(storage: &'buf mut [u8]) -> Self {
        Self {
            arena: Arena::new(storage),
        }
    }

    fn plan<'p, 'ir, V: LayoutView>(
        &'p mut self,
        view: &'ir V,
        cx: &mut DiagnosticContext<'_>,
    ) -> Result<ArenaScene<'p, 'ir, V>, GraphError> {
        // SAFETY: `&mut self` guarantees no scene from a previous plan
        // is alive (scenes borrow `'p`), so no carved slice is
        // reachable when the arena recycles its memory.
        unsafe { self.arena.reset() };
        let mut edges =
            super::mem::PlanBuf::carve(&self.arena, view.edge_count(), GraphError::ArenaOom)?;
        for _ in 0..view.edge_count() {
            edges.push(EdgePlan::default());
        }
        cx.events.push(2);
        // Shorten 'buf → 'p: covariant, safe, and the whole point.
        let edges: &'p [EdgePlan] = match edges {
            super::mem::PlanBuf::Slice { data, len } => &data[..len],
            #[cfg(feature = "alloc")]
            super::mem::PlanBuf::Heap(_) => unreachable!("carved"),
        };
        Ok(ArenaScene { edges, view })
    }
}

// ── The proofs ───────────────────────────────────────────────────────────

fn small_graph() -> Graph<'static> {
    let mut g = Graph::new();
    g.add_node(1usize, "A");
    g.add_node(2usize, "B");
    g.add_node(3usize, "C");
    g.add_edge(1usize, 2usize, None);
    g.add_edge(2usize, 3usize, None);
    g
}

#[test]
fn heap_planner_retains_capacity_across_plans() {
    let ir = hero_graph().compute_layout();
    let mut run = DiagnosticRun::new();
    let mut cx = run.context();
    let mut planner = ScenePlanner::new();

    let (ptr, cap);
    {
        let scene = planner.plan(&ir, &mut cx).unwrap();
        assert_eq!(scene.edge_count(), ir.edges().len());
        ptr = scene.edges.as_ptr();
        cap = ir.edges().len();
    } // scene drops — planner borrow released

    // Re-plan the same input: same allocation, no growth.
    let scene = planner.plan(&ir, &mut cx).unwrap();
    assert_eq!(scene.edges.as_ptr(), ptr, "capacity reused in place");
    assert_eq!(scene.edge_count(), cap);
}

#[test]
fn event_loop_shape_composes_many_frames_per_plan() {
    let ir_a = small_graph().compute_layout();
    let ir_b = hero_graph().compute_layout();
    let mut run = DiagnosticRun::new();
    let mut cx = run.context();
    let mut planner = ScenePlanner::new();

    // The TUI shape: outer iteration per graph change, inner per frame.
    let mut frames = 0usize;
    for ir in [&ir_a, &ir_b] {
        let scene = planner.plan(ir, &mut cx).unwrap();
        for _ in 0..3 {
            frames += scene.compose_frame().min(1);
        }
    }
    assert_eq!(frames, 6);
    let (_, events) = run.finish(Ok::<(), GraphError>(()));
    assert_eq!(
        events,
        [1, 1],
        "one planning event per plan, none per frame"
    );
}

#[test]
fn one_generic_plan_serves_both_backends() {
    let g = small_graph();
    let heap_ir = g.compute_layout();

    let mut csr_buf = vec![0u8; g.estimate_csr_arena_size() * 2];
    let mut csr_arena = Arena::new(&mut csr_buf);
    let csr = g.to_csr(&mut csr_arena).unwrap();
    let size = (g.estimate_layout_arena_size() * 2).max(256 * 1024);
    let mut temp_buf = vec![0u8; size];
    let mut out_buf = vec![0u8; size];
    let mut temp_arena = Arena::new(&mut temp_buf);
    let mut out_arena = Arena::new(&mut out_buf);
    let arena_ir = csr
        .compute_layout_arena(&LayoutConfig::standard(), &mut temp_arena, &mut out_arena)
        .unwrap();

    let mut run = DiagnosticRun::new();
    let mut cx = run.context();
    let mut planner = ScenePlanner::new();
    // Same method, monomorphized per view — option (b) of the sketch.
    let n_heap = planner.plan(&heap_ir, &mut cx).unwrap().edge_count();
    let n_csr = planner.plan(&arena_ir, &mut cx).unwrap().edge_count();
    assert_eq!(n_heap, n_csr);
}

#[test]
fn diagnostic_run_outlives_scene_on_success_and_error() {
    let ir = small_graph().compute_layout();

    // Success path: events emitted during plan, scene consumed, run
    // finished after the scene is gone.
    let mut run = DiagnosticRun::new();
    let outcome = {
        let mut cx = run.context();
        let mut planner = ScenePlanner::new();
        let cells = {
            let scene = planner.plan(&ir, &mut cx).unwrap();
            scene.compose_frame()
        };
        Ok::<usize, GraphError>(cells)
    };
    let (outcome, events) = run.finish(outcome);
    assert!(outcome.is_ok());
    assert_eq!(events, [1]);

    // Error path: an undersized arena planner fails; the run still
    // finishes with the events emitted before the failure preserved.
    let mut run = DiagnosticRun::new();
    let outcome = {
        let mut cx = run.context();
        cx.events.push(9); // pre-failure event
        let mut tiny = [0u8; 8];
        let mut planner = ArenaScenePlanner::new_in(&mut tiny);
        planner.plan(&ir, &mut cx).map(|s| s.edges.len())
    };
    let (outcome, events) = run.finish(outcome);
    assert!(matches!(outcome, Err(GraphError::ArenaOom)));
    assert_eq!(events, [9], "pre-failure diagnostics survive the error");
}

#[test]
fn arena_planner_resets_between_plans() {
    let ir = hero_graph().compute_layout();
    let mut run = DiagnosticRun::new();
    let mut cx = run.context();

    let mut storage = vec![0u8; 64 * 1024];
    let mut planner = ArenaScenePlanner::new_in(&mut storage);

    let first_ptr;
    {
        let scene = planner.plan(&ir, &mut cx).unwrap();
        assert_eq!(scene.edges.len(), ir.edges().len());
        assert!(scene.compose_frame() > 0);
        first_ptr = scene.edges.as_ptr();
    }
    // Second plan resets the arena and carves the SAME memory again —
    // a planner that did not reset would exhaust the buffer instead.
    for _ in 0..64 {
        let scene = planner.plan(&ir, &mut cx).unwrap();
        assert_eq!(
            scene.edges.as_ptr(),
            first_ptr,
            "reset recycles the same carve"
        );
    }
}

/// Manual cost report for the framework-callback pattern (a consumer
/// that cannot store the scene re-plans per draw). Run with:
///   cargo test --features arena replan_cost_report -- --ignored --nocapture
#[test]
#[ignore = "reporting tool, not an assertion"]
fn replan_cost_report() {
    let ir = hero_graph().compute_layout();
    let options = RenderOptions::plain();
    let iters = 1_000u32;
    let start = std::time::Instant::now();
    for _ in 0..iters {
        let plan = RenderPlan::build(&ir, &options);
        std::hint::black_box(plan.width());
    }
    let per = start.elapsed() / iters;
    eprintln!("hero RenderPlan::build: {per:?} per plan ({iters} iters)");
}

// ── Public-shape prototype: NON-generic Scene over a view enum ───────────
//
// The generic `Scene<V>` above proves borrows, but the public API
// promises a non-generic `Scene<'p, 'ir>` that does not leak the
// crate-private `LayoutView` trait. This wrapper proves that shape:
// a view enum inside, typed public entry points outside, the generic
// `plan` as the shared core underneath.

enum ViewRef<'ir> {
    Heap(&'ir crate::ir::LayoutIR<'ir>),
    Arena(&'ir crate::ir::arena::LayoutIRArena<'ir>),
}

struct PublicScene<'p, 'ir> {
    edges: &'p [EdgePlan],
    view: ViewRef<'ir>,
}

impl PublicScene<'_, '_> {
    fn node_count(&self) -> usize {
        match &self.view {
            ViewRef::Heap(v) => v.nodes().len(),
            ViewRef::Arena(v) => v.node_count(),
        }
    }
    fn edge_count(&self) -> usize {
        self.edges.len()
    }
}

impl ScenePlanner {
    fn plan_heap<'p, 'ir>(
        &'p mut self,
        ir: &'ir crate::ir::LayoutIR<'ir>,
        cx: &mut DiagnosticContext<'_>,
    ) -> Result<PublicScene<'p, 'ir>, GraphError> {
        let scene = self.plan(ir, cx)?;
        Ok(PublicScene {
            edges: scene.edges,
            view: ViewRef::Heap(ir),
        })
    }

    fn plan_csr<'p, 'ir>(
        &'p mut self,
        ir: &'ir crate::ir::arena::LayoutIRArena<'ir>,
        cx: &mut DiagnosticContext<'_>,
    ) -> Result<PublicScene<'p, 'ir>, GraphError> {
        let scene = self.plan(ir, cx)?;
        Ok(PublicScene {
            edges: scene.edges,
            view: ViewRef::Arena(ir),
        })
    }
}

/// The non-generic public shape works over both backends through the
/// generic core, without exposing `LayoutView`.
#[test]
fn public_scene_shape_serves_both_backends() {
    let g = small_graph();
    let heap_ir = g.compute_layout();

    let mut csr_buf = vec![0u8; g.estimate_csr_arena_size() * 2];
    let mut csr_arena = Arena::new(&mut csr_buf);
    let csr = g.to_csr(&mut csr_arena).unwrap();
    let size = (g.estimate_layout_arena_size() * 2).max(256 * 1024);
    let mut temp_buf = vec![0u8; size];
    let mut out_buf = vec![0u8; size];
    let mut temp_arena = Arena::new(&mut temp_buf);
    let mut out_arena = Arena::new(&mut out_buf);
    let arena_ir = csr
        .compute_layout_arena(&LayoutConfig::standard(), &mut temp_arena, &mut out_arena)
        .unwrap();

    let mut run = DiagnosticRun::new();
    let mut cx = run.context();
    let mut planner = ScenePlanner::new();
    let (hn, he) = {
        let scene = planner.plan_heap(&heap_ir, &mut cx).unwrap();
        (scene.node_count(), scene.edge_count())
    };
    let (an, ae) = {
        let scene = planner.plan_csr(&arena_ir, &mut cx).unwrap();
        (scene.node_count(), scene.edge_count())
    };
    assert_eq!((hn, he), (an, ae));
}

/// The realistic stored-state consumer: a widget struct owning planner
/// and IR, re-planning inside `draw(&mut self)` — the documented
/// fallback for frameworks that cannot hold a scene across calls.
/// Split field borrows make it compile without ceremony; capacity is
/// retained between draws.
#[test]
fn stored_state_widget_replans_per_draw() {
    struct Widget {
        planner: ScenePlanner,
        ir: crate::ir::LayoutIR<'static>,
        run: DiagnosticRun,
    }
    impl Widget {
        fn draw(&mut self) -> usize {
            let mut cx = self.run.context();
            let scene = self.planner.plan_heap(&self.ir, &mut cx).unwrap();
            scene.node_count() + scene.edge_count()
        }
    }
    let mut w = Widget {
        planner: ScenePlanner::new(),
        ir: hero_graph().compute_layout(),
        run: DiagnosticRun::new(),
    };
    let first = w.draw();
    for _ in 0..10 {
        assert_eq!(w.draw(), first);
    }
}
