//! ASCII rendering implementation for DAG visualization.

use crate::graph::{DAG, RenderMode};
use alloc::{string::String, vec, vec::Vec};
use core::fmt::Write;

// Type alias for connection groups to reduce complexity
type ConnectionGroup = (usize, Vec<(usize, usize, usize)>);

// Box drawing characters (Unicode)
pub(crate) const V_LINE: char = '│';
pub(crate) const H_LINE: char = '─';
pub(crate) const ARROW_DOWN: char = '↓';
pub(crate) const ARROW_RIGHT: char = '→';
pub(crate) const CYCLE_ARROW: char = '⇄'; // For cycle detection

// Convergence/divergence
pub(crate) const CORNER_DR: char = '└'; // Down-Right corner
pub(crate) const CORNER_DL: char = '┘'; // Down-Left corner
pub(crate) const TEE_DOWN: char = '┬'; // T pointing down
pub(crate) const TEE_UP: char = '┴'; // T pointing up
pub(crate) const CORNER_UR: char = '┌'; // Up-Right corner
pub(crate) const CORNER_UL: char = '┐'; // Up-Left corner
pub(crate) const CROSS: char = '┼'; // Cross junction

/// A virtual node in the layout - either a real node or a dummy for edge routing.
/// Memory: 8 bytes using tagged pointer (high bit = is_dummy flag)
/// This is 3x smaller than the naive enum representation.
#[derive(Clone, Copy, Debug)]
struct VirtualNode(usize);

impl VirtualNode {
    /// High bit indicates dummy node
    const DUMMY_FLAG: usize = 1 << (usize::BITS - 1);
    
    #[inline]
    fn real(idx: usize) -> Self {
        debug_assert!(idx & Self::DUMMY_FLAG == 0, "index too large");
        Self(idx)
    }
    
    #[inline]
    fn dummy(edge_idx: usize) -> Self {
        debug_assert!(edge_idx & Self::DUMMY_FLAG == 0, "edge index too large");
        Self(edge_idx | Self::DUMMY_FLAG)
    }
    
    #[inline]
    fn is_real(&self) -> bool {
        self.0 & Self::DUMMY_FLAG == 0
    }
    
    #[inline]
    fn is_dummy(&self) -> bool {
        self.0 & Self::DUMMY_FLAG != 0
    }

    #[inline]
    fn real_index(&self) -> Option<usize> {
        if self.is_real() {
            Some(self.0)
        } else {
            None
        }
    }
    
    #[inline]
    fn index(&self) -> usize {
        self.0 & !Self::DUMMY_FLAG
    }
}

/// Virtual layout with dummy nodes for proper edge routing.
/// Memory cost: O(N + E*D) where N=nodes, E=skip edges, D=avg level span
struct VirtualLayout {
    /// Virtual nodes at each level
    levels: Vec<Vec<VirtualNode>>,
    /// X-coordinate for each virtual node (indexed by level, then position in level)
    x_coords: Vec<Vec<usize>>,
    /// Width of each virtual node
    widths: Vec<Vec<usize>>,
    /// Edges grouped by source level for O(1) lookup: edges_by_level[level] = [(from_pos, to_pos), ...]
    edges_by_level: Vec<Vec<(usize, usize)>>,
}

impl<'a> DAG<'a> {
    /// Render the DAG to an ASCII string.
    ///
    /// # Examples
    ///
    /// ```
    /// use ascii_dag::graph::DAG;
    ///
    /// let dag = DAG::from_edges(
    ///     &[(1, "Start"), (2, "End")],
    ///     &[(1, 2)]
    /// );
    ///
    /// let output = dag.render();
    /// println!("{}", output);
    /// ```
    pub fn render(&self) -> String {
        let mut buf = String::with_capacity(self.estimate_size());
        self.render_to(&mut buf);
        buf
    }

    /// Render the DAG into a provided buffer (zero-allocation).
    ///
    /// # Examples
    ///
    /// ```
    /// use ascii_dag::graph::DAG;
    ///
    /// let dag = DAG::from_edges(
    ///     &[(1, "A")],
    ///     &[]
    /// );
    ///
    /// let mut buffer = String::new();
    /// dag.render_to(&mut buffer);
    /// assert!(!buffer.is_empty());
    /// ```
    pub fn render_to(&self, output: &mut String) {
        if self.nodes.is_empty() {
            output.push_str("Empty DAG");
            return;
        }

        // Check for cycles and render them specially
        if self.has_cycle() {
            self.render_cycle(output);
            return;
        }

        // Determine actual render mode
        let is_chain = self.is_simple_chain();
        let mode = match self.render_mode {
            RenderMode::Auto => {
                if is_chain {
                    RenderMode::Horizontal
                } else {
                    RenderMode::Vertical
                }
            }
            other => other,
        };

        match mode {
            RenderMode::Horizontal => self.render_horizontal(output),
            RenderMode::Vertical | RenderMode::Auto => self.render_vertical(output),
        }
    }

    /// Render a graph with cycles (not a valid DAG, but useful for error visualization).
    fn render_cycle(&self, output: &mut String) {
        writeln!(output, "⚠️  CYCLE DETECTED - Not a valid DAG").ok();
        writeln!(output).ok();

        // Find the cycle using DFS
        if let Some(cycle_nodes) = self.find_cycle_path() {
            writeln!(output, "Cyclic dependency chain:").ok();

            for (i, node_id) in cycle_nodes.iter().enumerate() {
                if let Some(&(id, label)) = self.nodes.iter().find(|(nid, _)| nid == node_id) {
                    self.write_node(output, id, label);

                    if i < cycle_nodes.len() - 1 {
                        write!(output, " → ").ok();
                    } else {
                        // Last node, show it cycles back
                        if let Some(&(first_id, first_label)) =
                            self.nodes.iter().find(|(nid, _)| nid == &cycle_nodes[0])
                        {
                            write!(output, " {} ", CYCLE_ARROW).ok();
                            self.write_node(output, first_id, first_label);
                        }
                    }
                }
            }
            writeln!(output).ok();
            writeln!(output).ok();
            writeln!(
                output,
                "This creates an infinite loop in error dependencies."
            )
            .ok();
        } else {
            writeln!(output, "Complex cycle detected in graph.").ok();
        }
    }

