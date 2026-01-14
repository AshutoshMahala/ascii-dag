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
#[derive(Clone)]
pub struct DAG<'a> {
    pub(crate) nodes: Vec<(usize, &'a str)>,
    pub(crate) edges: Vec<(usize, usize)>,
    pub(crate) render_mode: RenderMode,
    pub(crate) auto_created: HashSet<usize>, // Track auto-created nodes for visual distinction (O(1) lookups)
    pub(crate) id_to_index: HashMap<usize, usize>, // Cache id→index mapping (O(1) lookups)
    pub(crate) node_widths: Vec<usize>,      // Cached formatted widths
    pub(crate) children: Vec<Vec<usize>>,    // Adjacency list: children[idx] = child indices
    pub(crate) parents: Vec<Vec<usize>>,     // Adjacency list: parents[idx] = parent indices
    pub(crate) crossing_reduction_passes: usize, // Number of passes for crossing reduction algorithm
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
            children: Vec::new(),
            parents: Vec::new(),
            crossing_reduction_passes: 4, // Default to 4 passes as per original implementation
        }
    }
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
            crossing_reduction_passes: 4,
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

    /// Set the number of passes for the crossing reduction algorithm.
    ///
    /// - `0`: Skip crossing reduction entirely (fastest, useful for debugging or simple graphs)
    /// - `1-4`: Good for most graphs (default is 4)
    /// - `8-10`: Better layouts for complex tangled graphs, but slower
    ///
    /// Values > 20 will trigger a warning (diminishing returns).
    /// Values > 1000 are clamped to 0 with a warning (likely accidental).
    pub fn set_crossing_reduction_passes(&mut self, passes: usize) {
        self.crossing_reduction_passes = Self::validate_passes(passes);
    }

    /// Validate crossing reduction passes, returning a safe value.
    /// - Values > 1000 are treated as accidental (e.g., -1i32 as usize) and clamped to 0
    /// - Values > 20 trigger a warning about diminishing returns
    #[inline]
    fn validate_passes(passes: usize) -> usize {
        if passes > 1000 {
            // Likely accidental: -1 as usize wraps to usize::MAX
            #[cfg(feature = "std")]
            eprintln!(
                "[ascii-dag] Warning: crossing_reduction_passes={} is unreasonably large (possibly from negative value). Clamping to 0.",
                passes
            );
            0
        } else if passes > 20 {
            #[cfg(feature = "std")]
            eprintln!(
                "[ascii-dag] Warning: crossing_reduction_passes={} is high. Values >20 have diminishing returns and may be slow.",
                passes
            );
            passes
        } else {
            passes
        }
    }

    /// Builder method: set render mode (chainable).
    ///
    /// # Examples
    ///
    /// ```
    /// use ascii_dag::graph::{DAG, RenderMode};
    ///
    /// let dag = DAG::new()
    ///     .with_render_mode(RenderMode::Horizontal)
    ///     .with_crossing_reduction_passes(6);
    /// ```
    pub fn with_render_mode(mut self, mode: RenderMode) -> Self {
        self.render_mode = mode;
        self
    }

    /// Builder method: set crossing reduction passes (chainable).
    ///
    /// See [`set_crossing_reduction_passes`](Self::set_crossing_reduction_passes) for valid ranges.
    ///
    /// # Examples
    ///
    /// ```
    /// use ascii_dag::DAG;
    ///
    /// let dag = DAG::new()
    ///     .with_crossing_reduction_passes(8);  // More passes for complex graphs
    /// ```
    pub fn with_crossing_reduction_passes(mut self, passes: usize) -> Self {
        self.crossing_reduction_passes = Self::validate_passes(passes);
        self
    }

    /// Create a DAG with a specific render mode.
    ///
    /// **Deprecated**: Prefer `DAG::new().with_render_mode(mode)` for consistency.
    ///
    /// # Examples
    ///
    /// ```
    /// use ascii_dag::graph::{DAG, RenderMode};
    ///
    /// let dag = DAG::with_mode(RenderMode::Horizontal);
    /// ```
    pub fn with_mode(mode: RenderMode) -> Self {
        Self::new().with_render_mode(mode)
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

    /// Add a node with an explicit width override.
    ///
    /// This is useful for pixel-based renderers where node width is determined
    /// by rendered content (HTML elements, custom graphics) rather than label length.
    ///
    /// # Arguments
    ///
    /// * `id` - Unique node identifier
    /// * `label` - Node label text (used for display, not for width calculation)
    /// * `width` - Explicit width in layout units (character cells for ASCII, 
    ///             or arbitrary units that will be scaled by the renderer)
    ///
    /// # Examples
    ///
    /// ```
    /// use ascii_dag::graph::DAG;
    ///
    /// let mut dag = DAG::new();
    /// // Node with explicit width of 20 units (ignores label length)
    /// dag.add_node_with_width(1, "Short", 20);
    /// ```
    pub fn add_node_with_width(&mut self, id: usize, label: &'a str, width: usize) {
        // Check if node already exists (could be auto-created) - O(1) with HashMap
        if let Some(&idx) = self.id_to_index.get(&id) {
            // Promote auto-created node to explicit node
            self.nodes[idx] = (id, label);
            // Remove from auto_created set - O(1)
            self.auto_created.remove(&id);
            // Use explicit width instead of computed
            self.node_widths[idx] = width;
        } else {
            // Brand new node
            let idx = self.nodes.len();
            self.nodes.push((id, label));
            self.id_to_index.insert(id, idx);
            // Use explicit width instead of computed
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

        // Step 1: Calculate levels for real nodes
        let level_data = self.calculate_levels();
        let max_level = level_data.iter().map(|(_, l)| *l).max().unwrap_or(0);

        // Create level mapping for real nodes
        let mut node_levels: Vec<usize> = vec![0; self.nodes.len()];
        for (idx, level) in &level_data {
            node_levels[*idx] = *level;
        }

        // Step 2: Build virtual levels with dummy nodes for skip-level edges
        // Using the external VNode type that implements VNodeTrait
        let mut virtual_levels: Vec<Vec<VNode>> = vec![Vec::new(); max_level + 1];
        
        // Add real nodes to their levels
        for (idx, level) in &level_data {
            virtual_levels[*level].push(VNode::Real(*idx));
        }
        
        // Identify skip-level edges and insert dummy nodes
        for (edge_idx, &(from_id, to_id)) in self.edges.iter().enumerate() {
            if let (Some(from_idx), Some(to_idx)) = (self.node_index(from_id), self.node_index(to_id)) {
                let from_level = node_levels[from_idx];
                let to_level = node_levels[to_idx];
                
                if to_level > from_level + 1 {
                    // Skip-level edge - insert dummy nodes at intermediate levels
                    for level in (from_level + 1)..to_level {
                        virtual_levels[level].push(VNode::Dummy { edge_idx });
                    }
                }
            }
        }

        // Step 3: Apply crossing reduction WITH dummy nodes included
        // We need to run barycenter on the virtual levels
        // Convert to a format suitable for median ordering
        self.reduce_crossings_virtual(&mut virtual_levels, &node_levels, max_level);

        // Step 4: Assign x-coordinates to virtual nodes
        let mut x_coords: Vec<Vec<usize>> = Vec::with_capacity(virtual_levels.len());
        let mut widths: Vec<Vec<usize>> = Vec::with_capacity(virtual_levels.len());

        for level_vnodes in &virtual_levels {
            let mut level_x = Vec::with_capacity(level_vnodes.len());
            let mut level_w = Vec::with_capacity(level_vnodes.len());
            let mut x = 0;

            for vnode in level_vnodes {
                let width = match vnode {
                    VNode::Real(idx) => self.get_node_width(*idx),
                    VNode::Dummy { .. } => 1, // Dummy nodes are minimal width
                };
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

        // Step 5: Build LayoutIR
        let mut builder = LayoutIRBuilder::new().with_levels(max_level + 1);

        let lines_per_level = 3;
        let total_height = (max_level + 1) * lines_per_level;

        // Build lookup: for each real node, find its (level, position, x, width)
        let mut real_node_coords: Vec<(usize, usize, usize, usize)> = vec![(0, 0, 0, 0); self.nodes.len()];
        
        // Add real nodes to IR and build coordinate lookup
        for (level_idx, level_vnodes) in virtual_levels.iter().enumerate() {
            let level_width = level_widths[level_idx];
            let level_offset = if max_width > level_width {
                (max_width - level_width) / 2
            } else {
                0
            };

            for (pos, vnode) in level_vnodes.iter().enumerate() {
                if let VNode::Real(idx) = vnode {
                    let (id, label) = self.nodes[*idx];
                    let x = x_coords[level_idx][pos] + level_offset;
                    let width = widths[level_idx][pos];
                    let y = level_idx * lines_per_level;

                    real_node_coords[*idx] = (level_idx, pos, x, width);

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
        }

        // Build lookup for dummy node positions: edge_idx -> Vec<(level, x)>
        // Dummy nodes should be positioned along the line between source and target,
        // NOT based on their position in the level ordering.
        let mut dummy_positions: Vec<Vec<(usize, usize)>> = vec![Vec::new(); self.edges.len()];
        
        for (edge_idx, &(from_id, to_id)) in self.edges.iter().enumerate() {
            if let (Some(from_idx), Some(to_idx)) = (self.node_index(from_id), self.node_index(to_id)) {
                let from_level = node_levels[from_idx];
                let to_level = node_levels[to_idx];
                
                if to_level > from_level + 1 {
                    // Skip-level edge - compute dummy positions by interpolation
                    let (_, _, from_x_base, from_width) = real_node_coords[from_idx];
                    let (_, _, to_x_base, to_width) = real_node_coords[to_idx];
                    
                    let from_center = from_x_base + from_width / 2;
                    let to_center = to_x_base + to_width / 2;
                    
                    let total_span = to_level - from_level;
                    
                    for level in (from_level + 1)..to_level {
                        // Interpolate x position between source and target centers
                        // Use integer rounding: (value + 0.5) as usize, but in integer math
                        let t_num = level - from_level;
                        let t_denom = total_span;
                        // Compute: from_center * (1 - t) + to_center * t
                        // = from_center + (to_center - from_center) * t
                        // Use integer arithmetic with rounding
                        let delta = to_center as isize - from_center as isize;
                        let x = (from_center as isize + (delta * t_num as isize + t_denom as isize / 2) / t_denom as isize) as usize;
                        dummy_positions[edge_idx].push((level, x));
                    }
                }
            }
        }

        // Step 6: Add edges with proper routing
        for (edge_idx, &(from_id, to_id)) in self.edges.iter().enumerate() {
            if let (Some(from_idx), Some(to_idx)) = (self.node_index(from_id), self.node_index(to_id)) {
                let from_level = node_levels[from_idx];
                let to_level = node_levels[to_idx];
                
                let (_, _, from_x_base, from_width) = real_node_coords[from_idx];
                let (_, _, to_x_base, to_width) = real_node_coords[to_idx];
                
                let from_x = from_x_base + from_width / 2;
                let to_x = to_x_base + to_width / 2;
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
                    // Skip-level edge - use dummy node positions for MultiSegment path
                    let dummies = &dummy_positions[edge_idx];
                    if dummies.is_empty() {
                        // Fallback to corner if no dummies (shouldn't happen)
                        EdgePath::Corner { horizontal_y: from_y + 1 }
                    } else {
                        // Build waypoints through dummy nodes
                        let mut waypoints = Vec::with_capacity(dummies.len());
                        for &(level, x) in dummies {
                            waypoints.push((x, level * lines_per_level));
                        }
                        EdgePath::MultiSegment { waypoints }
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

        builder.set_dimensions(max_width, total_height);
        builder.build()
    }
    
    /// Crossing reduction for virtual levels (includes dummy nodes).
    /// Uses median heuristic with both real and dummy nodes participating.
    fn reduce_crossings_virtual(
        &self,
        levels: &mut [Vec<VNode>],
        _node_levels: &[usize],
        max_level: usize,
    ) {
        // Pre-allocate reusable buffers to avoid allocations in the hot loop
        // Estimate max level size for capacity hints
        let max_level_size = levels.iter().map(|l| l.len()).max().unwrap_or(0);
        
        // Reusable lookup tables (use new() for BTreeMap compatibility in no_std)
        let mut real_pos: HashMap<usize, usize> = HashMap::new();
        let mut dummy_pos: HashMap<usize, usize> = HashMap::new();
        
        // Reusable buffers for median computation
        let mut node_medians: Vec<(VNode, f32)> = Vec::with_capacity(max_level_size);
        let mut connected_positions: Vec<usize> = Vec::with_capacity(8); // Most nodes have few connections
        
        // Run multiple passes of median heuristic
        for _ in 0..self.crossing_reduction_passes {
            // Top-down pass
            for level_idx in 1..=max_level {
                let (prev_levels, rest) = levels.split_at_mut(level_idx);
                let parent_level = &prev_levels[level_idx - 1];
                self.order_virtual_by_median_reuse(
                    &mut rest[0], 
                    parent_level, 
                    true,
                    &mut real_pos,
                    &mut dummy_pos,
                    &mut node_medians,
                    &mut connected_positions,
                );
            }

            // Bottom-up pass
            for level_idx in (0..max_level).rev() {
                let (left, right) = levels.split_at_mut(level_idx + 1);
                let child_level = &right[0];
                self.order_virtual_by_median_reuse(
                    &mut left[level_idx], 
                    child_level, 
                    false,
                    &mut real_pos,
                    &mut dummy_pos,
                    &mut node_medians,
                    &mut connected_positions,
                );
            }
        }
    }

    /// Order virtual nodes by median position - version that reuses pre-allocated buffers.
    fn order_virtual_by_median_reuse(
        &self,
        level_nodes: &mut Vec<VNode>,
        adj_level: &[VNode],
        use_parents: bool,
        real_pos: &mut HashMap<usize, usize>,
        dummy_pos: &mut HashMap<usize, usize>,
        node_medians: &mut Vec<(VNode, f32)>,
        connected_positions: &mut Vec<usize>,
    ) {
        // Clear and rebuild lookup tables (reuses allocated capacity)
        real_pos.clear();
        dummy_pos.clear();
        
        for (pos, vnode) in adj_level.iter().enumerate() {
            match vnode {
                VNode::Real(idx) => { real_pos.insert(*idx, pos); }
                VNode::Dummy { edge_idx } => { dummy_pos.insert(*edge_idx, pos); }
            }
        }
        
        // Clear and rebuild medians (reuses allocated capacity)
        node_medians.clear();

        for (pos, &vnode) in level_nodes.iter().enumerate() {
            // Clear and reuse connected_positions
            connected_positions.clear();
            
            match (vnode.real_index(), vnode.dummy_edge()) {
                (Some(idx), _) => {
                    // Real node - find connected real nodes in adjacent level
                    let connected_indices = if use_parents {
                        self.get_parents_indices(idx)
                    } else {
                        self.get_children_indices(idx)
                    };
                    
                    // O(1) lookup per connected node
                    for &conn_idx in connected_indices {
                        if let Some(&p) = real_pos.get(&conn_idx) {
                            connected_positions.push(p);
                        }
                    }
                }
                (_, Some(edge_idx)) => {
                    // Dummy node - find the connected node or dummy for this edge
                    let &(from_id, to_id) = &self.edges[edge_idx];
                    let from_idx = self.node_index(from_id);
                    let to_idx = self.node_index(to_id);
                    
                    // Check for same edge's dummy in adjacent level
                    if let Some(&dpos) = dummy_pos.get(&edge_idx) {
                        connected_positions.push(dpos);
                    }
                    
                    // Check for real endpoint in adjacent level
                    if use_parents {
                        if let Some(fidx) = from_idx {
                            if let Some(&rpos) = real_pos.get(&fidx) {
                                connected_positions.push(rpos);
                            }
                        }
                    } else {
                        if let Some(tidx) = to_idx {
                            if let Some(&rpos) = real_pos.get(&tidx) {
                                connected_positions.push(rpos);
                            }
                        }
                    }
                }
                _ => {}
            };

            let median = if connected_positions.is_empty() {
                pos as f32
            } else {
                connected_positions.sort_unstable();
                if connected_positions.len() % 2 == 1 {
                    connected_positions[connected_positions.len() / 2] as f32
                } else {
                    let mid = connected_positions.len() / 2;
                    (connected_positions[mid - 1] + connected_positions[mid]) as f32 / 2.0
                }
            };

            node_medians.push((vnode, median));
        }

        node_medians.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
        
        // Rebuild level_nodes from sorted medians
        level_nodes.clear();
        for (v, _) in node_medians.iter() {
            level_nodes.push(*v);
        }
    }

    /// Order virtual nodes by median position of connected nodes in adjacent level.
    /// Legacy version - kept for API compatibility but internally uses the optimized version.
    #[allow(dead_code)]
    fn order_virtual_by_median(
        &self,
        level_nodes: &mut Vec<VNode>,
        adj_level: &[VNode],
        use_parents: bool,
    ) {
        // Create temporary buffers (less efficient, but maintains API)
        let mut real_pos = HashMap::new();
        let mut dummy_pos = HashMap::new();
        let mut node_medians = Vec::with_capacity(level_nodes.len());
        let mut connected_positions = Vec::with_capacity(8);
        
        self.order_virtual_by_median_reuse(
            level_nodes,
            adj_level,
            use_parents,
            &mut real_pos,
            &mut dummy_pos,
            &mut node_medians,
            &mut connected_positions,
        );
    }
}

/// Virtual node for layout computation - either a real node or a dummy for edge routing.
#[derive(Clone, Copy)]
enum VNode {
    Real(usize),
    Dummy { edge_idx: usize },
}

impl VNode {
    fn real_index(&self) -> Option<usize> {
        match self {
            VNode::Real(idx) => Some(*idx),
            VNode::Dummy { .. } => None,
        }
    }
    
    fn dummy_edge(&self) -> Option<usize> {
        match self {
            VNode::Real(_) => None,
            VNode::Dummy { edge_idx } => Some(*edge_idx),
        }
    }
}

