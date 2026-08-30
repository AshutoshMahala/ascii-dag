//! Spike: storage-neutral public views for the 0.11 scene work
//! (prototype stage — see temp/scene-api-sketch.md §9 and
//! temp/spike-4.0c-findings.md).
//!
//! **THROWAWAY CODE.** Test-only, ships in no build, deleted when the
//! real scene views land. It answers four questions:
//!
//! 1. Can every public view type be constructed from BOTH IRs with no
//!    allocation — `MultiSegment` waypoints and label/payload strings
//!    LENDING from either backend? (Heap stores `Vec` waypoints and
//!    `&str` labels; the arena stores a shared waypoint slice and a
//!    `&[u8]` label pool with offset/len pairs.)
//! 2. Can a heap-only type leak into a public view field? The `Copy`
//!    bound is a guardrail (it rejects owned `Vec`/`String` fields at
//!    compile time, though not `&Vec<T>`-shaped references); the
//!    PROOF is construction: every view built from both IRs by the
//!    same code path, contents compared, zero allocations measured.
//! 3. Does ONE view lifetime `'s` unify planner-owned (`'p`) and
//!    IR-owned (`'ir`) borrows in the same view value? (The scene
//!    holds both; views borrow the scene, and covariance shortens both
//!    sides to the scene borrow.)
//! 4. What happens to the IR's `Spline` stub at the view boundary?
//!    (Normalized to `Direct`, exactly the compositor's fallback — the
//!    stub never becomes public API surface.)
//!
//! The views project from the private `LayoutView`/`PathRef` lens,
//! which already serves both IRs — the public types are lens
//! projections, never IR types.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use super::view::{LayoutView, PathRef};
use crate::LayoutConfig;
use crate::graph::Graph;
use crate::ir::NodeKind;
use crate::render::engine::{BoxedNode, CustomNode};

// ── The public-shape prototypes (all Copy — question 2's proof) ──────────

/// Storage-neutral routed path — the public twin of the private
/// `PathRef` lens. No `Vec`, no arena offsets; waypoints lend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EdgePathViewSpike<'s> {
    Direct,
    Corner {
        bend_at: usize,
    },
    SideChannel {
        channel_at: usize,
        span_start: usize,
        span_end: usize,
    },
    MultiSegment {
        waypoints: &'s [(usize, usize)],
        start_offset: usize,
    },
    /// Decision 15 stand-in: a preserved self-loop record; `at` is the
    /// marker cell.
    SelfLoop {
        at: (usize, usize),
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LabelViewSpike<'s> {
    text: &'s str,
    at: (usize, usize),
}

/// The scene's real/dummy dichotomy (decision 16 shape), mapped by the
/// projection from the IR's flat `Explicit | Implicit | Dummy`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NodeKindViewSpike {
    Real { implicit: bool },
    Dummy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct NodeViewSpike<'s> {
    /// Graph id — `None` for dummies: their raw ids are SYNTHETIC and
    /// backend-specific, so the public view must not expose them.
    /// Dummies carry [`dummy_of`](Self::dummy_of) instead; this split
    /// is what makes cross-backend view parity field-for-field.
    id: Option<usize>,
    /// Backend-stable dummy identity: the owning edge's input index
    /// and the level the dummy occupies. `None` for real nodes.
    dummy_of: Option<(usize, usize)>,
    x: usize,
    y: usize,
    width: usize,
    height: usize,
    kind: NodeKindViewSpike,
    label: &'s str,
    /// Custom-node payload, rejoined by index through the lens.
    payload: Option<&'s str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct EdgeViewSpike<'s> {
    scene_index: usize,
    input_index: usize,
    from_id: usize,
    to_id: usize,
    directed: bool,
    reversed: bool,
    from: (usize, usize),
    to: (usize, usize),
    path: EdgePathViewSpike<'s>,
    label: Option<LabelViewSpike<'s>>,
    // Resolved paint decisions (color / weight / markers) join here in
    // the real EdgeView; they are Copy scalars from planner storage and
    // add nothing to the storage-neutrality question.
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SubgraphViewSpike<'s> {
    id: usize,
    parent: Option<usize>,
    label: &'s str,
    x: usize,
    y: usize,
    width: usize,
    height: usize,
}

