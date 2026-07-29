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

/// Rank direction — the axis levels flow along.
///
/// Recorded on the layout IR; IR coordinates are physical (they match
/// rendered cells). For `BottomUp`, the heap layout path flips its
/// result into physical coordinates.
///
/// Parses from the conventional short forms (case-insensitive):
/// `"TB"`/`"TD"`, `"BT"`, `"LR"`, `"RL"`.
///
/// The built-in renderers currently paint `TopDown` layouts only;
/// rendering an IR with any other direction is unspecified.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Direction {
    /// Levels flow top → bottom (default; edges point down).
    #[default]
    TopDown,
    /// Levels flow bottom → top (edges point up).
    BottomUp,
    /// Levels flow left → right (edges point right).
    LeftRight,
    /// Levels flow right → left (edges point left).
    RightLeft,
}

/// Error returned when parsing a [`Direction`] from an unknown string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParseDirectionError;

impl core::fmt::Display for ParseDirectionError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("unknown direction (expected TB/TD, BT, LR, or RL)")
    }
}

impl core::str::FromStr for Direction {
    type Err = ParseDirectionError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.eq_ignore_ascii_case("TB") || s.eq_ignore_ascii_case("TD") {
            Ok(Direction::TopDown)
        } else if s.eq_ignore_ascii_case("BT") {
            Ok(Direction::BottomUp)
        } else if s.eq_ignore_ascii_case("LR") {
            Ok(Direction::LeftRight)
        } else if s.eq_ignore_ascii_case("RL") {
            Ok(Direction::RightLeft)
        } else {
            Err(ParseDirectionError)
        }
    }
}

// Everything below requires the `alloc` feature (Vec, String, HashMap).
#[cfg(feature = "alloc")]
use alloc::{string::String, vec, vec::Vec};

#[cfg(feature = "alloc")]
use crate::algorithms::sugiyama::config::LayoutConfig;
#[cfg(feature = "alloc")]
#[allow(deprecated)]
use crate::algorithms::sugiyama::config::SugiyamaConfig;
#[cfg(feature = "alloc")]
use crate::algorithms::sugiyama::crossing::CrossingReducer;
#[cfg(feature = "alloc")]
use crate::errors::GraphError;

/// A named subgraph (cluster) for visual grouping.
///
/// Subgraphs define logical clusters that the layout engine renders with
/// box-drawing borders.  They can be nested (a subgraph may have a parent).
///
/// # Zero-Cost When Unused
///
/// If no subgraphs are added, the layout engine skips all subgraph-related
/// processing — there is no per-node overhead.
///
/// # Examples
///
/// ```
/// use ascii_dag::graph::Graph;
///
/// let mut g = Graph::new();
/// g.add_node(1, "Server");
/// g.add_node(2, "Database");
/// g.add_edge(1, 2, None);
///
/// let backend = g.add_subgraph("Backend");
/// g.put_nodes(&[1, 2]).inside(backend).unwrap();
/// ```
#[cfg(feature = "alloc")]
#[derive(Clone, Debug)]
pub struct Subgraph<'a> {
    /// Unique identifier (assigned by [`Graph::add_subgraph`]).
    pub id: usize,
    /// Display label rendered on the top border.
    pub label: &'a str,
    /// Parent subgraph ID for nesting (`None` = top-level).
    pub parent_id: Option<usize>,
}

#[cfg(all(feature = "alloc", feature = "std"))]
use std::collections::{HashMap, HashSet};

