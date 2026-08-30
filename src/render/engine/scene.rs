//! `ScenePlanner` / `Scene` — the resolved-scene core of the 0.11
//! pipeline (plan once, consume many times).
//!
//! A [`Scene`] is one layout resolved under one set of
//! [`PlanOptions`]: styles ran exactly once, label placement is
//! settled, geometry is indexed for hit-testing. It borrows its
//! planner's storage (`'p`) and the layout it was planned from
//! (`'ir`), so a stale pairing is a compile error, and exactly one
//! scene per planner is live at a time — enforced by the borrow
//! checker, not a runtime flag.
//!
//! The planner owns ONE retained workspace chunk and carves every
//! plan buffer from it through a fresh bump arena per `plan` call.
//! Carved borrows are tied to the `&mut self` borrow and cannot
//! outlive the scene, so there is no reset step and no `unsafe`
//! anywhere in this module: re-planning while a scene lives simply
//! does not compile, and dropping the scene releases the planner.
//!
//! Two workspace modes, one type: [`ScenePlanner::new`] retains a
//! growable heap chunk (replanning at steady state allocates nothing;
//! a bigger layout grows it once); [`ScenePlanner::new_in`] plans out
//! of a caller-provided byte slice and reports
//! [`GraphError::RenderPlanOom`](crate::GraphError::RenderPlanOom)
//! instead of growing.

use super::config::PlanOptions;
use super::plan::{HitResult, RenderPlan, plan_storage_bytes};
use super::view::LayoutView;
use crate::GraphError;
use crate::graph::arena::Arena;
use crate::ir::arena::LayoutIRArena;

#[cfg(feature = "alloc")]
use crate::ir::LayoutIR;

mod sealed {
    pub trait Sealed {}
    #[cfg(feature = "alloc")]
    impl Sealed for crate::ir::LayoutIR<'_> {}
    impl Sealed for crate::ir::arena::LayoutIRArena<'_> {}
}

/// A layout a [`ScenePlanner`] can plan: both IR types, nothing else
/// (sealed). The scene type stays the same whichever backend produced
/// the layout — the pipeline has no user-visible "backends".
pub trait LayoutSource: sealed::Sealed {
    /// Implementation detail: the planner's internal handle to this
    /// layout.
    #[doc(hidden)]
    fn source_ref(&self) -> SourceRef<'_>;
}

/// Opaque layout handle (implementation detail of [`LayoutSource`]).
pub struct SourceRef<'ir>(pub(crate) ViewRef<'ir>);

/// The scene's storage-neutral view of its layout: one private enum,
/// monomorphized cores behind it — `Scene` itself stays non-generic.
#[derive(Clone, Copy)]
pub(crate) enum ViewRef<'ir> {
    #[cfg(feature = "alloc")]
    Heap(&'ir LayoutIR<'ir>),
    Arena(&'ir LayoutIRArena<'ir>),
}

#[cfg(feature = "alloc")]
impl LayoutSource for LayoutIR<'_> {
    fn source_ref(&self) -> SourceRef<'_> {
        SourceRef(ViewRef::Heap(self))
    }
}

impl LayoutSource for LayoutIRArena<'_> {
    fn source_ref(&self) -> SourceRef<'_> {
        SourceRef(ViewRef::Arena(self))
    }
}

enum Workspace<'ws> {
    /// Growable retained chunk: grows to each new high-water mark,
    /// never shrinks, allocates nothing at steady state.
    #[cfg(feature = "alloc")]
    Heap(alloc::vec::Vec<u8>),
    /// Caller-provided fixed chunk: a layout that does not fit is a
    /// documented error, never a grow.
    Fixed(&'ws mut [u8]),
}

