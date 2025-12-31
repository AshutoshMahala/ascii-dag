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

/// A virtual node in the layout - either a real node or a dummy for edge routing.
/// Memory: ~24 bytes per node (1 byte enum tag + padding + 16 bytes data)
#[derive(Clone, Copy, Debug)]
enum VirtualNode {
    /// A real node from the DAG (stores index into DAG.nodes)
    Real(usize),
    /// A dummy node for routing skip-level edges (stores skip edge index)
    Dummy(usize),
}

impl VirtualNode {
    fn is_real(&self) -> bool {
        matches!(self, VirtualNode::Real(_))
    }

    fn real_index(&self) -> Option<usize> {
        match self {
            VirtualNode::Real(idx) => Some(*idx),
            VirtualNode::Dummy(_) => None,
        }
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
    /// Edges between adjacent levels: (from_level, from_pos, to_level, to_pos)
    edges: Vec<(usize, usize, usize, usize)>,
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
    fn is_simple_chain(&self) -> bool {
        if self.nodes.is_empty() {
            return false;
        }

        // If we have multiple disconnected subgraphs, it's not a simple chain
        let subgraphs = self.find_subgraphs();
        if subgraphs.len() > 1 {
            return false;
        }

        // Check if every node has at most 1 parent and 1 child
        for &(node_id, _) in &self.nodes {
            let parents = self.get_parents(node_id);
            let children = self.get_children(node_id);

            if parents.len() > 1 || children.len() > 1 {
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
            levels[*level].push(VirtualNode::Real(*idx));
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
                        level.push(VirtualNode::Dummy(edge_idx));
                    }
                }
            }
        }

        // Step 4: Apply crossing reduction on virtual levels
        // Convert to indices for the existing reduce_crossings logic
        let mut real_levels: Vec<Vec<usize>> = levels
            .iter()
            .map(|level| {
                level
                    .iter()
                    .filter_map(|vn| vn.real_index())
                    .collect()
            })
            .collect();

        self.reduce_crossings(&mut real_levels, max_level);

        // Rebuild levels with proper ordering (real nodes in optimized order, dummies at end)
        for (level_idx, real_order) in real_levels.iter().enumerate() {
            let dummies: Vec<_> = levels[level_idx]
                .iter()
                .filter(|vn| !vn.is_real())
                .copied()
                .collect();

            levels[level_idx].clear();
            for &idx in real_order {
                levels[level_idx].push(VirtualNode::Real(idx));
            }
            levels[level_idx].extend(dummies);
        }

        // Step 5: Assign x-coordinates
        let (x_coords, widths) = self.assign_virtual_x_coordinates(&levels, &node_levels);

        // Step 6: Build edge list between adjacent levels (using edge_idx to find dummies)
        let edges = self.build_virtual_edges(&levels, &node_levels);

