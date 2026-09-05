//! `LayoutView` — one lens over both IRs (temp/06 §2, N1/M1).
//!
//! The engine is generic over this trait (monomorphized, no `dyn`), so
//! there is exactly one paint path and "backend parity" stops being a
//! discipline at the render layer — it is the type system's problem.
//!
//! The view is accessor-only: small `Copy` reference structs with
//! unified field access. Differences between the IRs are absorbed here
//! and only here (heap `&str` labels vs arena offset resolution, heap
//! `Option` vs arena sentinel conventions, heap `Vec` waypoints vs
//! arena shared-slice waypoints).

use crate::graph::Direction;
use crate::ir::NodeKind;

/// A node as the engine sees it.
#[derive(Debug, Clone, Copy)]
// Parity-by-construction (N1): every IR field is mirrored here and
// checked by the equivalence tests, whether or not the paint path reads
// it yet — adding an IR field without wiring the view must stay a
// compile error. Fields awaiting their consumer carry an allow.
pub(crate) struct NodeRef<'a> {
    pub id: usize,
    pub label: &'a str,
    pub x: usize,
    pub y: usize,
    pub width: usize,
    pub height: usize,
    #[allow(dead_code)] // no paint consumer yet (parity contract, N1)
    pub center_x: usize,
    #[allow(dead_code)] // no paint consumer yet (parity contract, N1)
    pub center_y: usize,
    #[allow(dead_code)] // no paint consumer yet (parity contract, N1)
    pub level: usize,
    #[allow(dead_code)] // no paint consumer yet (parity contract, N1)
    pub level_position: usize,
    pub kind: NodeKind,
    /// `self_loop_at.is_some()` by the D5 invariant — paint and
    /// hit-testing read the cell directly.
    #[allow(dead_code)] // superseded by self_loop_at (parity contract, N1)
    pub has_self_loop: bool,
    /// Self-loop marker cell (arena sentinel normalized to `None`).
    pub self_loop_at: Option<(usize, usize)>,
    /// Owning edge for dummy nodes; `None` for real nodes.
    #[allow(dead_code)] // consumer lands with RW7 dummy introspection
    pub edge_index: Option<usize>,
    /// Declared content kind (raw `NodeKindTag` value) — routes
    /// `paint_node`.
    pub content_tag: u8,
}

/// An edge path as the engine sees it — waypoints are a plain slice in
/// both backends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PathRef<'a> {
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
        waypoints: &'a [(usize, usize)],
        start_offset: usize,
    },
    #[cfg(feature = "ports")]
    /// Explicit polyline — every bend stated (see
    /// `EdgePath::Orthogonal`).
    Orthogonal {
        bends: &'a [(usize, usize)],
    },
    Spline {
        cp1_x: usize,
        cp1_y: usize,
        cp2_x: usize,
        cp2_y: usize,
    },
}

/// An edge as the engine sees it.
#[derive(Debug, Clone, Copy)]
pub(crate) struct EdgeRef<'a> {
    pub from_id: usize,
    pub to_id: usize,
    pub from_x: usize,
    pub from_y: usize,
    pub to_x: usize,
    pub to_y: usize,
    pub edge_index: usize,
    /// `None` when the edge has no label (arena: empty label storage).
    pub label: Option<&'a str>,
    /// Meaningful iff `label.is_some()`.
    pub label_x: usize,
    /// Meaningful iff `label.is_some()`.
    pub label_y: usize,
    pub directed: bool,
    pub reversed: bool,
    /// Physical axis of the edge's trunk (temp/08 D2) — selects the
    /// compositor's paint path.
    pub flow_axis: crate::ir::FlowAxis,
    pub path: PathRef<'a>,
}