    /// Check if this is a simple chain (A → B → C, no branching).
    /// Optimized to avoid allocations by using count methods.
    fn is_simple_chain(&self) -> bool {
        if self.nodes.is_empty() {
            return false;
        }

        // If we have multiple disconnected subgraphs, it's not a simple chain
        let subgraphs = self.find_subgraphs();
        if subgraphs.len() > 1 {
            return false;
        }

        // Check if every node has at most 1 parent and 1 child (zero-allocation)
        for idx in 0..self.nodes.len() {
            if self.parents_count(idx) > 1 || self.children_count(idx) > 1 {
                return false;
            }
        }

        true
    }

    /// Render in horizontal mode: [A] → [B] → [C]
    fn render_horizontal(&self, output: &mut String) {
        // Find the root (node with no parents)
        let roots: Vec<_> = self
            .nodes
            .iter()
            .filter(|(id, _)| self.get_parents(*id).is_empty())
            .collect();

        if roots.is_empty() {
            output.push_str("(no root)");
            return;
        }

        // Follow the chain from root
        let mut current_id = roots[0].0;
        let mut visited = Vec::new();

        loop {
            visited.push(current_id);

            // Find node and format with appropriate brackets
            if let Some(&(id, label)) = self.nodes.iter().find(|(nid, _)| *nid == current_id) {
                self.write_node(output, id, label);
            }

            // Get children
            let children = self.get_children(current_id);

            if children.is_empty() {
                break;
            }

            // Draw arrow
            write!(output, " {} ", ARROW_RIGHT).ok();

            // Move to next
            current_id = children[0];

            // Avoid infinite loops
            if visited.contains(&current_id) {
                break;
            }
        }

        writeln!(output).ok();
    }

    /// Render in vertical mode (Sugiyama layout with dummy nodes for skip-level edges).
    fn render_vertical(&self, output: &mut String) {
        // Detect if we have multiple disconnected subgraphs
        let subgraphs = self.find_subgraphs();

        if subgraphs.len() > 1 {
            // Render each subgraph separately
            for (i, subgraph_nodes) in subgraphs.iter().enumerate() {
                if i > 0 {
                    writeln!(output).ok();
                }
                self.render_subgraph(output, subgraph_nodes);
            }
            return;
        }

        // Build virtual layout with dummy nodes
        let layout = self.build_virtual_layout();
        self.render_virtual_layout(output, &layout);
    }

    /// Build a virtual layout with dummy nodes for skip-level edges.
    /// Memory: O(N + E*D) where N=nodes, E=skip edges, D=avg level span
    fn build_virtual_layout(&self) -> VirtualLayout {
        // Step 1: Calculate levels for real nodes
        let level_data = self.calculate_levels();
        let max_level = level_data.iter().map(|(_, l)| *l).max().unwrap_or(0);

        // Create level mapping for real nodes
        let mut node_levels: Vec<usize> = vec![0; self.nodes.len()];
        for (idx, level) in &level_data {
            node_levels[*idx] = *level;
        }

        // Step 2: Group real nodes by level
        let mut levels: Vec<Vec<VirtualNode>> = vec![Vec::new(); max_level + 1];
        for (idx, level) in &level_data {
            levels[*level].push(VirtualNode::real(*idx));
        }

        // Step 3: Identify skip-level edges and insert dummy nodes
        // We use the edge_idx in Dummy nodes so we can find them after reordering
        for (edge_idx, &(from_id, to_id)) in self.edges.iter().enumerate() {
            if let Some(from_idx) = self.node_index(from_id)
                && let Some(to_idx) = self.node_index(to_id)
            {
                let from_level = node_levels[from_idx];
                let to_level = node_levels[to_idx];

                if to_level > from_level + 1 {
                    // This is a skip edge - insert dummy nodes at intermediate levels
                    for level in &mut levels[(from_level + 1)..to_level] {
                        level.push(VirtualNode::dummy(edge_idx));
                    }
                }
            }
        }

        // Step 4: Apply crossing reduction on virtual levels
        // Convert to indices for the existing reduce_crossings logic
        let mut real_levels: Vec<Vec<usize>> = levels
            .iter()
            .map(|level| level.iter().filter_map(|vn| vn.real_index()).collect())
            .collect();

        self.reduce_crossings(&mut real_levels, max_level);

        // Rebuild levels with proper ordering (real nodes in optimized order)
        // Position dummies intelligently based on their source node's position
        for (level_idx, real_order) in real_levels.iter().enumerate() {
            let dummies: Vec<_> = levels[level_idx]
                .iter()
                .filter(|vn| !vn.is_real())
                .copied()
                .collect();

            levels[level_idx].clear();
            for &idx in real_order {
                levels[level_idx].push(VirtualNode::real(idx));
            }
            // Dummies will be repositioned after we have x-coordinates
            levels[level_idx].extend(dummies);
        }

        // Step 4b: Reposition dummies to align with their skip-edge source nodes
        // This creates visually cleaner vertical paths for skip edges
        self.reposition_dummies(&mut levels, &node_levels);

        // Step 5: Assign x-coordinates
        let (x_coords, widths) = self.assign_virtual_x_coordinates(&levels, &node_levels);

        // Step 6: Build edge list grouped by source level for O(1) lookup during rendering
        let edges_by_level = self.build_virtual_edges_by_level(&levels, &node_levels);

        VirtualLayout {
            levels,
            x_coords,
            widths,
            edges_by_level,
        }
    }