/// Plans layouts into [`Scene`]s, retaining its workspace across
/// plans.
///
/// Options are inputs to [`plan`](Self::plan), not construction state
/// — one planner serves any sequence of layouts and option sets.
///
/// Exactly one scene per planner can be live: `plan` borrows the
/// planner mutably for the scene's whole lifetime, so re-planning
/// while a scene is still held is rejected at compile time:
///
/// ```compile_fail,E0499
/// use ascii_dag::{Graph, PlanOptions, ScenePlanner};
///
/// let g = Graph::from_edges(&[(1, "A"), (2, "B")], &[(1, 2)]);
/// let ir = g.compute_layout();
/// let mut planner = ScenePlanner::new();
/// let scene = planner.plan(&ir, &PlanOptions::new()).unwrap();
/// let second = planner.plan(&ir, &PlanOptions::new()); // ERROR: `scene` is still live
/// scene.hit_test(0, 0);
/// ```
///
/// Dropping the scene releases the planner; plan-once/compose-many
/// and re-plan-per-draw (for stored-state frameworks) are both
/// natural shapes:
///
/// ```
/// use ascii_dag::{Graph, PlanOptions, ScenePlanner};
///
/// let g = Graph::from_edges(&[(1, "A"), (2, "B")], &[(1, 2)]);
/// let ir = g.compute_layout();
/// let mut planner = ScenePlanner::new();
/// {
///     let scene = planner.plan(&ir, &PlanOptions::new()).unwrap();
///     assert!(scene.width() > 0);
/// } // scene drops…
/// let again = planner.plan(&ir, &PlanOptions::new()).unwrap(); // …planner is free
/// assert!(again.height() > 0);
/// ```
pub struct ScenePlanner<'ws> {
    ws: Workspace<'ws>,
}

#[cfg(feature = "alloc")]
impl ScenePlanner<'static> {
    /// Heap planner: owns a growable workspace, retains capacity
    /// across plans (steady-state replanning allocates nothing).
    pub fn new() -> Self {
        Self {
            ws: Workspace::Heap(alloc::vec::Vec::new()),
        }
    }
}

#[cfg(feature = "alloc")]
impl Default for ScenePlanner<'static> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'ws> ScenePlanner<'ws> {
    /// No-alloc planner over a caller-provided workspace. A layout
    /// whose plan does not fit reports
    /// [`GraphError::RenderPlanOom`](crate::GraphError::RenderPlanOom)
    /// (`E.Render.Plan.026`) instead of growing.
    pub fn new_in(workspace: &'ws mut [u8]) -> Self {
        Self {
            ws: Workspace::Fixed(workspace),
        }
    }

    /// Resolve one layout into a [`Scene`] under `options`. Style
    /// callbacks run here, once per element, never again; the scene
    /// borrows this planner (`'p`) and the layout (`'ir`).
    pub fn plan<'p, 'ir, L: LayoutSource>(
        &'p mut self,
        layout: &'ir L,
        options: &PlanOptions,
    ) -> Result<Scene<'p, 'ir>, GraphError> {
        match layout.source_ref().0 {
            #[cfg(feature = "alloc")]
            ViewRef::Heap(v) => {
                let plan = Self::plan_core(&mut self.ws, v, options)?;
                Ok(Scene {
                    plan,
                    view: ViewRef::Heap(v),
                })
            }
            ViewRef::Arena(v) => {
                let plan = Self::plan_core(&mut self.ws, v, options)?;
                Ok(Scene {
                    plan,
                    view: ViewRef::Arena(v),
                })
            }
        }
    }

    /// One generic core over the private lens — monomorphized per
    /// backend like the paint path; the enum above keeps the PUBLIC
    /// types non-generic.
    fn plan_core<'p, V: LayoutView>(
        ws: &'p mut Workspace<'ws>,
        view: &V,
        options: &PlanOptions,
    ) -> Result<RenderPlan<'p>, GraphError> {
        let needed = plan_storage_bytes(view);
        let chunk: &'p mut [u8] = match ws {
            #[cfg(feature = "alloc")]
            Workspace::Heap(buf) => {
                if needed > buf.len() {
                    buf.resize(needed, 0); // the only allocating event
                }
                buf.as_mut_slice()
            }
            Workspace::Fixed(buf) => {
                if needed > buf.len() {
                    return Err(GraphError::RenderPlanOom);
                }
                &mut buf[..]
            }
        };
        // A fresh bump arena over the retained chunk each plan: carves
        // are tied to the `&mut self` borrow, so no carved reference
        // can outlive the scene and no reset (or `unsafe`) exists to
        // get wrong.
        let arena = Arena::new(chunk);
        RenderPlan::build_in(view, options, &arena)
    }
}