// ── Projections: lens → view (both IRs, one code path) ───────────────────

fn path_view(p: PathRef<'_>) -> EdgePathViewSpike<'_> {
    match p {
        // The Spline stub is a forward-compat placeholder the layout
        // engine never emits; the compositor already falls back to
        // Direct, and the view boundary does the same (question 4).
        PathRef::Direct | PathRef::Spline { .. } => EdgePathViewSpike::Direct,
        PathRef::Corner { bend_at } => EdgePathViewSpike::Corner { bend_at },
        PathRef::SideChannel {
            channel_at,
            span_start,
            span_end,
        } => EdgePathViewSpike::SideChannel {
            channel_at,
            span_start,
            span_end,
        },
        PathRef::MultiSegment {
            waypoints,
            start_offset,
        } => EdgePathViewSpike::MultiSegment {
            waypoints,
            start_offset,
        },
    }
}

fn node_view<V: LayoutView>(view: &V, index: usize) -> NodeViewSpike<'_> {
    let n = view.node(index);
    let (painter, payload) = view.node_custom(index);
    let dummy = matches!(n.kind, NodeKind::Dummy);
    NodeViewSpike {
        id: (!dummy).then_some(n.id),
        dummy_of: dummy.then(|| (n.edge_index.unwrap_or(usize::MAX), n.level)),
        x: n.x,
        y: n.y,
        width: n.width,
        height: n.height,
        kind: match n.kind {
            NodeKind::Explicit => NodeKindViewSpike::Real { implicit: false },
            NodeKind::Implicit => NodeKindViewSpike::Real { implicit: true },
            NodeKind::Dummy => NodeKindViewSpike::Dummy,
        },
        label: n.label,
        // The lens yields (None, "") for nodes without a custom entry;
        // a blank custom node stores no entry, so this is lossless.
        payload: if painter.is_none() && payload.is_empty() {
            None
        } else {
            Some(payload)
        },
    }
}

fn edge_view<V: LayoutView>(view: &V, index: usize) -> EdgeViewSpike<'_> {
    let e = view.edge(index);
    EdgeViewSpike {
        scene_index: index,
        input_index: e.edge_index,
        from_id: e.from_id,
        to_id: e.to_id,
        directed: e.directed,
        reversed: e.reversed,
        from: (e.from_x, e.from_y),
        to: (e.to_x, e.to_y),
        path: path_view(e.path),
        label: e.label.map(|text| LabelViewSpike {
            text,
            at: (e.label_x, e.label_y),
        }),
    }
}

fn subgraph_view<V: LayoutView>(view: &V, index: usize) -> SubgraphViewSpike<'_> {
    let sg = view.subgraph(index);
    SubgraphViewSpike {
        id: sg.id,
        parent: sg.parent,
        label: sg.label,
        x: sg.x,
        y: sg.y,
        width: sg.width,
        height: sg.height,
    }
}

// ── Scene stub: one 's over planner ('p) and IR ('ir) borrows ────────────

enum ViewRefSpike<'ir> {
    Heap(&'ir crate::ir::LayoutIR<'ir>),
    Arena(&'ir crate::ir::arena::LayoutIRArena<'ir>),
}

/// Two-lifetime scene stand-in: `loop_label` models planner-owned
/// storage (decision 15 — the planner synthesizes self-loop records;
/// text it derives lives in `'p`, not the IR), `view` is the `'ir`
/// side.
struct SceneStubSpike<'p, 'ir> {
    loop_label: &'p str,
    view: ViewRefSpike<'ir>,
}