    /// Assign x-coordinates to virtual nodes (real + dummy).
    /// Real nodes get sequential x-coordinates.
    /// Dummy nodes get x-coordinates aligned with their source node for visual continuity.
    fn assign_virtual_x_coordinates(
        &self,
        levels: &[Vec<VirtualNode>],
        node_levels: &[usize],
    ) -> (Vec<Vec<usize>>, Vec<Vec<usize>>) {
        let mut x_coords: Vec<Vec<usize>> = Vec::with_capacity(levels.len());
        let mut widths: Vec<Vec<usize>> = Vec::with_capacity(levels.len());

        // First pass: assign x-coordinates to all nodes sequentially
        for level_nodes in levels {
            let mut level_x = Vec::with_capacity(level_nodes.len());
            let mut level_w = Vec::with_capacity(level_nodes.len());
            let mut x = 0;

            for vnode in level_nodes {
                let width = if vnode.is_real() {
                    self.get_node_width(vnode.index())
                } else {
                    1
                };

                level_x.push(x);
                level_w.push(width);
                x += width + 3; // Standard spacing for all nodes
            }

            x_coords.push(level_x);
            widths.push(level_w);
        }

        // Second pass: adjust dummy x-coordinates to align with their source node's center
        // Collect all dummy adjustments first
        let mut dummy_adjustments: Vec<(usize, usize, usize)> = Vec::new(); // (level_idx, dummy_pos, target_x)

        for (edge_idx, &(from_id, to_id)) in self.edges.iter().enumerate() {
            if let Some(from_idx) = self.node_index(from_id)
                && let Some(to_idx) = self.node_index(to_id)
            {
                let from_level = node_levels[from_idx];
                let to_level = node_levels[to_idx];

                if to_level > from_level + 1 {
                    // This is a skip edge - find source node's center x
                    if let Some(source_pos) = levels[from_level]
                        .iter()
                        .position(|vn| vn.is_real() && vn.index() == from_idx)
                    {
                        let source_center_x = x_coords[from_level][source_pos]
                            + widths[from_level][source_pos] / 2;

                        // Queue each dummy's x adjustment
                        for level_idx in (from_level + 1)..to_level {
                            if let Some(dummy_pos) = levels[level_idx].iter().position(
                                |vn| vn.is_dummy() && vn.index() == edge_idx,
                            ) {
                                dummy_adjustments.push((level_idx, dummy_pos, source_center_x));
                            }
                        }
                    }
                }
            }
        }

        // Apply adjustments - for each level, set dummy x-coord and push any nodes that would overlap
        for (level_idx, dummy_pos, target_x) in dummy_adjustments {
            // Find the end position of the node just before the dummy (if any)
            let prev_end = if dummy_pos > 0 {
                x_coords[level_idx][dummy_pos - 1] + widths[level_idx][dummy_pos - 1] + 3
            } else {
                0
            };

            // Dummy goes at target_x, but at least after the previous node
            let dummy_x = target_x.max(prev_end);
            x_coords[level_idx][dummy_pos] = dummy_x;

            // Push any subsequent nodes to avoid overlap
            let mut min_next_x = dummy_x + 1 + 3; // dummy width (1) + spacing (3)
            for pos in (dummy_pos + 1)..levels[level_idx].len() {
                if x_coords[level_idx][pos] < min_next_x {
                    x_coords[level_idx][pos] = min_next_x;
                }
                // Update minimum x for next node
                min_next_x = x_coords[level_idx][pos] + widths[level_idx][pos] + 3;
            }
        }

        (x_coords, widths)
    }

    /// Reposition dummy nodes in the level arrays.
    /// Note: The actual x-coordinate alignment happens in assign_virtual_x_coordinates.
    #[allow(clippy::needless_range_loop)]
    fn reposition_dummies(&self, levels: &mut [Vec<VirtualNode>], node_levels: &[usize]) {
        // For each skip edge, find where its source node is positioned and place
        // its dummies right after that position in each intermediate level
        for (edge_idx, &(from_id, to_id)) in self.edges.iter().enumerate() {
            if let Some(from_idx) = self.node_index(from_id)
                && let Some(to_idx) = self.node_index(to_id)
            {
                let from_level = node_levels[from_idx];
                let to_level = node_levels[to_idx];

                if to_level > from_level + 1 {
                    // This is a skip edge - reposition its dummies

                    // Find where the source node is in its level
                    let source_pos = levels[from_level]
                        .iter()
                        .position(|vn| vn.is_real() && vn.index() == from_idx);

                    if let Some(src_pos) = source_pos {
                        // For each intermediate level, move this edge's dummy to after src_pos
                        for level_idx in (from_level + 1)..to_level {
                            // Find and remove the dummy for this edge
                            if let Some(dummy_pos) = levels[level_idx].iter().position(
                                |vn| vn.is_dummy() && vn.index() == edge_idx,
                            ) {
                                let dummy = levels[level_idx].remove(dummy_pos);

                                // Insert it right after the source position (but clamped to valid range)
                                // Count how many real nodes are in this level
                                let real_count = levels[level_idx].iter().filter(|vn| vn.is_real()).count();

                                // Insert position: try to align with source, but after real nodes
                                // if source position is beyond what we have
                                let insert_pos = if src_pos < real_count {
                                    // Insert right after the equivalent position
                                    src_pos + 1
                                } else {
                                    // Source position is beyond this level's real nodes,
                                    // insert at end of real nodes
                                    real_count
                                };

                                // Insert, clamping to valid range
                                let insert_pos = insert_pos.min(levels[level_idx].len());
                                levels[level_idx].insert(insert_pos, dummy);
                            }
                        }
                    }
                }
            }
        }
    }

