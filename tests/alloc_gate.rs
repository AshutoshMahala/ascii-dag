//! The steady-state allocation gate, run exactly as specified: a
//! counting global allocator observes the whole process, so this
//! binary holds ONE serial test; the planner, renderers, composer, and
//! presized sinks are all constructed BEFORE the measured window, and
//! only steady-state repaint is measured.
//!
//! The gate: one scene emits Unicode, ASCII, colored, and plain —
//! plus a full semantic cell visit — repeatedly, with zero replanning
//! and ZERO allocations.

#![cfg(all(feature = "std", feature = "layout-vertical"))]

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicU64, Ordering};

use ascii_dag::render::colors::Palette;
use ascii_dag::{
    ComposeBudget, Graph, RenderOptions, SceneComposer, ScenePlanner, TerminalRenderer,
};

static ALLOCATIONS: AtomicU64 = AtomicU64::new(0);

struct CountingAlloc;

unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        unsafe { System.alloc_zeroed(layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        unsafe { System.realloc(ptr, layout, new_size) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static COUNTING: CountingAlloc = CountingAlloc;

fn graph() -> Graph<'static> {
    let mut g = Graph::new();
    g.add_node(1usize, "Start");
    g.add_node(2usize, "Middle");
    g.add_node(3usize, "End");
    g.add_edge(1usize, 2usize, Some("go"));
    g.add_edge(2usize, 3usize, None);
    g.add_edge(1usize, 3usize, None);
    g.add_edge(2usize, 2usize, Some("respin")); // labeled self-loop → legend
    // A label that fits nowhere inline: guarantees the legend is
    // NON-EMPTY, so the measured window exercises legend emission —
    // the gate must never pass because the legend path was vacuously
    // skipped.
    g.add_edge(
        3usize,
        2usize,
        Some("an-extremely-long-label-that-cannot-possibly-fit-inline-anywhere-at-all"),
    );
    let sg = g.add_subgraph("Stage");
    g.put_nodes(&[2]).inside(sg).unwrap();
    g
}

#[test]
fn steady_state_repaint_allocates_nothing() {
    let g = graph();
    let ir = g.compute_layout();

    // ── Construction: everything allocates HERE, before the window ──
    let plan_options = RenderOptions::colored(Palette::Ansi).plan;
    let mut planner = ScenePlanner::new();
    let scene = planner.plan(&ir, &plan_options).quiet().unwrap();
    let budget = ComposeBudget::new();
    let req = scene.composition_requirements(&budget);
    assert!(
        scene.legend().next().is_some(),
        "fixture must overflow a label — the window has to cover legend emission"
    );

    let emits = [
        RenderOptions::plain().emit,
        RenderOptions::ascii().emit,
        RenderOptions::colored(Palette::Ansi).emit,
        RenderOptions::ascii_colored(Palette::Ansi).emit,
    ];
    let mut renderers: Vec<TerminalRenderer<'static>> = emits
        .iter()
        .map(|e| TerminalRenderer::new(e, req))
        .collect();
    let mut composer = SceneComposer::new(req);

    // Presized sinks, warmed once so every retained buffer reaches its
    // high-water mark before measurement.
    let mut sinks: Vec<String> = Vec::new();
    for renderer in renderers.iter_mut() {
        let mut out = String::new();
        renderer.render(&scene, &mut out).unwrap();
        sinks.push(out);
    }
    let mut acc = 0u64;
    composer.visit_cells(&scene, |_, _, _| {}).unwrap();

    // ── The measured window: steady-state repaint only ──
    let before = ALLOCATIONS.load(Ordering::Relaxed);
    for _ in 0..25 {
        for (renderer, sink) in renderers.iter_mut().zip(sinks.iter_mut()) {
            sink.clear(); // keeps capacity
            renderer.render(&scene, sink).unwrap();
        }
        composer
            .visit_cells(&scene, |x, y, cell| {
                acc = acc
                    .wrapping_add(x as u64)
                    .wrapping_add(y as u64)
                    .wrapping_add(cell.color.as_ansi256().unwrap_or(0) as u64);
            })
            .unwrap();
    }
    let after = ALLOCATIONS.load(Ordering::Relaxed);
    std::hint::black_box(acc);
    assert_eq!(
        after - before,
        0,
        "steady-state repaint allocated ({} times) — one scene, four \
         emission modes, one semantic visit, all through retained \
         workspaces",
        after - before
    );

    // And the outputs are the real thing: each matches its one-step
    // wrapper under the same options.
    for (emit, sink) in emits.iter().zip(sinks.iter()) {
        let wrapper = RenderOptions {
            plan: plan_options,
            emit: *emit,
            compose: budget,
        };
        assert_eq!(sink, &ir.render_string(&wrapper));
    }
}
