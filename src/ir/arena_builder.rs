//! Builder for constructing [`LayoutIRArena`] from arena memory.

use super::arena::{
    CustomNodeArena, EdgePathArena, LayoutEdgeArena, LayoutIRArena, LayoutNodeArena, SelfLoopArena,
    SubgraphInfoArena,
};
use crate::graph::arena::Arena;

/// Builder for constructing LayoutIRArena from arena memory.
pub struct LayoutIRArenaBuilder<'a> {
    #[allow(dead_code)] // Stored for potential future arena operations
    arena: &'a mut Arena<'a>,
    // Temporary data (will be copied to arena)
    nodes: &'a mut [LayoutNodeArena],
    node_count: usize,
    edges: &'a mut [LayoutEdgeArena],
    edge_count: usize,
    waypoints: &'a mut [(usize, usize)],
    waypoint_count: usize,
    labels: &'a mut [u8],
    label_offset: usize,
    level_offsets: &'a mut [usize],
    level_data: &'a mut [usize],
    level_data_offset: usize,
    subgraphs: &'a mut [SubgraphInfoArena],
    subgraph_count: usize,
    custom_nodes: &'a mut [CustomNodeArena],
    custom_count: usize,
    self_loops: &'a mut [SelfLoopArena],
    self_loop_count: usize,
    width: usize,
    height: usize,
    level_count: usize,
}

impl<'a> LayoutIRArenaBuilder<'a> {
    /// Create a new builder with pre-allocated capacity.
    pub fn new(
        arena: &'a mut Arena<'a>,
        max_nodes: usize,
        max_edges: usize,
        max_waypoints: usize,
        max_label_bytes: usize,
        max_levels: usize,
    ) -> Option<Self> {
        Self::new_with_subgraphs(
            arena,
            max_nodes,
            max_edges,
            max_waypoints,
            max_label_bytes,
            max_levels,
            0,
            0,
            max_edges, // self-loops are a subset of the edge list
        )
    }

    /// Create a new builder with subgraph and custom-content support.
    ///
    /// `max_label_bytes` must cover custom payload bytes too — payloads
    /// ride the label storage. `max_custom` sizes the sparse
    /// custom-content entry array (0 when no node declares content).
    #[allow(clippy::too_many_arguments)]
    pub fn new_with_subgraphs(
        arena: &'a mut Arena<'a>,
        max_nodes: usize,
        max_edges: usize,
        max_waypoints: usize,
        max_label_bytes: usize,
        max_levels: usize,
        max_subgraphs: usize,
        max_custom: usize,
        max_self_loops: usize,
    ) -> Option<Self> {
        // Allocate all buffers upfront
        let (nodes_ptr, _) = arena.alloc_raw::<LayoutNodeArena>(max_nodes)?;
        let (edges_ptr, _) = arena.alloc_raw::<LayoutEdgeArena>(max_edges)?;
        let (waypoints_ptr, _) = arena.alloc_raw::<(usize, usize)>(max_waypoints)?;
        let (labels_ptr, _) = arena.alloc_raw::<u8>(max_label_bytes)?;
        let (self_loops_ptr, _) = arena.alloc_raw::<SelfLoopArena>(max_self_loops)?;
        let (level_offsets_ptr, _) = arena.alloc_raw::<usize>(max_levels + 1)?;
        let (level_data_ptr, _) = arena.alloc_raw::<usize>(max_nodes)?;
        let sg_ptr = if max_subgraphs > 0 {
            Some(arena.alloc_raw::<SubgraphInfoArena>(max_subgraphs)?.0)
        } else {
            None
        };
        let custom_ptr = if max_custom > 0 {
            Some(arena.alloc_raw::<CustomNodeArena>(max_custom)?.0)
        } else {
            None
        };

        unsafe {
            let nodes = core::slice::from_raw_parts_mut(nodes_ptr, max_nodes);
            let edges = core::slice::from_raw_parts_mut(edges_ptr, max_edges);
            let waypoints = core::slice::from_raw_parts_mut(waypoints_ptr, max_waypoints);
            let labels = core::slice::from_raw_parts_mut(labels_ptr, max_label_bytes);
            let level_offsets = core::slice::from_raw_parts_mut(level_offsets_ptr, max_levels + 1);
            let level_data = core::slice::from_raw_parts_mut(level_data_ptr, max_nodes);
            let subgraphs = if let Some(ptr) = sg_ptr {
                core::slice::from_raw_parts_mut(ptr, max_subgraphs)
            } else {
                &mut []
            };
            let custom_nodes = if let Some(ptr) = custom_ptr {
                core::slice::from_raw_parts_mut(ptr, max_custom)
            } else {
                &mut []
            };
            let self_loops = core::slice::from_raw_parts_mut(self_loops_ptr, max_self_loops);

            // Initialize level_offsets to zero
            for offset in level_offsets.iter_mut() {
                *offset = 0;
            }

            Some(Self {
                arena,
                nodes,
                node_count: 0,
                edges,
                edge_count: 0,
                waypoints,
                waypoint_count: 0,
                labels,
                label_offset: 0,
                level_offsets,
                level_data,
                level_data_offset: 0,
                subgraphs,
                subgraph_count: 0,
                custom_nodes,
                custom_count: 0,
                self_loops,
                self_loop_count: 0,
                width: 0,
                height: 0,
                level_count: 0,
            })
        }
    }