    /// Build edges grouped by source level for O(1) lookup during rendering.
    /// Returns edges_by_level[level] = [(from_pos, to_pos), ...] for edges from level to level+1.
    fn build_virtual_edges_by_level(
        &self,
        levels: &[Vec<VirtualNode>],
        node_levels: &[usize],
    ) -> Vec<Vec<(usize, usize)>> {
        // Initialize empty vec for each level (except last which has no outgoing edges)
        let mut edges_by_level: Vec<Vec<(usize, usize)>> = vec![Vec::new(); levels.len()];

        // Process each DAG edge
        for (edge_idx, &(from_id, to_id)) in self.edges.iter().enumerate() {
            if let Some(from_idx) = self.node_index(from_id)
                && let Some(to_idx) = self.node_index(to_id)
            {
                let from_level = node_levels[from_idx];
                let to_level = node_levels[to_idx];

                if to_level == from_level + 1 {
                    // Direct adjacent edge
                    if let Some(from_pos) = levels[from_level]
                        .iter()
                        .position(|vn| vn.is_real() && vn.index() == from_idx)
                        && let Some(to_pos) = levels[to_level]
                            .iter()
                            .position(|vn| vn.is_real() && vn.index() == to_idx)
                    {
                        edges_by_level[from_level].push((from_pos, to_pos));
                    }
                } else if to_level > from_level + 1 {
                    // Skip edge - route through dummies identified by edge_idx
                    // Find source position
                    if let Some(from_pos) = levels[from_level]
                        .iter()
                        .position(|vn| vn.is_real() && vn.index() == from_idx)
                    {
                        // Find first dummy at from_level + 1
                        if let Some(first_dummy_pos) = levels[from_level + 1]
                            .iter()
                            .position(|vn| vn.is_dummy() && vn.index() == edge_idx)
                        {
                            // Edge from source to first dummy
                            edges_by_level[from_level].push((from_pos, first_dummy_pos));

                            // Edges between consecutive dummies
                            for level in (from_level + 1)..(to_level - 1) {
                                if let Some(curr_pos) = levels[level].iter().position(
                                    |vn| vn.is_dummy() && vn.index() == edge_idx,
                                ) && let Some(next_pos) = levels[level + 1].iter().position(
                                    |vn| vn.is_dummy() && vn.index() == edge_idx,
                                ) {
                                    edges_by_level[level].push((curr_pos, next_pos));
                                }
                            }

                            // Edge from last dummy to target
                            if let Some(last_dummy_pos) = levels[to_level - 1].iter().position(
                                |vn| vn.is_dummy() && vn.index() == edge_idx,
                            ) && let Some(to_pos) = levels[to_level]
                                .iter()
                                .position(|vn| vn.is_real() && vn.index() == to_idx)
                            {
                                edges_by_level[to_level - 1].push((last_dummy_pos, to_pos));
                            }
                        }
                    }
                }
            }
        }

        edges_by_level
    }

    /// Render the virtual layout to output.
    fn render_virtual_layout(&self, output: &mut String, layout: &VirtualLayout) {
        // Calculate canvas dimensions
        let level_widths: Vec<usize> = layout
            .x_coords
            .iter()
            .zip(layout.widths.iter())
            .map(|(xs, ws)| {
                xs.iter()
                    .zip(ws.iter())
                    .map(|(x, w)| x + w)
                    .max()
                    .unwrap_or(0)
            })
            .collect();

        let max_canvas_width = *level_widths.iter().max().unwrap_or(&0);

        for (current_level, level_nodes) in layout.levels.iter().enumerate() {
            if level_nodes.is_empty() {
                continue;
            }

            // Calculate centering offset for this level
            let level_width = level_widths[current_level];
            let level_offset = if max_canvas_width > level_width {
                (max_canvas_width - level_width) / 2
            } else {
                0
            };

            // Render nodes at their assigned x-coordinates
            let mut current_col = 0;
            for (pos, vnode) in level_nodes.iter().enumerate() {
                let node_x = layout.x_coords[current_level][pos] + level_offset;

                // Add spacing to reach this node's position (batch operation)
                if node_x > current_col {
                    output.extend(core::iter::repeat(' ').take(node_x - current_col));
                    current_col = node_x;
                }

                if vnode.is_real() {
                    let (id, label) = self.nodes[vnode.index()];
                    self.write_node(output, id, label);
                    current_col += layout.widths[current_level][pos];
                } else {
                    // Dummy nodes show as vertical line to indicate skip-level edge passing through
                    output.push(V_LINE);
                    current_col += 1;
                }
            }
            writeln!(output).ok();

            // Draw connections if not last level
            if current_level < layout.levels.len() - 1 {
                let next_level_width = level_widths[current_level + 1];
                let next_level_offset = if max_canvas_width > next_level_width {
                    (max_canvas_width - next_level_width) / 2
                } else {
                    0
                };

                self.draw_virtual_connections(
                    output,
                    layout,
                    current_level,
                    level_offset,
                    next_level_offset,
                );
            }
        }
    }