#[cfg(all(feature = "alloc", not(feature = "std")))]
use alloc::collections::{BTreeMap as HashMap, BTreeSet as HashSet};

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
#[cfg(feature = "alloc")]
#[allow(deprecated)]
#[derive(Clone, Default)]
pub struct Graph<'a> {
    pub(crate) nodes: Vec<(usize, &'a str)>,
    pub(crate) edges: Vec<(usize, usize, Option<&'a str>)>,
    pub(crate) render_mode: RenderMode,
    pub(crate) direction: Direction,
    pub(crate) auto_created: HashSet<usize>, // Track auto-created nodes for visual distinction (O(1) lookups)
    pub(crate) id_to_index: HashMap<usize, usize>, // Cache id→index mapping (O(1) lookups)
    pub(crate) node_widths: Vec<usize>,      // Cached formatted widths
    pub(crate) node_heights: Vec<usize>,     // Cached node heights (1 = single-line)
    pub(crate) children: Vec<Vec<usize>>,    // Adjacency list: children[idx] = child indices
    pub(crate) parents: Vec<Vec<usize>>,     // Adjacency list: parents[idx] = parent indices
    pub(crate) sugiyama_config: SugiyamaConfig, // Full Sugiyama pipeline configuration
    pub(crate) subgraphs: Vec<Subgraph<'a>>, // Named clusters
    pub(crate) node_subgraph: HashMap<usize, usize>, // node_id → subgraph_id
    pub(crate) next_subgraph_id: usize,      // Monotonic ID counter
}

#[cfg(feature = "alloc")]
#[allow(deprecated)]
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
            direction: Direction::default(),
            auto_created: HashSet::new(),
            id_to_index: HashMap::new(),
            node_widths: Vec::new(),
            node_heights: Vec::new(),
            children: Vec::new(),
            parents: Vec::new(),
            sugiyama_config: SugiyamaConfig::default(),
            subgraphs: Vec::new(),
            node_subgraph: HashMap::new(),
            next_subgraph_id: 0,
        };

        // Build id_to_index map, widths cache, and heights cache
        for (idx, &(id, label)) in dag.nodes.iter().enumerate() {
            dag.id_to_index.insert(id, idx);
            let width = dag.compute_node_width(id, label);
            dag.node_widths.push(width);
            dag.node_heights.push(1);
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
            direction: Direction::default(),
            auto_created: HashSet::new(),
            id_to_index: HashMap::new(),
            node_widths: Vec::new(),
            node_heights: Vec::new(),
            children: Vec::new(),
            parents: Vec::new(),
            sugiyama_config: SugiyamaConfig::default(),
            subgraphs: Vec::new(),
            node_subgraph: HashMap::new(),
            next_subgraph_id: 0,
        };

        // Build id_to_index map, widths cache, and heights cache
        for (idx, &(id, label)) in dag.nodes.iter().enumerate() {
            dag.id_to_index.insert(id, idx);
            let width = dag.compute_node_width(id, label);
            dag.node_widths.push(width);
            dag.node_heights.push(1);
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

    /// Set the rank direction (see [`Direction`]).
    ///
    /// Applies when computing the layout via [`render`](Self::render) or
    /// [`compute_layout`](Self::compute_layout). When you build a
    /// [`LayoutConfig`] yourself and call
    /// [`compute_layout_with_config`](Self::compute_layout_with_config),
    /// the config's `direction` wins.
    ///
    /// The built-in renderers currently paint `TopDown` layouts only.
    ///
    /// # Examples
    ///
    /// ```
    /// use ascii_dag::graph::{Direction, Graph};
    ///
    /// let mut dag = Graph::new();
    /// dag.set_direction(Direction::BottomUp);
    /// ```
    pub fn set_direction(&mut self, direction: Direction) {
        self.direction = direction;
    }

    /// Set the full Sugiyama pipeline configuration.
    ///
    /// # Examples
    ///
    /// ```
    /// use ascii_dag::graph::Graph;
    /// use ascii_dag::SugiyamaConfig;
    ///
    /// let mut dag = Graph::new();
    /// dag.set_sugiyama_config(SugiyamaConfig::quality());
    /// ```
    pub fn set_sugiyama_config(&mut self, config: SugiyamaConfig) {
        self.sugiyama_config = config;
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
        self.sugiyama_config.crossing_pipeline = if p == 0 {
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
        self.sugiyama_config.crossing_pipeline = pipeline.to_vec();
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

    /// Builder method: set the rank direction (chainable).
    pub fn with_direction(mut self, direction: Direction) -> Self {
        self.direction = direction;
        self
    }

    /// Builder method: apply a full [`SugiyamaConfig`] (chainable).
    ///
    /// # Examples
    ///
    /// ```
    /// use ascii_dag::Graph;
    /// use ascii_dag::SugiyamaConfig;
    ///
    /// let dag = Graph::new()
    ///     .with_sugiyama_config(SugiyamaConfig::quality());
    /// ```
    pub fn with_sugiyama_config(mut self, config: SugiyamaConfig) -> Self {
        self.sugiyama_config = config;
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
        self.sugiyama_config.crossing_pipeline = pipeline.to_vec();
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
            // Update cached width, keep height=1
            let width = self.compute_node_width(id, label);
            self.node_widths[idx] = width;
            self.node_heights[idx] = 1;
        } else {
            // Brand new node
            let idx = self.nodes.len();
            self.nodes.push((id, label));
            self.id_to_index.insert(id, idx);
            let width = self.compute_node_width(id, label);
            self.node_widths.push(width);
            self.node_heights.push(1);
            // Extend adjacency lists
            self.children.push(Vec::new());
            self.parents.push(Vec::new());
        }
    }

    /// Add a node with explicit width and height overrides.
    ///
    /// This is useful for pixel-based renderers or complex nodes where size
    /// is determined by rendered content (HTML elements, custom graphics,
    /// multi-line content) rather than label length.
    ///
    /// # Arguments
    ///
    /// * `id` - Unique node identifier
    /// * `label` - Node label text (used for display, not for size calculation)
    /// * `width` - Explicit width in layout units
    /// * `height` - Explicit height in layout rows (1 = single-line node)
    ///
    /// # Examples
    ///
    /// ```
    /// use ascii_dag::graph::Graph;
    ///
    /// let mut dag = Graph::new();
    /// // Single-line node with explicit width
    /// dag.add_node_with_size(1, "Short", 20, 1);
    /// // Multi-row node for complex content
    /// dag.add_node_with_size(2, "Pipeline", 12, 3);
    /// ```
    pub fn add_node_with_size(&mut self, id: usize, label: &'a str, width: usize, height: usize) {
        let height = height.max(1);
        if let Some(&idx) = self.id_to_index.get(&id) {
            self.nodes[idx] = (id, label);
            self.auto_created.remove(&id);
            self.node_widths[idx] = width;
            self.node_heights[idx] = height;
        } else {
            let idx = self.nodes.len();
            self.nodes.push((id, label));
            self.id_to_index.insert(id, idx);
            self.node_widths.push(width);
            self.node_heights.push(height);
            self.children.push(Vec::new());
            self.parents.push(Vec::new());
        }
    }

    /// Add a node with an explicit width override (height defaults to 1).
    #[deprecated(since = "0.9.0", note = "Use `add_node_with_size` instead")]
    pub fn add_node_with_width(&mut self, id: usize, label: &'a str, width: usize) {
        self.add_node_with_size(id, label, width, 1);
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
            self.node_heights.push(1);
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

    /// Get cached height for a node index
    #[inline]
    pub(crate) fn get_node_height(&self, idx: usize) -> usize {
        self.node_heights.get(idx).copied().unwrap_or(1)
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
        let mut config: LayoutConfig<'_> = LayoutConfig::from(&self.sugiyama_config);
        config.direction = self.direction;
        crate::algorithms::sugiyama::heap::compute_layout_cfg(self, &config)
    }

    /// Compute the layout using a custom [`LayoutConfig`].
    ///
    /// This is the preferred API for controlling layout behaviour.
    /// The config borrows its crossing pipeline, so it can be constructed
    /// from static presets or from a `SugiyamaConfig`.
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
    /// let ir = dag.compute_layout_with_config(&LayoutConfig::quality());
    /// ```
    pub fn compute_layout_with_config(&self, config: &LayoutConfig<'_>) -> crate::ir::LayoutIR<'a> {
        let mut dag = self.clone();
        dag.render_mode = config.render_mode;
        crate::algorithms::sugiyama::heap::compute_layout_cfg(&dag, config)
    }

    /// Compute the layout using a custom [`SugiyamaConfig`].
    ///
    /// Applies the config's full pipeline (cycle breaking, layering,
    /// crossing, positioning) then computes the layout.
    ///
    /// # Examples
    ///
    /// ```
    /// use ascii_dag::{Graph, SugiyamaConfig};
    ///
    /// let dag = Graph::from_edges(
    ///     &[(1, "A"), (2, "B"), (3, "C")],
    ///     &[(1, 2), (2, 3)]
    /// );
    ///
    /// let ir = dag.compute_layout_with(&SugiyamaConfig::quality());
    /// ```
    #[deprecated(
        since = "0.9.0",
        note = "use compute_layout_with_config(&LayoutConfig) instead"
    )]
    pub fn compute_layout_with(
        &self,
        config: &crate::algorithms::sugiyama::config::SugiyamaConfig,
    ) -> crate::ir::LayoutIR<'a> {
        let lc = LayoutConfig::from(config);
        self.compute_layout_with_config(&lc)
    }

    // ── Subgraph / Cluster API ───────────────────────────────────────────

    /// Create a new named subgraph (cluster) and return its ID.
    ///
    /// The subgraph has no members and no parent until you call
    /// [`put_nodes`](Self::put_nodes) or [`put_subgraphs`](Self::put_subgraphs).
    ///
    /// # Examples
    ///
    /// ```
    /// use ascii_dag::graph::Graph;
    ///
    /// let mut g = Graph::new();
    /// let backend = g.add_subgraph("Backend");
    /// let frontend = g.add_subgraph("Frontend");
    /// assert_ne!(backend, frontend);
    /// ```
    pub fn add_subgraph(&mut self, label: &'a str) -> usize {
        let id = self.next_subgraph_id;
        self.next_subgraph_id += 1;
        self.subgraphs.push(Subgraph {
            id,
            label,
            parent_id: None,
        });
        id
    }

    /// Start a fluent builder to place nodes inside a subgraph.
    ///
    /// Returns a [`NodePlacer`] whose [`.inside(sg)`](NodePlacer::inside)
    /// method assigns all listed nodes to the given subgraph.
    ///
    /// # Examples
    ///
    /// ```
    /// use ascii_dag::graph::Graph;
    ///
    /// let mut g = Graph::new();
    /// g.add_node(1, "A");
    /// g.add_node(2, "B");
    /// let sg = g.add_subgraph("Cluster");
    /// g.put_nodes(&[1, 2]).inside(sg).unwrap();
    /// assert_eq!(g.node_subgraph(1), Some(sg));
    /// ```
    pub fn put_nodes<'g>(&'g mut self, node_ids: &'g [usize]) -> NodePlacer<'g, 'a> {
        NodePlacer {
            graph: self,
            node_ids,
        }
    }

    /// Start a fluent builder to nest subgraphs inside a parent.
    ///
    /// Returns a [`SubgraphPlacer`] whose [`.inside(parent)`](SubgraphPlacer::inside)
    /// method sets the parent for all listed subgraphs.  Cycle detection
    /// prevents a subgraph from being nested inside itself or a descendant.
    ///
    /// # Examples
    ///
    /// ```
    /// use ascii_dag::graph::Graph;
    ///
    /// let mut g = Graph::new();
    /// let outer = g.add_subgraph("Outer");
    /// let inner = g.add_subgraph("Inner");
    /// g.put_subgraphs(&[inner]).inside(outer).unwrap();
    /// ```
    pub fn put_subgraphs<'g>(&'g mut self, sg_ids: &'g [usize]) -> SubgraphPlacer<'g, 'a> {
        SubgraphPlacer {
            graph: self,
            sg_ids,
        }
    }

    /// Number of subgraphs defined on this graph.
    #[inline]
    pub fn subgraph_count(&self) -> usize {
        self.subgraphs.len()
    }

    /// Which subgraph a node belongs to, if any.
    #[inline]
    pub fn node_subgraph(&self, node_id: usize) -> Option<usize> {
        self.node_subgraph.get(&node_id).copied()
    }

    /// Whether any subgraphs have been defined.
    ///
    /// The layout engine uses this to short-circuit all subgraph-related
    /// processing when there are none.
    #[inline]
    pub fn has_subgraphs(&self) -> bool {
        !self.subgraphs.is_empty()
    }

    /// Get a subgraph by ID.
    #[inline]
    pub fn subgraph(&self, id: usize) -> Option<&Subgraph<'a>> {
        self.subgraphs.iter().find(|sg| sg.id == id)
    }

    /// Get all subgraphs.
    #[inline]
    pub fn subgraphs(&self) -> &[Subgraph<'a>] {
        &self.subgraphs
    }

    /// Check whether `ancestor` is an ancestor of `sg_id` in the nesting
    /// hierarchy (or is `sg_id` itself).  Used for cycle detection.
    fn is_ancestor(&self, sg_id: usize, ancestor: usize) -> bool {
        let mut current = Some(sg_id);
        while let Some(id) = current {
            if id == ancestor {
                return true;
            }
            current = self
                .subgraphs
                .iter()
                .find(|s| s.id == id)
                .and_then(|s| s.parent_id);
        }
        false
    }
}