/// A preserved self-loop as the engine sees it.
#[derive(Debug, Clone, Copy)]
pub(crate) struct SelfLoopRef<'a> {
    /// The node the loop is on (user id).
    pub node_id: usize,
    /// The node's position in the view's node table (O(1) join;
    /// resolved at layout — hand-built IRs own its bounds).
    pub node_index: usize,
    /// Original graph insertion index (the style-callback convention).
    pub input_index: usize,
    /// `None` when unlabeled (arena: empty label storage).
    pub label: Option<&'a str>,
}

/// A subgraph (cluster) as the engine sees it.
#[derive(Debug, Clone, Copy)]
pub(crate) struct SubgraphRef<'a> {
    pub id: usize,
    /// Parent subgraph id; `None` for root-level clusters.
    pub parent: Option<usize>,
    pub label: &'a str,
    pub x: usize,
    pub y: usize,
    pub width: usize,
    pub height: usize,
}

/// Accessor-only view over a laid-out graph. Implemented by both IRs;
/// the engine never touches an IR type directly.
pub(crate) trait LayoutView {
    fn width(&self) -> usize;
    fn height(&self) -> usize;
    #[allow(dead_code)] // no paint consumer yet (parity contract, N1)
    fn level_count(&self) -> usize;
    /// Never consulted by paint code (M4 — flow derives from
    /// coordinates); exposed for introspection consumers.
    #[allow(dead_code)]
    fn direction(&self) -> Direction;
    fn node_count(&self) -> usize;
    fn node(&self, index: usize) -> NodeRef<'_>;
    /// A node's declared custom content: painter + payload. Returns
    /// `(None, "")` for nodes without an entry — including blank
    /// custom nodes, which reserve their area and paint nothing.
    fn node_custom(&self, index: usize) -> (Option<super::style::NodePaintFn>, &str);
    fn edge_count(&self) -> usize;
    fn edge(&self, index: usize) -> EdgeRef<'_>;
    fn subgraph_count(&self) -> usize;
    fn subgraph(&self, index: usize) -> SubgraphRef<'_>;
    fn self_loop_count(&self) -> usize;
    fn self_loop(&self, index: usize) -> SelfLoopRef<'_>;
}

// ── Heap IR ──────────────────────────────────────────────────────────────