    /// Draw connections between adjacent levels in the virtual layout.
    fn draw_virtual_connections(
        &self,
        output: &mut String,
        layout: &VirtualLayout,
        current_level: usize,
        current_offset: usize,
        next_offset: usize,
    ) {
        let next_level = current_level + 1;

        // O(1) lookup: edges are pre-grouped by source level
        let level_edges = &layout.edges_by_level[current_level];

        if level_edges.is_empty() {
            return;
        }

        // Calculate center positions for connections
        let mut connections: Vec<(usize, usize, bool, bool)> = Vec::with_capacity(level_edges.len());

        for &(from_pos, to_pos) in level_edges {
            let from_x = layout.x_coords[current_level][from_pos]
                + layout.widths[current_level][from_pos] / 2
                + current_offset;
            let to_x = layout.x_coords[next_level][to_pos]
                + layout.widths[next_level][to_pos] / 2
                + next_offset;

            let from_is_dummy = !layout.levels[current_level][from_pos].is_real();
            let to_is_dummy = !layout.levels[next_level][to_pos].is_real();

            connections.push((from_x, to_x, from_is_dummy, to_is_dummy));
        }

        // Group by target for convergence detection
        let mut target_groups: Vec<(usize, Vec<(usize, bool)>)> = Vec::new();
        for &(from_x, to_x, from_is_dummy, _) in &connections {
            match target_groups.binary_search_by_key(&to_x, |(k, _)| *k) {
                Ok(idx) => target_groups[idx].1.push((from_x, from_is_dummy)),
                Err(idx) => target_groups.insert(idx, (to_x, vec![(from_x, from_is_dummy)])),
            }
        }

        // Group by source for divergence detection
        let mut source_groups: Vec<(usize, Vec<(usize, bool)>)> = Vec::new();
        for &(from_x, to_x, _, to_is_dummy) in &connections {
            match source_groups.binary_search_by_key(&from_x, |(k, _)| *k) {
                Ok(idx) => source_groups[idx].1.push((to_x, to_is_dummy)),
                Err(idx) => source_groups.insert(idx, (from_x, vec![(to_x, to_is_dummy)])),
            }
        }

        let has_convergence = target_groups.iter().any(|(_, v)| v.len() > 1);
        let has_divergence = source_groups.iter().any(|(_, v)| v.len() > 1);

        let max_pos = connections
            .iter()
            .flat_map(|(f, t, _, _)| [*f, *t])
            .max()
            .unwrap_or(0);

        // Draw based on pattern - now with proper handling of mixed cases
        if has_convergence && has_divergence {
            // Mixed case: draw with proper crossing handling
            self.draw_mixed_connections(output, &connections, max_pos);
        } else if has_convergence {
            self.draw_convergence_connections(output, &target_groups, max_pos);
        } else if has_divergence {
            self.draw_divergence_connections(output, &source_groups, max_pos);
        } else {
            // Simple 1-to-1 connections
            self.draw_simple_connections(output, &connections, max_pos);
        }
    }

    /// Draw mixed convergence and divergence (the previously broken case).
    /// Optimized with O(1) position lookups using boolean arrays.
    #[allow(clippy::needless_range_loop)]
    fn draw_mixed_connections(
        &self,
        output: &mut String,
        connections: &[(usize, usize, bool, bool)],
        max_pos: usize,
    ) {
        // Use boolean arrays for O(1) lookups instead of Vec::contains O(n)
        let mut is_source = vec![false; max_pos + 1];
        let mut is_target = vec![false; max_pos + 1];
        
        for &(from_x, to_x, _, _) in connections {
            is_source[from_x] = true;
            is_target[to_x] = true;
        }

        // Classify connections
        let mut straight_down: Vec<usize> = Vec::new(); // from_x == to_x
        let mut going_right: Vec<(usize, usize)> = Vec::new(); // from_x < to_x
        let mut going_left: Vec<(usize, usize)> = Vec::new(); // from_x > to_x

        for &(from_x, to_x, _, _) in connections {
            if from_x == to_x {
                straight_down.push(from_x);
            } else if from_x < to_x {
                going_right.push((from_x, to_x));
            } else {
                going_left.push((from_x, to_x));
            }
        }

        // Check if we have true crossings (both going_left and going_right with overlapping spans)
        let has_crossings = !going_right.is_empty() && !going_left.is_empty();

        // Line 1: Vertical drops from all sources
        for i in 0..=max_pos {
            output.push(if is_source[i] { V_LINE } else { ' ' });
        }
        writeln!(output).ok();

        if has_crossings {
            // Complex case: multiple sources converging to multiple targets with crossings
            // Lines come DOWN from sources, turn horizontal, then continue down to targets
            // 
            // Source positions: lines come FROM ABOVE → use ┴ (TEE_UP)
            // Target positions: lines go DOWN TO → use ┬ (TEE_DOWN)
            // Both source AND target: use ┼ (CROSS)
            
            // Find the overall span of the horizontal routing line
            let all_positions: Vec<usize> = going_right
                .iter()
                .flat_map(|(f, t)| [*f, *t])
                .chain(going_left.iter().flat_map(|(f, t)| [*f, *t]))
                .chain(straight_down.iter().copied())
                .collect();
            
            let min_x = *all_positions.iter().min().unwrap_or(&0);
            let max_x = *all_positions.iter().max().unwrap_or(&0);
            
            let mut line2a: Vec<char> = vec![' '; max_pos + 1];

            // First, draw the horizontal line across the entire span
            for i in min_x..=max_x {
                line2a[i] = H_LINE;
            }
            
            // Mark source positions (where lines come down from above) with ┴
            for i in min_x..=max_x {
                if is_source[i] {
                    line2a[i] = TEE_UP; // ┴ - line comes from above
                }
            }
            
            // Mark target positions (where lines go down to) with ┬
            // If already ┴ (source), upgrade to ┼ (cross)
            for i in min_x..=max_x {
                if is_target[i] {
                    line2a[i] = match line2a[i] {
                        TEE_UP => CROSS,  // Both source and target → cross
                        H_LINE => TEE_DOWN, // Only target → ┬
                        _ => line2a[i],
                    };
                }
            }
            
            // Fix the endpoints based on whether they're source or target
            // Left endpoint
            if min_x < line2a.len() {
                line2a[min_x] = if is_source[min_x] && is_target[min_x] {
                    CROSS // Both
                } else if is_source[min_x] {
                    CORNER_DR // └ - source only (line from above)
                } else if is_target[min_x] {
                    CORNER_UR // ┌ - target only (line goes down)
                } else {
                    line2a[min_x]
                };
            }
            // Right endpoint
            if max_x < line2a.len() {
                line2a[max_x] = if is_source[max_x] && is_target[max_x] {
                    CROSS // Both
                } else if is_source[max_x] {
                    CORNER_DL // ┘ - source only (line from above)
                } else if is_target[max_x] {
                    CORNER_UL // ┐ - target only (line goes down)
                } else {
                    line2a[max_x]
                };
            }

            for ch in &line2a {
                output.push(*ch);
            }
            writeln!(output).ok();

            // Line 2b: Vertical continuation - use bitmap for straight_down too
            let mut is_straight = vec![false; max_pos + 1];
            for &x in &straight_down {
                is_straight[x] = true;
            }
            for i in 0..=max_pos {
                output.push(if is_target[i] || is_straight[i] {
                    V_LINE
                } else {
                    ' '
                });
            }
            writeln!(output).ok();
        } else {
            // Simpler case: single routing line
            let mut line2: Vec<char> = vec![' '; max_pos + 1];

            for &(from_x, to_x) in &going_right {
                for i in from_x..=to_x {
                    if i == from_x {
                        line2[i] = CORNER_DR;
                    } else if i == to_x {
                        line2[i] = match line2[i] {
                            CORNER_DR => TEE_UP,
                            _ => CORNER_DL,
                        };
                    } else if line2[i] == ' ' {
                        line2[i] = H_LINE;
                    }
                }
            }

            for &(from_x, to_x) in &going_left {
                for i in to_x..=from_x {
                    if i == from_x {
                        line2[i] = match line2[i] {
                            CORNER_DR => TEE_UP,
                            _ => CORNER_DL,
                        };
                    } else if i == to_x {
                        line2[i] = match line2[i] {
                            CORNER_DL => TEE_UP,
                            H_LINE => TEE_UP,
                            _ => CORNER_DR,
                        };
                    } else if line2[i] == ' ' {
                        line2[i] = H_LINE;
                    }
                }
            }

            for &x in &straight_down {
                if line2[x] == ' ' {
                    line2[x] = V_LINE;
                } else if line2[x] == H_LINE {
                    line2[x] = TEE_UP;
                }
            }

            for ch in &line2 {
                output.push(*ch);
            }
            writeln!(output).ok();
        }

        // Final line: Arrows at targets
        for i in 0..=max_pos {
            output.push(if is_target[i] {
                ARROW_DOWN
            } else {
                ' '
            });
        }
        writeln!(output).ok();
    }

