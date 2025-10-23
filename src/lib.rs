//! # ascii-dag
//!
//! Lightweight ASCII DAG (Directed Acyclic Graph) renderer for error chains,
//! build systems, and dependency visualization.
//!
//! ## Features
//!
//! - **Tiny**: ~41KB WASM, zero dependencies
//! - **Fast**: O(log n) grouping with binary search, zero-copy rendering
//! - **no_std**: Works in embedded/WASM environments
//! - **Flexible**: Builder API or batch construction
//! - **Safe**: Cycle detection built-in
//!
//! ## Quick Start
//!
//! ```rust
//! use ascii_dag::DAG;
//!
//! // Batch construction (fast!)
//! let dag = DAG::from_edges(
//!     &[(1, "Error1"), (2, "Error2"), (3, "Error3")],
//!     &[(1, 2), (2, 3)]
//! );
//!
//! println!("{}", dag.render());
//! ```
//!
//! ## Builder API
//!
//! ```rust
//! use ascii_dag::DAG;
//!
//! let mut dag = DAG::new();
//! dag.add_node(1, "A");
//! dag.add_node(2, "B");
//! dag.add_edge(1, 2);
//!
//! println!("{}", dag.render());
//! ```

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

use alloc::{string::String, vec::Vec, vec};
use core::fmt::Write;

#[cfg(feature = "std")]
use std::collections::{HashMap, HashSet};

#[cfg(not(feature = "std"))]
use alloc::collections::{BTreeMap as HashMap, BTreeSet as HashSet};

// Box drawing characters (Unicode)
const V_LINE: char = '│';
const H_LINE: char = '─';
const ARROW_DOWN: char = '↓';
const ARROW_RIGHT: char = '→';
const CYCLE_ARROW: char = '⇄'; // For cycle detection

// Convergence/divergence
const CORNER_DR: char = '└'; // Down-Right corner
const CORNER_DL: char = '┘'; // Down-Left corner  
const TEE_DOWN: char = '┬';  // T pointing down
const TEE_UP: char = '┴';    // T pointing up
const CORNER_UR: char = '┌'; // Up-Right corner
const CORNER_UL: char = '┐'; // Up-Left corner

/// Rendering mode for the DAG visualization.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderMode {
    /// Render chains vertically (takes more vertical space)
    Vertical,
    
    /// Render chains horizontally when possible (compact, one-line for simple chains)
    Horizontal,
    
    /// Auto-detect: horizontal for simple chains, vertical for complex graphs
    Auto,
}

impl Default for RenderMode {
    fn default() -> Self {
        RenderMode::Auto
    }
}

/// A Directed Acyclic Graph (DAG) with ASCII rendering capabilities.
///
/// # Examples
///
/// ```
/// use ascii_dag::DAG;
///
/// let mut dag = DAG::new();
/// dag.add_node(1, "Start");
/// dag.add_node(2, "End");
/// dag.add_edge(1, 2);
///
/// let output = dag.render();
/// assert!(output.contains("Start"));
/// assert!(output.contains("End"));
/// ```
#[derive(Clone)]
pub struct DAG<'a> {
    nodes: Vec<(usize, &'a str)>,
    edges: Vec<(usize, usize)>,
    render_mode: RenderMode,
    auto_created: HashSet<usize>,  // Track auto-created nodes for visual distinction (O(1) lookups)
    id_to_index: HashMap<usize, usize>,  // Cache id→index mapping (O(1) lookups)
    node_widths: Vec<usize>,  // Cached formatted widths
}

impl<'a> Default for DAG<'a> {
    fn default() -> Self {
        Self {
            nodes: Vec::new(),
            edges: Vec::new(),
            render_mode: RenderMode::default(),
            auto_created: HashSet::new(),
            id_to_index: HashMap::new(),
            node_widths: Vec::new(),
        }
    }
}

