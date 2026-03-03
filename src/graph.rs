//! Core graph data structure.
//!
//! This module provides the fundamental graph structure with nodes and edges.
//! The primary type is [`Graph`].
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

pub mod arena;
pub mod csr;

use alloc::{vec, string::String, vec::Vec};

use crate::algorithms::sugiyama::crossing::{CrossingReducer, STANDARD};

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

/// A directed graph with ASCII rendering capabilities.
///
/// Despite the crate name (`ascii-dag`), `Graph` supports cycles — they are
/// detected and broken automatically during layout.  Use [`Requirements::dag()`]
/// if you need to validate acyclicity before layout.
///
/// [`Requirements::dag()`]: crate::Requirements::dag
///
/// # Examples
///
/// ```
/// use ascii_dag::Graph;
///
/// let mut g = Graph::new();
/// g.add_node(1, "Start");
/// g.add_node(2, "End");
/// g.add_edge(1, 2, None);
///
/// let output = g.render();
/// assert!(output.contains("Start"));
/// assert!(output.contains("End"));
/// ```
#[derive(Clone)]
pub struct Graph<'a> {
    pub(crate) nodes: Vec<(usize, &'a str)>,
    pub(crate) edges: Vec<(usize, usize, Option<&'a str>)>,
    pub(crate) render_mode: RenderMode,
    pub(crate) auto_created: HashSet<usize>, // Track auto-created nodes for visual distinction (O(1) lookups)
    pub(crate) id_to_index: HashMap<usize, usize>, // Cache id→index mapping (O(1) lookups)
    pub(crate) node_widths: Vec<usize>,      // Cached formatted widths
    pub(crate) children: Vec<Vec<usize>>,    // Adjacency list: children[idx] = child indices
    pub(crate) parents: Vec<Vec<usize>>,     // Adjacency list: parents[idx] = parent indices
    pub(crate) crossing_pipeline: Vec<CrossingReducer>, // Composable crossing reduction pipeline
}

impl<'a> Default for Graph<'a> {
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
            crossing_pipeline: STANDARD.to_vec(),
        }
    }
}

