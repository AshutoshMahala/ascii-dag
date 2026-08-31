//! Scene element views — storage-neutral, read-only projections.
//!
//! Every view is a small `Copy` value constructed on demand from the
//! scene's private lens: no view is materialized or stored, borrowed
//! text and waypoints LEND from whichever IR backend produced the
//! layout (zero allocation, pinned by test), and no heap-only type can
//! appear in a field (`Copy` is the guardrail; the cross-backend
//! construction tests are the proof). Both backends produce
//! field-for-field identical views.

use super::color::CellColor;
use super::node_content::NodeKindTag;
use super::plan::{LabelPlan, RenderPlan};
use super::scene::{Scene, ViewRef};
use super::style::{LabelPosition, LineWeight, MarkerShape, SubgraphBorder};
use super::view::{LayoutView, PathRef};
use crate::ir::FlowAxis;

/// Real-vs-dummy: the scene's top-level node dichotomy (deliberately
/// EXHAUSTIVE — a closed promise, matchable without a wildcard).
/// Provenance detail lives inside [`Real`](Self::Real) and can grow
/// without disturbing real/dummy matches.
///
/// This is the scene vocabulary; the IR keeps its flat
/// [`ir::NodeKind`](crate::ir::NodeKind) (`Explicit | Implicit |
/// Dummy`) untouched until the 0.12 IR overhaul.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeKind {
    /// A node of the input graph.
    Real {
        /// How the node entered the graph.
        origin: NodeOrigin,
    },
    /// A routing waypoint synthesized by layout. Appears in
    /// [`Scene::nodes`] only when
    /// [`PlanOptions::show_dummy_nodes`](super::config::PlanOptions::show_dummy_nodes)
    /// is set (matching what paints and what hit-tests).
    Dummy,
}

/// How a real node entered the graph.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeOrigin {
    /// Declared via `add_node`.
    Declared,
    /// Auto-created by an edge referencing an undeclared id.
    EdgeInferred,
}

/// One node of the scene.
#[non_exhaustive]
#[derive(Debug, Clone, Copy)]
pub struct NodeView<'s> {
    /// Graph id — `None` for dummies: their raw ids are synthetic and
    /// backend-specific, so the scene never exposes them. Dummies
    /// carry [`dummy_of`](Self::dummy_of) instead.
    pub id: Option<usize>,
    /// Backend-stable dummy identity: the owning edge's INPUT index
    /// (the style-callback / [`EdgeView::input_index`] convention) and
    /// the level the dummy occupies — the same pair
    /// [`HitResult::Dummy`](super::plan::HitResult::Dummy) reports.
    /// `None` for real nodes.
    pub dummy_of: Option<(usize, usize)>,
    /// Left edge, in cells.
    pub x: usize,
    /// Top edge, in rows.
    pub y: usize,
    /// Width in cells (including brackets/padding).
    pub width: usize,
    /// Height in rows.
    pub height: usize,
    /// Real-vs-dummy dichotomy.
    pub kind: NodeKind,
    /// Label text (empty for dummies).
    pub label: &'s str,
    /// Declared content kind (simple / boxed / custom).
    pub content: NodeKindTag,
    /// Custom-node payload — the **data** half of the template/data
    /// pair, rejoined by the scene ("geometry and keyed content meet
    /// again here"). `None` when the node declared no custom content.
    pub payload: Option<&'s str>,
}