    /// Set dimensions.
    pub fn set_dimensions(&mut self, width: usize, height: usize) {
        self.width = width;
        self.height = height;
    }

    /// Set level count.
    pub fn set_level_count(&mut self, count: usize) {
        self.level_count = count;
    }

    /// Add a node with its label.
    ///
    /// `edge_index` is the owning edge for dummy nodes; pass
    /// `usize::MAX` for real nodes (sentinel convention).
    /// `content_tag` is the raw `NodeKindTag` value (0 = simple,
    /// 1 = boxed, 2 = custom; dummies pass 0).
    #[allow(clippy::too_many_arguments)]
    pub fn add_node(
        &mut self,
        id: usize,
        label: &str,
        x: usize,
        y: usize,
        width: usize,
        height: usize,
        level: usize,
        level_position: usize,
        kind: crate::ir::NodeKind,
        edge_index: usize,
        content_tag: u8,
    ) -> Option<usize> {
        if self.node_count >= self.nodes.len() {
            return None;
        }
        if self.label_offset + label.len() > self.labels.len() {
            return None;
        }

        // Copy label bytes
        let label_bytes = label.as_bytes();
        self.labels[self.label_offset..self.label_offset + label_bytes.len()]
            .copy_from_slice(label_bytes);

        let node_idx = self.node_count;
        self.nodes[node_idx] = LayoutNodeArena {
            id,
            label_offset: self.label_offset,
            label_len: label_bytes.len(),
            x,
            y,
            width,
            height,
            center_x: x + width / 2,
            center_y: y + height.saturating_sub(1) / 2,
            level,
            level_position,
            kind,
            has_self_loop: false,
            self_loop_at: (usize::MAX, usize::MAX),
            edge_index,
            content_tag,
        };

        self.label_offset += label_bytes.len();
        self.node_count += 1;

        Some(node_idx)
    }

    /// Attach custom content (painter + payload) to an already-added
    /// node. Payload bytes are copied into label storage (size
    /// `max_label_bytes` to include them). Entries must be added in
    /// ascending `node_idx` order — emission loops do this naturally.
    pub fn add_custom(
        &mut self,
        node_idx: usize,
        painter: Option<crate::render::engine::NodePaintFn>,
        payload: &str,
    ) -> Option<()> {
        if self.custom_count >= self.custom_nodes.len() {
            return None;
        }
        if self.label_offset + payload.len() > self.labels.len() {
            return None;
        }
        let bytes = payload.as_bytes();
        self.labels[self.label_offset..self.label_offset + bytes.len()].copy_from_slice(bytes);
        self.custom_nodes[self.custom_count] = CustomNodeArena {
            node_idx,
            painter,
            payload_offset: self.label_offset,
            payload_len: bytes.len(),
        };
        self.label_offset += bytes.len();
        self.custom_count += 1;
        Some(())
    }

    /// Record a preserved self-loop (label bytes are copied into the
    /// shared label pool — `max_label_bytes` must cover them, which
    /// the layout's label sum already does since loops live in the
    /// input edge list).
    pub fn add_self_loop(
        &mut self,
        node_id: usize,
        node_index: usize,
        edge_index: usize,
        label: &str,
    ) -> Option<()> {
        if self.self_loop_count >= self.self_loops.len() {
            return None;
        }
        let bytes = label.as_bytes();
        if self.label_offset + bytes.len() > self.labels.len() {
            return None;
        }
        self.labels[self.label_offset..self.label_offset + bytes.len()].copy_from_slice(bytes);
        self.self_loops[self.self_loop_count] = SelfLoopArena {
            node_id,
            node_index,
            edge_index,
            label_offset: self.label_offset,
            label_len: bytes.len(),
        };
        self.label_offset += bytes.len();
        self.self_loop_count += 1;
        Some(())
    }

