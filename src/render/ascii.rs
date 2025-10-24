//! ASCII rendering implementation for DAG visualization.

use crate::graph::{DAG, RenderMode};
use alloc::{string::String, vec, vec::Vec};
use core::fmt::Write;

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
        let mode = match self.render_mode {
            RenderMode::Auto => {
                if self.is_simple_chain() {
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

    /// Render in vertical mode (Sugiyama layout).
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

        // Single connected graph - 4-Pass Sugiyama-inspired layout
        let level_data = self.calculate_levels();
        let max_level = level_data.iter().map(|(_, l)| *l).max().unwrap_or(0);

        // Group nodes by level
        let mut levels: Vec<Vec<usize>> = vec![Vec::new(); max_level + 1];
        for (idx, level) in &level_data {
            levels[*level].push(*idx);
        }

        // === PASS 1: Crossing Reduction (Median Heuristic) ===
        self.reduce_crossings(&mut levels, max_level);

        // === PASS 2: Character-Level Coordinate Assignment ===
        let node_x_coords = self.assign_x_coordinates(&mut levels, max_level);

        // === PASS 3: Calculate Canvas Width and Centering ===
        let (level_widths, max_canvas_width) =
            self.calculate_canvas_dimensions(&levels, &node_x_coords);

        // === PASS 4: Render with Manhattan Routing ===
        for (current_level, level_nodes) in levels.iter().enumerate() {
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

            // Find minimum x-coordinate in this level
            let min_x = level_nodes
                .iter()
                .map(|&idx| node_x_coords[idx])
                .min()
                .unwrap_or(0);

            // Render nodes at their assigned x-coordinates
            let mut current_col = 0;
            for &idx in level_nodes {
                let node_x = node_x_coords[idx] - min_x + level_offset;

                // Add spacing to reach this node's position
                while current_col < node_x {
                    output.push(' ');
                    current_col += 1;
                }

                let (id, label) = self.nodes[idx];
                // Write directly to avoid intermediate allocation
                self.write_node(output, id, label);
                current_col += self.get_node_width(idx); // Use cached width
            }
            writeln!(output).ok();

            // Draw connections if not last level
            if current_level < max_level {
                let next_level_width = level_widths[current_level + 1];
                let next_level_offset = if max_canvas_width > next_level_width {
                    (max_canvas_width - next_level_width) / 2
                } else {
                    0
                };

                self.draw_connections_sugiyama(
                    output,
                    level_nodes,
                    &levels[current_level + 1],
                    &node_x_coords,
                    min_x,
                    level_offset,
                    next_level_offset,
                );
            }
        }
    }

    /// PASS 4: Draw connections with Manhattan routing.
    fn draw_connections_sugiyama(
        &self,
        output: &mut String,
        current_nodes: &[usize],
        next_nodes: &[usize],
        x_coords: &[usize],
        current_min_x: usize,
        current_offset: usize,
        next_offset: usize,
    ) {
        if current_nodes.is_empty() || next_nodes.is_empty() {
            return;
        }

        // Calculate center positions
        let current_centers: Vec<(usize, usize)> = current_nodes
            .iter()
            .map(|&idx| {
                let width = self.get_node_width(idx);
                let center = x_coords[idx] - current_min_x + current_offset + width / 2;
                (idx, center)
            })
            .collect();

        let next_min_x = next_nodes
            .iter()
            .map(|&idx| x_coords[idx])
            .min()
            .unwrap_or(0);
        let next_centers: Vec<(usize, usize)> = next_nodes
            .iter()
            .map(|&idx| {
                let width = self.get_node_width(idx);
                let center = x_coords[idx] - next_min_x + next_offset + width / 2;
                (idx, center)
            })
            .collect();

        // Find connections
        let mut connections: Vec<(usize, usize)> = Vec::new();
        for &(curr_idx, from_pos) in &current_centers {
            let node_id = self.nodes[curr_idx].0;
            for child_id in self.get_children(node_id) {
                if let Some(&(_, to_pos)) = next_centers
                    .iter()
                    .find(|(idx, _)| self.nodes[*idx].0 == child_id)
                {
                    connections.push((from_pos, to_pos));
                }
            }
        }

        if connections.is_empty() {
            return;
        }

        // Group by target/source for convergence/divergence detection
        let mut target_groups: Vec<(usize, Vec<usize>)> = Vec::new();
        for &(from, to) in &connections {
            match target_groups.binary_search_by_key(&to, |(k, _)| *k) {
                Ok(idx) => target_groups[idx].1.push(from),
                Err(idx) => target_groups.insert(idx, (to, vec![from])),
            }
        }

        let mut source_groups: Vec<(usize, Vec<usize>)> = Vec::new();
        for &(from, to) in &connections {
            match source_groups.binary_search_by_key(&from, |(k, _)| *k) {
                Ok(idx) => source_groups[idx].1.push(to),
                Err(idx) => source_groups.insert(idx, (from, vec![to])),
            }
        }

        let has_convergence = target_groups.iter().any(|(_, v)| v.len() > 1);
        let has_divergence = source_groups.iter().any(|(_, v)| v.len() > 1);

        // Find the range we need to draw - always start from 0 since nodes are positioned from 0
        let min_pos = 0;
        let max_pos = connections
            .iter()
            .flat_map(|(f, t)| [*f, *t])
            .max()
            .unwrap_or(0);

        // Draw based on pattern
        if has_convergence && !has_divergence {
            self.draw_convergence_manhattan(output, &target_groups, min_pos, max_pos);
        } else if has_divergence && !has_convergence {
            self.draw_divergence_manhattan(output, &source_groups, min_pos, max_pos);
        } else {
            self.draw_simple_manhattan(output, &connections, min_pos, max_pos);
        }
    }

    fn draw_convergence_manhattan(
        &self,
        output: &mut String,
        target_groups: &[(usize, Vec<usize>)],
        min_pos: usize,
        max_pos: usize,
    ) {
        let all_sources: Vec<usize> = target_groups
            .iter()
            .flat_map(|(_, sources)| sources.iter().copied())
            .collect();

        // Line 1: Vertical drops
        for i in min_pos..=max_pos {
            output.push(if all_sources.contains(&i) {
                V_LINE
            } else {
                ' '
            });
        }
        writeln!(output).ok();

        // Line 2: Horizontal convergence └──┴──┘
        for i in min_pos..=max_pos {
            let mut ch = ' ';
            for (_, sources) in target_groups.iter() {
                if sources.len() <= 1 {
                    continue;
                }
                let min_src = *sources.iter().min().unwrap();
                let max_src = *sources.iter().max().unwrap();
                if i == min_src {
                    ch = CORNER_DR;
                } else if i == max_src {
                    ch = CORNER_DL;
                } else if sources.contains(&i) {
                    ch = TEE_UP;
                } else if i > min_src && i < max_src {
                    ch = H_LINE;
                }
            }
            output.push(ch);
        }
        writeln!(output).ok();

        // Line 3: Arrows down
        for i in min_pos..=max_pos {
            output.push(if target_groups.iter().any(|(t, _)| *t == i) {
                ARROW_DOWN
            } else {
                ' '
            });
        }
        writeln!(output).ok();
    }

    fn draw_divergence_manhattan(
        &self,
        output: &mut String,
        source_groups: &[(usize, Vec<usize>)],
        min_pos: usize,
        max_pos: usize,
    ) {
        let all_sources: Vec<usize> = source_groups.iter().map(|(s, _)| *s).collect();

        // Line 1: Vertical from sources
        for i in min_pos..=max_pos {
            output.push(if all_sources.contains(&i) {
                V_LINE
            } else {
                ' '
            });
        }
        writeln!(output).ok();

        // Line 2: Horizontal divergence ┌──┬──┐
        for i in min_pos..=max_pos {
            let mut ch = ' ';
            for (_, targets) in source_groups.iter() {
                if targets.len() <= 1 {
                    continue;
                }
                let min_tgt = *targets.iter().min().unwrap();
                let max_tgt = *targets.iter().max().unwrap();
                if i == min_tgt {
                    ch = CORNER_UR;
                } else if i == max_tgt {
                    ch = CORNER_UL;
                } else if targets.contains(&i) {
                    ch = TEE_DOWN;
                } else if i > min_tgt && i < max_tgt {
                    ch = H_LINE;
                }
            }
            output.push(ch);
        }
        writeln!(output).ok();

        // Line 3: Arrows down
        let all_targets: Vec<usize> = source_groups
            .iter()
            .flat_map(|(_, t)| t.iter().copied())
            .collect();
        for i in min_pos..=max_pos {
            output.push(if all_targets.contains(&i) {
                ARROW_DOWN
            } else {
                ' '
            });
        }
        writeln!(output).ok();
    }

    fn draw_simple_manhattan(
        &self,
        output: &mut String,
        connections: &[(usize, usize)],
        min_pos: usize,
        max_pos: usize,
    ) {
        // Line 1: Vertical
        for i in min_pos..=max_pos {
            output.push(if connections.iter().any(|(f, _)| *f == i) {
                V_LINE
            } else {
                ' '
            });
        }
        writeln!(output).ok();

        // Line 2: Arrows
        for i in min_pos..=max_pos {
            output.push(if connections.iter().any(|(f, _)| *f == i) {
                ARROW_DOWN
            } else {
                ' '
            });
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
        let mut target_groups: Vec<(usize, Vec<(usize, usize, usize)>)> = Vec::new();

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
        let mut source_groups: Vec<(usize, Vec<(usize, usize, usize)>)> = Vec::new();

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
        target_groups: &[(usize, Vec<(usize, usize, usize)>)],
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
                } else if i > min_source && i < max_source {
                    if char_at_pos == ' ' {
                        char_at_pos = H_LINE; // ─
                    }
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
        source_groups: &[(usize, Vec<(usize, usize, usize)>)],
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
                    } else if i > min_target && i < max_target {
                        if char_at_pos == ' ' {
                            char_at_pos = H_LINE; // ─
                        }
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