/// One edge of the scene, fully resolved (style callbacks already
/// ran, exactly once, at plan time).
#[non_exhaustive]
#[derive(Debug, Clone, Copy)]
pub struct EdgeView<'s> {
    /// Position in the scene's edge list — the hit-testing, legend,
    /// and palette-default convention
    /// ([`HitResult::Edge`](super::plan::HitResult::Edge)).
    pub scene_index: usize,
    /// Original graph insertion index — the style-callback convention
    /// ([`EdgeStyleCtx::edge_index`](super::style::EdgeStyleCtx)).
    /// The two diverge when self-loops exist (self-loops are absent
    /// from the routed list).
    pub input_index: usize,
    /// Source node id.
    pub from_id: usize,
    /// Target node id.
    pub to_id: usize,
    /// Whether an arrowhead was requested.
    pub directed: bool,
    /// Reversed during cycle breaking (renders dashed by default).
    pub reversed: bool,
    /// Resolved stroke color (plain emission ignores it).
    pub color: CellColor,
    /// Resolved stroke weight.
    pub weight: LineWeight,
    /// Marker at the LOGICAL source end. Which geometric end paints
    /// follows [`reversed`](Self::reversed) — the view exposes both
    /// markers plus `reversed` so consumers can reconstruct either
    /// framing.
    pub marker_source: MarkerShape,
    /// Marker at the LOGICAL target end.
    pub marker_target: MarkerShape,
    /// Routed source anchor, physical `(x, y)`.
    pub from: (usize, usize),
    /// Routed target anchor, physical `(x, y)`.
    pub to: (usize, usize),
    /// Routed geometry.
    pub path: EdgePathView<'s>,
    /// Physical axis of the edge's trunk.
    pub flow_axis: FlowAxis,
    /// The edge's label, if it declared one — with its resolved color
    /// and where it ended up (inline, legend, or omitted).
    pub label: Option<LabelView<'s>>,
}

/// Storage-neutral routed path. The heap IR's `Vec` waypoints and the
/// arena IR's shared-slice waypoints both LEND here — no IR type
/// appears in the scene surface.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgePathView<'s> {
    /// Straight flow segment.
    Direct,
    /// L-shaped connection with one cross-axis segment at `bend_at`.
    Corner {
        /// The level-axis line the cross segment runs on.
        bend_at: usize,
    },
    /// Routed through a far cross-axis channel.
    SideChannel {
        /// Cross-axis line of the channel.
        channel_at: usize,
        /// Level-axis start of the channel span.
        span_start: usize,
        /// Level-axis end of the channel span.
        span_end: usize,
    },
    /// Multi-segment path through routing waypoints — physical
    /// `(x, y)` cells, lent from the IR.
    MultiSegment {
        /// The waypoint cells, in path order.
        waypoints: &'s [(usize, usize)],
        /// Level-axis offset of the first bend past the source.
        start_offset: usize,
    },
    /// A preserved self-loop: no routed path, one marker cell.
    SelfLoop {
        /// The `↺` marker cell.
        at: (usize, usize),
    },
}

/// An edge label, resolved: text, color, and where it landed.
#[non_exhaustive]
#[derive(Debug, Clone, Copy)]
pub struct LabelView<'s> {
    /// Label text (never transliterated).
    pub text: &'s str,
    /// Resolved label color.
    pub color: CellColor,
    /// Where the label ended up under the plan's label policy.
    pub slot: LabelSlot,
}

/// Where a label landed.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LabelSlot {
    /// Painted inline at `(x, y)`, spanning `len` cells (quotes
    /// included).
    Inline {
        /// Left cell of the painted span.
        x: usize,
        /// Row of the painted span.
        y: usize,
        /// Painted length in cells.
        len: usize,
    },
    /// Overflowed to the legend
    /// ([`LabelOverflow::Legend`](super::config::LabelOverflow::Legend)).
    Legend,
    /// Unplaceable, and the policy says omit — the label appears
    /// nowhere in the output.
    Omitted,
}

/// One cluster of the scene.
#[non_exhaustive]
#[derive(Debug, Clone, Copy)]
pub struct SubgraphView<'s> {
    /// Subgraph id.
    pub id: usize,
    /// Immediate parent cluster, if nested.
    pub parent: Option<usize>,
    /// Cluster label.
    pub label: &'s str,
    /// Left edge, in cells.
    pub x: usize,
    /// Top edge, in rows.
    pub y: usize,
    /// Width in cells.
    pub width: usize,
    /// Height in rows.
    pub height: usize,
    /// Resolved border style.
    pub border: SubgraphBorder,
    /// Resolved border color.
    pub color: CellColor,
    /// Resolved label position.
    pub label_position: LabelPosition,
}

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