/// One layout, resolved: styles ran once, label placement is settled,
/// geometry is indexed. Borrow-bound to its planner (`'p`) and layout
/// (`'ir`).
pub struct Scene<'p, 'ir> {
    plan: RenderPlan<'p>,
    view: ViewRef<'ir>,
}

impl Scene<'_, '_> {
    /// Rendered width in cells.
    pub fn width(&self) -> usize {
        self.plan.width()
    }

    /// Rendered height in rows.
    pub fn height(&self) -> usize {
        self.plan.height()
    }

    /// What occupies the cell at `(x, y)`? Nodes win over edges,
    /// edges over subgraph boxes, matching the visual z-order; edge
    /// labels, box labels, and self-loop markers belong to their
    /// owning element. Out-of-canvas queries return
    /// [`HitResult::None`], never a panic.
    pub fn hit_test(&self, x: usize, y: usize) -> HitResult {
        match self.view {
            #[cfg(feature = "alloc")]
            ViewRef::Heap(v) => self.plan.element_at(v, x, y),
            ViewRef::Arena(v) => self.plan.element_at(v, x, y),
        }
    }

    /// Edge indices (IR-list order) whose labels go to the legend
    /// under the options this scene was planned with. Empty unless
    /// [`LabelOverflow::Legend`](super::config::LabelOverflow::Legend)
    /// was set.
    pub fn legend_entries(&self) -> &[usize] {
        self.plan.legend_entries()
    }

    /// The resolved plan (views/composer/emission internals).
    pub(crate) fn plan(&self) -> &RenderPlan<'_> {
        &self.plan
    }

    /// The layout view (views/composer/emission internals).
    pub(crate) fn view(&self) -> &ViewRef<'_> {
        &self.view
    }
}

#[cfg(all(test, feature = "std", feature = "layout-vertical"))]
mod tests {
    use super::*;
    use crate::graph::Graph;
    use crate::render::engine::test_alloc::allocations_on_this_thread;

