//! Core DAG (Directed Acyclic Graph) data structure.
//!
//! This module provides the fundamental graph structure with nodes and edges.
//!
//! ## Performance Characteristics
//!
//! - **Node/Edge Insertion**: O(1) amortized with HashMap and cached adjacency lists
//! - **Child/Parent Lookups**: O(1) via cached adjacency lists (not O(E) iteration)
//! - **ID→Index Mapping**: O(1) via HashMap (not O(N) scan)
//! - **Node Width**: O(1) via pre-computed cache
//!
//! ## Memory Overhead
//!
//! Per node:
//! - ~100 bytes (node data, caches, adjacency list headers)
//!
//! Per edge:
//! - ~16 bytes (adjacency list entries, both directions)
//!
//! ## Security
//!
//! - No unsafe code
//! - For untrusted input, consider limiting maximum nodes/edges to prevent resource exhaustion
//! - Maximum node ID: `usize::MAX` (up to 20 decimal digits)

use alloc::{string::String, vec, vec::Vec};

#[cfg(feature = "std")]
use std::collections::{HashMap, HashSet};

#[cfg(not(feature = "std"))]
use alloc::collections::{BTreeMap as HashMap, BTreeSet as HashSet};

/// Rendering mode for the DAG visualization.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RenderMode {
    /// Render chains vertically (takes more vertical space)
    Vertical,

    /// Render chains horizontally when possible (compact, one-line for simple chains)
    Horizontal,

    /// Auto-detect: horizontal for simple chains, vertical for complex graphs
    #[default]
    Auto,
}

/// A Directed Acyclic Graph (DAG) with ASCII rendering capabilities.
///
/// # Examples
///
/// ```
/// use ascii_dag::graph::DAG;
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
#[derive(Clone, Default)]
pub struct DAG<'a> {
    pub(crate) nodes: Vec<(usize, &'a str)>,
    pub(crate) edges: Vec<(usize, usize)>,
    pub(crate) render_mode: RenderMode,
    pub(crate) auto_created: HashSet<usize>, // Track auto-created nodes for visual distinction (O(1) lookups)
    pub(crate) id_to_index: HashMap<usize, usize>, // Cache id→index mapping (O(1) lookups)
    pub(crate) node_widths: Vec<usize>,      // Cached formatted widths
    pub(crate) children: Vec<Vec<usize>>,    // Adjacency list: children[idx] = child indices
    pub(crate) parents: Vec<Vec<usize>>,     // Adjacency list: parents[idx] = parent indices
}