#[cfg(feature = "alloc")]
impl LayoutView for crate::ir::LayoutIR<'_> {
    fn width(&self) -> usize {
        crate::ir::LayoutIR::width(self)
    }

    fn height(&self) -> usize {
        crate::ir::LayoutIR::height(self)
    }

    fn level_count(&self) -> usize {
        crate::ir::LayoutIR::level_count(self)
    }

    fn direction(&self) -> Direction {
        crate::ir::LayoutIR::direction(self)
    }

    fn node_count(&self) -> usize {
        self.nodes().len()
    }

    fn node(&self, index: usize) -> NodeRef<'_> {
        let n = &self.nodes()[index];
        NodeRef {
            id: n.id,
            label: n.label,
            x: n.x,
            y: n.y,
            width: n.width,
            height: n.height,
            center_x: n.center_x,
            center_y: n.center_y,
            level: n.level,
            level_position: n.level_position,
            kind: n.kind,
            has_self_loop: n.has_self_loop,
            self_loop_at: n.self_loop_at,
            edge_index: n.edge_index,
            content_tag: n.content_tag,
        }
    }

    fn node_custom(&self, index: usize) -> (Option<super::style::NodePaintFn>, &str) {
        match self
            .custom_nodes
            .binary_search_by_key(&index, |entry| entry.0)
        {
            Ok(pos) => {
                let (_, painter, payload) = self.custom_nodes[pos];
                (painter, payload)
            }
            Err(_) => (None, ""),
        }
    }

    fn edge_count(&self) -> usize {
        self.edges().len()
    }

    fn edge(&self, index: usize) -> EdgeRef<'_> {
        let e = &self.edges()[index];
        let path = match &e.path {
            crate::ir::EdgePath::Direct => PathRef::Direct,
            crate::ir::EdgePath::Corner { bend_at } => PathRef::Corner { bend_at: *bend_at },
            crate::ir::EdgePath::SideChannel {
                channel_at,
                span_start,
                span_end,
            } => PathRef::SideChannel {
                channel_at: *channel_at,
                span_start: *span_start,
                span_end: *span_end,
            },
            crate::ir::EdgePath::MultiSegment {
                waypoints,
                start_offset,
            } => PathRef::MultiSegment {
                waypoints: waypoints.as_slice(),
                start_offset: *start_offset,
            },
            #[cfg(feature = "ports")]
            crate::ir::EdgePath::Orthogonal { bends } => PathRef::Orthogonal {
                bends: bends.as_slice(),
            },
            crate::ir::EdgePath::Spline {
                cp1_x,
                cp1_y,
                cp2_x,
                cp2_y,
            } => PathRef::Spline {
                cp1_x: *cp1_x,
                cp1_y: *cp1_y,
                cp2_x: *cp2_x,
                cp2_y: *cp2_y,
            },
        };
        EdgeRef {
            from_id: e.from_id,
            to_id: e.to_id,
            from_x: e.from_x,
            from_y: e.from_y,
            to_x: e.to_x,
            to_y: e.to_y,
            edge_index: e.edge_index,
            label: e.label,
            label_x: e.label_x,
            label_y: e.label_y,
            directed: e.directed,
            reversed: e.reversed,
            flow_axis: e.flow_axis,
            path,
        }
    }

    fn subgraph_count(&self) -> usize {
        self.subgraphs().len()
    }

    fn subgraph(&self, index: usize) -> SubgraphRef<'_> {
        let sg = &self.subgraphs()[index];
        SubgraphRef {
            id: sg.id,
            parent: sg.parent_id,
            label: sg.label,
            x: sg.x,
            y: sg.y,
            width: sg.width,
            height: sg.height,
        }
    }

    fn self_loop_count(&self) -> usize {
        self.self_loops().len()
    }

    fn self_loop(&self, index: usize) -> SelfLoopRef<'_> {
        let r = &self.self_loops()[index];
        SelfLoopRef {
            node_id: r.node_id,
            node_index: r.node_index,
            input_index: r.edge_index,
            // Empty = none, even for hand-built records (the arena
            // twin's len-0 storage cannot say Some("")).
            label: r.label.filter(|l| !l.is_empty()),
        }
    }
}

// ── Arena IR ─────────────────────────────────────────────────────────────