    fn small_graph() -> Graph<'static> {
        let mut g = Graph::new();
        g.add_node(1usize, "A");
        g.add_node(2usize, "B");
        g.add_node(3usize, "C");
        g.add_edge(1usize, 2usize, Some("go"));
        g.add_edge(2usize, 3usize, None);
        g
    }

    fn arena_ir_parts(g: &Graph<'_>) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
        (
            vec![0u8; g.estimate_csr_arena_size() * 2],
            vec![0u8; (g.estimate_layout_arena_size() * 2).max(256 * 1024)],
            vec![0u8; (g.estimate_layout_arena_size() * 2).max(256 * 1024)],
        )
    }

    /// One planner, both backends, identical scene answers — the
    /// public types never go generic.
    #[test]
    fn one_planner_serves_both_backends() {
        let g = small_graph();
        let heap_ir = g.compute_layout();

        let g2 = small_graph();
        let (mut csr_buf, mut temp_buf, mut out_buf) = arena_ir_parts(&g2);
        let mut csr_arena = Arena::new(&mut csr_buf);
        let csr = g2.to_csr(&mut csr_arena).unwrap();
        let mut temp_arena = Arena::new(&mut temp_buf);
        let mut out_arena = Arena::new(&mut out_buf);
        let cfg = crate::LayoutConfig::standard();
        let arena_ir = csr
            .compute_layout_arena(&cfg, &mut temp_arena, &mut out_arena)
            .unwrap();

        let mut planner = ScenePlanner::new();
        let options = PlanOptions::new();

        let (hw, hh, hits_heap): (usize, usize, Vec<HitResult>) = {
            let scene = planner.plan(&heap_ir, &options).unwrap();
            let hits = (0..scene.width()).map(|x| scene.hit_test(x, 0)).collect();
            (scene.width(), scene.height(), hits)
        };
        let scene = planner.plan(&arena_ir, &options).unwrap();
        assert_eq!((scene.width(), scene.height()), (hw, hh));
        let hits_arena: Vec<HitResult> = (0..scene.width()).map(|x| scene.hit_test(x, 0)).collect();
        assert_eq!(hits_heap, hits_arena);
    }

    /// The heap planner retains its chunk: replanning at steady state
    /// performs zero allocations (measured), and only a larger layout
    /// grows it.
    #[test]
    fn heap_planner_replans_without_allocating() {
        let g = small_graph();
        let ir = g.compute_layout();
        let options = PlanOptions::new();
        let mut planner = ScenePlanner::new();
        drop(planner.plan(&ir, &options).unwrap()); // warm-up sizes the chunk

        let before = allocations_on_this_thread();
        for _ in 0..50 {
            let scene = planner.plan(&ir, &options).unwrap();
            std::hint::black_box(scene.hit_test(0, 0));
        }
        assert_eq!(
            allocations_on_this_thread() - before,
            0,
            "steady-state replanning allocated"
        );
    }

    /// The fixed-workspace planner serves fitting layouts and reports
    /// the documented plan-storage error on a misfit — workspace
    /// intact afterwards.
    #[test]
    fn fixed_workspace_planner_errors_on_misfit_and_survives() {
        let g = small_graph();
        let ir = g.compute_layout();

        let mut big = Graph::new();
        big.add_node(0usize, "R");
        for i in 1..=200usize {
            big.add_node(i, "leaf");
            big.add_edge(0usize, i, None);
        }
        let big_ir = big.compute_layout();

        let mut ws = vec![0u8; crate::render::engine::plan::plan_storage_bytes(&ir)];
        let mut planner = ScenePlanner::new_in(&mut ws);
        let options = PlanOptions::new();

        assert!(matches!(
            planner.plan(&big_ir, &options),
            Err(GraphError::RenderPlanOom)
        ));
        let scene = planner.plan(&ir, &options).unwrap();
        assert!(scene.width() > 0);
    }

    /// The stored-state framework shape: a widget owning planner +
    /// layout, replanning inside `draw(&mut self)` — compiles without
    /// ceremony (split field borrows) and reuses the chunk.
    #[test]
    fn stored_state_widget_replans_per_draw() {
        struct Widget {
            planner: ScenePlanner<'static>,
            ir: crate::ir::LayoutIR<'static>,
        }
        impl Widget {
            fn draw(&mut self) -> usize {
                let scene = self.planner.plan(&self.ir, &PlanOptions::new()).unwrap();
                scene.width() * scene.height()
            }
        }
        let mut w = Widget {
            planner: ScenePlanner::new(),
            ir: small_graph().compute_layout(),
        };
        let a = w.draw();
        let b = w.draw();
        assert_eq!(a, b);
        assert!(a > 0);
    }

    /// Scene answers match the plan machinery they wrap.
    #[test]
    fn scene_agrees_with_plan_queries() {
        let g = small_graph();
        let ir = g.compute_layout();
        let mut planner = ScenePlanner::new();
        let options = PlanOptions::new();
        let scene = planner.plan(&ir, &options).unwrap();
        let plan = RenderPlan::build(&ir, &options);
        assert_eq!(scene.width(), plan.width());
        assert_eq!(scene.height(), plan.height());
        assert_eq!(scene.legend_entries(), plan.legend_entries());
        for y in 0..scene.height() {
            for x in 0..scene.width() {
                assert_eq!(scene.hit_test(x, y), plan.element_at(&ir, x, y));
            }
        }
    }
}