impl Scene<'_, '_> {
    /// The scene's nodes. Dummies (routing waypoints) appear only
    /// when the scene was planned with `show_dummy_nodes` — matching
    /// what paints and what hit-tests.
    pub fn nodes(&self) -> impl Iterator<Item = NodeView<'_>> + '_ {
        let count = with_view!(self, v => LayoutView::node_count(v));
        let show_dummies = self.plan().show_dummy_nodes();
        (0..count)
            .map(move |i| self.node_view_at(i))
            .filter(move |n| show_dummies || !matches!(n.kind, NodeKind::Dummy))
    }

    /// One node by its graph id (real nodes only — dummies have no
    /// graph id; find them via [`nodes`](Self::nodes)).
    pub fn node(&self, id: usize) -> Option<NodeView<'_>> {
        let count = with_view!(self, v => LayoutView::node_count(v));
        (0..count)
            .map(|i| self.node_view_at(i))
            .find(|n| n.id == Some(id))
    }

    /// The scene's edges, in scene order: routed edges first, then
    /// preserved self-loops (identity, label, and resolved style
    /// intact — style callbacks ran for them at plan time).
    pub fn edges(&self) -> impl Iterator<Item = EdgeView<'_>> + '_ {
        (0..self.scene_edge_count()).map(move |i| self.edge_view_at(i))
    }

    /// One edge by scene index (self-loops sit after the routed list).
    pub fn edge(&self, scene_index: usize) -> Option<EdgeView<'_>> {
        (scene_index < self.scene_edge_count()).then(|| self.edge_view_at(scene_index))
    }

    fn scene_edge_count(&self) -> usize {
        with_view!(self, v => LayoutView::edge_count(v) + LayoutView::self_loop_count(v))
    }

    /// The scene's clusters, in declaration order.
    pub fn subgraphs(&self) -> impl Iterator<Item = SubgraphView<'_>> + '_ {
        let count = with_view!(self, v => LayoutView::subgraph_count(v));
        (0..count).map(move |i| self.subgraph_view_at(i))
    }

    /// Edges whose labels overflowed to the legend, in emission order
    /// (the same list [`legend_entries`](Self::legend_entries)
    /// indexes) — self-loop labels included: they have no inline
    /// placement host, so a labeled loop always lands here (or is
    /// omitted, per the overflow policy).
    pub fn legend(&self) -> impl Iterator<Item = EdgeView<'_>> + '_ {
        self.legend_entries()
            .iter()
            .map(move |&i| self.edge_view_at(i))
    }

    fn node_view_at(&self, index: usize) -> NodeView<'_> {
        with_view!(self, v => {
            let n = LayoutView::node(v, index);
            let (painter, payload) = LayoutView::node_custom(v, index);
            let kind = match n.kind {
                crate::ir::NodeKind::Explicit => NodeKind::Real {
                    origin: NodeOrigin::Declared,
                },
                crate::ir::NodeKind::Implicit => NodeKind::Real {
                    origin: NodeOrigin::EdgeInferred,
                },
                crate::ir::NodeKind::Dummy => NodeKind::Dummy,
            };
            let dummy = matches!(kind, NodeKind::Dummy);
            NodeView {
                id: (!dummy).then_some(n.id),
                dummy_of: if dummy {
                    Some((n.edge_index.unwrap_or(usize::MAX), n.level))
                } else {
                    None
                },
                x: n.x,
                y: n.y,
                width: n.width,
                height: n.height,
                kind,
                label: n.label,
                content: NodeKindTag::from_u8(n.content_tag),
                // The lens yields `(None, "")` for nodes without a
                // custom entry; a fully blank custom node stores no
                // entry, so this mapping is lossless.
                payload: if painter.is_none() && payload.is_empty() {
                    None
                } else {
                    Some(payload)
                },
            }
        })
    }

    fn edge_view_at(&self, index: usize) -> EdgeView<'_> {
        let plan = self.plan();
        let routed = with_view!(self, v => LayoutView::edge_count(v));
        if index >= routed {
            return self.self_loop_view_at(index, index - routed);
        }
        with_view!(self, v => {
            let e = LayoutView::edge(v, index);
            let ep = plan.edge_plan(index);
            EdgeView {
                scene_index: index,
                input_index: e.edge_index,
                from_id: e.from_id,
                to_id: e.to_id,
                directed: e.directed,
                reversed: e.reversed,
                color: ep.color,
                weight: ep.weight,
                marker_source: ep.marker_start,
                marker_target: ep.marker_end,
                from: (e.from_x, e.from_y),
                to: (e.to_x, e.to_y),
                path: match e.path {
                    // The IR's `Spline` variant is a forward-compat
                    // stub the layout engine never emits; the
                    // compositor falls back to `Direct`, and the view
                    // boundary does the same.
                    PathRef::Direct | PathRef::Spline { .. } => EdgePathView::Direct,
                    PathRef::Corner { bend_at } => EdgePathView::Corner { bend_at },
                    PathRef::SideChannel {
                        channel_at,
                        span_start,
                        span_end,
                    } => EdgePathView::SideChannel {
                        channel_at,
                        span_start,
                        span_end,
                    },
                    PathRef::MultiSegment {
                        waypoints,
                        start_offset,
                    } => EdgePathView::MultiSegment {
                        waypoints,
                        start_offset,
                    },
                },
                flow_axis: e.flow_axis,
                label: e.label.map(|text| LabelView {
                    text,
                    color: ep.label_color,
                    slot: label_slot(plan, index),
                }),
            }
        })
    }

    /// Synthesize the view of preserved self-loop `j` (scene index
    /// `scene_index`). The marker cell anchors both endpoints; the
    /// label — which has no inline placement host — reports where it
    /// actually went (legend or omitted).
    fn self_loop_view_at(&self, scene_index: usize, j: usize) -> EdgeView<'_> {
        let plan = self.plan();
        with_view!(self, v => {
            let r = LayoutView::self_loop(v, j);
            let at = (0..LayoutView::node_count(v))
                .map(|i| LayoutView::node(v, i))
                .find(|n| n.id == r.node_id)
                .and_then(|n| n.self_loop_at)
                .unwrap_or((0, 0));
            let ep = plan.scene_edge_plan(scene_index);
            let flow_axis = flow_axis_of(LayoutView::direction(v));
            EdgeView {
                scene_index,
                input_index: r.input_index,
                from_id: r.node_id,
                to_id: r.node_id,
                directed: true,
                reversed: false,
                color: ep.color,
                weight: ep.weight,
                marker_source: ep.marker_start,
                marker_target: ep.marker_end,
                from: at,
                to: at,
                path: EdgePathView::SelfLoop { at },
                flow_axis,
                label: r.label.map(|text| LabelView {
                    text,
                    color: ep.label_color,
                    slot: if plan.legend_entries().binary_search(&scene_index).is_ok() {
                        LabelSlot::Legend
                    } else {
                        LabelSlot::Omitted
                    },
                }),
            }
        })
    }

    fn subgraph_view_at(&self, index: usize) -> SubgraphView<'_> {
        let plan = self.plan();
        with_view!(self, v => {
            let sg = LayoutView::subgraph(v, index);
            let sp = plan.subgraph_plan(index);
            SubgraphView {
                id: sg.id,
                parent: sg.parent,
                label: sg.label,
                x: sg.x,
                y: sg.y,
                width: sg.width,
                height: sg.height,
                border: sp.border,
                color: sp.color,
                label_position: sp.label_pos,
            }
        })
    }
}