        VirtualLayout {
            levels,
            x_coords,
            widths,
            edges,
        }
    }

    /// Assign x-coordinates to virtual nodes (real + dummy).
    /// Dummies are positioned at the end of each level, giving them their own column
    /// for skip edge visualization.
    fn assign_virtual_x_coordinates(
        &self,
        levels: &[Vec<VirtualNode>],
        _node_levels: &[usize],
    ) -> (Vec<Vec<usize>>, Vec<Vec<usize>>) {
        let mut x_coords: Vec<Vec<usize>> = Vec::with_capacity(levels.len());
        let mut widths: Vec<Vec<usize>> = Vec::with_capacity(levels.len());

        for level_nodes in levels {
            let mut level_x = Vec::with_capacity(level_nodes.len());
            let mut level_w = Vec::with_capacity(level_nodes.len());
            let mut x = 0;

            for vnode in level_nodes {
                let width = match vnode {
                    VirtualNode::Real(idx) => self.get_node_width(*idx),
                    VirtualNode::Dummy(_) => 1, // Dummy nodes are 1 char wide (just a vertical line)
                };

                level_x.push(x);
                level_w.push(width);
                x += width + 3; // Standard spacing
            }

            x_coords.push(level_x);
            widths.push(level_w);
        }

        // Note: We intentionally DON'T center dummies over their source.
        // By keeping dummies at their natural position (end of each level),
        // they get their own column which makes skip edges visible.

        (x_coords, widths)
    }

    /// Build edges between adjacent levels in the virtual layout.
    fn build_virtual_edges(
        &self,
        levels: &[Vec<VirtualNode>],
        node_levels: &[usize],
    ) -> Vec<(usize, usize, usize, usize)> {
        let mut edges = Vec::new();

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
                        .position(|vn| matches!(vn, VirtualNode::Real(i) if *i == from_idx))
                        && let Some(to_pos) = levels[to_level]
                            .iter()
                            .position(|vn| matches!(vn, VirtualNode::Real(i) if *i == to_idx))
                    {
                        edges.push((from_level, from_pos, to_level, to_pos));
                    }
                } else if to_level > from_level + 1 {
                    // Skip edge - route through dummies identified by edge_idx
                    // Find source position
                    if let Some(from_pos) = levels[from_level]
                        .iter()
                        .position(|vn| matches!(vn, VirtualNode::Real(i) if *i == from_idx))
                    {
                        // Find first dummy at from_level + 1
                        if let Some(first_dummy_pos) = levels[from_level + 1]
                            .iter()
                            .position(|vn| matches!(vn, VirtualNode::Dummy(ei) if *ei == edge_idx))
                        {
                            // Edge from source to first dummy
                            edges.push((from_level, from_pos, from_level + 1, first_dummy_pos));

                            // Edges between consecutive dummies
                            for level in (from_level + 1)..(to_level - 1) {
                                if let Some(curr_pos) = levels[level]
                                    .iter()
                                    .position(|vn| matches!(vn, VirtualNode::Dummy(ei) if *ei == edge_idx))
                                    && let Some(next_pos) = levels[level + 1]
                                        .iter()
                                        .position(|vn| matches!(vn, VirtualNode::Dummy(ei) if *ei == edge_idx))
                                {
                                    edges.push((level, curr_pos, level + 1, next_pos));
                                }
                            }

                            // Edge from last dummy to target
                            if let Some(last_dummy_pos) = levels[to_level - 1]
                                .iter()
                                .position(|vn| matches!(vn, VirtualNode::Dummy(ei) if *ei == edge_idx))
                                && let Some(to_pos) = levels[to_level]
                                    .iter()
                                    .position(|vn| matches!(vn, VirtualNode::Real(i) if *i == to_idx))
                            {
                                edges.push((to_level - 1, last_dummy_pos, to_level, to_pos));
                            }
                        }
                    }
                }
            }
        }

        edges
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

                // Add spacing to reach this node's position
                while current_col < node_x {
                    output.push(' ');
                    current_col += 1;
                }

                match vnode {
                    VirtualNode::Real(idx) => {
                        let (id, label) = self.nodes[*idx];
                        self.write_node(output, id, label);
                        current_col += layout.widths[current_level][pos];
                    }
                    VirtualNode::Dummy(_) => {
                        // Dummy nodes are invisible in the node row
                        output.push(' ');
                        current_col += 1;
                    }
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

        // Find all edges from current_level to next_level
        let level_edges: Vec<_> = layout
            .edges
            .iter()
            .filter(|(fl, _, tl, _)| *fl == current_level && *tl == next_level)
            .collect();

        if level_edges.is_empty() {
            return;
        }

        // Calculate center positions for connections
        let mut connections: Vec<(usize, usize, bool, bool)> = Vec::new(); // (from_x, to_x, from_is_dummy, to_is_dummy)

        for &&(_, from_pos, _, to_pos) in &level_edges {
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
    #[allow(clippy::needless_range_loop)]
    fn draw_mixed_connections(
        &self,
        output: &mut String,
        connections: &[(usize, usize, bool, bool)],
        max_pos: usize,
    ) {
        let sources: Vec<usize> = connections.iter().map(|(f, _, _, _)| *f).collect();
        let targets: Vec<usize> = connections.iter().map(|(_, t, _, _)| *t).collect();

        // Classify connections
        let mut straight_down: Vec<usize> = Vec::new();  // from_x == to_x
        let mut going_right: Vec<(usize, usize)> = Vec::new();  // from_x < to_x
        let mut going_left: Vec<(usize, usize)> = Vec::new();   // from_x > to_x

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
            output.push(if sources.contains(&i) { V_LINE } else { ' ' });
        }
        writeln!(output).ok();

        if has_crossings {
            // Complex case: draw divergence first (sources branching out)
            // Line 2a: Divergence from sources
            let mut line2a: Vec<char> = vec![' '; max_pos + 1];
            
            // Draw all going_right and going_left as divergence
            for &(from_x, to_x) in &going_right {
                for i in from_x..=to_x {
                    if i == from_x {
                        line2a[i] = if line2a[i] == CORNER_UL { TEE_DOWN } else { CORNER_UR };
                    } else if i == to_x {
                        line2a[i] = CORNER_UL;
                    } else if line2a[i] == ' ' {
                        line2a[i] = H_LINE;
                    }
                }
            }
            for &(from_x, to_x) in &going_left {
                for i in to_x..=from_x {
                    if i == from_x {
                        line2a[i] = if line2a[i] == CORNER_UR { TEE_DOWN } else { CORNER_UL };
                    } else if i == to_x {
                        line2a[i] = if line2a[i] == CORNER_UL { TEE_DOWN } else { CORNER_UR };
                    } else if line2a[i] == ' ' {
                        line2a[i] = H_LINE;
                    }
                }
            }
            // Add straight down
            for &x in &straight_down {
                if line2a[x] == ' ' {
                    line2a[x] = V_LINE;
                } else if line2a[x] == H_LINE {
                    line2a[x] = TEE_DOWN;
                }
            }

            for ch in &line2a {
                output.push(*ch);
            }
            writeln!(output).ok();

            // Line 2b: Vertical continuation
            for i in 0..=max_pos {
                output.push(if targets.contains(&i) || straight_down.contains(&i) { V_LINE } else { ' ' });
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
            output.push(if targets.contains(&i) { ARROW_DOWN } else { ' ' });
        }
        writeln!(output).ok();
    }

    /// Draw pure convergence pattern.
    fn draw_convergence_connections(
        &self,
        output: &mut String,
        target_groups: &[(usize, Vec<(usize, bool)>)],
        max_pos: usize,
    ) {
        let all_sources: Vec<usize> = target_groups
            .iter()
            .flat_map(|(_, sources)| sources.iter().map(|(x, _)| *x))
            .collect();

        // Line 1: Vertical drops
        for i in 0..=max_pos {
            output.push(if all_sources.contains(&i) { V_LINE } else { ' ' });
        }
        writeln!(output).ok();

        // Line 2: Horizontal convergence
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
            output.push(ch);
        }
        writeln!(output).ok();

        // Line 3: Arrows
        for i in 0..=max_pos {
            if target_groups.iter().any(|(t, _)| *t == i) {
                output.push(ARROW_DOWN);
            } else {
                output.push(' ');
            }
        }
        writeln!(output).ok();
    }

    /// Draw pure divergence pattern.
    fn draw_divergence_connections(
        &self,
        output: &mut String,
        source_groups: &[(usize, Vec<(usize, bool)>)],
        max_pos: usize,
    ) {
        let all_sources: Vec<usize> = source_groups.iter().map(|(s, _)| *s).collect();

        // Line 1: Vertical from sources
        for i in 0..=max_pos {
            output.push(if all_sources.contains(&i) { V_LINE } else { ' ' });
        }
        writeln!(output).ok();

        // Line 2: Horizontal divergence
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
                    ch = CORNER_UR;
                } else if i == max_tgt {
                    ch = CORNER_UL;
                } else if target_xs.contains(&i) {
                    ch = TEE_DOWN;
                } else if i > min_tgt && i < max_tgt && ch == ' ' {
                    ch = H_LINE;
                }
            }
            output.push(ch);
        }
        writeln!(output).ok();

        // Line 3: Arrows at targets
        let all_targets: Vec<usize> = source_groups
            .iter()
            .flat_map(|(_, t)| t.iter().map(|(x, _)| *x))
            .collect();
        for i in 0..=max_pos {
            output.push(if all_targets.contains(&i) { ARROW_DOWN } else { ' ' });
        }
        writeln!(output).ok();
    }

    /// Draw simple 1-to-1 connections.
    fn draw_simple_connections(
        &self,
        output: &mut String,
        connections: &[(usize, usize, bool, bool)],
        max_pos: usize,
    ) {
        let sources: Vec<usize> = connections.iter().map(|(f, _, _, _)| *f).collect();

        // Line 1: Vertical
        for i in 0..=max_pos {
            output.push(if sources.contains(&i) { V_LINE } else { ' ' });
        }
        writeln!(output).ok();

        // Line 2: Arrows (straight down for 1-to-1)
        for i in 0..=max_pos {
            output.push(if sources.contains(&i) { ARROW_DOWN } else { ' ' });
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

    fn draw_multiple_convergences(
        &self,
        output: &mut String,
        target_groups: &[ConnectionGroup],
    ) {
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
                } else if i > min_source && i < max_source
                    && char_at_pos == ' ' {
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

    fn draw_multiple_divergences(
        &self,
        output: &mut String,
        source_groups: &[ConnectionGroup],
    ) {
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
                    } else if i > min_target && i < max_target
                        && char_at_pos == ' ' {
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