impl<'a> Graph<'a> {
    /// Create a new empty DAG.
    ///
    /// # Examples
    ///
    /// ```
    /// use ascii_dag::graph::Graph;
    /// let dag = Graph::new();
    /// ```
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a DAG from pre-defined nodes and edges (batch construction).
    ///
    /// This is more efficient than using the builder API for static graphs.
    /// For edges with labels, use [`from_edges_labeled`](Self::from_edges_labeled).
    ///
    /// # Examples
    ///
    /// ```
    /// use ascii_dag::graph::Graph;
    ///
    /// let dag = Graph::from_edges(
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
            crossing_pipeline: STANDARD.to_vec(),
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
            dag.add_edge(from, to, None);
        }

        dag
    }

    /// Create a DAG from pre-defined nodes and labeled edges (batch construction).
    ///
    /// # Examples
    ///
    /// ```
    /// use ascii_dag::graph::Graph;
    ///
    /// let dag = Graph::from_edges_labeled(
    ///     &[(1, "A"), (2, "B"), (3, "C")],
    ///     &[(1, 2, Some("uses")), (2, 3, None)]
    /// );
    /// ```
    pub fn from_edges_labeled(
        nodes: &[(usize, &'a str)],
        edges: &[(usize, usize, Option<&'a str>)],
    ) -> Self {
        let mut dag = Self {
            nodes: nodes.to_vec(),
            edges: Vec::new(),
            render_mode: RenderMode::default(),
            auto_created: HashSet::new(),
            id_to_index: HashMap::new(),
            node_widths: Vec::new(),
            children: Vec::new(),
            parents: Vec::new(),
            crossing_pipeline: STANDARD.to_vec(),
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
        for &(from, to, label) in edges {
            dag.add_edge(from, to, label);
        }

        dag
    }

    /// Set the rendering mode.
    ///
    /// # Examples
    ///
    /// ```
    /// use ascii_dag::graph::{Graph, RenderMode};
    ///
    /// let mut dag = Graph::new();
    /// dag.set_render_mode(RenderMode::Horizontal);
    /// ```
    pub fn set_render_mode(&mut self, mode: RenderMode) {
        self.render_mode = mode;
    }

    /// Set the number of passes for the crossing reduction algorithm.
    ///
    /// This is a **compatibility shim** — it replaces the entire pipeline
    /// with `[Median(passes)]`.  Prefer [`set_crossing_pipeline`](Self::set_crossing_pipeline)
    /// for full control.
    ///
    /// - `0`: Skip crossing reduction entirely
    /// - `1-4`: Good for most graphs
    /// - `8-10`: Better layouts for complex tangled graphs, but slower
    ///
    /// Values > 20 trigger a warning.  Values > 1000 are clamped to 0.
    pub fn set_crossing_reduction_passes(&mut self, passes: usize) {
        let p = Self::validate_passes(passes);
        self.crossing_pipeline = if p == 0 {
            Vec::new()
        } else {
            vec![CrossingReducer::Median(p)]
        };
    }

    /// Set the crossing reduction pipeline.
    ///
    /// The pipeline is a sequence of [`CrossingReducer`] strategies applied
    /// in order.  Use the presets [`FAST`](crate::algorithms::sugiyama::crossing::FAST),
    /// [`STANDARD`](crate::algorithms::sugiyama::crossing::STANDARD), or
    /// [`QUALITY`](crate::algorithms::sugiyama::crossing::QUALITY), or build
    /// your own.
    ///
    /// # Examples
    ///
    /// ```
    /// use ascii_dag::graph::Graph;
    /// use ascii_dag::algorithms::sugiyama::crossing::{CrossingReducer, QUALITY};
    ///
    /// let mut dag = Graph::new();
    /// dag.set_crossing_pipeline(QUALITY);
    /// ```
    pub fn set_crossing_pipeline(&mut self, pipeline: &[CrossingReducer]) {
        self.crossing_pipeline = pipeline.to_vec();
    }

    /// Validate crossing reduction passes, returning a safe value.
    /// - Values > 1000 are treated as accidental (e.g., -1i32 as usize) and clamped to 0
    /// - Values > 20 trigger a warning about diminishing returns
    #[inline]
    fn validate_passes(passes: usize) -> usize {
        if passes > 1000 {
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
    /// use ascii_dag::graph::{Graph, RenderMode};
    ///
    /// let dag = Graph::new()
    ///     .with_render_mode(RenderMode::Horizontal)
    ///     .with_crossing_reduction_passes(6);
    /// ```
    pub fn with_render_mode(mut self, mode: RenderMode) -> Self {
        self.render_mode = mode;
        self
    }

    /// Builder method: set crossing reduction passes (chainable).
    ///
    /// **Compatibility shim** — see [`set_crossing_reduction_passes`](Self::set_crossing_reduction_passes).
    ///
    /// # Examples
    ///
    /// ```
    /// use ascii_dag::Graph;
    ///
    /// let dag = Graph::new()
    ///     .with_crossing_reduction_passes(8);  // More passes for complex graphs
    /// ```
    pub fn with_crossing_reduction_passes(mut self, passes: usize) -> Self {
        self.set_crossing_reduction_passes(passes);
        self
    }

    /// Builder method: set crossing reduction pipeline (chainable).
    ///
    /// # Examples
    ///
    /// ```
    /// use ascii_dag::Graph;
    /// use ascii_dag::algorithms::sugiyama::crossing::QUALITY;
    ///
    /// let dag = Graph::new()
    ///     .with_crossing_pipeline(QUALITY);
    /// ```
    pub fn with_crossing_pipeline(mut self, pipeline: &[CrossingReducer]) -> Self {
        self.crossing_pipeline = pipeline.to_vec();
        self
    }

    /// Create a DAG with a specific render mode.
    ///
    /// **Deprecated**: Prefer `Graph::new().with_render_mode(mode)` for consistency.
    ///
    /// # Examples
    ///
    /// ```
    /// use ascii_dag::graph::{Graph, RenderMode};
    ///
    /// let dag = Graph::with_mode(RenderMode::Horizontal);
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
    /// use ascii_dag::graph::Graph;
    ///
    /// let mut dag = Graph::new();
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
    ///   or arbitrary units that will be scaled by the renderer)
    ///
    /// # Examples
    ///
    /// ```
    /// use ascii_dag::graph::Graph;
    ///
    /// let mut dag = Graph::new();
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

    /// Add an edge from one node to another with an optional label.
    ///
    /// If either node doesn't exist, it will be auto-created as a placeholder.
    /// You can later call `add_node` to provide a label for auto-created nodes.
    ///
    /// # Examples
    ///
    /// ```
    /// use ascii_dag::graph::Graph;
    ///
    /// let mut dag = Graph::new();
    /// dag.add_node(1, "A");
    /// dag.add_node(2, "B");
    /// dag.add_node(3, "C");
    /// dag.add_edge(1, 2, None);  // A -> B (no label)
    /// dag.add_edge(2, 3, Some("depends on"));  // B -> C with label
    /// ```
    pub fn add_edge(&mut self, from: usize, to: usize, label: Option<&'a str>) {
        self.ensure_node_exists(from);
        self.ensure_node_exists(to);
        self.edges.push((from, to, label));

        // Update adjacency lists (O(1) lookups)
        if let (Some(&from_idx), Some(&to_idx)) =
            (self.id_to_index.get(&from), self.id_to_index.get(&to))
        {
            self.children[from_idx].push(to_idx);
            self.parents[to_idx].push(from_idx);
        }
    }

    /// Get the label for an edge, if any.
    #[inline]
    pub fn edge_label(&self, edge_idx: usize) -> Option<&'a str> {
        self.edges.get(edge_idx).and_then(|e| e.2)
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
    /// use ascii_dag::graph::Graph;
    ///
    /// let dag = Graph::from_edges(
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
    /// use ascii_dag::Graph;
    ///
    /// let dag = Graph::from_edges(
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
        crate::algorithms::sugiyama::heap::compute_layout(self)
    }

    /// Compute the layout using a custom [`LayoutConfig`].
    ///
    /// Temporarily applies the config's crossing pipeline and render mode,
    /// computes the layout, then restores the original settings.
    ///
    /// # Examples
    ///
    /// ```
    /// use ascii_dag::{Graph, LayoutConfig};
    ///
    /// let dag = Graph::from_edges(
    ///     &[(1, "A"), (2, "B"), (3, "C")],
    ///     &[(1, 2), (2, 3)]
    /// );
    ///
    /// let ir = dag.compute_layout_with(&LayoutConfig::quality());
    /// ```
    pub fn compute_layout_with(
        &self,
        config: &crate::algorithms::sugiyama::crossing::LayoutConfig,
    ) -> crate::ir::LayoutIR<'a> {
        // Clone self, apply config, compute.
        let mut dag = self.clone();
        dag.crossing_pipeline = config.crossing_pipeline.clone();
        dag.render_mode = config.render_mode;
        crate::algorithms::sugiyama::heap::compute_layout(&dag)
    }
}