// ── Fluent placement builders ────────────────────────────────────────────

/// Fluent builder returned by [`Graph::put_nodes`].
///
/// Call [`.inside(subgraph_id)`](NodePlacer::inside) to assign nodes to a cluster.
#[cfg(feature = "alloc")]
pub struct NodePlacer<'g, 'a> {
    graph: &'g mut Graph<'a>,
    node_ids: &'g [usize],
}

#[cfg(feature = "alloc")]
impl<'g, 'a> NodePlacer<'g, 'a> {
    /// Assign every node in the list to the given subgraph.
    ///
    /// # Errors
    ///
    /// - [`GraphError::SubgraphNotFound`] if `sg_id` does not exist.
    /// - [`GraphError::NodeNotFound`] if any node ID is not in the graph.
    pub fn inside(self, sg_id: usize) -> Result<(), GraphError> {
        // Validate subgraph exists
        if !self.graph.subgraphs.iter().any(|s| s.id == sg_id) {
            return Err(GraphError::SubgraphNotFound(sg_id));
        }
        // Validate & assign each node
        for &nid in self.node_ids {
            if !self.graph.id_to_index.contains_key(&nid) {
                return Err(GraphError::NodeNotFound(nid));
            }
            self.graph.node_subgraph.insert(nid, sg_id);
        }
        Ok(())
    }
}

#[cfg(feature = "alloc")]
/// Fluent builder returned by [`Graph::put_subgraphs`].
///
/// Call [`.inside(parent_id)`](SubgraphPlacer::inside) to nest subgraphs.
pub struct SubgraphPlacer<'g, 'a> {
    graph: &'g mut Graph<'a>,
    sg_ids: &'g [usize],
}