    /// Draw pure convergence pattern.
    /// Optimized with O(1) position lookups.
    fn draw_convergence_connections(
        &self,
        output: &mut String,
        target_groups: &[(usize, Vec<(usize, bool)>)],
        max_pos: usize,
    ) {
        // Build source bitmap for O(1) lookup
        let mut is_source = vec![false; max_pos + 1];
        for (_, sources) in target_groups {
            for (x, _) in sources {
                is_source[*x] = true;
            }
        }
        
        // Identify 1-to-1 connections (targets with only 1 source) - these are "pass-through"
        let mut is_pass_through_src = vec![false; max_pos + 1];
        for (_, sources) in target_groups.iter().filter(|(_, s)| s.len() == 1) {
            is_pass_through_src[sources[0].0] = true;
        }
        
        // Build target bitmap
        let mut is_target = vec![false; max_pos + 1];
        for (target, _) in target_groups {
            is_target[*target] = true;
        }

        // Line 1: Vertical drops
        for i in 0..=max_pos {
            output.push(if is_source[i] { V_LINE } else { ' ' });
        }
        writeln!(output).ok();

        // Line 2: Horizontal convergence + vertical pass-through
        for i in 0..=max_pos {
            let mut ch = ' ';
            
            for (_, sources) in target_groups.iter() {
                if sources.len() <= 1 {
                    continue;
                }
                let source_xs: Vec<usize> = sources.iter().map(|(x, _)| *x).collect();
                let min_src = *source_xs.iter().min().unwrap();
                let max_src = *source_xs.iter().max().unwrap();

                if i == min_src {
                    ch = CORNER_DR;
                } else if i == max_src {
                    ch = CORNER_DL;
                } else if source_xs.contains(&i) {
                    ch = TEE_UP;
                } else if i > min_src && i < max_src && ch == ' ' {
                    ch = H_LINE;
                }
            }
            
            // If this position is a pass-through and not already part of convergence line
            if is_pass_through_src[i] && ch == ' ' {
                ch = V_LINE;
            }
            
            output.push(ch);
        }
        writeln!(output).ok();

        // Line 3: Arrows
        for i in 0..=max_pos {
            output.push(if is_target[i] { ARROW_DOWN } else { ' ' });
        }
        writeln!(output).ok();
    }

    /// Draw pure divergence pattern.
    /// Uses top corners (┌, ┐) because lines go DOWN from the horizontal routing line.
    /// Optimized with O(1) position lookups.
    fn draw_divergence_connections(
        &self,
        output: &mut String,
        source_groups: &[(usize, Vec<(usize, bool)>)],
        max_pos: usize,
    ) {
        // Build source bitmap
        let mut is_source = vec![false; max_pos + 1];
        for (s, _) in source_groups {
            is_source[*s] = true;
        }
        
        // Build target bitmap
        let mut is_target = vec![false; max_pos + 1];
        for (_, targets) in source_groups {
            for (x, _) in targets {
                is_target[*x] = true;
            }
        }

        // Line 1: Vertical from sources
        for i in 0..=max_pos {
            output.push(if is_source[i] { V_LINE } else { ' ' });
        }
        writeln!(output).ok();

        // Line 2: Horizontal divergence - each source fans out with TOP corners
        for i in 0..=max_pos {
            let mut ch = ' ';
            for (_, targets) in source_groups.iter() {
                if targets.len() <= 1 {
                    continue;
                }
                let target_xs: Vec<usize> = targets.iter().map(|(x, _)| *x).collect();
                let min_tgt = *target_xs.iter().min().unwrap();
                let max_tgt = *target_xs.iter().max().unwrap();

                if i == min_tgt {
                    ch = CORNER_UR; // ┌
                } else if i == max_tgt {
                    ch = CORNER_UL; // ┐
                } else if target_xs.contains(&i) {
                    ch = TEE_DOWN; // ┬
                } else if i > min_tgt && i < max_tgt && ch == ' ' {
                    ch = H_LINE;
                }
            }
            output.push(ch);
        }
        writeln!(output).ok();

        // Line 3: Arrows at targets
        for i in 0..=max_pos {
            output.push(if is_target[i] { ARROW_DOWN } else { ' ' });
        }
        writeln!(output).ok();
    }