/// The physical trunk axis for a given rank direction (an `if`
/// rather than a `match` so each single-axis build sees only the
/// variants it has).
fn flow_axis_of(direction: crate::graph::Direction) -> FlowAxis {
    #[cfg(feature = "layout-horizontal")]
    if matches!(
        direction,
        crate::graph::Direction::LeftRight | crate::graph::Direction::RightLeft
    ) {
        return FlowAxis::X;
    }
    let _ = direction;
    FlowAxis::Y
}

/// Where edge `index`'s label landed under this plan. Both plan lists
/// are built in ascending edge order, so lookups are binary searches.
fn label_slot(plan: &RenderPlan<'_>, index: usize) -> LabelSlot {
    let labels = plan.labels();
    let Ok(pos) = labels.binary_search_by_key(&index, |l: &LabelPlan| l.edge_index) else {
        // An edge with label text but no label plan cannot happen for
        // routed edges today; be conservative rather than panic.
        return LabelSlot::Omitted;
    };
    let label = &labels[pos];
    if label.paints_under(plan.label_placement()) {
        LabelSlot::Inline {
            x: label.x,
            y: label.y,
            len: label.len,
        }
    } else if plan.legend_entries().binary_search(&index).is_ok() {
        LabelSlot::Legend
    } else {
        LabelSlot::Omitted
    }
}