#[cfg(feature = "alloc")]
impl<'g, 'a> SubgraphPlacer<'g, 'a> {
    /// Nest every subgraph in the list inside the given parent.
    ///
    /// # Errors
    ///
    /// - [`GraphError::SubgraphNotFound`] if `parent_id` or any child ID
    ///   does not exist.
    /// - [`GraphError::SubgraphCycle`] if nesting would create a cycle
    ///   (e.g., A inside B inside A).
    pub fn inside(self, parent_id: usize) -> Result<(), GraphError> {
        // Validate parent exists
        if !self.graph.subgraphs.iter().any(|s| s.id == parent_id) {
            return Err(GraphError::SubgraphNotFound(parent_id));
        }
        for &child_id in self.sg_ids {
            // Validate child exists
            if !self.graph.subgraphs.iter().any(|s| s.id == child_id) {
                return Err(GraphError::SubgraphNotFound(child_id));
            }
            // Cycle check: parent must not be a descendant of child
            if self.graph.is_ancestor(parent_id, child_id) {
                return Err(GraphError::SubgraphCycle);
            }
            // Set parent
            if let Some(sg) = self.graph.subgraphs.iter_mut().find(|s| s.id == child_id) {
                sg.parent_id = Some(parent_id);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[cfg(feature = "alloc")]
mod tests {
    use super::*;

    // ── Subgraph creation ──────────────────────────────────────────────

    #[test]
    fn add_subgraph_returns_unique_ids() {
        let mut g = Graph::new();
        let a = g.add_subgraph("A");
        let b = g.add_subgraph("B");
        assert_ne!(a, b);
        assert_eq!(g.subgraph_count(), 2);
    }

    #[test]
    fn subgraph_accessor() {
        let mut g = Graph::new();
        let id = g.add_subgraph("Backend");
        let sg = g.subgraph(id).unwrap();
        assert_eq!(sg.label, "Backend");
        assert_eq!(sg.parent_id, None);
    }

    #[test]
    fn has_subgraphs_flag() {
        let mut g = Graph::new();
        assert!(!g.has_subgraphs());
        g.add_subgraph("X");
        assert!(g.has_subgraphs());
    }

    // ── put_nodes().inside() ───────────────────────────────────────────

    #[test]
    fn put_nodes_inside_subgraph() {
        let mut g = Graph::new();
        g.add_node(1, "A");
        g.add_node(2, "B");
        let sg = g.add_subgraph("cluster");
        g.put_nodes(&[1, 2]).inside(sg).unwrap();
        assert_eq!(g.node_subgraph(1), Some(sg));
        assert_eq!(g.node_subgraph(2), Some(sg));
    }

    #[test]
    fn put_nodes_unknown_subgraph_returns_error() {
        let mut g = Graph::new();
        g.add_node(1, "A");
        let result = g.put_nodes(&[1]).inside(999);
        assert!(result.is_err());
        match result.unwrap_err() {
            GraphError::SubgraphNotFound(id) => assert_eq!(id, 999),
            other => panic!("unexpected error: {:?}", other),
        }
    }

    #[test]
    fn put_nodes_unknown_node_returns_error() {
        let mut g = Graph::new();
        let sg = g.add_subgraph("X");
        let result = g.put_nodes(&[42]).inside(sg);
        assert!(result.is_err());
        match result.unwrap_err() {
            GraphError::NodeNotFound(id) => assert_eq!(id, 42),
            other => panic!("unexpected error: {:?}", other),
        }
    }

    // ── put_subgraphs().inside() (nesting) ─────────────────────────────

    #[test]
    fn nest_subgraph_inside_parent() {
        let mut g = Graph::new();
        let parent = g.add_subgraph("outer");
        let child = g.add_subgraph("inner");
        g.put_subgraphs(&[child]).inside(parent).unwrap();
        assert_eq!(g.subgraph(child).unwrap().parent_id, Some(parent));
    }

    #[test]
    fn nesting_cycle_detection() {
        let mut g = Graph::new();
        let a = g.add_subgraph("A");
        let b = g.add_subgraph("B");
        g.put_subgraphs(&[b]).inside(a).unwrap();
        // Trying to put A inside B creates a cycle
        let result = g.put_subgraphs(&[a]).inside(b);
        assert!(result.is_err());
        match result.unwrap_err() {
            GraphError::SubgraphCycle => {}
            other => panic!("unexpected error: {:?}", other),
        }
    }

    #[test]
    fn deep_nesting_cycle_detection() {
        let mut g = Graph::new();
        let a = g.add_subgraph("A");
        let b = g.add_subgraph("B");
        let c = g.add_subgraph("C");
        g.put_subgraphs(&[b]).inside(a).unwrap();
        g.put_subgraphs(&[c]).inside(b).unwrap();
        // A → B → C; putting A inside C creates A → B → C → A
        let result = g.put_subgraphs(&[a]).inside(c);
        assert!(result.is_err());
    }

    // ── Layout integration ─────────────────────────────────────────────

    #[test]
    fn single_cluster_produces_ir_subgraph() {
        let mut g = Graph::new();
        g.add_node(1, "X");
        g.add_node(2, "Y");
        g.add_edge(1, 2, None);
        let sg = g.add_subgraph("cluster");
        g.put_nodes(&[1, 2]).inside(sg).unwrap();

        let ir = g.compute_layout();
        assert_eq!(ir.subgraphs().len(), 1);
        let info = &ir.subgraphs()[0];
        assert_eq!(info.label, "cluster");
        assert!(info.width > 0);
        assert!(info.height > 0);
    }

    #[test]
    fn sibling_clusters_no_bbox_overlap() {
        let mut g = Graph::new();
        g.add_node(1, "A");
        g.add_node(2, "B");
        g.add_node(3, "C");
        g.add_node(4, "D");
        g.add_edge(1, 2, None);
        g.add_edge(3, 4, None);

        let s1 = g.add_subgraph("Left");
        let s2 = g.add_subgraph("Right");
        g.put_nodes(&[1, 2]).inside(s1).unwrap();
        g.put_nodes(&[3, 4]).inside(s2).unwrap();

        let ir = g.compute_layout();
        assert_eq!(ir.subgraphs().len(), 2);

        let b1 = &ir.subgraphs()[0];
        let b2 = &ir.subgraphs()[1];

        // Bounding boxes should not overlap in BOTH x and y simultaneously
        let x_overlap = b1.x < b2.x + b2.width && b2.x < b1.x + b1.width;
        let y_overlap = b1.y < b2.y + b2.height && b2.y < b1.y + b1.height;
        assert!(
            !(x_overlap && y_overlap),
            "sibling bounding boxes overlap: {:?} and {:?}",
            (b1.x, b1.y, b1.width, b1.height),
            (b2.x, b2.y, b2.width, b2.height),
        );
    }

    #[test]
    fn nested_cluster_bbox_contained_by_parent() {
        let mut g = Graph::new();
        g.add_node(1, "API");
        g.add_node(2, "DB");
        g.add_edge(1, 2, None);

        let outer = g.add_subgraph("Backend");
        let inner = g.add_subgraph("Database");
        g.put_nodes(&[1, 2]).inside(outer).unwrap();
        g.put_nodes(&[2]).inside(inner).unwrap();
        g.put_subgraphs(&[inner]).inside(outer).unwrap();

        let ir = g.compute_layout();
        assert_eq!(ir.subgraphs().len(), 2);

        let parent = ir
            .subgraphs()
            .iter()
            .find(|s| s.label == "Backend")
            .unwrap();
        let child = ir
            .subgraphs()
            .iter()
            .find(|s| s.label == "Database")
            .unwrap();

        // Child must be fully contained within parent
        assert!(
            child.x >= parent.x,
            "child.x={} < parent.x={}",
            child.x,
            parent.x
        );
        assert!(
            child.y >= parent.y,
            "child.y={} < parent.y={}",
            child.y,
            parent.y
        );
        assert!(
            child.x + child.width <= parent.x + parent.width,
            "child right edge {} > parent right edge {}",
            child.x + child.width,
            parent.x + parent.width,
        );
        assert!(
            child.y + child.height <= parent.y + parent.height,
            "child bottom {} > parent bottom {}",
            child.y + child.height,
            parent.y + parent.height,
        );
    }

    #[test]
    fn render_with_subgraph_does_not_panic() {
        let mut g = Graph::new();
        g.add_node(1, "A");
        g.add_node(2, "B");
        g.add_edge(1, 2, None);
        let sg = g.add_subgraph("test");
        g.put_nodes(&[1, 2]).inside(sg).unwrap();
        // Should not panic (scanline renderer has border support)
        let ir = g.compute_layout();
        let output = ir.render_string(&crate::render::engine::RenderOptions::plain());
        assert!(!output.is_empty());
        // Border characters should appear (double-line style)
        assert!(output.contains('╔'));
        assert!(output.contains('╝'));
    }

    #[test]
    fn render_label_appears_in_output() {
        let mut g = Graph::new();
        g.add_node(1, "Node");
        let sg = g.add_subgraph("MyCluster");
        g.put_nodes(&[1]).inside(sg).unwrap();
        let ir = g.compute_layout();
        let output = ir.render_string(&crate::render::engine::RenderOptions::plain());
        assert!(
            output.contains("MyCluster"),
            "label not found in output:\n{output}",
        );
    }

    // ── Spacing config (regression: fields were silently ignored) ──────

    /// R fans out to A/B/C (three adjacent nodes on level 1), converging on S.
    fn spacing_test_graph() -> Graph<'static> {
        Graph::from_edges(
            &[(1, "R"), (2, "A"), (3, "B"), (4, "C"), (5, "S")],
            &[(1, 2), (1, 3), (1, 4), (2, 5), (3, 5), (4, 5)],
        )
    }

    #[test]
    fn node_spacing_config_is_applied() {
        let g = spacing_test_graph();
        for spacing in [3usize, 8] {
            let mut cfg = LayoutConfig::standard();
            cfg.node_spacing = spacing;
            let ir = g.compute_layout_with_config(&cfg);
            let mut boxes: Vec<(usize, usize)> = ir
                .nodes()
                .iter()
                .filter(|n| n.level == 1)
                .map(|n| (n.x, n.width))
                .collect();
            boxes.sort_unstable();
            assert_eq!(boxes.len(), 3);
            for pair in boxes.windows(2) {
                let gap = pair[1].0 - (pair[0].0 + pair[0].1);
                assert_eq!(
                    gap, spacing,
                    "gap between adjacent nodes should equal node_spacing={spacing}"
                );
            }
        }
    }

    #[test]
    fn level_spacing_config_is_applied() {
        let g = spacing_test_graph();
        let base = g.compute_layout_with_config(&LayoutConfig::standard());
        let mut cfg = LayoutConfig::standard();
        cfg.level_spacing = 2;
        let spaced = g.compute_layout_with_config(&cfg);

        let y_at = |ir: &crate::ir::LayoutIR<'_>, level: usize| {
            ir.nodes().iter().find(|n| n.level == level).unwrap().y
        };
        // Each of the two inter-level gaps grows by level_spacing.
        assert_eq!(y_at(&spaced, 1), y_at(&base, 1) + 2);
        assert_eq!(y_at(&spaced, 2), y_at(&base, 2) + 4);
        // No trailing gap after the last level: total height grows by
        // exactly (levels - 1) * level_spacing.
        assert_eq!(spaced.height(), base.height() + 4);
    }

    // ── Cluster-width feedback (regression: external nodes rendered
    //    inside subgraph borders) ────────────────────────────────────────

    fn assert_externals_clear(ir: &crate::ir::LayoutIR<'_>, externals: &[&str]) {
        for sg in ir.subgraphs() {
            assert!(
                sg.x + sg.width <= ir.width(),
                "canvas clips subgraph '{}' (border right {} > width {})",
                sg.label,
                sg.x + sg.width,
                ir.width(),
            );
            for n in ir.nodes().iter().filter(|n| externals.contains(&n.label)) {
                let x_overlap = n.x < sg.x + sg.width && n.x + n.width > sg.x;
                let y_overlap = n.y >= sg.y && n.y < sg.y + sg.height;
                assert!(
                    !(x_overlap && y_overlap),
                    "external node '{}' overlaps subgraph '{}' box",
                    n.label,
                    sg.label,
                );
            }
        }
    }

    #[test]
    fn label_widened_subgraph_does_not_swallow_external_nodes() {
        let mut g = Graph::new();
        g.add_node(1, "X");
        g.add_node(2, "E");
        g.add_node(3, "X2");
        g.add_node(4, "E2");
        g.add_edge(1, 3, None);
        g.add_edge(2, 4, None);
        let sg = g.add_subgraph("VeryLongSubgraphLabelHere");
        g.put_nodes(&[1, 3]).inside(sg).unwrap();
        assert_externals_clear(&g.compute_layout(), &["E", "E2"]);
    }

    #[test]
    fn levels_tighten_after_sibling_cluster_shifts() {
        // Tier-2 pattern from examples/subgraph_stress.rs: crossing
        // reduction orders Analytics between the two Core DBs; sibling
        // overlap repair then shifts Data Pipeline right, leaving a hole.
        // tighten_levels must reclaim it so children align with parents.
        let mut g = Graph::new();
        for (id, label) in [
            (1, "LoadBalancer"),
            (2, "CDN"),
            (10, "WebApp"),
            (11, "MobileAPI"),
            (12, "SSR-Engine"),
            (20, "AuthSvc"),
            (21, "SessionDB"),
            (22, "UserSvc"),
            (23, "ProfileDB"),
            (30, "Analytics"),
            (31, "Warehouse"),
            (32, "ETL"),
        ] {
            g.add_node(id, label);
        }
        for (f, t) in [
            (1, 10),
            (1, 11),
            (2, 12),
            (10, 20),
            (11, 20),
            (12, 22),
            (20, 21),
            (22, 23),
            (20, 30),
            (22, 30),
            (30, 31),
            (30, 32),
        ] {
            g.add_edge(f, t, None);
        }
        let frontend = g.add_subgraph("Frontend");
        let backend = g.add_subgraph("Backend");
        let core = g.add_subgraph("Core");
        let data = g.add_subgraph("Data Pipeline");
        g.put_nodes(&[10, 11, 12]).inside(frontend).unwrap();
        g.put_nodes(&[20, 21, 22, 23]).inside(core).unwrap();
        g.put_nodes(&[30, 31, 32]).inside(data).unwrap();
        g.put_subgraphs(&[core, data]).inside(backend).unwrap();

        let ir = g.compute_layout();
        let center = |label: &str| {
            let n = ir.nodes().iter().find(|n| n.label == label).unwrap();
            n.x + n.width / 2
        };
        assert!(
            center("SessionDB").abs_diff(center("AuthSvc")) <= 3,
            "SessionDB (center {}) should sit under AuthSvc (center {})",
            center("SessionDB"),
            center("AuthSvc"),
        );
        assert!(
            center("ProfileDB").abs_diff(center("UserSvc")) <= 3,
            "ProfileDB (center {}) should sit under UserSvc (center {})",
            center("ProfileDB"),
            center("UserSvc"),
        );
    }

    /// Tier-3 pattern from examples/subgraph_stress.rs: nested regions,
    /// a CI/CD cluster whose member has children deep inside eu-west-1,
    /// and cross-cluster edges. Shared by the tightening/compaction tests.
    fn tier3_like_graph() -> Graph<'static> {
        let mut g = Graph::new();
        for (id, label) in [
            (1, "ALB"),
            (2, "WAF"),
            (3, "Web-A1"),
            (4, "App-A1"),
            (5, "DB-A1"),
            (6, "Web-A2"),
            (7, "App-A2"),
            (8, "DB-A2"),
            (9, "Web-B1"),
            (10, "App-B1"),
            (11, "Redis-B1"),
            (12, "DB-B1"),
            (13, "Web-B2"),
            (14, "App-B2"),
            (15, "Redis-B2"),
            (16, "DB-B2"),
            (17, "RabbitMQ"),
            (18, "S3-Bucket"),
            (19, "Vault"),
            (20, "Jenkins"),
            (21, "ArgoCD"),
            (22, "ECR"),
            (23, "Prometheus"),
            (24, "Grafana"),
            (25, "Loki"),
        ] {
            g.add_node(id, label);
        }
        for (f, t) in [
            (1, 2),
            (2, 3),
            (3, 4),
            (4, 5),
            (2, 6),
            (6, 7),
            (7, 8),
            (2, 9),
            (9, 10),
            (10, 11),
            (11, 12),
            (2, 13),
            (13, 14),
            (14, 15),
            (15, 16),
            (4, 17),
            (10, 17),
            (17, 18),
            (20, 22),
            (22, 21),
            (21, 3),
            (21, 9),
            (23, 24),
            (25, 24),
            (4, 23),
            (10, 25),
        ] {
            g.add_edge(f, t, None);
        }
        let az_a1 = g.add_subgraph("AZ-1");
        let az_a2 = g.add_subgraph("AZ-2");
        let region_a = g.add_subgraph("us-east-1");
        g.put_nodes(&[3, 4, 5]).inside(az_a1).unwrap();
        g.put_nodes(&[6, 7, 8]).inside(az_a2).unwrap();
        g.put_subgraphs(&[az_a1, az_a2]).inside(region_a).unwrap();
        let az_b1 = g.add_subgraph("AZ-1");
        let az_b2 = g.add_subgraph("AZ-2");
        let region_b = g.add_subgraph("eu-west-1");
        g.put_nodes(&[9, 10, 11, 12]).inside(az_b1).unwrap();
        g.put_nodes(&[13, 14, 15, 16]).inside(az_b2).unwrap();
        g.put_subgraphs(&[az_b1, az_b2]).inside(region_b).unwrap();
        let shared = g.add_subgraph("Shared Services");
        g.put_nodes(&[17, 18, 19]).inside(shared).unwrap();
        let cicd = g.add_subgraph("CI/CD");
        g.put_nodes(&[20, 21, 22]).inside(cicd).unwrap();
        let obs = g.add_subgraph("Observability");
        g.put_nodes(&[23, 24, 25]).inside(obs).unwrap();
        g
    }

    #[test]
    fn tightening_never_overlaps_disjoint_boxes() {
        // ArgoCD (CI/CD) has children deep inside eu-west-1. Tightening
        // must not pull it out of the CI/CD envelope — that would expand
        // the CI/CD box into the eu-west-1 box after the sibling shifts
        // already separated them.
        let g = tier3_like_graph();
        let ir = g.compute_layout();
        let boxes = ir.subgraphs();
        let is_ancestor = |anc: usize, mut id: usize| -> bool {
            loop {
                let parent = boxes.iter().find(|s| s.id == id).and_then(|s| s.parent_id);
                match parent {
                    Some(p) if p == anc => return true,
                    Some(p) => id = p,
                    None => return false,
                }
            }
        };
        for a in boxes {
            for b in boxes {
                if a.id >= b.id || is_ancestor(a.id, b.id) || is_ancestor(b.id, a.id) {
                    continue;
                }
                let x_overlap = a.x < b.x + b.width && b.x < a.x + a.width;
                let y_overlap = a.y < b.y + b.height && b.y < a.y + a.height;
                assert!(
                    !(x_overlap && y_overlap),
                    "boxes '{}' ({},{} {}x{}) and '{}' ({},{} {}x{}) overlap",
                    a.label,
                    a.x,
                    a.y,
                    a.width,
                    a.height,
                    b.label,
                    b.x,
                    b.y,
                    b.width,
                    b.height,
                );
            }
        }
    }

    /// Tier-5 pattern from examples/subgraph_stress.rs (80 nodes, 24
    /// clusters, depth 4): the only known reproducer for stale dummy
    /// waypoints crossing node text after inter-cluster compaction.
    fn tier5_like_graph() -> Graph<'static> {
        let mut g = Graph::new();
        let mut id = 0usize;
        let mut next = || {
            id += 1;
            id
        };

        // ── Global Entry ──
        let dns = next();
        g.add_node(dns, "Route53");
        let cdn = next();
        g.add_node(cdn, "CloudFront");
        let waf = next();
        g.add_node(waf, "WAF");
        g.add_edge(dns, cdn, None);
        g.add_edge(cdn, waf, None);

        // ── US Region ──
        //   Frontend
        let us_web = next();
        g.add_node(us_web, "US-Web");
        let us_mobile = next();
        g.add_node(us_mobile, "US-Mobile");
        let us_ssr = next();
        g.add_node(us_ssr, "US-SSR");
        g.add_edge(waf, us_web, None);
        g.add_edge(waf, us_mobile, None);
        g.add_edge(us_web, us_ssr, None);

        //   Auth micro
        let us_auth = next();
        g.add_node(us_auth, "US-Auth");
        let us_token = next();
        g.add_node(us_token, "US-Token");
        let us_mfa = next();
        g.add_node(us_mfa, "US-MFA");
        g.add_edge(us_web, us_auth, None);
        g.add_edge(us_mobile, us_auth, None);
        g.add_edge(us_auth, us_token, None);
        g.add_edge(us_auth, us_mfa, None);

        //   Biz logic
        let us_order = next();
        g.add_node(us_order, "US-Orders");
        let us_pay = next();
        g.add_node(us_pay, "US-Pay");
        let us_inv = next();
        g.add_node(us_inv, "US-Inv");
        let us_ship = next();
        g.add_node(us_ship, "US-Ship");
        g.add_edge(us_auth, us_order, None);
        g.add_edge(us_order, us_pay, None);
        g.add_edge(us_order, us_inv, None);
        g.add_edge(us_order, us_ship, None);

        //   Databases
        let us_pg = next();
        g.add_node(us_pg, "US-Postgres");
        let us_redis = next();
        g.add_node(us_redis, "US-Redis");
        let us_s3 = next();
        g.add_node(us_s3, "US-S3");
        g.add_edge(us_order, us_pg, None);
        g.add_edge(us_pay, us_pg, None);
        g.add_edge(us_auth, us_redis, None);
        g.add_edge(us_inv, us_s3, None);

        // ── EU Region (mirror, smaller) ──
        let eu_web = next();
        g.add_node(eu_web, "EU-Web");
        let eu_auth = next();
        g.add_node(eu_auth, "EU-Auth");
        let eu_order = next();
        g.add_node(eu_order, "EU-Orders");
        let eu_pay = next();
        g.add_node(eu_pay, "EU-Pay");
        let eu_ship = next();
        g.add_node(eu_ship, "EU-Ship");
        let eu_pg = next();
        g.add_node(eu_pg, "EU-Postgres");
        let eu_redis = next();
        g.add_node(eu_redis, "EU-Redis");
        g.add_edge(waf, eu_web, None);
        g.add_edge(eu_web, eu_auth, None);
        g.add_edge(eu_auth, eu_order, None);
        g.add_edge(eu_order, eu_pay, None);
        g.add_edge(eu_order, eu_ship, None);
        g.add_edge(eu_order, eu_pg, None);
        g.add_edge(eu_auth, eu_redis, None);

        // ── APAC Region ──
        let ap_web = next();
        g.add_node(ap_web, "AP-Web");
        let ap_auth = next();
        g.add_node(ap_auth, "AP-Auth");
        let ap_order = next();
        g.add_node(ap_order, "AP-Orders");
        let ap_pg = next();
        g.add_node(ap_pg, "AP-Postgres");
        g.add_edge(waf, ap_web, None);
        g.add_edge(ap_web, ap_auth, None);
        g.add_edge(ap_auth, ap_order, None);
        g.add_edge(ap_order, ap_pg, None);

        // ── Messaging Layer ──
        let kafka1 = next();
        g.add_node(kafka1, "Kafka-1");
        let kafka2 = next();
        g.add_node(kafka2, "Kafka-2");
        let zk = next();
        g.add_node(zk, "Zookeeper");
        let schema = next();
        g.add_node(schema, "SchemaReg");
        g.add_edge(us_order, kafka1, None);
        g.add_edge(eu_order, kafka1, None);
        g.add_edge(kafka1, kafka2, None);
        g.add_edge(kafka1, zk, None);
        g.add_edge(kafka2, schema, None);

        // ── Data Platform ──
        let spark = next();
        g.add_node(spark, "Spark");
        let flink = next();
        g.add_node(flink, "Flink");
        let airflow = next();
        g.add_node(airflow, "Airflow");
        let datalake = next();
        g.add_node(datalake, "DataLake");
        let redshift = next();
        g.add_node(redshift, "Redshift");
        let tableau = next();
        g.add_node(tableau, "Tableau");
        g.add_edge(kafka2, spark, None);
        g.add_edge(kafka2, flink, None);
        g.add_edge(airflow, spark, None);
        g.add_edge(spark, datalake, None);
        g.add_edge(flink, datalake, None);
        g.add_edge(datalake, redshift, None);
        g.add_edge(redshift, tableau, None);

        // ── ML Platform ──
        let mlflow = next();
        g.add_node(mlflow, "MLflow");
        let sagemaker = next();
        g.add_node(sagemaker, "SageMaker");
        let model_reg = next();
        g.add_node(model_reg, "ModelReg");
        let inference = next();
        g.add_node(inference, "Inference");
        g.add_edge(datalake, mlflow, None);
        g.add_edge(mlflow, sagemaker, None);
        g.add_edge(sagemaker, model_reg, None);
        g.add_edge(model_reg, inference, None);
        g.add_edge(inference, us_order, None); // predictions feed back

        // ── Observability ──
        let prom = next();
        g.add_node(prom, "Prometheus");
        let grafana = next();
        g.add_node(grafana, "Grafana");
        let jaeger = next();
        g.add_node(jaeger, "Jaeger");
        let loki = next();
        g.add_node(loki, "Loki");
        let cortex = next();
        g.add_node(cortex, "Cortex");
        let pager = next();
        g.add_node(pager, "PagerDuty");
        let opsgenie = next();
        g.add_node(opsgenie, "OpsGenie");
        g.add_edge(us_order, prom, None);
        g.add_edge(eu_order, prom, None);
        g.add_edge(prom, cortex, None);
        g.add_edge(cortex, grafana, None);
        g.add_edge(prom, jaeger, None);
        g.add_edge(prom, loki, None);
        g.add_edge(grafana, pager, None);
        g.add_edge(grafana, opsgenie, None);

        // ── Security ──
        let vault = next();
        g.add_node(vault, "Vault");
        let cert_mgr = next();
        g.add_node(cert_mgr, "CertMgr");
        let guard = next();
        g.add_node(guard, "GuardDuty");
        let inspector = next();
        g.add_node(inspector, "Inspector");
        g.add_edge(us_auth, vault, None);
        g.add_edge(eu_auth, vault, None);
        g.add_edge(vault, cert_mgr, None);
        g.add_edge(vault, guard, None);
        g.add_edge(guard, inspector, None);

        // ── DevOps / Platform ──
        let github = next();
        g.add_node(github, "GitHub");
        let ci = next();
        g.add_node(ci, "Actions");
        let ecr = next();
        g.add_node(ecr, "ECR");
        let argo = next();
        g.add_node(argo, "ArgoCD");
        let tf = next();
        g.add_node(tf, "Terraform");
        let k8s_us = next();
        g.add_node(k8s_us, "EKS-US");
        let k8s_eu = next();
        g.add_node(k8s_eu, "EKS-EU");
        let k8s_ap = next();
        g.add_node(k8s_ap, "EKS-AP");
        g.add_edge(github, ci, None);
        g.add_edge(ci, ecr, None);
        g.add_edge(ecr, argo, None);
        g.add_edge(argo, k8s_us, None);
        g.add_edge(argo, k8s_eu, None);
        g.add_edge(argo, k8s_ap, None);
        g.add_edge(tf, k8s_us, None);
        g.add_edge(tf, k8s_eu, None);
        g.add_edge(tf, k8s_ap, None);

        // ── Notifications ──
        let sns = next();
        g.add_node(sns, "SNS");
        let ses = next();
        g.add_node(ses, "SES");
        let slack_hook = next();
        g.add_node(slack_hook, "Slack");
        g.add_edge(pager, sns, None);
        g.add_edge(sns, ses, None);
        g.add_edge(sns, slack_hook, None);

        // ── Build subgraph hierarchy ──

        // US Region → Frontend, Auth, Business, Data
        let us_fe = g.add_subgraph("US-Frontend");
        g.put_nodes(&[us_web, us_mobile, us_ssr])
            .inside(us_fe)
            .unwrap();
        let us_au = g.add_subgraph("US-Auth");
        g.put_nodes(&[us_auth, us_token, us_mfa])
            .inside(us_au)
            .unwrap();
        let us_biz = g.add_subgraph("US-Business");
        g.put_nodes(&[us_order, us_pay, us_inv, us_ship])
            .inside(us_biz)
            .unwrap();
        let us_data = g.add_subgraph("US-Data");
        g.put_nodes(&[us_pg, us_redis, us_s3])
            .inside(us_data)
            .unwrap();
        let region_us = g.add_subgraph("US-East");
        g.put_subgraphs(&[us_fe, us_au, us_biz, us_data])
            .inside(region_us)
            .unwrap();

        // EU Region
        let eu_svc = g.add_subgraph("EU-Services");
        g.put_nodes(&[eu_web, eu_auth, eu_order, eu_pay, eu_ship])
            .inside(eu_svc)
            .unwrap();
        let eu_db = g.add_subgraph("EU-Data");
        g.put_nodes(&[eu_pg, eu_redis]).inside(eu_db).unwrap();
        let region_eu = g.add_subgraph("EU-West");
        g.put_subgraphs(&[eu_svc, eu_db]).inside(region_eu).unwrap();

        // APAC Region
        let region_ap = g.add_subgraph("APAC");
        g.put_nodes(&[ap_web, ap_auth, ap_order, ap_pg])
            .inside(region_ap)
            .unwrap();

        // Messaging
        let sg_msg = g.add_subgraph("Event Bus");
        g.put_nodes(&[kafka1, kafka2, zk, schema])
            .inside(sg_msg)
            .unwrap();

        // Data Platform
        let sg_ingest = g.add_subgraph("Ingestion");
        g.put_nodes(&[spark, flink]).inside(sg_ingest).unwrap();
        let sg_store = g.add_subgraph("Storage");
        g.put_nodes(&[datalake, redshift]).inside(sg_store).unwrap();
        let sg_dp = g.add_subgraph("Data Platform");
        g.put_nodes(&[airflow, tableau]).inside(sg_dp).unwrap();
        g.put_subgraphs(&[sg_ingest, sg_store])
            .inside(sg_dp)
            .unwrap();

        // ML Platform
        let sg_ml = g.add_subgraph("ML Platform");
        g.put_nodes(&[mlflow, sagemaker, model_reg, inference])
            .inside(sg_ml)
            .unwrap();

        // Observability
        let sg_metrics = g.add_subgraph("Metrics");
        g.put_nodes(&[prom, cortex, grafana])
            .inside(sg_metrics)
            .unwrap();
        let sg_tracing = g.add_subgraph("Tracing");
        g.put_nodes(&[jaeger, loki]).inside(sg_tracing).unwrap();
        let sg_alert = g.add_subgraph("Alerting");
        g.put_nodes(&[pager, opsgenie]).inside(sg_alert).unwrap();
        let sg_obs = g.add_subgraph("Observability");
        g.put_subgraphs(&[sg_metrics, sg_tracing, sg_alert])
            .inside(sg_obs)
            .unwrap();

        // Security
        let sg_sec = g.add_subgraph("Security");
        g.put_nodes(&[vault, cert_mgr, guard, inspector])
            .inside(sg_sec)
            .unwrap();

        // DevOps
        let sg_cicd = g.add_subgraph("CI/CD");
        g.put_nodes(&[github, ci, ecr, argo])
            .inside(sg_cicd)
            .unwrap();
        let sg_infra = g.add_subgraph("Infrastructure");
        g.put_nodes(&[tf, k8s_us, k8s_eu, k8s_ap])
            .inside(sg_infra)
            .unwrap();
        let sg_devops = g.add_subgraph("Platform Eng");
        g.put_subgraphs(&[sg_cicd, sg_infra])
            .inside(sg_devops)
            .unwrap();

        // Notifications
        let sg_notif = g.add_subgraph("Notifications");
        g.put_nodes(&[sns, ses, slack_hook])
            .inside(sg_notif)
            .unwrap();

        g
    }