    /// Draw simple 1-to-1 connections.
    /// Optimized with O(1) position lookups.
    fn draw_simple_connections(
        &self,
        output: &mut String,
        connections: &[(usize, usize, bool, bool)],
        max_pos: usize,
    ) {
        // Build source bitmap
        let mut is_source = vec![false; max_pos + 1];
        for (f, _, _, _) in connections {
            is_source[*f] = true;
        }

        // Line 1: Vertical
        for i in 0..=max_pos {
            output.push(if is_source[i] { V_LINE } else { ' ' });
        }
        writeln!(output).ok();

        // Line 2: Arrows (straight down for 1-to-1)
        for i in 0..=max_pos {
            output.push(if is_source[i] { ARROW_DOWN } else { ' ' });
        }
        writeln!(output).ok();
    }

    /// Render a specific subgraph.
    pub(crate) fn render_subgraph(&self, output: &mut String, subgraph_indices: &[usize]) {
        // Build a mini-DAG with just these nodes
        let _subgraph_node_ids: Vec<usize> = subgraph_indices
            .iter()
            .map(|&idx| self.nodes[idx].0)
            .collect();

        // Calculate levels for this subgraph
        let level_data = self.calculate_levels_for_subgraph(subgraph_indices);
        let max_level = level_data.iter().map(|(_, l)| *l).max().unwrap_or(0);

        // Group nodes by level
        let mut levels: Vec<Vec<usize>> = vec![Vec::new(); max_level + 1];
        for (idx, level) in level_data {
            levels[level].push(idx);
        }

        // Check if it's a simple chain - render horizontally
        if self.is_subgraph_simple_chain(subgraph_indices) {
            // Render horizontally
            let roots: Vec<_> = subgraph_indices
                .iter()
                .filter(|&&idx| {
                    let node_id = self.nodes[idx].0;
                    self.get_parents(node_id).is_empty()
                })
                .collect();

            if let Some(&&root_idx) = roots.first() {
                let mut current_id = self.nodes[root_idx].0;
                let mut visited = Vec::new();

                loop {
                    visited.push(current_id);

                    if let Some(&(id, label)) =
                        self.nodes.iter().find(|(nid, _)| *nid == current_id)
                    {
                        self.write_node(output, id, label);
                    }

                    let children = self.get_children(current_id);

                    if children.is_empty() {
                        break;
                    }

                    write!(output, " {} ", ARROW_RIGHT).ok();
                    current_id = children[0];

                    if visited.contains(&current_id) {
                        break;
                    }
                }

                writeln!(output).ok();
            }
            return;
        }

        // Render vertically for complex subgraphs
        for (current_level, node_indices) in levels.iter().enumerate() {
            if node_indices.is_empty() {
                continue;
            }

            // Draw nodes with appropriate formatting
            for (pos, &idx) in node_indices.iter().enumerate() {
                let (id, label) = self.nodes[idx];
                self.write_node(output, id, label);

                if pos < node_indices.len() - 1 {
                    output.push_str("   ");
                }
            }
            writeln!(output).ok();

            // Draw connections if not last level
            if current_level < max_level {
                self.draw_vertical_connections(output, node_indices, &levels[current_level + 1]);
            }
        }
    }

    fn draw_vertical_connections(
        &self,
        output: &mut String,
        current_nodes: &[usize],
        next_nodes: &[usize],
    ) {
        if current_nodes.is_empty() || next_nodes.is_empty() {
            return;
        }

        // Calculate center positions for each node in current level
        let mut current_positions = Vec::new();
        let mut pos = 0;
        for &idx in current_nodes {
            let label_len = self.get_node_width(idx);
            let center = pos + label_len / 2;
            current_positions.push((idx, center, pos, pos + label_len));
            pos += label_len + 3; // +3 for spacing
        }

        // Calculate center positions for each node in next level
        let mut next_positions = Vec::new();
        let mut pos = 0;
        for &idx in next_nodes {
            let label_len = self.get_node_width(idx);
            let center = pos + label_len / 2;
            next_positions.push((idx, center));
            pos += label_len + 3; // +3 for spacing
        }

        // Find connections
        let mut connections: Vec<(usize, usize, usize)> = Vec::new(); // (from_idx, from_pos, to_pos)

        for &(current_idx, from_pos, _, _) in &current_positions {
            let node_id = self.nodes[current_idx].0;
            let children = self.get_children(node_id);

            for child_id in children {
                if let Some(&(_, to_pos)) = next_positions
                    .iter()
                    .find(|(idx, _)| self.nodes[*idx].0 == child_id)
                {
                    connections.push((current_idx, from_pos, to_pos));
                }
            }
        }

        if connections.is_empty() {
            return;
        }

        // Group connections by target to find convergence patterns
        // Using sorted Vec with binary search for O(log n) lookup
        let mut target_groups: Vec<ConnectionGroup> = Vec::new();

        for &conn in &connections {
            // Binary search to find existing group or insertion point
            match target_groups.binary_search_by_key(&conn.2, |(k, _)| *k) {
                Ok(idx) => target_groups[idx].1.push(conn),
                Err(idx) => target_groups.insert(idx, (conn.2, vec![conn])),
            }
        }

        // Check if we have any convergence (multiple sources to one target)
        let has_any_convergence = target_groups.iter().any(|(_, v)| v.len() > 1);

        // Group connections by source to find divergence patterns
        let mut source_groups: Vec<ConnectionGroup> = Vec::new();

        for &conn in &connections {
            match source_groups.binary_search_by_key(&conn.0, |(k, _)| *k) {
                Ok(idx) => source_groups[idx].1.push(conn),
                Err(idx) => source_groups.insert(idx, (conn.0, vec![conn])),
            }
        }

        // Check if we have any divergence (one source to multiple targets)
        let has_any_divergence = source_groups.iter().any(|(_, v)| v.len() > 1);

        // Choose rendering strategy based on pattern complexity
        if has_any_convergence && !has_any_divergence {
            // Pure convergence pattern(s)
            self.draw_multiple_convergences(output, &target_groups);
        } else if has_any_divergence && !has_any_convergence {
            // Pure divergence pattern(s)
            self.draw_multiple_divergences(output, &source_groups);
        } else if has_any_convergence && has_any_divergence {
            // Mixed pattern - draw simple connections
            self.draw_simple_verticals(output, &connections);
        } else {
            // Simple 1-to-1 connections
            self.draw_simple_verticals(output, &connections);
        }
    }