impl LayoutView for crate::ir::arena::LayoutIRArena<'_> {
    fn width(&self) -> usize {
        crate::ir::arena::LayoutIRArena::width(self)
    }

    fn height(&self) -> usize {
        crate::ir::arena::LayoutIRArena::height(self)
    }

    fn level_count(&self) -> usize {
        crate::ir::arena::LayoutIRArena::level_count(self)
    }

    fn direction(&self) -> Direction {
        crate::ir::arena::LayoutIRArena::direction(self)
    }

    fn node_count(&self) -> usize {
        crate::ir::arena::LayoutIRArena::node_count(self)
    }

    fn node(&self, index: usize) -> NodeRef<'_> {
        let n = self.node(index);
        NodeRef {
            id: n.id,
            label: self.node_label(index),
            x: n.x,
            y: n.y,
            width: n.width,
            height: n.height,
            center_x: n.center_x,
            center_y: n.center_y,
            level: n.level,
            level_position: n.level_position,
            kind: n.kind,
            has_self_loop: n.has_self_loop,
            self_loop_at: if n.self_loop_at == (usize::MAX, usize::MAX) {
                None
            } else {
                Some(n.self_loop_at)
            },
            edge_index: if n.edge_index == usize::MAX {
                None
            } else {
                Some(n.edge_index)
            },
            content_tag: n.content_tag,
        }
    }

    fn node_custom(&self, index: usize) -> (Option<super::style::NodePaintFn>, &str) {
        let entries = self.custom_nodes();
        match entries.binary_search_by_key(&index, |entry| entry.node_idx) {
            Ok(pos) => {
                let entry = &entries[pos];
                (entry.painter, self.custom_payload(entry))
            }
            Err(_) => (None, ""),
        }
    }

    fn edge_count(&self) -> usize {
        crate::ir::arena::LayoutIRArena::edge_count(self)
    }

    fn edge(&self, index: usize) -> EdgeRef<'_> {
        let e = self.edge(index);
        let path = match e.path {
            crate::ir::arena::EdgePathArena::Direct => PathRef::Direct,
            crate::ir::arena::EdgePathArena::Corner { bend_at } => PathRef::Corner { bend_at },
            crate::ir::arena::EdgePathArena::SideChannel {
                channel_at,
                span_start,
                span_end,
            } => PathRef::SideChannel {
                channel_at,
                span_start,
                span_end,
            },
            crate::ir::arena::EdgePathArena::MultiSegment {
                waypoints_start,
                waypoints_len,
                start_offset,
            } => PathRef::MultiSegment {
                waypoints: self.edge_waypoints_raw(waypoints_start, waypoints_len),
                start_offset,
            },
            #[cfg(feature = "ports")]
            crate::ir::arena::EdgePathArena::Orthogonal {
                bends_start,
                bends_len,
            } => PathRef::Orthogonal {
                bends: self.edge_waypoints_raw(bends_start, bends_len),
            },
            crate::ir::arena::EdgePathArena::Spline {
                cp1_x,
                cp1_y,
                cp2_x,
                cp2_y,
            } => PathRef::Spline {
                cp1_x,
                cp1_y,
                cp2_x,
                cp2_y,
            },
        };
        EdgeRef {
            from_id: e.from_id,
            to_id: e.to_id,
            from_x: e.from_x,
            from_y: e.from_y,
            to_x: e.to_x,
            to_y: e.to_y,
            edge_index: e.edge_index,
            label: if e.label_len > 0 {
                Some(self.edge_label(index))
            } else {
                None
            },
            label_x: e.label_x,
            label_y: e.label_y,
            directed: e.directed,
            reversed: e.reversed,
            flow_axis: e.flow_axis,
            path,
        }
    }

    fn subgraph_count(&self) -> usize {
        crate::ir::arena::LayoutIRArena::subgraph_count(self)
    }

    fn subgraph(&self, index: usize) -> SubgraphRef<'_> {
        let sg = &self.subgraphs()[index];
        SubgraphRef {
            id: sg.id,
            parent: if sg.parent_idx == usize::MAX {
                None
            } else {
                Some(sg.parent_idx)
            },
            label: self.subgraph_label(index),
            x: sg.x,
            y: sg.y,
            width: sg.width,
            height: sg.height,
        }
    }

    fn self_loop_count(&self) -> usize {
        self.self_loops().len()
    }

    fn self_loop(&self, index: usize) -> SelfLoopRef<'_> {
        let r = &self.self_loops()[index];
        SelfLoopRef {
            node_id: r.node_id,
            node_index: r.node_index,
            input_index: r.edge_index,
            label: if r.label_len == 0 {
                None
            } else {
                Some(self.self_loop_label(index))
            },
        }
    }
}

// ── Accessor-equivalence tests (RW1 exit criteria) ───────────────────────

#[cfg(all(test, feature = "arena", feature = "alloc", feature = "std"))]
mod tests {
    use super::*;
    use crate::algorithms::sugiyama::config::LayoutConfig;
    use crate::graph::Graph;
    use crate::graph::arena::Arena;
    use alloc::string::String;
    use alloc::vec;
    use alloc::vec::Vec;