    /// Assert no edge's vertical segment runs through node text.
    fn assert_no_edge_crosses_nodes(ir: &crate::ir::LayoutIR<'_>) {
        use crate::ir::EdgePath;
        for edge in ir.edges() {
            let EdgePath::MultiSegment { waypoints, .. } = &edge.path else {
                continue;
            };
            // Vertical segments: (column, y range) per polyline leg.
            let mut segments: Vec<(usize, usize, usize)> = Vec::new();
            if let Some(&(x0, y0)) = waypoints.first() {
                // The painter starts this vertical at from_y + 1 + offset —
                // strictly below the source's node row.
                let start = edge.from_y + 1;
                segments.push((x0, start.min(y0), start.max(y0)));
            }
            for pair in waypoints.windows(2) {
                let (_, y_prev) = pair[0];
                let (x, y) = pair[1];
                segments.push((x, y_prev.min(y), y_prev.max(y)));
            }
            if let Some(&(_, y_last)) = waypoints.last() {
                segments.push((edge.to_x, y_last.min(edge.to_y), y_last.max(edge.to_y)));
            }
            for node in ir.nodes() {
                if node.id == edge.from_id || node.id == edge.to_id {
                    continue;
                }
                for &(col, y0, y1) in &segments {
                    let x_hit = col >= node.x && col < node.x + node.width;
                    let y_hit = y0 < node.y + node.height && node.y <= y1;
                    assert!(
                        !(x_hit && y_hit),
                        "edge {}→{} vertical at column {} (rows {}..{}) crosses node '{}' at ({},{} {}x{})",
                        edge.from_id,
                        edge.to_id,
                        col,
                        y0,
                        y1,
                        node.label,
                        node.x,
                        node.y,
                        node.width,
                        node.height,
                    );
                }
            }
        }
    }