impl<'a> DAG<'a> {
    /// Create a new empty DAG.
    ///
    /// # Examples
    ///
    /// ```
    /// use ascii_dag::graph::DAG;
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
    /// use ascii_dag::graph::DAG;
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
            children: Vec::new(),
            parents: Vec::new(),
        };

        // Build id_to_index map and widths cache
        for (idx, &(id, label)) in dag.nodes.iter().enumerate() {
            dag.id_to_index.insert(id, idx);
            let width = dag.compute_node_width(id, label);
            dag.node_widths.push(width);
        }

        // Initialize adjacency lists
        dag.children.resize(dag.nodes.len(), Vec::new());
        dag.parents.resize(dag.nodes.len(), Vec::new());

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
    /// use ascii_dag::graph::{DAG, RenderMode};
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
    /// use ascii_dag::graph::{DAG, RenderMode};
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
            children: Vec::new(),
            parents: Vec::new(),
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
    /// use ascii_dag::graph::DAG;
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
            // Extend adjacency lists
            self.children.push(Vec::new());
            self.parents.push(Vec::new());
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
    /// use ascii_dag::graph::DAG;
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

        // Update adjacency lists (O(1) lookups)
        if let (Some(&from_idx), Some(&to_idx)) =
            (self.id_to_index.get(&from), self.id_to_index.get(&to))
        {
            self.children[from_idx].push(to_idx);
            self.parents[to_idx].push(from_idx);
        }
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
            self.auto_created.insert(id); // O(1) insert
            self.id_to_index.insert(id, idx); // O(1) insert
            let width = self.compute_node_width(id, "");
            self.node_widths.push(width);
            // Extend adjacency lists
            self.children.push(Vec::new());
            self.parents.push(Vec::new());
        }
    }

    /// Check if a node was auto-created (for visual distinction)
    pub(crate) fn is_auto_created(&self, id: usize) -> bool {
        self.auto_created.contains(&id) // O(1) with HashSet
    }

    /// Write an unsigned integer to a string buffer without allocation.
    /// This avoids format! bloat in no_std builds.
    #[inline]
    pub(crate) fn write_usize(buf: &mut String, mut n: usize) {
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
        if n == 0 {
            return 1;
        }
        let mut count = 0;
        while n > 0 {
            count += 1;
            n /= 10;
        }
        count
    }

    /// Compute the formatted width of a node
    pub(crate) fn compute_node_width(&self, id: usize, label: &str) -> usize {
        if label.is_empty() || self.is_auto_created(id) {
            // ⟨ID⟩ format
            2 + Self::count_digits(id) // ⟨ + digits + ⟩
        } else {
            // [Label] format
            2 + label.chars().count() // [ + label + ]
        }
    }

    /// Write a formatted node directly to output buffer (avoids intermediate String allocation)
    #[inline]
    pub(crate) fn write_node(&self, output: &mut String, id: usize, label: &str) {
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

    /// Get children of a node (returns IDs, not indices).
    /// Uses cached adjacency lists for O(1) lookup instead of O(E) iteration.
    /// NOTE: This allocates a new Vec. For hot paths, use `children_count` + `get_children_indices`.
    #[inline]
    pub(crate) fn get_children(&self, node_id: usize) -> Vec<usize> {
        if let Some(&idx) = self.id_to_index.get(&node_id) {
            // Convert child indices back to IDs
            self.children[idx]
                .iter()
                .map(|&child_idx| self.nodes[child_idx].0)
                .collect()
        } else {
            Vec::new()
        }
    }

    /// Get parents of a node (returns IDs, not indices).
    /// Uses cached adjacency lists for O(1) lookup instead of O(E) iteration.
    /// NOTE: This allocates a new Vec. For hot paths, use `parents_count` + `get_parents_indices`.
    #[inline]
    pub(crate) fn get_parents(&self, node_id: usize) -> Vec<usize> {
        if let Some(&idx) = self.id_to_index.get(&node_id) {
            // Convert parent indices back to IDs
            self.parents[idx]
                .iter()
                .map(|&parent_idx| self.nodes[parent_idx].0)
                .collect()
        } else {
            Vec::new()
        }
    }

    /// Count children of a node by index (zero-allocation).
    #[inline]
    pub(crate) fn children_count(&self, node_idx: usize) -> usize {
        self.children.get(node_idx).map_or(0, |c| c.len())
    }

    /// Count parents of a node by index (zero-allocation).
    #[inline]
    pub(crate) fn parents_count(&self, node_idx: usize) -> usize {
        self.parents.get(node_idx).map_or(0, |p| p.len())
    }

    /// Get children indices directly (no ID conversion) - faster for internal use.
    #[inline]
    pub(crate) fn get_children_indices(&self, node_idx: usize) -> &[usize] {
        &self.children[node_idx]
    }

    /// Get parent indices directly (no ID conversion) - faster for internal use.
    #[inline]
    pub(crate) fn get_parents_indices(&self, node_idx: usize) -> &[usize] {
        &self.parents[node_idx]
    }

    /// Get node index from ID using O(1) HashMap lookup
    #[inline]
    pub(crate) fn node_index(&self, id: usize) -> Option<usize> {
        self.id_to_index.get(&id).copied()
    }

    /// Get cached width for a node index
    #[inline]
    pub(crate) fn get_node_width(&self, idx: usize) -> usize {
        self.node_widths.get(idx).copied().unwrap_or(0)
    }

    /// Estimate the buffer size needed for rendering.
    ///
    /// Use this to pre-allocate a buffer for [`render_to`](Self::render_to).
    ///
    /// # Examples
    ///
    /// ```
    /// use ascii_dag::graph::DAG;
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
        // Estimate based on empirical measurements:
        // - Each level takes ~width characters (canvas can be very wide)
        // - Vertical layouts have many levels with connection lines
        // - For layered graphs with skip-edges, canvas can be quite wide
        //
        // Vertical layout: nodes spread across canvas width + connection lines
        // Each node level line + ~5 connection lines per level
        // Canvas width roughly: nodes_per_level * 15 chars
        // Height roughly: levels * 6 lines
        let n = self.nodes.len();
        let est_levels = n.isqrt().max(1);
        let est_width = (n / est_levels).max(1) * 15;
        let est_height = est_levels * 6;
        let base = est_width * est_height * 3; // UTF-8 chars average ~3 bytes
        base.max(n * 100) // Ensure minimum sensible estimate
    }

    /// Compute the layout intermediate representation for this DAG.
    ///
    /// This returns a renderer-agnostic representation of the laid-out graph
    /// that can be consumed by various renderers (ASCII, ANSI colors, SVG, etc.).
    ///
    /// # Examples
    ///
    /// ```
    /// use ascii_dag::DAG;
    ///
    /// let dag = DAG::from_edges(
    ///     &[(1, "A"), (2, "B"), (3, "C")],
    ///     &[(1, 2), (1, 3), (2, 3)]
    /// );
    ///
    /// let ir = dag.compute_layout();
    ///
    /// // Inspect layout
    /// println!("Width: {}, Height: {}", ir.width(), ir.height());
    /// for node in ir.nodes() {
    ///     println!("{} at ({}, {})", node.label, node.x, node.y);
    /// }
    /// ```
    pub fn compute_layout(&self) -> crate::ir::LayoutIR<'a> {
        use crate::ir::{EdgePath, LayoutEdge, LayoutIRBuilder, LayoutNode};

        if self.nodes.is_empty() {
            return LayoutIRBuilder::new().build();
        }

        // Check for cycles - can't layout cyclic graphs
        if self.has_cycle() {
            return LayoutIRBuilder::new().build();
        }

        // Step 1: Calculate levels
        let level_data = self.calculate_levels();
        let max_level = level_data.iter().map(|(_, l)| *l).max().unwrap_or(0);

        // Create level mapping
        let mut node_levels: Vec<usize> = vec![0; self.nodes.len()];
        for (idx, level) in &level_data {
            node_levels[*idx] = *level;
        }

        // Step 2: Group nodes by level and apply crossing reduction
        let mut levels: Vec<Vec<usize>> = vec![Vec::new(); max_level + 1];
        for (idx, level) in &level_data {
            levels[*level].push(*idx);
        }
        self.reduce_crossings(&mut levels, max_level);

        // Step 3: Assign x-coordinates
        let mut x_coords: Vec<Vec<usize>> = Vec::with_capacity(levels.len());
        let mut widths: Vec<Vec<usize>> = Vec::with_capacity(levels.len());

        for level_nodes in &levels {
            let mut level_x = Vec::with_capacity(level_nodes.len());
            let mut level_w = Vec::with_capacity(level_nodes.len());
            let mut x = 0;

            for &idx in level_nodes {
                let width = self.get_node_width(idx);
                level_x.push(x);
                level_w.push(width);
                x += width + 3; // Standard spacing
            }

            x_coords.push(level_x);
            widths.push(level_w);
        }

        // Calculate total width and centering offsets
        let level_widths: Vec<usize> = x_coords
            .iter()
            .zip(widths.iter())
            .map(|(xs, ws)| {
                xs.iter()
                    .zip(ws.iter())
                    .map(|(x, w)| x + w)
                    .max()
                    .unwrap_or(0)
            })
            .collect();

        let max_width = *level_widths.iter().max().unwrap_or(&0);

        // Step 4: Build LayoutIR
        let mut builder = LayoutIRBuilder::new().with_levels(max_level + 1);

        // Lines per level: node line + connector lines (arrows, horizontal bars, etc.)
        // Rough estimate: 3 lines per level (node, connector start, connector end)
        let lines_per_level = 3;
        let total_height = (max_level + 1) * lines_per_level;

        // Add nodes with centered positions
        for (level_idx, level_nodes) in levels.iter().enumerate() {
            let level_width = level_widths[level_idx];
            let level_offset = if max_width > level_width {
                (max_width - level_width) / 2
            } else {
                0
            };

            for (pos, &idx) in level_nodes.iter().enumerate() {
                let (id, label) = self.nodes[idx];
                let x = x_coords[level_idx][pos] + level_offset;
                let width = widths[level_idx][pos];
                let y = level_idx * lines_per_level;

                builder.add_node(LayoutNode {
                    id,
                    label,
                    x,
                    y,
                    width,
                    center_x: x + width / 2,
                    level: level_idx,
                    level_position: pos,
                });
            }
        }

        // Add edges with routing info
        // Track max channel_x for width calculation
        let mut max_channel_x = max_width;
        
        for (edge_idx, &(from_id, to_id)) in self.edges.iter().enumerate() {
            if let (Some(from_idx), Some(to_idx)) =
                (self.node_index(from_id), self.node_index(to_id))
            {
                let from_level = node_levels[from_idx];
                let to_level = node_levels[to_idx];

                // Find positions in their levels
                let from_pos = levels[from_level].iter().position(|&i| i == from_idx);
                let to_pos = levels[to_level].iter().position(|&i| i == to_idx);

                if let (Some(fp), Some(tp)) = (from_pos, to_pos) {
                    let from_level_offset = if max_width > level_widths[from_level] {
                        (max_width - level_widths[from_level]) / 2
                    } else {
                        0
                    };
                    let to_level_offset = if max_width > level_widths[to_level] {
                        (max_width - level_widths[to_level]) / 2
                    } else {
                        0
                    };

                    let from_x =
                        x_coords[from_level][fp] + from_level_offset + widths[from_level][fp] / 2;
                    let to_x = x_coords[to_level][tp] + to_level_offset + widths[to_level][tp] / 2;
                    let from_y = from_level * lines_per_level;
                    let to_y = to_level * lines_per_level;

                    let path = if to_level == from_level + 1 {
                        // Adjacent levels - direct or corner connection
                        if from_x == to_x {
                            EdgePath::Direct
                        } else {
                            EdgePath::Corner {
                                horizontal_y: from_y + 1,
                            }
                        }
                    } else {
                        // Skip-level edge - side channel routing
                        // Channel must be to the RIGHT of all nodes at ALL intermediate levels
                        let mut max_right = from_x.max(to_x);
                        for level in from_level..=to_level {
                            // Get rightmost node edge at this level (after centering)
                            let level_offset = if max_width > level_widths[level] {
                                (max_width - level_widths[level]) / 2
                            } else {
                                0
                            };
                            let right_edge = level_widths[level] + level_offset;
                            max_right = max_right.max(right_edge);
                        }
                        let channel_x = max_right + 2; // 2 chars spacing from rightmost node
                        max_channel_x = max_channel_x.max(channel_x + 1);
                        EdgePath::SideChannel {
                            channel_x,
                            start_y: from_y + 1,
                            end_y: to_y - 1,
                        }
                    };

                    builder.add_edge(LayoutEdge {
                        from_id,
                        to_id,
                        from_x,
                        from_y,
                        to_x,
                        to_y,
                        path,
                        edge_index: edge_idx,
                    });
                }
            }
        }

        builder.set_dimensions(max_channel_x, total_height);
        builder.build()
    }
}