    /// Add an edge.
    pub fn add_edge(&mut self, edge: LayoutEdgeArena) -> Option<usize> {
        if self.edge_count >= self.edges.len() {
            return None;
        }

        let edge_idx = self.edge_count;
        self.edges[edge_idx] = edge;
        self.edge_count += 1;

        Some(edge_idx)
    }

    /// Mark a node as having a self-loop edge. Also derives the
    /// marker cell (`self_loop_at`, temp/08 D5): one cell right of
    /// the node's top row — the legacy `↺` position for vertical
    /// flows. Layout uses [`set_self_loop_at`](Self::set_self_loop_at)
    /// to place the cell axis-correctly; this derivation serves
    /// hand-built vertical IRs.
    pub fn set_self_loop(&mut self, node_idx: usize) {
        if node_idx < self.node_count {
            let n = &mut self.nodes[node_idx];
            n.has_self_loop = true;
            n.self_loop_at = (n.x + n.width, n.y);
        }
    }

    /// Mark a node as having a self-loop edge, with the marker cell
    /// computed by layout (temp/08 D5: one cell past the node on the
    /// cross axis, at its level-leading line — axis-dependent, so the
    /// layout supplies it).
    pub fn set_self_loop_at(&mut self, node_idx: usize, cell: (usize, usize)) {
        if node_idx < self.node_count {
            let n = &mut self.nodes[node_idx];
            n.has_self_loop = true;
            n.self_loop_at = cell;
        }
    }

    /// Add waypoints for a multi-segment edge, returns (start, len).
    pub fn add_waypoints(&mut self, points: &[(usize, usize)]) -> Option<(usize, usize)> {
        if self.waypoint_count + points.len() > self.waypoints.len() {
            return None;
        }

        let start = self.waypoint_count;
        for (i, &point) in points.iter().enumerate() {
            self.waypoints[start + i] = point;
        }
        self.waypoint_count += points.len();

        Some((start, points.len()))
    }

    /// Add an edge label to the label storage, returns (offset, len).
    pub fn add_edge_label(&mut self, label: &str) -> Option<(usize, usize)> {
        let label_bytes = label.as_bytes();
        if self.label_offset + label_bytes.len() > self.labels.len() {
            return None;
        }

        let offset = self.label_offset;
        self.labels[self.label_offset..self.label_offset + label_bytes.len()]
            .copy_from_slice(label_bytes);
        self.label_offset += label_bytes.len();

        Some((offset, label_bytes.len()))
    }

    /// Record a node index at a level.
    pub fn add_node_to_level(&mut self, level: usize, node_idx: usize) -> Option<()> {
        if self.level_data_offset >= self.level_data.len() {
            return None;
        }
        if level >= self.level_offsets.len() {
            return None;
        }

        self.level_data[self.level_data_offset] = node_idx;
        self.level_data_offset += 1;

        // Update offset for next level
        if level + 1 < self.level_offsets.len() {
            self.level_offsets[level + 1] = self.level_data_offset;
        }

        Some(())
    }

    /// Finalize level offsets after all nodes added.
    pub fn finalize_levels(&mut self) {
        // Ensure all subsequent offsets point to end of data
        for i in 1..self.level_offsets.len() {
            if self.level_offsets[i] < self.level_offsets[i - 1] {
                self.level_offsets[i] = self.level_offsets[i - 1];
            }
        }
    }

