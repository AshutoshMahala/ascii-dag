//! Classic renderer for subgraph and simple vertical rendering.
//!
//! This module handles the pre-0.6 classic rendering approach including
//! subgraph rendering, vertical connections, and convergence/divergence
//! patterns for the classic layout mode.

use crate::graph::DAG;
use alloc::{vec, vec::Vec};
use core::fmt::Write;

use super::ascii::{ConnectionGroup, ARROW_RIGHT};
use super::chars::{
    ARROW_DOWN, CORNER_DL, CORNER_DR, CORNER_UL, CORNER_UR, H_LINE, TEE_DOWN, TEE_UP, V_LINE,
};

impl<'a> DAG<'a> {
    /// Render a specific subgraph.
    pub(crate) fn render_subgraph(&self, output: &mut alloc::string::String, subgraph_indices: &[usize]) {
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
        output: &mut alloc::string::String,
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
        output: &mut alloc::string::String,
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

    fn draw_multiple_divergences(
        &self,
        output: &mut alloc::string::String,
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

    fn draw_simple_verticals(
        &self,
        output: &mut alloc::string::String,
        connections: &[(usize, usize, usize)],
    ) {
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