#[cfg(all(test, feature = "std", feature = "layout-vertical"))]
mod tests {
    use super::super::config::{LabelOverflow, LabelPolicy, PlanOptions};
    use super::super::scene::ScenePlanner;
    use super::super::test_alloc::allocations_on_this_thread;
    use super::*;
    use crate::graph::Graph;
    use crate::graph::arena::Arena;
    use crate::render::engine::{BoxedNode, CustomNode};

    /// Plain, boxed, custom-payload, and auto-created nodes; a labeled
    /// edge; a level-skipping edge (waypoints); a cluster.
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
        g.add_edge(4usize, 5usize, Some("ships")); // 5 auto-created
        g.add_edge(1usize, 5usize, None); // skips levels → waypoints
        g.add_edge(4usize, 4usize, Some("retry")); // preserved self-loop
        let sg = g.add_subgraph("grp");
        g.put_nodes(&[2, 3]).inside(sg).unwrap();
        g
    }

    fn layout_config(direction: crate::graph::Direction) -> crate::LayoutConfig<'static> {
        let mut cfg = crate::LayoutConfig::standard();
        cfg.direction = direction;
        cfg.include_dummy_nodes = true;
        cfg
    }

    /// Run `check` once per backend, over a scene planned with
    /// `options`.
    fn with_both_backend_scenes(
        direction: crate::graph::Direction,
        options: &PlanOptions,
        check: &mut dyn FnMut(&Scene<'_, '_>),
    ) {
        let cfg = layout_config(direction);

        let g = corpus_graph();
        let heap_ir = g.compute_layout_with_config(&cfg);
        let mut planner = ScenePlanner::new();
        check(&planner.plan(&heap_ir, options).unwrap());

        let g = corpus_graph();
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
        check(&planner.plan(&arena_ir, options).unwrap());
    }

    /// No owning type can be a view field — `Vec`, `String`, and every
    /// other heap-only type fails the `Copy` bound at compile time
    /// (the guardrail; the cross-backend construction below is the
    /// proof).
    #[test]
    fn every_view_type_is_copy() {
        fn assert_copy<T: Copy>() {}
        assert_copy::<NodeView<'static>>();
        assert_copy::<EdgeView<'static>>();
        assert_copy::<EdgePathView<'static>>();
        assert_copy::<LabelView<'static>>();
        assert_copy::<SubgraphView<'static>>();
        assert_copy::<NodeKind>();
        assert_copy::<NodeOrigin>();
        assert_copy::<LabelSlot>();
    }

    /// Both backends produce field-for-field identical views — the
    /// `Debug` fingerprints cover EVERY public field, with no
    /// exclusions (dummy identity included), in every enabled
    /// direction.
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
            let options = PlanOptions::new().with_show_dummy_nodes(true);
            let mut per_backend: Vec<Vec<String>> = Vec::new();
            with_both_backend_scenes(dir, &options, &mut |scene| {
                let mut prints: Vec<String> = Vec::new();
                prints.extend(scene.nodes().map(|n| format!("{n:?}")));
                prints.extend(scene.edges().map(|e| format!("{e:?}")));
                prints.extend(scene.subgraphs().map(|s| format!("{s:?}")));
                prints.sort();
                per_backend.push(prints);
            });
            let [heap, arena] = per_backend.as_slice() else {
                panic!("expected two backends");
            };
            assert_eq!(heap, arena, "view parity failed for {dir:?}");

            // The corpus must actually exercise the lending paths.
            let all = heap.join("\n");
            assert!(all.contains("waypoints: ["), "no MultiSegment:\n{all}");
            assert!(all.contains("\"ships\""), "edge label missing:\n{all}");
            assert!(all.contains("\"row1;row2\""), "payload missing:\n{all}");
            assert!(all.contains("Dummy"), "no dummy views:\n{all}");
            assert!(all.contains("SelfLoop"), "no self-loop view:\n{all}");
        }
    }

    /// Constructing and reading every view performs zero allocations
    /// on both backends — text, payloads, and waypoints all LEND.
    #[test]
    fn views_construct_without_allocation() {
        fn fold(acc: u64, v: u64) -> u64 {
            (acc ^ v).wrapping_mul(0x100000001b3)
        }
        fn fold_str(mut acc: u64, s: &str) -> u64 {
            for b in s.bytes() {
                acc = fold(acc, u64::from(b));
            }
            acc
        }

        let options = PlanOptions::new().with_show_dummy_nodes(true);
        with_both_backend_scenes(crate::graph::Direction::TopDown, &options, &mut |scene| {
            let mut acc = 0u64;
            let before = allocations_on_this_thread();
            for n in scene.nodes() {
                acc = fold_str(fold(acc, n.id.unwrap_or(0) as u64), n.label);
                if let Some(p) = n.payload {
                    acc = fold_str(acc, p);
                }
            }
            for e in scene.edges() {
                acc = fold(acc, e.input_index as u64);
                if let EdgePathView::MultiSegment { waypoints, .. } = e.path {
                    for &(x, y) in waypoints {
                        acc = fold(fold(acc, x as u64), y as u64);
                    }
                }
                if let Some(l) = e.label {
                    acc = fold_str(acc, l.text);
                }
            }
            for s in scene.subgraphs() {
                acc = fold_str(fold(acc, s.id as u64), s.label);
            }
            let after = allocations_on_this_thread();
            std::hint::black_box(acc);
            assert_eq!(after - before, 0, "view construction allocated");
        });
    }

    /// The IR's `Spline` stub never reaches the public view: it
    /// normalizes to `Direct`, the compositor's own fallback.
    #[test]
    fn spline_normalizes_to_direct() {
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
            kind: crate::ir::NodeKind::Explicit,
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
        let mut planner = ScenePlanner::new();
        let scene = planner.plan(&ir, &PlanOptions::new()).unwrap();
        assert_eq!(scene.edge(0).unwrap().path, EdgePathView::Direct);
        assert!(scene.edge(1).is_none(), "one edge only");
    }

    /// Dummies appear in `nodes()` exactly when the scene shows them —
    /// with `id: None` and a backend-stable `dummy_of` identity —
    /// and `node(id)` finds real nodes only.
    #[test]
    fn dummy_visibility_follows_the_plan() {
        let g = corpus_graph();
        let ir = g.compute_layout_with_config(&layout_config(crate::graph::Direction::TopDown));
        let mut planner = ScenePlanner::new();

        let hidden = planner.plan(&ir, &PlanOptions::new()).unwrap();
        assert!(
            hidden.nodes().all(|n| !matches!(n.kind, NodeKind::Dummy)),
            "dummies hidden by default"
        );
        drop(hidden);

        let shown = planner
            .plan(&ir, &PlanOptions::new().with_show_dummy_nodes(true))
            .unwrap();
        let dummies: Vec<NodeView<'_>> = shown
            .nodes()
            .filter(|n| matches!(n.kind, NodeKind::Dummy))
            .collect();
        assert!(!dummies.is_empty(), "skip edge must produce dummies");
        for d in &dummies {
            assert_eq!(d.id, None);
            assert!(d.dummy_of.is_some());
            assert!(d.label.is_empty());
        }
        assert!(shown.node(1).is_some());
        assert_eq!(
            shown.node(1).unwrap().kind,
            NodeKind::Real {
                origin: NodeOrigin::Declared
            }
        );
        assert_eq!(
            shown.node(5).unwrap().kind,
            NodeKind::Real {
                origin: NodeOrigin::EdgeInferred
            }
        );
    }

    /// Two labels, one placeable and one that fits nowhere (longer
    /// than anything the canvas offers).
    fn labeled_graph() -> Graph<'static> {
        let mut g = Graph::new();
        g.add_node(1usize, "A");
        g.add_node(2usize, "B");
        g.add_node(3usize, "C");
        g.add_edge(
            1usize,
            2usize,
            Some("an-extremely-long-label-that-cannot-possibly-fit-inline-anywhere-at-all"),
        );
        g.add_edge(1usize, 3usize, Some("ok"));
        g
    }

    /// Label slots mirror the plan's policy exactly: `Inline` iff the
    /// label paints, `Legend` iff it overflowed to the legend list,
    /// `Omitted` otherwise — and `legend()` yields exactly the
    /// overflowed edges, in emission order.
    #[test]
    fn label_slots_reflect_policy() {
        let ir = labeled_graph().compute_layout();
        let mut planner = ScenePlanner::new();

        let overflow_legend = PlanOptions::new()
            .with_label_policy(LabelPolicy::new().with_overflow(LabelOverflow::Legend));
        let scene = planner.plan(&ir, &overflow_legend).unwrap();
        let mut inline = 0usize;
        let mut legend = Vec::new();
        for e in scene.edges() {
            match e.label.map(|l| l.slot) {
                Some(LabelSlot::Inline { len, .. }) => {
                    assert!(len > 0);
                    inline += 1;
                }
                Some(LabelSlot::Legend) => legend.push(e.scene_index),
                Some(LabelSlot::Omitted) => panic!("Legend overflow never omits"),
                None => {}
            }
        }
        assert!(inline > 0, "the short label places inline");
        assert!(!legend.is_empty(), "the giant label overflows");
        assert_eq!(legend, scene.legend_entries().to_vec());
        let legend_views: Vec<usize> = scene.legend().map(|e| e.scene_index).collect();
        assert_eq!(legend, legend_views);
        drop(scene);

        // Same placement, Omit overflow: the same unplaced labels are
        // now Omitted and the legend is empty.
        let scene = planner.plan(&ir, &PlanOptions::new()).unwrap();
        let omitted = scene
            .edges()
            .filter(|e| matches!(e.label.map(|l| l.slot), Some(LabelSlot::Omitted)))
            .count();
        assert_eq!(omitted, legend.len(), "unplaced labels become Omitted");
        assert!(scene.legend_entries().is_empty());
        assert_eq!(scene.legend().count(), 0);
    }
}