    /// Add a subgraph bounding box. Label text is stored in shared label storage.
    pub fn add_subgraph(
        &mut self,
        id: usize,
        parent_idx: Option<usize>,
        label: &str,
        x: usize,
        y: usize,
        width: usize,
        height: usize,
    ) -> Option<usize> {
        if self.subgraph_count >= self.subgraphs.len() {
            return None;
        }

        // Store label in shared label storage
        let label_bytes = label.as_bytes();
        let (label_offset, label_len) = if !label_bytes.is_empty() {
            if self.label_offset + label_bytes.len() > self.labels.len() {
                return None;
            }
            let offset = self.label_offset;
            self.labels[offset..offset + label_bytes.len()].copy_from_slice(label_bytes);
            self.label_offset += label_bytes.len();
            (offset, label_bytes.len())
        } else {
            (0, 0)
        };

        let idx = self.subgraph_count;
        self.subgraphs[idx] = SubgraphInfoArena {
            id,
            parent_idx: parent_idx.unwrap_or(usize::MAX),
            label_offset,
            label_len,
            x,
            y,
            width,
            height,
        };
        self.subgraph_count += 1;
        Some(idx)
    }

    /// Vertically mirror every recorded coordinate in place (involutive).
    ///
    /// The arena twin of `LayoutIR::flip_vertical`, applied once for
    /// `Direction::BottomUp` layouts so the emitted IR carries physical
    /// coordinates (they match rendered cells). Must run **before**
    /// [`build`](Self::build) — the slices freeze into shared references
    /// afterwards — and after the final `set_dimensions` call, since the
    /// mirror is computed from `self.height`.
    ///
    /// Both backends must apply the identical transform to every path
    /// variant (see `LayoutIR::flip_vertical`) or their IRs can drift.
    #[cfg(feature = "layout-vertical")]
    pub(crate) fn flip_vertical(&mut self) {
        let h = self.height;
        let flip_row = |y: usize| h.saturating_sub(1).saturating_sub(y);

        for node in &mut self.nodes[..self.node_count] {
            node.y = h.saturating_sub(node.y + node.height);
            node.center_y = flip_row(node.center_y);
            // Re-anchor (not point-map): the marker stays one cell right
            // of the FINAL top row — the engine's direction-blind rule.
            if node.self_loop_at != (usize::MAX, usize::MAX) {
                node.self_loop_at = (node.x + node.width, node.y);
            }
        }

        for edge in &mut self.edges[..self.edge_count] {
            edge.from_y = flip_row(edge.from_y);
            edge.to_y = flip_row(edge.to_y);
            // label_y is only meaningful when a label exists; the 0-default
            // of unlabeled edges must not turn into a bottom-row garbage value.
            if edge.label_len > 0 {
                edge.label_y = flip_row(edge.label_y);
            }
            match &mut edge.path {
                EdgePathArena::Corner { bend_at } => *bend_at = flip_row(*bend_at),
                EdgePathArena::SideChannel {
                    span_start,
                    span_end,
                    ..
                } => {
                    // Mirror each in place — never swap (see the heap
                    // twin): the anchors carry source/target roles a
                    // mirror does not exchange.
                    *span_start = flip_row(*span_start);
                    *span_end = flip_row(*span_end);
                }
                EdgePathArena::MultiSegment {
                    waypoints_start,
                    waypoints_len,
                    ..
                } => {
                    let range = *waypoints_start..*waypoints_start + *waypoints_len;
                    for wp in &mut self.waypoints[range] {
                        wp.1 = flip_row(wp.1);
                    }
                }
                #[cfg(feature = "ports")]
                EdgePathArena::Orthogonal {
                    bends_start,
                    bends_len,
                } => {
                    let range = *bends_start..*bends_start + *bends_len;
                    for b in &mut self.waypoints[range] {
                        b.1 = flip_row(b.1);
                    }
                }
                EdgePathArena::Spline { cp1_y, cp2_y, .. } => {
                    *cp1_y = flip_row(*cp1_y);
                    *cp2_y = flip_row(*cp2_y);
                }
                EdgePathArena::Direct => {}
            }
            // The occupied row span mirrors too: old max becomes new min.
            let (new_min, new_max) = (flip_row(edge.max_y), flip_row(edge.min_y));
            edge.min_y = new_min;
            edge.max_y = new_max;
        }

        for sg in &mut self.subgraphs[..self.subgraph_count] {
            sg.y = h.saturating_sub(sg.y + sg.height);
        }
    }