    #[test]
    fn edge_verticals_never_cross_nodes() {
        // The coordinate passes (overlap repair, tightening, compaction)
        // move real nodes; dummy waypoints must be realigned and nudged so
        // no edge's vertical segment runs through node text. Crossing a
        // subgraph border is acceptable; crossing a node never is.
        assert_no_edge_crosses_nodes(&tier3_like_graph().compute_layout());
        assert_no_edge_crosses_nodes(&tier5_like_graph().compute_layout());
    }

    #[test]
    fn clusters_compact_after_separation() {
        // compact_clusters pulls root clusters back together after the
        // sibling-overlap shifts. Without it this layout leaves wide empty
        // gulfs between boxes and the canvas exceeds 450 columns.
        let g = tier3_like_graph();
        let ir = g.compute_layout();
        assert!(
            ir.width() <= 420,
            "canvas width {} suggests inter-cluster compaction regressed",
            ir.width(),
        );
    }

    #[test]
    fn book_length_subgraph_label_is_capped() {
        let long_label = "L".repeat(300);
        let mut g = Graph::new();
        g.add_node(1, "A");
        g.add_node(2, "B");
        g.add_edge(1, 2, None);
        let sg = g.add_subgraph(&long_label);
        g.put_nodes(&[1, 2]).inside(sg).unwrap();
        let ir = g.compute_layout();
        let info = &ir.subgraphs()[0];
        assert!(
            info.width <= 40,
            "label must not widen the box past the cap (got {})",
            info.width,
        );
        assert!(
            ir.width() < 100,
            "canvas must not scale with label length (got {})",
            ir.width(),
        );
        // Renderer truncates the label to the box interior.
        let out = ir.render_string(&crate::render::engine::RenderOptions::plain());
        assert!(out.lines().all(|l| l.chars().count() <= ir.width()));
    }

    #[test]
    fn cross_level_subgraph_envelope_does_not_swallow_external_nodes() {
        let mut g = Graph::new();
        g.add_node(1, "WideMemberNodeAAA");
        g.add_node(2, "WideMemberNodeBBB");
        g.add_node(3, "m");
        g.add_node(4, "ext");
        g.add_edge(1, 3, None);
        g.add_edge(2, 3, None);
        g.add_edge(1, 4, None);
        let sg = g.add_subgraph("C");
        g.put_nodes(&[1, 2, 3]).inside(sg).unwrap();
        assert_externals_clear(&g.compute_layout(), &["ext"]);
    }
}