#[cfg(all(test, feature = "std", feature = "layout-vertical"))]
mod self_loop_tests {
    use super::super::config::{LabelOverflow, LabelPolicy, PlanOptions};
    use super::super::plan::HitResult;
    use super::super::scene::ScenePlanner;
    use super::*;
    use crate::graph::Graph;

    /// Loop FIRST, so input and scene indices diverge: input 0 is the
    /// loop, inputs 1..3 are routed edges at scene 0..2, and the loop's
    /// scene index is 3 (after the routed list).
    fn divergent_graph() -> Graph<'static> {
        let mut g = Graph::new();
        g.add_node(1usize, "Gate");
        g.add_node(2usize, "Mid");
        g.add_node(3usize, "End");
        g.add_edge(1usize, 1usize, Some("retry")); // input 0: self-loop
        g.add_edge(1usize, 2usize, None); // input 1 → scene 0
        g.add_edge(2usize, 3usize, None); // input 2 → scene 1
        g.add_edge(1usize, 3usize, None); // input 3 → scene 2
        g
    }

    /// The preserved loop is a full scene edge: both index conventions
    /// named, label carried, marker as its path — and the `↺` cell
    /// hit-tests as the loop EDGE, not the node.
    #[test]
    fn self_loop_identity_survives_to_the_scene() {
        let ir = divergent_graph().compute_layout();
        let mut planner = ScenePlanner::new();
        let options = PlanOptions::new()
            .with_label_policy(LabelPolicy::new().with_overflow(LabelOverflow::Legend));
        let scene = planner.plan(&ir, &options).unwrap();

        assert_eq!(scene.edges().count(), 4, "3 routed + 1 loop");
        let luup = scene.edge(3).expect("loop sits after the routed list");
        assert_eq!(luup.scene_index, 3);
        assert_eq!(luup.input_index, 0, "original insertion index");
        assert_eq!((luup.from_id, luup.to_id), (1, 1));
        let EdgePathView::SelfLoop { at } = luup.path else {
            panic!("loop path is the marker cell");
        };
        assert_eq!(luup.from, at);
        let label = luup.label.expect("label preserved");
        assert_eq!(label.text, "retry");
        assert_eq!(label.slot, LabelSlot::Legend);
        assert_eq!(scene.legend().map(|e| e.scene_index).next(), Some(3));

        // The marker cell belongs to the loop EDGE now.
        assert_eq!(scene.hit_test(at.0, at.1), HitResult::Edge(3));
    }

    /// Style callbacks run for preserved loops — with the loop's own
    /// input identity — and their answers land in the view.
    #[test]
    fn style_callbacks_run_for_self_loops() {
        fn styled(ctx: super::super::style::EdgeStyleCtx<'_>) -> super::super::style::EdgeStyle {
            if ctx.from_id == ctx.to_id {
                assert_eq!(ctx.edge_index, 0, "loop keeps its input index");
                assert_eq!(ctx.label, Some("retry"));
                super::super::style::EdgeStyle {
                    color: CellColor::ansi256(199),
                    ..Default::default()
                }
            } else {
                super::super::style::EdgeStyle::default()
            }
        }
        let ir = divergent_graph().compute_layout();
        let mut planner = ScenePlanner::new();
        let scene = planner
            .plan(&ir, &PlanOptions::new().with_edge_style_fn(styled))
            .unwrap();
        assert_eq!(scene.edge(3).unwrap().color, CellColor::ansi256(199));
    }

    /// Acceptance: routed-list palette indices NEVER shift — inserting
    /// a self-loop (even first) leaves every routed edge's default
    /// color exactly as the loop-free graph resolves it.
    #[test]
    fn routed_palette_never_shifts() {
        let mut without = Graph::new();
        without.add_node(1usize, "Gate");
        without.add_node(2usize, "Mid");
        without.add_node(3usize, "End");
        without.add_edge(1usize, 2usize, None);
        without.add_edge(2usize, 3usize, None);
        without.add_edge(1usize, 3usize, None);

        let ir_without = without.compute_layout();
        let ir_with = divergent_graph().compute_layout();
        let mut planner = ScenePlanner::new();
        let options = PlanOptions::new();

        let colors_without: Vec<CellColor> = {
            let scene = planner.plan(&ir_without, &options).unwrap();
            (0..3).map(|i| scene.edge(i).unwrap().color).collect()
        };
        let scene = planner.plan(&ir_with, &options).unwrap();
        let colors_with: Vec<CellColor> = (0..3).map(|i| scene.edge(i).unwrap().color).collect();
        assert_eq!(colors_without, colors_with, "routed palette must not shift");
    }
}