    /// Horizontally mirror every coordinate in place (involutive).
    ///
    /// The x-axis twin of [`flip_vertical`](Self::flip_vertical),
    /// applied pre-build for `Direction::RightLeft` — after the final
    /// `set_dimensions`, since the mirror is computed from
    /// `self.width`. Both backends must apply the identical transform
    /// to every path variant or their IRs can drift.
    ///
    /// The pair covers the axes their directions produce: `RightLeft`
    /// mirrors horizontal layouts, `BottomUp` vertical ones.
    #[cfg(feature = "layout-horizontal")]
    pub(crate) fn flip_horizontal(&mut self) {
        let w = self.width;
        let flip_col = |x: usize| w.saturating_sub(1).saturating_sub(x);

        for node in &mut self.nodes[..self.node_count] {
            node.x = w.saturating_sub(node.x + node.width);
            node.center_x = flip_col(node.center_x);
            // Point-map (unlike the vertical flip's re-anchor): the LR
            // marker sits at the node's LEADING column, and its mirror
            // is the flipped node's trailing column — the leading side
            // again under right-to-left flow. Role rule and point
            // mirror coincide on this axis.
            if node.self_loop_at != (usize::MAX, usize::MAX) {
                node.self_loop_at = (flip_col(node.self_loop_at.0), node.self_loop_at.1);
            }
        }

        for edge in &mut self.edges[..self.edge_count] {
            edge.from_x = flip_col(edge.from_x);
            edge.to_x = flip_col(edge.to_x);
            // A label occupies a SPAN of cells and mirrors as one —
            // measured in CHARACTERS (`label_len` is bytes) — and only
            // when it exists.
            if edge.label_len > 0 {
                let bytes = &self.labels[edge.label_offset..edge.label_offset + edge.label_len];
                let chars = core::str::from_utf8(bytes)
                    .map(|t| t.chars().count())
                    .unwrap_or(edge.label_len);
                edge.label_x = w.saturating_sub(edge.label_x + chars + 2);
            }
            // `flow_axis` is mirror-invariant (D2); `start_offset` is
            // flow-relative. The level-axis path scalars flip only when
            // the level axis IS x — i.e. for horizontal trunks.
            let x_flow = matches!(edge.flow_axis, crate::ir::FlowAxis::X);
            match &mut edge.path {
                EdgePathArena::Corner { bend_at } => {
                    if x_flow {
                        *bend_at = flip_col(*bend_at);
                    }
                }
                EdgePathArena::SideChannel {
                    channel_at,
                    span_start,
                    span_end,
                } => {
                    if x_flow {
                        // Both spans are columns and each mirrors in
                        // place — NOT swapped: `span_start` is
                        // source-associated, `span_end`
                        // target-associated. The channel line is a
                        // row, untouched.
                        *span_start = flip_col(*span_start);
                        *span_end = flip_col(*span_end);
                    } else {
                        *channel_at = flip_col(*channel_at);
                    }
                }
                EdgePathArena::MultiSegment {
                    waypoints_start,
                    waypoints_len,
                    ..
                } => {
                    let range = *waypoints_start..*waypoints_start + *waypoints_len;
                    for wp in &mut self.waypoints[range] {
                        wp.0 = flip_col(wp.0);
                    }
                }
                #[cfg(feature = "ports")]
                EdgePathArena::Orthogonal {
                    bends_start,
                    bends_len,
                } => {
                    let range = *bends_start..*bends_start + *bends_len;
                    for b in &mut self.waypoints[range] {
                        b.0 = flip_col(b.0);
                    }
                }
                EdgePathArena::Spline { cp1_x, cp2_x, .. } => {
                    *cp1_x = flip_col(*cp1_x);
                    *cp2_x = flip_col(*cp2_x);
                }
                EdgePathArena::Direct => {}
            }
            // `min_y`/`max_y` are ROW bounds — an x-flip leaves them.
        }

        for sg in &mut self.subgraphs[..self.subgraph_count] {
            sg.x = w.saturating_sub(sg.x + sg.width);
        }
    }

    /// Build the final LayoutIRArena.
    /// Note: The returned IR borrows from the arena, so it must outlive this builder.
    pub fn build(self) -> LayoutIRArena<'a> {
        LayoutIRArena::from_parts(
            &self.nodes[..self.node_count],
            &self.edges[..self.edge_count],
            &self.waypoints[..self.waypoint_count],
            &self.labels[..self.label_offset],
            &self.subgraphs[..self.subgraph_count],
            self.width,
            self.height,
            self.level_count,
            &self.level_offsets[..self.level_count + 1],
            &self.level_data[..self.level_data_offset],
            &self.custom_nodes[..self.custom_count],
            &self.self_loops[..self.self_loop_count],
        )
    }
}