    /// Labeled edge + subgraph + back edge + skip edge + self-loop +
    /// 2-node cycle: every field the view exposes is exercised.
    fn corpus_graph() -> Graph<'static> {
        let mut g = Graph::new();
        g.add_node(1, "Start");
        g.add_node(2, "Middle");
        g.add_node(3, "End");
        g.add_node(4, "Late");
        g.add_node(5, "Ping");
        g.add_node(6, "Pong");
        g.add_edge(1, 2, Some("go"));
        g.add_edge(2, 3, None);
        g.add_edge(3, 4, None);
        g.add_edge(1, 4, None); // skip-level → dummies
        g.add_edge(4, 1, None); // back edge
        g.add_edge(2, 2, None); // self-loop
        g.add_edge(5, 6, None); // 2-node cycle
        g.add_edge(6, 5, None);
        let sg = g.add_subgraph("Stage");
        g.put_nodes(&[2]).inside(sg).unwrap();
        g
    }

    fn engine_config(direction: Direction) -> LayoutConfig<'static> {
        let mut config = LayoutConfig::standard();
        config.direction = direction;
        config.include_dummy_nodes = true;
        config
    }

    fn with_both_views(
        direction: Direction,
        check: impl FnOnce(&crate::ir::LayoutIR<'_>, &crate::ir::arena::LayoutIRArena<'_>),
    ) {
        let config = engine_config(direction);

        let heap_g = corpus_graph();
        let heap_ir = heap_g.compute_layout_with_config(&config);

        let g = corpus_graph();
        let mut csr_buf = vec![0u8; g.estimate_csr_arena_size() * 2];
        let mut csr_arena = Arena::new(&mut csr_buf);
        let csr = g.to_csr(&mut csr_arena).expect("CSR conversion");
        let size = (g.estimate_layout_arena_size() * 2).max(256 * 1024);
        let mut temp_buf = vec![0u8; size];
        let mut out_buf = vec![0u8; size];
        let mut temp_arena = Arena::new(&mut temp_buf);
        let mut out_arena = Arena::new(&mut out_buf);
        let csr_ir = csr
            .compute_layout_arena(&config, &mut temp_arena, &mut out_arena)
            .expect("CSR layout");

        check(&heap_ir, &csr_ir);
    }

    /// Order-independent node key: real nodes by id, dummies by
    /// (owning edge, level) — synthetic ids are not compared.
    fn node_key(n: &NodeRef<'_>) -> (usize, usize, usize) {
        match n.edge_index {
            Some(e) => (1, e, n.level),
            None => (0, n.id, 0),
        }
    }

    fn node_fingerprint(n: &NodeRef<'_>) -> String {
        use core::fmt::Write as _;
        let mut s = String::new();
        let _ = write!(
            s,
            "label={:?} x={} y={} w={} h={} cx={} cy={} level={} pos={} kind={:?} loop={}",
            n.label,
            n.x,
            n.y,
            n.width,
            n.height,
            n.center_x,
            n.center_y,
            n.level,
            n.level_position,
            n.kind,
            n.has_self_loop,
        );
        let _ = write!(s, " loop_at={:?}", n.self_loop_at);
        s
    }

    fn edge_fingerprint(e: &EdgeRef<'_>) -> String {
        use core::fmt::Write as _;
        let mut s = String::new();
        let _ = write!(
            s,
            "{}→{} fx={} fy={} tx={} ty={} label={:?} lx={} ly={} dir={} rev={} path={:?}",
            e.from_id,
            e.to_id,
            e.from_x,
            e.from_y,
            e.to_x,
            e.to_y,
            e.label,
            if e.label.is_some() { e.label_x } else { 0 },
            if e.label.is_some() { e.label_y } else { 0 },
            e.directed,
            e.reversed,
            e.path,
        );
        let _ = write!(s, " axis={:?}", e.flow_axis);
        s
    }

    /// Every field of every element must be identical through the view,
    /// for both directions — the whole-IR generalization of the
    /// field-by-field parity tests in tests/layout_output.rs.
    #[test]
    fn views_are_equivalent_across_backends() {
        #[cfg(feature = "layout-vertical")]
        let directions = [Direction::TopDown, Direction::BottomUp];
        #[cfg(not(feature = "layout-vertical"))]
        let directions = [Direction::LeftRight, Direction::RightLeft];
        for direction in directions {
            with_both_views(direction, |heap, csr| {
                assert_eq!(LayoutView::width(heap), LayoutView::width(csr));
                assert_eq!(LayoutView::height(heap), LayoutView::height(csr));
                assert_eq!(LayoutView::level_count(heap), LayoutView::level_count(csr));
                assert_eq!(LayoutView::direction(heap), direction);
                assert_eq!(LayoutView::direction(csr), direction);

                // Nodes: emission order differs between backends
                // (level-major vs graph-index-major) — compare keyed sets.
                assert_eq!(LayoutView::node_count(heap), LayoutView::node_count(csr));
                let collect_nodes =
                    |view: &dyn LayoutView| -> Vec<((usize, usize, usize), String)> {
                        let mut v: Vec<_> = (0..view.node_count())
                            .map(|i| {
                                let n = view.node(i);
                                (node_key(&n), node_fingerprint(&n))
                            })
                            .collect();
                        v.sort();
                        v
                    };
                assert_eq!(
                    collect_nodes(heap),
                    collect_nodes(csr),
                    "node views diverge ({direction:?})"
                );

                // Edges: keyed by edge_index (original edge order).
                assert_eq!(LayoutView::edge_count(heap), LayoutView::edge_count(csr));
                let collect_edges = |view: &dyn LayoutView| -> Vec<(usize, String)> {
                    let mut v: Vec<_> = (0..view.edge_count())
                        .map(|i| {
                            let e = view.edge(i);
                            (e.edge_index, edge_fingerprint(&e))
                        })
                        .collect();
                    v.sort();
                    v
                };
                assert_eq!(
                    collect_edges(heap),
                    collect_edges(csr),
                    "edge views diverge ({direction:?})"
                );

                // Subgraphs: same order (id-registration order) in both.
                assert_eq!(
                    LayoutView::subgraph_count(heap),
                    LayoutView::subgraph_count(csr)
                );
                for i in 0..LayoutView::subgraph_count(heap) {
                    let h = LayoutView::subgraph(heap, i);
                    let c = LayoutView::subgraph(csr, i);
                    assert_eq!(
                        (h.id, h.parent, h.label, h.x, h.y, h.width, h.height),
                        (c.id, c.parent, c.label, c.x, c.y, c.width, c.height),
                        "subgraph views diverge ({direction:?})"
                    );
                }
            });
        }
    }

    /// The view's convention adapters behave: arena sentinels become
    /// `None`, empty arena labels become `None`, waypoint slices are
    /// directly accessible in both backends.
    #[test]
    fn view_normalizes_backend_conventions() {
        with_both_views(Direction::DEFAULT, |heap, csr| {
            for view in [heap as &dyn LayoutView, csr as &dyn LayoutView] {
                for i in 0..view.node_count() {
                    let n = view.node(i);
                    match n.kind {
                        NodeKind::Dummy => {
                            assert!(n.edge_index.is_some());
                            assert_eq!(n.label, "");
                        }
                        _ => assert!(n.edge_index.is_none()),
                    }
                }
                let labeled: Vec<_> = (0..view.edge_count())
                    .map(|i| view.edge(i))
                    .filter(|e| e.label.is_some())
                    .collect();
                assert_eq!(labeled.len(), 1);
                assert_eq!(labeled[0].label, Some("go"));
                for i in 0..view.edge_count() {
                    if let PathRef::MultiSegment { waypoints, .. } = view.edge(i).path {
                        assert!(!waypoints.is_empty());
                    }
                }
            }
        });
    }
}