impl SceneStubSpike<'_, '_> {
    fn edge_view(&self, index: usize) -> EdgeViewSpike<'_> {
        match self.view {
            ViewRefSpike::Heap(v) => edge_view(v, index),
            ViewRefSpike::Arena(v) => edge_view(v, index),
        }
    }

    fn edge_count(&self) -> usize {
        match self.view {
            ViewRefSpike::Heap(v) => LayoutView::edge_count(v),
            ViewRefSpike::Arena(v) => LayoutView::edge_count(v),
        }
    }

    /// BORROW-MIXING STAND-IN ONLY — this does NOT prove decision 15's
    /// self-loop `EdgeView`. It scans node markers, returns the first
    /// loop, and fabricates `input_index`, directedness, and label
    /// storage; the real projection needs preserved self-loop RECORDS
    /// (new IR storage + a lens accessor on both backends) and stays
    /// open until that shape exists. What this function does prove:
    /// one view value can hold a planner-storage borrow (`'p` label)
    /// and IR borrows (`'ir` marker) under the single `&self`
    /// lifetime.
    fn self_loop_view(&self) -> Option<EdgeViewSpike<'_>> {
        let find = |count: usize, node: &dyn Fn(usize) -> (usize, Option<(usize, usize)>)| {
            (0..count).find_map(|i| {
                let (id, at) = node(i);
                at.map(|at| (id, at))
            })
        };
        let (id, at) = match self.view {
            ViewRefSpike::Heap(v) => find(LayoutView::node_count(v), &|i| {
                let n = LayoutView::node(v, i);
                (n.id, n.self_loop_at)
            }),
            ViewRefSpike::Arena(v) => find(LayoutView::node_count(v), &|i| {
                let n = LayoutView::node(v, i);
                (n.id, n.self_loop_at)
            }),
        }?;
        Some(EdgeViewSpike {
            scene_index: self.edge_count(), // appended after routed edges
            input_index: usize::MAX,        // real records carry the input index
            from_id: id,
            to_id: id,
            directed: true,
            reversed: false,
            from: at,
            to: at,
            path: EdgePathViewSpike::SelfLoop { at },
            label: Some(LabelViewSpike {
                text: self.loop_label,
                at,
            }),
        })
    }
}

// ── Allocation counting (spike-only; R6's counting gate supersedes) ──────

std::thread_local! {
    static ALLOC_COUNT: Cell<u64> = const { Cell::new(0) };
}

/// Delegates to `System`, counting per thread. Installed for the whole
/// lib-test binary while this spike exists (a per-allocation
/// thread-local bump; other tests are unaffected beyond that).
struct CountingAlloc;

unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOC_COUNT.with(|c| c.set(c.get() + 1));
        unsafe { System.alloc(layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        ALLOC_COUNT.with(|c| c.set(c.get() + 1));
        unsafe { System.alloc_zeroed(layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOC_COUNT.with(|c| c.set(c.get() + 1));
        unsafe { System.realloc(ptr, layout, new_size) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static COUNTING_ALLOC: CountingAlloc = CountingAlloc;

/// Shared with the other spike modules (composer_spike's R6 gate).
pub(super) fn allocations_on_this_thread() -> u64 {
    ALLOC_COUNT.with(|c| c.get())
}

// ── Fixture: every lending feature in one graph ──────────────────────────

/// Plain, boxed, custom-payload, and auto-created nodes; a labeled
/// edge; a level-skipping edge (MultiSegment waypoints); a self-loop;
/// a cluster.
fn corpus_graph() -> Graph<'static> {
    let mut g = Graph::new();
    g.add_node(1usize, "Root");
    g.add_node(2usize, BoxedNode("Boxed"));
    g.add_node(
        3usize,
        CustomNode {
            label: "card",
            width: 9,
            height: 3,
            painter: None,
            payload: "row1;row2",
        },
    );
    g.add_node(4usize, "Mid");
    g.add_edge(1usize, 2usize, None);
    g.add_edge(1usize, 3usize, None);
    g.add_edge(2usize, 4usize, None);
    g.add_edge(3usize, 4usize, None);
    g.add_edge(4usize, 5usize, Some("ships")); // 5 auto-created (implicit)
    g.add_edge(1usize, 5usize, None); // skips levels → MultiSegment
    g.add_edge(4usize, 4usize, Some("retry")); // self-loop
    let sg = g.add_subgraph("grp");
    g.put_nodes(&[2, 3]).inside(sg).unwrap();
    g
}

fn layout_config(direction: crate::graph::Direction) -> LayoutConfig<'static> {
    let mut cfg = LayoutConfig::standard();
    cfg.direction = direction;
    cfg.include_dummy_nodes = true; // dummy labels must lend ("") too
    cfg
}

/// Runs `check` twice: once with the heap IR, once with the arena IR.
fn with_both_backends(direction: crate::graph::Direction, check: &mut dyn FnMut(&dyn ErasedViews)) {
    let cfg = layout_config(direction);

    let g = corpus_graph();
    let heap_ir = g.compute_layout_with_config(&cfg);

    let g = corpus_graph();
    let mut csr_buf = vec![0u8; g.estimate_csr_arena_size() * 2];
    let mut csr_arena = crate::graph::arena::Arena::new(&mut csr_buf);
    let csr = g.to_csr(&mut csr_arena).unwrap();
    let size = (g.estimate_layout_arena_size() * 2).max(256 * 1024);
    let mut temp_buf = vec![0u8; size];
    let mut out_buf = vec![0u8; size];
    let mut temp_arena = crate::graph::arena::Arena::new(&mut temp_buf);
    let mut out_arena = crate::graph::arena::Arena::new(&mut out_buf);
    let arena_ir = csr
        .compute_layout_arena(&cfg, &mut temp_arena, &mut out_arena)
        .unwrap();

    check(&heap_ir);
    check(&arena_ir);
}

/// Object-safe adapter so the test driver can hand both concrete IRs
/// through one channel; each method monomorphizes the generic
/// projections underneath (the real API stays generic-free publicly,
/// enum-dispatched privately — spike 4.0a's facade).
trait ErasedViews {
    fn node_views(&self, out: &mut dyn FnMut(NodeViewSpike<'_>));
    fn edge_views(&self, out: &mut dyn FnMut(EdgeViewSpike<'_>));
    fn subgraph_views(&self, out: &mut dyn FnMut(SubgraphViewSpike<'_>));
}

impl<V: LayoutView> ErasedViews for V {
    fn node_views(&self, out: &mut dyn FnMut(NodeViewSpike<'_>)) {
        for i in 0..self.node_count() {
            out(node_view(self, i));
        }
    }
    fn edge_views(&self, out: &mut dyn FnMut(EdgeViewSpike<'_>)) {
        for i in 0..self.edge_count() {
            out(edge_view(self, i));
        }
    }
    fn subgraph_views(&self, out: &mut dyn FnMut(SubgraphViewSpike<'_>)) {
        for i in 0..self.subgraph_count() {
            out(subgraph_view(self, i));
        }
    }
}

// ── Fingerprints (allocating; parity test only) ──────────────────────────

fn node_fingerprint(n: &NodeViewSpike<'_>) -> String {
    // EVERY public field prints — parity is field-for-field, with no
    // exclusions: dummies have `id: None` plus a backend-stable
    // `dummy_of` identity, so nothing needs to be hidden.
    format!(
        "id={:?} dummy_of={:?} kind={:?} rect=({},{},{},{}) label={:?} payload={:?}",
        n.id, n.dummy_of, n.kind, n.x, n.y, n.width, n.height, n.label, n.payload
    )
}

fn edge_fingerprint(e: &EdgeViewSpike<'_>) -> String {
    format!(
        "input={} {}->{} directed={} reversed={} from={:?} to={:?} path={:?} label={:?}",
        e.input_index, e.from_id, e.to_id, e.directed, e.reversed, e.from, e.to, e.path, e.label
    )
}

fn subgraph_fingerprint(s: &SubgraphViewSpike<'_>) -> String {
    format!(
        "id={} parent={:?} label={:?} rect=({},{},{},{})",
        s.id, s.parent, s.label, s.x, s.y, s.width, s.height
    )
}

// ── The proofs ───────────────────────────────────────────────────────────

/// Question 2: no owning type can be a view field — `Vec`, `String`,
/// and every other heap-only type fails the `Copy` bound at compile
/// time.
#[test]
fn every_view_type_is_copy() {
    fn assert_copy<T: Copy>() {}
    assert_copy::<EdgePathViewSpike<'static>>();
    assert_copy::<LabelViewSpike<'static>>();
    assert_copy::<NodeViewSpike<'static>>();
    assert_copy::<EdgeViewSpike<'static>>();
    assert_copy::<SubgraphViewSpike<'static>>();
}

/// Question 1, half one: both backends produce identical views —
/// waypoint contents, label text, and custom payloads included — in
/// every enabled direction.
#[test]
fn views_agree_across_backends() {
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

    for dir in directions {
        let mut per_backend: Vec<Vec<String>> = Vec::new();
        with_both_backends(dir, &mut |views| {
            let mut prints = Vec::new();
            views.node_views(&mut |n| prints.push(node_fingerprint(&n)));
            views.edge_views(&mut |e| prints.push(edge_fingerprint(&e)));
            views.subgraph_views(&mut |s| prints.push(subgraph_fingerprint(&s)));
            prints.sort();
            per_backend.push(prints);
        });
        let [heap, arena] = per_backend.as_slice() else {
            panic!("expected two backends");
        };
        assert_eq!(heap, arena, "view parity failed for {dir:?}");

        // The corpus must actually exercise the lending paths.
        let all = heap.join("\n");
        assert!(
            all.contains("waypoints: ["),
            "no MultiSegment edge in corpus for {dir:?}:\n{all}"
        );
        assert!(all.contains("\"ships\""), "edge label missing:\n{all}");
        assert!(all.contains("\"row1;row2\""), "payload missing:\n{all}");
        assert!(all.contains("kind=Dummy"), "no dummy views:\n{all}");
    }
}

/// Question 1, half two: constructing and reading every view from BOTH
/// backends performs zero allocations — everything lends.
#[test]
fn views_construct_without_allocation() {
    fn fold(acc: u64, byte: u64) -> u64 {
        (acc ^ byte).wrapping_mul(0x100000001b3)
    }
    fn fold_str(mut acc: u64, s: &str) -> u64 {
        for b in s.bytes() {
            acc = fold(acc, u64::from(b));
        }
        acc
    }

    with_both_backends(crate::graph::Direction::TopDown, &mut |views| {
        let mut acc = 0u64;
        let before = allocations_on_this_thread();
        views.node_views(&mut |n| {
            let identity = n.id.or(n.dummy_of.map(|(e, _)| e)).unwrap_or(0) as u64;
            acc = fold_str(fold(acc, identity), n.label);
            if let Some(p) = n.payload {
                acc = fold_str(acc, p);
            }
        });
        views.edge_views(&mut |e| {
            acc = fold(acc, e.input_index as u64);
            if let EdgePathViewSpike::MultiSegment { waypoints, .. } = e.path {
                for &(x, y) in waypoints {
                    acc = fold(fold(acc, x as u64), y as u64);
                }
            }
            if let Some(l) = e.label {
                acc = fold_str(acc, l.text);
            }
        });
        views.subgraph_views(&mut |s| {
            acc = fold_str(fold(acc, s.id as u64), s.label);
        });
        let after = allocations_on_this_thread();
        std::hint::black_box(acc);
        assert_eq!(
            after - before,
            0,
            "view construction allocated {} times",
            after - before
        );
    });
}

/// Question 4: the IR's `Spline` stub never reaches the public view —
/// through the lens (both backends' lenses map it structurally) and
/// end-to-end from a hand-built heap IR.
#[test]
fn spline_normalizes_to_direct() {
    let lens = PathRef::Spline {
        cp1_x: 1,
        cp1_y: 2,
        cp2_x: 3,
        cp2_y: 4,
    };
    assert_eq!(path_view(lens), EdgePathViewSpike::Direct);

    let literal_node = |id: usize, label: &'static str, y: usize| crate::ir::LayoutNode {
        id,
        label,
        x: 0,
        y,
        width: 3,
        height: 1,
        center_x: 1,
        center_y: y,
        level: y / 2,
        level_position: 0,
        kind: NodeKind::Explicit,
        has_self_loop: false,
        self_loop_at: None,
        edge_index: None,
        content_tag: 0,
    };
    let mut b = crate::ir::LayoutIRBuilder::new().with_levels(2);
    b.add_node(literal_node(0, "a", 0));
    b.add_node(literal_node(1, "b", 2));
    b.add_edge(crate::ir::LayoutEdge {
        from_id: 0,
        to_id: 1,
        from_x: 1,
        from_y: 0,
        to_x: 1,
        to_y: 2,
        path: crate::ir::EdgePath::Spline {
            cp1_x: 1,
            cp1_y: 2,
            cp2_x: 3,
            cp2_y: 4,
        },
        flow_axis: crate::ir::FlowAxis::Y,
        edge_index: 0,
        label: None,
        label_x: 0,
        label_y: 0,
        directed: true,
        reversed: false,
    });
    b.set_dimensions(8, 3);
    let ir = b.build();
    assert_eq!(edge_view(&ir, 0).path, EdgePathViewSpike::Direct);
}

/// Question 3: one view value holding a planner-storage borrow (`'p`
/// label text) and IR borrows (`'ir` marker/waypoints) at once, both
/// shortened to the scene borrow — while a second view from the same
/// scene lends `'ir` waypoints. Exercised over both backends.
#[test]
fn planner_and_ir_borrows_unify_in_one_view_lifetime() {
    let planner_text = String::from("retry (planner-synthesized)");
    let cfg = layout_config(crate::graph::Direction::TopDown);

    let g = corpus_graph();
    let heap_ir = g.compute_layout_with_config(&cfg);

    let g = corpus_graph();
    let mut csr_buf = vec![0u8; g.estimate_csr_arena_size() * 2];
    let mut csr_arena = crate::graph::arena::Arena::new(&mut csr_buf);
    let csr = g.to_csr(&mut csr_arena).unwrap();
    let size = (g.estimate_layout_arena_size() * 2).max(256 * 1024);
    let mut temp_buf = vec![0u8; size];
    let mut out_buf = vec![0u8; size];
    let mut temp_arena = crate::graph::arena::Arena::new(&mut temp_buf);
    let mut out_arena = crate::graph::arena::Arena::new(&mut out_buf);
    let arena_ir = csr
        .compute_layout_arena(&cfg, &mut temp_arena, &mut out_arena)
        .unwrap();

    for view in [ViewRefSpike::Heap(&heap_ir), ViewRefSpike::Arena(&arena_ir)] {
        let scene = SceneStubSpike {
            loop_label: &planner_text,
            view,
        };

        let loop_view = scene.self_loop_view().expect("corpus has a self-loop");
        let multi = (0..scene.edge_count())
            .map(|i| scene.edge_view(i))
            .find(|e| matches!(e.path, EdgePathViewSpike::MultiSegment { .. }))
            .expect("corpus has a MultiSegment edge");

        // Both views alive together, one 's each, mixed origins.
        assert_eq!(loop_view.label.unwrap().text, "retry (planner-synthesized)");
        assert!(matches!(loop_view.path, EdgePathViewSpike::SelfLoop { .. }));
        let EdgePathViewSpike::MultiSegment { waypoints, .. } = multi.path else {
            unreachable!();
        };
        assert!(!waypoints.is_empty());
    }
}