    fn draw_multiple_convergences(&self, output: &mut String, target_groups: &[ConnectionGroup]) {
        // Find all unique source and target positions
        let all_connections: Vec<_> = target_groups
            .iter()
            .flat_map(|(_, v)| v.iter().copied())
            .collect();
        let min_pos = all_connections
            .iter()
            .map(|(_, from, to)| (*from).min(*to))
            .min()
            .unwrap_or(0);
        let max_pos = all_connections
            .iter()
            .map(|(_, from, to)| (*from).max(*to))
            .max()
            .unwrap_or(0);

        // Line 1: Vertical drops from sources
        for i in min_pos..=max_pos {
            if all_connections.iter().any(|(_, from, _)| *from == i) {
                output.push(V_LINE);
            } else {
                output.push(' ');
            }
        }
        writeln!(output).ok();

        // Line 2: Draw convergence lines for each target
        for i in min_pos..=max_pos {
            let mut char_at_pos = ' ';

            for (_, conns) in target_groups.iter() {
                if conns.len() <= 1 {
                    continue;
                }

                let sources: Vec<_> = conns.iter().map(|(_, from, _)| from).collect();
                let min_source = **sources.iter().min().unwrap();
                let max_source = **sources.iter().max().unwrap();

                if i == min_source {
                    char_at_pos = CORNER_DR; // └
                } else if i == max_source {
                    char_at_pos = CORNER_DL; // ┘
                } else if sources.contains(&&i) {
                    char_at_pos = TEE_UP; // ┴
                } else if i > min_source && i < max_source && char_at_pos == ' ' {
                    char_at_pos = H_LINE; // ─
                }
            }

            output.push(char_at_pos);
        }
        writeln!(output).ok();

        // Line 3: Arrows pointing down to targets
        for i in min_pos..=max_pos {
            if target_groups.iter().any(|(target_pos, _)| *target_pos == i) {
                output.push(ARROW_DOWN);
            } else {
                output.push(' ');
            }
        }
        writeln!(output).ok();
    }

    fn draw_multiple_divergences(&self, output: &mut String, source_groups: &[ConnectionGroup]) {
        let all_connections: Vec<_> = source_groups
            .iter()
            .flat_map(|(_, v)| v.iter().copied())
            .collect();
        let min_pos = all_connections
            .iter()
            .map(|(_, from, to)| (*from).min(*to))
            .min()
            .unwrap_or(0);
        let max_pos = all_connections
            .iter()
            .map(|(_, from, to)| (*from).max(*to))
            .max()
            .unwrap_or(0);

        // Line 1: Vertical lines from sources (using from_pos, not source_pos key)
        for i in 0..=max_pos {
            if i < min_pos {
                output.push(' ');
            } else if all_connections.iter().any(|(_, from, _)| *from == i) {
                output.push(V_LINE);
            } else {
                output.push(' ');
            }
        }
        writeln!(output).ok();

        // Line 2: Draw divergence lines
        for i in 0..=max_pos {
            let mut char_at_pos = ' ';

            if i >= min_pos {
                for (_, conns) in source_groups.iter() {
                    if conns.len() <= 1 {
                        continue;
                    }

                    let targets: Vec<_> = conns.iter().map(|(_, _, to)| to).collect();
                    let min_target = **targets.iter().min().unwrap();
                    let max_target = **targets.iter().max().unwrap();

                    if i == min_target {
                        char_at_pos = CORNER_UR; // ┌
                    } else if i == max_target {
                        char_at_pos = CORNER_UL; // ┐
                    } else if targets.contains(&&i) {
                        char_at_pos = TEE_DOWN; // ┬
                    } else if i > min_target && i < max_target && char_at_pos == ' ' {
                        char_at_pos = H_LINE; // ─
                    }
                }
            }

            output.push(char_at_pos);
        }
        writeln!(output).ok();

        // Line 3: Arrows pointing down
        for i in 0..=max_pos {
            if i < min_pos {
                output.push(' ');
            } else if all_connections.iter().any(|(_, _, to)| *to == i) {
                output.push(ARROW_DOWN);
            } else {
                output.push(' ');
            }
        }
        writeln!(output).ok();
    }

    fn draw_simple_verticals(&self, output: &mut String, connections: &[(usize, usize, usize)]) {
        let max_pos = connections
            .iter()
            .map(|(_, from, to)| (*from).max(*to))
            .max()
            .unwrap_or(0);

        // Line 1: Vertical lines
        for i in 0..=max_pos {
            if connections.iter().any(|(_, from, _)| *from == i) {
                output.push(V_LINE);
            } else {
                output.push(' ');
            }
        }
        writeln!(output).ok();

        // Line 2: Arrows
        for i in 0..=max_pos {
            if connections.iter().any(|(_, from, _)| *from == i) {
                output.push(ARROW_DOWN);
            } else {
                output.push(' ');
            }
        }
        writeln!(output).ok();
    }
}