impl<'a> DAG<'a> {
    /// Create a new empty DAG.
    ///
    /// # Examples
    ///
    /// ```
    /// use ascii_dag::DAG;
    /// let dag = DAG::new();
    /// ```
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a DAG from pre-defined nodes and edges (batch construction).
    ///
    /// This is more efficient than using the builder API for static graphs.
    ///
    /// # Examples
    ///
    /// ```
    /// use ascii_dag::DAG;
    ///
    /// let dag = DAG::from_edges(
    ///     &[(1, "A"), (2, "B"), (3, "C")],
    ///     &[(1, 2), (2, 3)]
    /// );
    /// ```
    pub fn from_edges(nodes: &[(usize, &'a str)], edges: &[(usize, usize)]) -> Self {
        let mut dag = Self {
            nodes: nodes.to_vec(),
            edges: Vec::new(),
            render_mode: RenderMode::default(),
            auto_created: HashSet::new(),
            id_to_index: HashMap::new(),
            node_widths: Vec::new(),
        };
        
        // Build id_to_index map and widths cache
        for (idx, &(id, label)) in dag.nodes.iter().enumerate() {
            dag.id_to_index.insert(id, idx);
            let width = dag.compute_node_width(id, label);
            dag.node_widths.push(width);
        }
        
        // Add edges (may auto-create missing nodes)
        for &(from, to) in edges {
            dag.add_edge(from, to);
        }
        
        dag
    }

    /// Set the rendering mode.
    ///
    /// # Examples
    ///
    /// ```
    /// use ascii_dag::{DAG, RenderMode};
    ///
    /// let mut dag = DAG::new();
    /// dag.set_render_mode(RenderMode::Horizontal);
    /// ```
    pub fn set_render_mode(&mut self, mode: RenderMode) {
        self.render_mode = mode;
    }

    /// Create a DAG with a specific render mode.
    ///
    /// # Examples
    ///
    /// ```
    /// use ascii_dag::{DAG, RenderMode};
    ///
    /// let dag = DAG::with_mode(RenderMode::Horizontal);
    /// ```
    pub fn with_mode(mode: RenderMode) -> Self {
        Self {
            nodes: Vec::new(),
            edges: Vec::new(),
            render_mode: mode,
            auto_created: HashSet::new(),
            id_to_index: HashMap::new(),
            node_widths: Vec::new(),
        }
    }

    /// Add a node to the DAG.
    ///
    /// If the node was previously auto-created by `add_edge`, this will promote it
    /// by setting its label and removing the auto-created flag.
    ///
    /// # Examples
    ///
    /// ```
    /// use ascii_dag::DAG;
    ///
    /// let mut dag = DAG::new();
    /// dag.add_node(1, "MyNode");
    /// ```
    pub fn add_node(&mut self, id: usize, label: &'a str) {
        // Check if node already exists (could be auto-created) - O(1) with HashMap
        if let Some(&idx) = self.id_to_index.get(&id) {
            // Promote auto-created node to explicit node
            self.nodes[idx] = (id, label);
            // Remove from auto_created set - O(1)
            self.auto_created.remove(&id);
            // Update cached width
            let width = self.compute_node_width(id, label);
            self.node_widths[idx] = width;
        } else {
            // Brand new node
            let idx = self.nodes.len();
            self.nodes.push((id, label));
            self.id_to_index.insert(id, idx);
            let width = self.compute_node_width(id, label);
            self.node_widths.push(width);
        }
    }

    /// Add an edge from one node to another.
    ///
    /// If either node doesn't exist, it will be auto-created as a placeholder.
    /// You can later call `add_node` to provide a label for auto-created nodes.
    ///
    /// # Examples
    ///
    /// ```
    /// use ascii_dag::DAG;
    ///
    /// let mut dag = DAG::new();
    /// dag.add_node(1, "A");
    /// dag.add_node(2, "B");
    /// dag.add_edge(1, 2);  // A -> B
    /// ```
    pub fn add_edge(&mut self, from: usize, to: usize) {
        self.ensure_node_exists(from);
        self.ensure_node_exists(to);
        self.edges.push((from, to));
    }

    /// Ensure a node exists, auto-creating if missing.
    /// Auto-created nodes will be visually distinct (rendered with ⟨⟩ instead of [])
    /// until explicitly defined with add_node.
    fn ensure_node_exists(&mut self, id: usize) {
        // O(1) lookup with HashMap
        if !self.id_to_index.contains_key(&id) {
            #[cfg(feature = "warnings")]
            {
                eprintln!(
                    "[ascii-dag] Warning: Node {} missing - auto-creating as placeholder. \
                     Call add_node({}, \"label\") before add_edge() to provide a label.",
                    id, id
                );
            }
            
            // Create node with empty label
            let idx = self.nodes.len();
            self.nodes.push((id, ""));
            self.auto_created.insert(id);  // O(1) insert
            self.id_to_index.insert(id, idx);  // O(1) insert
            let width = self.compute_node_width(id, "");
            self.node_widths.push(width);
        }
    }

    /// Check if a node was auto-created (for visual distinction)
    fn is_auto_created(&self, id: usize) -> bool {
        self.auto_created.contains(&id)  // O(1) with HashSet
    }

    /// Write an unsigned integer to a string buffer without allocation.
    /// This avoids format! bloat in no_std builds.
    #[inline]
    fn write_usize(buf: &mut String, mut n: usize) {
        if n == 0 {
            buf.push('0');
            return;
        }
        let mut digits = [0u8; 20]; // Max digits for u64
        let mut i = 0;
        while n > 0 {
            digits[i] = (n % 10) as u8 + b'0';
            n /= 10;
            i += 1;
        }
        // Write in reverse order
        while i > 0 {
            i -= 1;
            buf.push(digits[i] as char);
        }
    }

    /// Count digits in a number (for width calculation)
    #[inline]
    fn count_digits(mut n: usize) -> usize {
        if n == 0 { return 1; }
        let mut count = 0;
        while n > 0 {
            count += 1;
            n /= 10;
        }
        count
    }

    /// Compute the formatted width of a node
    fn compute_node_width(&self, id: usize, label: &str) -> usize {
        if label.is_empty() || self.is_auto_created(id) {
            // ⟨ID⟩ format
            2 + Self::count_digits(id)  // ⟨ + digits + ⟩
        } else {
            // [Label] format
            2 + label.chars().count()  // [ + label + ]
        }
    }

    /// Format a node with appropriate brackets:
    /// - Normal nodes (with labels): [Label]
    /// - Auto-created nodes (no label): ⟨ID⟩
    fn format_node(&self, id: usize, label: &str) -> String {
        let mut buf = String::new();
        if label.is_empty() || self.is_auto_created(id) {
            // Auto-created: angle brackets ⟨⟩ and show ID
            buf.push('⟨');
            Self::write_usize(&mut buf, id);
            buf.push('⟩');
        } else {
            // Normal: square brackets []
            buf.push('[');
            buf.push_str(label);
            buf.push(']');
        }
        buf
    }

    /// Write a formatted node directly to output buffer (avoids intermediate String allocation)
    #[inline]
    fn write_node(&self, output: &mut String, id: usize, label: &str) {
        if label.is_empty() || self.is_auto_created(id) {
            output.push('⟨');
            Self::write_usize(output, id);
            output.push('⟩');
        } else {
            output.push('[');
            output.push_str(label);
            output.push(']');
        }
    }

    /// Check if the graph contains cycles (making it not a valid DAG).
    ///
    /// # Examples
    ///
    /// ```
    /// use ascii_dag::DAG;
    ///
    /// let mut dag = DAG::new();
    /// dag.add_node(1, "A");
    /// dag.add_node(2, "B");
    /// dag.add_edge(1, 2);
    /// dag.add_edge(2, 1);  // Creates a cycle!
    ///
    /// assert!(dag.has_cycle());
    /// ```
    pub fn has_cycle(&self) -> bool {
        let mut visited = vec![false; self.nodes.len()];
        let mut rec_stack = vec![false; self.nodes.len()];

        for i in 0..self.nodes.len() {
            if self.has_cycle_util(i, &mut visited, &mut rec_stack) {
                return true;
            }
        }
        false
    }

    fn has_cycle_util(&self, idx: usize, visited: &mut [bool], rec_stack: &mut [bool]) -> bool {
        if rec_stack[idx] {
            return true;
        }
        if visited[idx] {
            return false;
        }

        visited[idx] = true;
        rec_stack[idx] = true;

        let node_id = self.nodes[idx].0;
        for &(from, to) in &self.edges {
            if from == node_id {
                // O(1) HashMap lookup instead of O(n) scan
                if let Some(child_idx) = self.node_index(to) {
                    if self.has_cycle_util(child_idx, visited, rec_stack) {
                        return true;
                    }
                }
            }
        }

        rec_stack[idx] = false;
        false
    }

    /// Build adjacency lists (forward and reverse) once to avoid repeated allocations.
    /// Returns (children_map, parents_map) where each is Vec<Vec<usize>> indexed by node index.
    /// 
    /// Currently unused but available for future optimization where adjacency lists
    /// could be cached on the DAG struct instead of rebuilding from edges.
    #[allow(dead_code)]
    fn build_adjacency_lists(&self) -> (Vec<Vec<usize>>, Vec<Vec<usize>>) {
        let n = self.nodes.len();
        let mut children = vec![Vec::new(); n];
        let mut parents = vec![Vec::new(); n];
        
        for &(from_id, to_id) in &self.edges {
            // Use cached O(1) lookups instead of O(n) scans
            if let (Some(&from_idx), Some(&to_idx)) = (
                self.id_to_index.get(&from_id),
                self.id_to_index.get(&to_id)
            ) {
                // Store indices (not IDs) to match node-indexed vectors used elsewhere
                children[from_idx].push(to_idx);
                parents[to_idx].push(from_idx);
            }
        }
        
        (children, parents)
    }

    /// Estimate the buffer size needed for rendering.
    ///
    /// Use this to pre-allocate a buffer for [`render_to`](Self::render_to).
    ///
    /// # Examples
    ///
    /// ```
    /// use ascii_dag::DAG;
    ///
    /// let dag = DAG::from_edges(
    ///     &[(1, "A"), (2, "B")],
    ///     &[(1, 2)]
    /// );
    ///
    /// let size = dag.estimate_size();
    /// let mut buffer = String::with_capacity(size);
    /// dag.render_to(&mut buffer);
    /// ```
    pub fn estimate_size(&self) -> usize {
        // Rough estimate: nodes * avg_label_size + edges * connection_chars + box
        self.nodes.len() * 25 + self.edges.len() * 15 + 200
    }

    /// Render the DAG to an ASCII string.
    ///
    /// # Examples
    ///
    /// ```
    /// use ascii_dag::DAG;
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
    /// use ascii_dag::DAG;
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

    /// Render a graph with cycles (not a valid DAG, but useful for error visualization)
    fn render_cycle(&self, output: &mut String) {
        writeln!(output, "⚠️  CYCLE DETECTED - Not a valid DAG").ok();
        writeln!(output).ok();
        
        // Find the cycle using DFS
        if let Some(cycle_nodes) = self.find_cycle_path() {
            writeln!(output, "Cyclic dependency chain:").ok();
            
            for (i, node_id) in cycle_nodes.iter().enumerate() {
                if let Some(&(id, label)) = self.nodes.iter().find(|(nid, _)| nid == node_id) {
                    output.push_str(&self.format_node(id, label));
                    
                    if i < cycle_nodes.len() - 1 {
                        write!(output, " → ").ok();
                    } else {
                        // Last node, show it cycles back
                        if let Some(&(first_id, first_label)) = self.nodes.iter().find(|(nid, _)| nid == &cycle_nodes[0]) {
                            write!(output, " {} ", CYCLE_ARROW).ok();
                            output.push_str(&self.format_node(first_id, first_label));
                        }
                    }
                }
            }
            writeln!(output).ok();
            writeln!(output).ok();
            writeln!(output, "This creates an infinite loop in error dependencies.").ok();
        } else {
            writeln!(output, "Complex cycle detected in graph.").ok();
        }
    }

    /// Find a cycle path in the graph
    fn find_cycle_path(&self) -> Option<Vec<usize>> {
        for i in 0..self.nodes.len() {
            let mut visited = vec![false; self.nodes.len()];
            let mut path = Vec::new();
            
            if let Some(cycle) = self.find_cycle_from(i, &mut visited, &mut path) {
                return Some(cycle);
            }
        }
        None
    }

    fn find_cycle_from(&self, start_idx: usize, visited: &mut [bool], path: &mut Vec<usize>) -> Option<Vec<usize>> {
        if visited[start_idx] {
            // Found a cycle - extract it from path
            if let Some(cycle_start) = path.iter().position(|&idx| idx == start_idx) {
                return Some(path[cycle_start..].iter().map(|&idx| self.nodes[idx].0).collect());
            }
            return None;
        }

        visited[start_idx] = true;
        path.push(start_idx);

        let node_id = self.nodes[start_idx].0;
        for &(from, to) in &self.edges {
            if from == node_id {
                // O(1) HashMap lookup instead of O(n) scan
                if let Some(child_idx) = self.node_index(to) {
                    if let Some(cycle) = self.find_cycle_from(child_idx, visited, path) {
                        return Some(cycle);
                    }
                }
            }
        }

        path.pop();
        None
    }

    /// Check if this is a simple chain (A → B → C, no branching)
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
        let roots: Vec<_> = self.nodes.iter()
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
                output.push_str(&self.format_node(id, label));
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

    /// Render in vertical mode (existing logic)
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
        let (level_widths, max_canvas_width) = self.calculate_canvas_dimensions(&levels, &node_x_coords);

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
            let min_x = level_nodes.iter()
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
                current_col += self.get_node_width(idx);  // Use cached width
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
                    next_level_offset
                );
            }
        }
    }

    /// PASS 1: Reduce edge crossings using median heuristic
    fn reduce_crossings(&self, levels: &mut [Vec<usize>], max_level: usize) {
        // Iterate a few times for better results (diminishing returns after 4-5 iterations)
        for _ in 0..4 {
            // Top-down pass: order nodes by median of parents
            for level_idx in 1..=max_level {
                // Split borrows to avoid clone
                let (prev_levels, rest) = levels.split_at_mut(level_idx);
                let parent_level = &prev_levels[level_idx - 1];
                self.order_by_median_parents(&mut rest[0], parent_level);
            }
            
            // Bottom-up pass: order nodes by median of children
            for level_idx in (0..max_level).rev() {
                // Split borrows to avoid clone
                let (left, right) = levels.split_at_mut(level_idx + 1);
                let child_level = &right[0];
                self.order_by_median_children(&mut left[level_idx], child_level);
            }
        }
    }

    fn order_by_median_parents(&self, level_nodes: &mut Vec<usize>, parent_level: &[usize]) {
        let mut node_medians: Vec<(usize, f32)> = Vec::new();
        
        for (pos, &idx) in level_nodes.iter().enumerate() {
            let node_id = self.nodes[idx].0;
            let parents = self.get_parents(node_id);
            
            if parents.is_empty() {
                node_medians.push((idx, pos as f32));
            } else {
                // Find positions of parents in the parent level
                let mut parent_positions: Vec<usize> = parents.iter()
                    .filter_map(|&p_id| parent_level.iter().position(|&i| self.nodes[i].0 == p_id))
                    .collect();
                parent_positions.sort_unstable();
                
                let median = if parent_positions.is_empty() {
                    pos as f32
                } else if parent_positions.len() % 2 == 1 {
                    parent_positions[parent_positions.len() / 2] as f32
                } else {
                    let mid = parent_positions.len() / 2;
                    (parent_positions[mid - 1] + parent_positions[mid]) as f32 / 2.0
                };
                
                node_medians.push((idx, median));
            }
        }
        
        // Sort by median
        node_medians.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
        *level_nodes = node_medians.iter().map(|(idx, _)| *idx).collect();
    }

    fn order_by_median_children(&self, level_nodes: &mut Vec<usize>, child_level: &[usize]) {
        let mut node_medians: Vec<(usize, f32)> = Vec::new();
        
        for (pos, &idx) in level_nodes.iter().enumerate() {
            let node_id = self.nodes[idx].0;
            let children = self.get_children(node_id);
            
            if children.is_empty() {
                node_medians.push((idx, pos as f32));
            } else {
                // Find positions of children in the child level
                let mut child_positions: Vec<usize> = children.iter()
                    .filter_map(|&c_id| child_level.iter().position(|&i| self.nodes[i].0 == c_id))
                    .collect();
                child_positions.sort_unstable();
                
                let median = if child_positions.is_empty() {
                    pos as f32
                } else if child_positions.len() % 2 == 1 {
                    child_positions[child_positions.len() / 2] as f32
                } else {
                    let mid = child_positions.len() / 2;
                    (child_positions[mid - 1] + child_positions[mid]) as f32 / 2.0
                };
                
                node_medians.push((idx, median));
            }
        }
        
        node_medians.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
        *level_nodes = node_medians.iter().map(|(idx, _)| *idx).collect();
    }

    /// PASS 2: Assign x-coordinates to each node (character-level positioning)
    fn assign_x_coordinates(&self, levels: &mut [Vec<usize>], max_level: usize) -> Vec<usize> {
        let mut x_coords = vec![0usize; self.nodes.len()];
        
        // Start with left-to-right layout within each level, preserving crossing reduction order
        for level_nodes in levels.iter() {
            let mut x = 0;
            for &idx in level_nodes.iter() {
                x_coords[idx] = x;
                let (id, label) = self.nodes[idx];
                let width = self.format_node(id, label).chars().count();
                x += width + 3;
            }
        }
        
        // Refine positions using median of connected nodes to center under parents/over children
        // But maintain relative order within levels
        for _ in 0..2 {
            // Top-down: center under parents where possible
            for level_idx in 1..=max_level {
                for &idx in &levels[level_idx] {
                    let node_id = self.nodes[idx].0;
                    let parents = self.get_parents(node_id);
                    
                    if !parents.is_empty() {
                        let mut parent_centers: Vec<usize> = Vec::new();
                        for &p_id in &parents {
                            // O(1) HashMap lookup instead of O(n) scan
                            if let Some(p_idx) = self.node_index(p_id) {
                                let width = self.get_node_width(p_idx);  // Use cached width
                                parent_centers.push(x_coords[p_idx] + width / 2);
                            }
                        }
                        
                        if !parent_centers.is_empty() {
                            parent_centers.sort_unstable();
                            let median = parent_centers[parent_centers.len() / 2];
                            let (id, label) = self.nodes[idx];
                            let width = self.format_node(id, label).chars().count();
                            // Shift toward median but don't reorder
                            let target = median.saturating_sub(width / 2);
                            x_coords[idx] = (x_coords[idx] + target) / 2;
                        }
                    }
                }
                
                // Re-compact this level to remove overlaps and reorder to match x-coords
                self.compact_level(&mut x_coords, &mut levels[level_idx]);
            }
        }
        
        x_coords
    }

    /// Compact a level to remove overlaps and reorder nodes left-to-right by x-coordinate
    fn compact_level(&self, x_coords: &mut [usize], level_nodes: &mut Vec<usize>) {
        if level_nodes.is_empty() {
            return;
        }
        
        // Sort nodes by their current x position
        let mut sorted: Vec<_> = level_nodes.iter()
            .map(|&idx| (x_coords[idx], idx))
            .collect();
        sorted.sort_by_key(|(x, _)| *x);
        
        // Reassign x-coords to remove overlaps and update level_nodes order
        level_nodes.clear();
        let mut x = 0;
        for (_, idx) in sorted {
            level_nodes.push(idx);
            x_coords[idx] = x;
            let (id, label) = self.nodes[idx];
            let width = self.format_node(id, label).chars().count();
            x += width + 3;
        }
    }

    /// PASS 3: Calculate canvas dimensions
    fn calculate_canvas_dimensions(&self, levels: &[Vec<usize>], x_coords: &[usize]) -> (Vec<usize>, usize) {
        let mut level_widths = Vec::new();
        let mut max_width = 0;
        
        for level_nodes in levels {
            if level_nodes.is_empty() {
                level_widths.push(0);
                continue;
            }
            
            let min_x = level_nodes.iter().map(|&idx| x_coords[idx]).min().unwrap_or(0);
            let max_node_idx = level_nodes.iter()
                .max_by_key(|&&idx| x_coords[idx])
                .unwrap();
            let (id, label) = self.nodes[*max_node_idx];
            let width = self.format_node(id, label).chars().count();
            let level_width = (x_coords[*max_node_idx] - min_x) + width;
            
            level_widths.push(level_width);
            max_width = max_width.max(level_width);
        }
        
        (level_widths, max_width)
    }

    /// PASS 4: Draw connections with Manhattan routing
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
        let current_centers: Vec<(usize, usize)> = current_nodes.iter().map(|&idx| {
            let (id, label) = self.nodes[idx];
            let width = self.format_node(id, label).chars().count();
            let center = x_coords[idx] - current_min_x + current_offset + width / 2;
            (idx, center)
        }).collect();

        let next_min_x = next_nodes.iter().map(|&idx| x_coords[idx]).min().unwrap_or(0);
        let next_centers: Vec<(usize, usize)> = next_nodes.iter().map(|&idx| {
            let (id, label) = self.nodes[idx];
            let width = self.format_node(id, label).chars().count();
            let center = x_coords[idx] - next_min_x + next_offset + width / 2;
            (idx, center)
        }).collect();

        // Find connections
        let mut connections: Vec<(usize, usize)> = Vec::new();
        for &(curr_idx, from_pos) in &current_centers {
            let node_id = self.nodes[curr_idx].0;
            for child_id in self.get_children(node_id) {
                if let Some(&(_, to_pos)) = next_centers.iter().find(|(idx, _)| self.nodes[*idx].0 == child_id) {
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
        let max_pos = connections.iter().flat_map(|(f, t)| [*f, *t]).max().unwrap_or(0);

        // Draw based on pattern
        if has_convergence && !has_divergence {
            self.draw_convergence_manhattan(output, &target_groups, min_pos, max_pos);
        } else if has_divergence && !has_convergence {
            self.draw_divergence_manhattan(output, &source_groups, min_pos, max_pos);
        } else {
            self.draw_simple_manhattan(output, &connections, min_pos, max_pos);
        }
    }

    fn draw_convergence_manhattan(&self, output: &mut String, target_groups: &[(usize, Vec<usize>)], min_pos: usize, max_pos: usize) {
        let all_sources: Vec<usize> = target_groups.iter().flat_map(|(_, sources)| sources.iter().copied()).collect();

        // Line 1: Vertical drops
        for i in min_pos..=max_pos {
            output.push(if all_sources.contains(&i) { V_LINE } else { ' ' });
        }
        writeln!(output).ok();

        // Line 2: Horizontal convergence └──┴──┘
        for i in min_pos..=max_pos {
            let mut ch = ' ';
            for (_, sources) in target_groups.iter() {
                if sources.len() <= 1 { continue; }
                let min_src = *sources.iter().min().unwrap();
                let max_src = *sources.iter().max().unwrap();
                if i == min_src { ch = CORNER_DR; }
                else if i == max_src { ch = CORNER_DL; }
                else if sources.contains(&i) { ch = TEE_UP; }
                else if i > min_src && i < max_src { ch = H_LINE; }
            }
            output.push(ch);
        }
        writeln!(output).ok();

        // Line 3: Arrows down
        for i in min_pos..=max_pos {
            output.push(if target_groups.iter().any(|(t, _)| *t == i) { ARROW_DOWN } else { ' ' });
        }
        writeln!(output).ok();
    }

    fn draw_divergence_manhattan(&self, output: &mut String, source_groups: &[(usize, Vec<usize>)], min_pos: usize, max_pos: usize) {
        let all_sources: Vec<usize> = source_groups.iter().map(|(s, _)| *s).collect();

        // Line 1: Vertical from sources
        for i in min_pos..=max_pos {
            output.push(if all_sources.contains(&i) { V_LINE } else { ' ' });
        }
        writeln!(output).ok();

        // Line 2: Horizontal divergence ┌──┬──┐
        for i in min_pos..=max_pos {
            let mut ch = ' ';
            for (_, targets) in source_groups.iter() {
                if targets.len() <= 1 { continue; }
                let min_tgt = *targets.iter().min().unwrap();
                let max_tgt = *targets.iter().max().unwrap();
                if i == min_tgt { ch = CORNER_UR; }
                else if i == max_tgt { ch = CORNER_UL; }
                else if targets.contains(&i) { ch = TEE_DOWN; }
                else if i > min_tgt && i < max_tgt { ch = H_LINE; }
            }
            output.push(ch);
        }
        writeln!(output).ok();

        // Line 3: Arrows down
        let all_targets: Vec<usize> = source_groups.iter().flat_map(|(_, t)| t.iter().copied()).collect();
        for i in min_pos..=max_pos {
            output.push(if all_targets.contains(&i) { ARROW_DOWN } else { ' ' });
        }
        writeln!(output).ok();
    }

    fn draw_simple_manhattan(&self, output: &mut String, connections: &[(usize, usize)], min_pos: usize, max_pos: usize) {
        // Line 1: Vertical
        for i in min_pos..=max_pos {
            output.push(if connections.iter().any(|(f, _)| *f == i) { V_LINE } else { ' ' });
        }
        writeln!(output).ok();

        // Line 2: Arrows
        for i in min_pos..=max_pos {
            output.push(if connections.iter().any(|(f, _)| *f == i) { ARROW_DOWN } else { ' ' });
        }
        writeln!(output).ok();
    }

    /// Find disconnected subgraphs
    fn find_subgraphs(&self) -> Vec<Vec<usize>> {
        let mut visited = vec![false; self.nodes.len()];
        let mut subgraphs = Vec::new();

        for i in 0..self.nodes.len() {
            if !visited[i] {
                let mut subgraph = Vec::new();
                self.collect_connected(i, &mut visited, &mut subgraph);
                subgraphs.push(subgraph);
            }
        }

        subgraphs
    }

    /// Collect all nodes connected to the given node
    fn collect_connected(&self, start_idx: usize, visited: &mut [bool], subgraph: &mut Vec<usize>) {
        if visited[start_idx] {
            return;
        }

        visited[start_idx] = true;
        subgraph.push(start_idx);

        let node_id = self.nodes[start_idx].0;

        // Follow edges in both directions
        for &(from, to) in &self.edges {
            if from == node_id {
                // O(1) HashMap lookup instead of O(n) scan
                if let Some(child_idx) = self.node_index(to) {
                    self.collect_connected(child_idx, visited, subgraph);
                }
            }
            if to == node_id {
                // O(1) HashMap lookup instead of O(n) scan
                if let Some(parent_idx) = self.node_index(from) {
                    self.collect_connected(parent_idx, visited, subgraph);
                }
            }
        }
    }

    /// Render a specific subgraph
    fn render_subgraph(&self, output: &mut String, subgraph_indices: &[usize]) {
        // Build a mini-DAG with just these nodes
        let _subgraph_node_ids: Vec<usize> = subgraph_indices.iter()
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
            let roots: Vec<_> = subgraph_indices.iter()
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
                    
                    if let Some(&(id, label)) = self.nodes.iter().find(|(nid, _)| *nid == current_id) {
                        output.push_str(&self.format_node(id, label));
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
                output.push_str(&self.format_node(id, label));

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

    fn is_subgraph_simple_chain(&self, subgraph_indices: &[usize]) -> bool {
        for &idx in subgraph_indices {
            let node_id = self.nodes[idx].0;
            let parents = self.get_parents(node_id);
            let children = self.get_children(node_id);
            
            if parents.len() > 1 || children.len() > 1 {
                return false;
            }
        }
        true
    }

    fn calculate_levels_for_subgraph(&self, subgraph_indices: &[usize]) -> Vec<(usize, usize)> {
        let subgraph_node_ids: Vec<usize> = subgraph_indices.iter()
            .map(|&idx| self.nodes[idx].0)
            .collect();

        let mut levels = vec![0usize; self.nodes.len()];
        let mut changed = true;

        while changed {
            changed = false;
            for &(from, to) in &self.edges {
                // Only process edges within this subgraph
                if !subgraph_node_ids.contains(&from) || !subgraph_node_ids.contains(&to) {
                    continue;
                }

                // Guard against missing nodes - O(1) HashMap lookups
                if let Some(from_idx) = self.node_index(from) {
                    if let Some(to_idx) = self.node_index(to) {
                        let new_level = levels[from_idx] + 1;
                        if new_level > levels[to_idx] {
                            levels[to_idx] = new_level;
                            changed = true;
                        }
                    }
                }
            }
        }

        subgraph_indices.iter().map(|&idx| (idx, levels[idx])).collect()
    }

    fn draw_vertical_connections(&self, output: &mut String, current_nodes: &[usize], next_nodes: &[usize]) {
        if current_nodes.is_empty() || next_nodes.is_empty() {
            return;
        }

        // Calculate center positions for each node in current level
        let mut current_positions = Vec::new();
        let mut pos = 0;
        for &idx in current_nodes {
            let (id, label) = self.nodes[idx];
            let formatted = self.format_node(id, label);
            let label_len = formatted.chars().count(); // Use actual formatted length
            let center = pos + label_len / 2;
            current_positions.push((idx, center, pos, pos + label_len));
            pos += label_len + 3; // +3 for spacing
        }

        // Calculate center positions for each node in next level
        let mut next_positions = Vec::new();
        let mut pos = 0;
        for &idx in next_nodes {
            let (id, label) = self.nodes[idx];
            let formatted = self.format_node(id, label);
            let label_len = formatted.chars().count(); // Use actual formatted length
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
                if let Some(&(_, to_pos)) = next_positions.iter().find(|(idx, _)| {
                    self.nodes[*idx].0 == child_id
                }) {
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
        target_groups: &[(usize, Vec<(usize, usize, usize)>)]
    ) {
        // Find all unique source and target positions
        let all_connections: Vec<_> = target_groups.iter()
            .flat_map(|(_, v)| v.iter().copied())
            .collect();
        let min_pos = all_connections.iter()
            .map(|(_, from, to)| (*from).min(*to))
            .min()
            .unwrap_or(0);
        let max_pos = all_connections.iter()
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
        source_groups: &[(usize, Vec<(usize, usize, usize)>)]
    ) {
        let all_connections: Vec<_> = source_groups.iter()
            .flat_map(|(_, v)| v.iter().copied())
            .collect();
        let min_pos = all_connections.iter()
            .map(|(_, from, to)| (*from).min(*to))
            .min()
            .unwrap_or(0);
        let max_pos = all_connections.iter()
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
        let max_pos = connections.iter()
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

    fn calculate_levels(&self) -> Vec<(usize, usize)> {
        let mut levels = vec![0usize; self.nodes.len()];
        let mut changed = true;

        while changed {
            changed = false;
            for &(from, to) in &self.edges {
                // Guard against missing nodes - O(1) HashMap lookups
                if let Some(from_idx) = self.node_index(from) {
                    if let Some(to_idx) = self.node_index(to) {
                        let new_level = levels[from_idx] + 1;
                        if new_level > levels[to_idx] {
                            levels[to_idx] = new_level;
                            changed = true;
                        }
                    }
                }
            }
        }

        levels.into_iter().enumerate().collect()
    }

    fn get_children(&self, node_id: usize) -> Vec<usize> {
        self.edges
            .iter()
            .filter(|(from, _)| *from == node_id)
            .map(|(_, to)| *to)
            .collect()
    }

    fn get_parents(&self, node_id: usize) -> Vec<usize> {
        self.edges
            .iter()
            .filter(|(_, to)| *to == node_id)
            .map(|(from, _)| *from)
            .collect()
    }

    /// Get node index from ID using O(1) HashMap lookup
    #[inline]
    fn node_index(&self, id: usize) -> Option<usize> {
        self.id_to_index.get(&id).copied()
    }

    /// Get cached width for a node index
    #[inline]
    fn get_node_width(&self, idx: usize) -> usize {
        self.node_widths.get(idx).copied().unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_dag() {
        let dag = DAG::new();
        assert_eq!(dag.render(), "Empty DAG");
    }

    #[test]
    fn test_simple_chain() {
        let dag = DAG::from_edges(
            &[(1, "A"), (2, "B"), (3, "C")],
            &[(1, 2), (2, 3)],
        );
        
        let output = dag.render();
        assert!(output.contains("A"));
        assert!(output.contains("B"));
        assert!(output.contains("C"));
    }

    #[test]
    fn test_cycle_detection() {
        let mut dag = DAG::new();
        dag.add_node(1, "A");
        dag.add_node(2, "B");
        dag.add_edge(1, 2);
        dag.add_edge(2, 1); // Cycle!

        assert!(dag.has_cycle());
    }

    #[test]
    fn test_no_cycle() {
        let dag = DAG::from_edges(
            &[(1, "A"), (2, "B")],
            &[(1, 2)],
        );

        assert!(!dag.has_cycle());
    }

    #[test]
    fn test_diamond() {
        let dag = DAG::from_edges(
            &[(1, "A"), (2, "B"), (3, "C"), (4, "D")],
            &[(1, 2), (1, 3), (2, 4), (3, 4)],
        );

        assert!(!dag.has_cycle());
        let output = dag.render();
        assert!(output.contains("A"));
        assert!(output.contains("D"));
    }

    #[test]
    fn test_auto_created_nodes() {
        let mut dag = DAG::new();
        dag.add_node(1, "A");
        dag.add_edge(1, 2);  // Auto-creates node 2
        dag.add_node(3, "C");
        dag.add_edge(2, 3);
        
        let output = dag.render();
        
        // Normal nodes have square brackets
        assert!(output.contains("[A]"));
        assert!(output.contains("[C]"));
        
        // Auto-created node has angle brackets
        assert!(output.contains("⟨2⟩"));
        
        // Verify auto_created tracking
        assert!(dag.is_auto_created(2));
        assert!(!dag.is_auto_created(1));
        assert!(!dag.is_auto_created(3));
    }

    #[test]
    fn test_no_auto_creation_when_explicit() {
        let mut dag = DAG::new();
        dag.add_node(1, "A");
        dag.add_node(2, "B");  // Explicit!
        dag.add_edge(1, 2);
        
        let output = dag.render();
        
        // Both should be square brackets
        assert!(output.contains("[A]"));
        assert!(output.contains("[B]"));
        assert!(!output.contains("⟨"));  // No angle brackets
        
        // Verify nothing was auto-created
        assert!(!dag.is_auto_created(1));
        assert!(!dag.is_auto_created(2));
    }

    #[test]
    fn test_edge_to_missing_node_no_panic() {
        let mut dag = DAG::new();
        dag.add_node(1, "A");
        dag.add_edge(1, 2);  // Node 2 doesn't exist - should auto-create
        
        // Should NOT panic
        let output = dag.render();
        
        // Should render successfully
        assert!(output.contains("[A]"));
        assert!(output.contains("⟨2⟩"));
    }

    #[test]
    fn test_cross_level_edges() {
        let mut dag = DAG::new();
        
        dag.add_node(1, "Root");
        dag.add_node(2, "Middle");
        dag.add_node(3, "End");
        
        dag.add_edge(1, 2);
        dag.add_edge(1, 3);
        dag.add_edge(2, 3);
        
        let output = dag.render();
        
        assert!(output.contains("[Root]"));
        assert!(output.contains("[Middle]"));
        assert!(output.contains("[End]"));
    }

    #[test]
    fn test_crossing_reduction() {
        // Diamond graph to test that crossing reduction runs without panicking
        let mut dag = DAG::new();
        
        dag.add_node(1, "Top");
        dag.add_node(2, "Right");
        dag.add_node(3, "Left");
        dag.add_node(4, "Bottom");
        
        dag.add_edge(1, 3);
        dag.add_edge(1, 2);
        dag.add_edge(3, 4);
        dag.add_edge(2, 4);
        
        let output = dag.render();
        
        // All nodes should appear
        assert!(output.contains("[Top]"));
        assert!(output.contains("[Left]"));
        assert!(output.contains("[Right]"));
        assert!(output.contains("[Bottom]"));
        
        // The crossing reduction pass should complete without panic
        // and produce a valid rendering (nodes are reordered to minimize crossings)
        let lines: Vec<&str> = output.lines().collect();
        assert!(lines.len() >= 5, "Should have multiple lines for diamond pattern");
    }

    #[test]
    fn test_cycle_with_auto_created_nodes() {
        let mut dag = DAG::new();
        dag.add_node(1, "A");
        // Node 2 will be auto-created
        dag.add_edge(1, 2);
        dag.add_edge(2, 1); // Creates cycle
        
        let output = dag.render();
        
        // Should show cycle warning
        assert!(output.contains("CYCLE DETECTED"));
        
        // Auto-created node should use ⟨2⟩ format in cycle output
        assert!(output.contains("⟨2⟩"));
        
        // Normal node should use [A] format
        assert!(output.contains("[A]"));
    }

    #[test]
    fn test_auto_created_node_promotion() {
        let mut dag = DAG::new();
        
        dag.add_node(1, "A");
        dag.add_edge(1, 2);  // Auto-creates node 2 as placeholder
        
        // Verify initially auto-created
        assert!(dag.is_auto_created(2));
        let output = dag.render();
        assert!(output.contains("⟨2⟩"), "Before promotion, should show ⟨2⟩");
        assert!(!output.contains("[B]"), "Before promotion, should not show [B]");
        
        // Now promote the placeholder
        dag.add_node(2, "B");
        
        // Verify promotion worked
        assert!(!dag.is_auto_created(2), "After promotion, should not be auto-created");
        let output_after = dag.render();
        assert!(output_after.contains("[B]"), "After promotion, should show [B]");
        assert!(!output_after.contains("⟨2⟩"), "After promotion, should not show ⟨2⟩");
        
        // Verify no duplicate nodes were created
        let node_count = dag.nodes.iter().filter(|(id, _)| *id == 2).count();
        assert_eq!(node_count, 1, "Should only have one node with id=2");
    }

    #[test]
    fn test_skewed_children_rendering_order() {
        // Test that nodes are rendered left-to-right by x-coordinate,
        // even when median centering moves nodes around.
        let mut dag = DAG::new();
        
        // Create a level with multiple nodes
        dag.add_node(1, "Top");
        dag.add_node(2, "A");
        dag.add_node(3, "B");
        dag.add_node(4, "C");
        
        // Top connects to all children
        dag.add_edge(1, 2);
        dag.add_edge(1, 3);
        dag.add_edge(1, 4);
        
        let output = dag.render();
        
        // All children should be on the same line
        let lines: Vec<&str> = output.lines().collect();
        let child_line = lines.iter()
            .find(|line| line.contains("[A]") && line.contains("[B]") && line.contains("[C]"))
            .expect("Should find line with all children");
        
        // Find positions of A, B, C on that line
        let a_pos = child_line.find("[A]").unwrap();
        let b_pos = child_line.find("[B]").unwrap();
        let c_pos = child_line.find("[C]").unwrap();
        
        // They should be in left-to-right order
        assert!(a_pos < b_pos, "A should be left of B");
        assert!(b_pos < c_pos, "B should be left of C");
    }
}
