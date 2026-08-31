//! The three no-alloc sizing contracts, pinned at EXACT size — the
//! release gate for embedded and no-alloc callers: an estimate must
//! never under-report, so every contract is exercised end-to-end from
//! a buffer of precisely the estimated size.
//!
//! | Buffer | Sized by | Owner |
//! |---|---|---|
//! | Scene storage | `ir.estimate_scene_size(&PlanOptions)` | `ScenePlanner::new_in` |
//! | Composition workspace | `scene.composition_requirements(&budget)` — `workspace_bytes()` (semantic) / `terminal_workspace_bytes(&emit)` (terminal) | `SceneComposer::new_in` / `TerminalRenderer::new_in` |
//! | Output bytes | `scene.estimate_output_size(&emit)` | caller |
//!
//! An estimate that under-reports fails these tests the moment it
//! drifts. (The one-shot wrapper keeps its combined
//! `estimate_render_arena_size`/`estimate_render_output_size` pair,
//! pinned by the parity suite's exact-arena renders.)

#![cfg(feature = "layout-vertical")]

use ascii_dag::render::colors::Palette;
use ascii_dag::{
    ComposeBudget, Graph, RenderOptions, SceneComposer, ScenePlanner, TerminalRenderer,
};

fn corpus() -> Vec<(&'static str, Graph<'static>)> {
    let mut labeled = Graph::new();
    labeled.add_node(1usize, "Start");
    labeled.add_node(2usize, "Middle");
    labeled.add_node(3usize, "End");
    labeled.add_edge(1usize, 2usize, Some("go"));
    labeled.add_edge(2usize, 3usize, None);
    labeled.add_edge(1usize, 3usize, None); // skip → dummies exist
    labeled.add_edge(2usize, 2usize, Some("respin")); // preserved self-loop

    let mut clusters = Graph::new();
    clusters.add_node(1usize, "a");
    clusters.add_node(2usize, "b");
    clusters.add_node(3usize, "c");
    clusters.add_node(4usize, "d");
    let s1 = clusters.add_subgraph("S1");
    let s2 = clusters.add_subgraph("S2");
    clusters.put_nodes(&[1, 2]).inside(s1).unwrap();
    clusters.put_nodes(&[3, 4]).inside(s2).unwrap();
    clusters.add_edge(1usize, 4usize, None);
    clusters.add_edge(2usize, 3usize, Some("cross"));

    let mut fan = Graph::new();
    fan.add_node(0usize, "Root");
    for i in 1..=25usize {
        fan.add_node(i, "leaf");
        fan.add_edge(0usize, i, None);
    }

    // One short node, one very long labeled self-loop, NO routed
    // edges: the loop's legend line dominates the output, so no
    // routed-edge slack can hide an estimator that forgets loops.
    let mut loop_only = Graph::new();
    loop_only.add_node(1usize, "N");
    loop_only.add_edge(
        1usize,
        1usize,
        Some("a-self-loop-legend-line-long-enough-that-no-per-edge-slack-could-ever-absorb-it"),
    );

    vec![
        ("labeled", labeled),
        ("clusters", clusters),
        ("fan", fan),
        ("loop-only", loop_only),
    ]
}

#[test]
fn exact_size_contracts_hold() {
    for (what, g) in corpus() {
        let ir = g.compute_layout();
        check_exact_sizing_heap(&ir, what);
    }
}

#[cfg(feature = "arena")]
#[test]
fn exact_size_contracts_hold_on_the_arena_backend() {
    use ascii_dag::LayoutConfig;
    use ascii_dag::graph::arena::Arena;
    for (what, g) in corpus() {
        let mut csr_buf = vec![0u8; g.estimate_csr_arena_size() * 2];
        let mut csr_arena = Arena::new(&mut csr_buf);
        let csr = g.to_csr(&mut csr_arena).unwrap();
        let size = (g.estimate_layout_arena_size() * 2).max(256 * 1024);
        let mut temp_buf = vec![0u8; size];
        let mut out_buf = vec![0u8; size];
        let mut temp_arena = Arena::new(&mut temp_buf);
        let mut out_arena = Arena::new(&mut out_buf);
        let ir = csr
            .compute_layout_arena(&LayoutConfig::standard(), &mut temp_arena, &mut out_arena)
            .unwrap();
        check_exact_sizing_arena(&ir, what);
    }
}

macro_rules! concrete_check {
    ($name:ident, $ir:ty, $render:expr) => {
        fn $name(ir: &$ir, what: &str) {
            let plan_options = RenderOptions::colored(Palette::Ansi).plan;
            let budget = ComposeBudget::new();
            let emits = [
                RenderOptions::plain().emit,
                RenderOptions::colored(Palette::Ansi).emit,
            ];

            let mut scene_ws = vec![0u8; ir.estimate_scene_size(&plan_options)];
            let mut planner = ScenePlanner::new_in(&mut scene_ws);
            let scene = planner
                .plan(ir, &plan_options)
                .quiet()
                .unwrap_or_else(|e| panic!("{what}: exact scene storage failed: {e}"));
            let req = scene.composition_requirements(&budget);

            let mut comp_ws = vec![0u8; req.workspace_bytes().unwrap()];
            let mut composer = SceneComposer::new_in(req, &mut comp_ws)
                .unwrap_or_else(|e| panic!("{what}: exact composer workspace failed: {e}"));
            let mut cells = 0usize;
            composer.visit_cells(&scene, |_, _, _| cells += 1).unwrap();
            assert_eq!(cells, scene.width() * scene.height(), "{what}: full visit");

            for emit in emits {
                let mut term_ws = vec![0u8; req.terminal_workspace_bytes(&emit).unwrap()];
                let mut renderer = TerminalRenderer::new_in(&emit, req, &mut term_ws)
                    .unwrap_or_else(|e| panic!("{what}: exact terminal workspace failed: {e}"));

                let mut out = vec![0u8; scene.estimate_output_size(&emit)];
                let n = renderer
                    .render_into(&scene, &mut out)
                    .unwrap_or_else(|e| panic!("{what}: exact output buffer failed: {e}"));

                let wrapper = RenderOptions {
                    plan: plan_options,
                    emit,
                    compose: budget,
                };
                let reference: String = $render(ir, &wrapper);
                assert_eq!(
                    core::str::from_utf8(&out[..n]).unwrap(),
                    reference,
                    "{what}: exact-size render diverged"
                );
            }
        }
    };
}

concrete_check!(
    check_exact_sizing_heap,
    ascii_dag::LayoutIR<'_>,
    |ir: &ascii_dag::LayoutIR<'_>, o: &RenderOptions| ir.render_string(o)
);

#[cfg(feature = "arena")]
concrete_check!(
    check_exact_sizing_arena,
    ascii_dag::ir::arena::LayoutIRArena<'_>,
    |ir: &ascii_dag::ir::arena::LayoutIRArena<'_>, o: &RenderOptions| {
        let mut s = String::new();
        ir.render_with(o, &mut s).unwrap();
        s
    }
);
