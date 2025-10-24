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

use alloc::{string::String, vec::Vec};

#[cfg(feature = "std")]
use std::collections::{HashMap, HashSet};

#[cfg(not(feature = "std"))]
use alloc::collections::{BTreeMap as HashMap, BTreeSet as HashSet};

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
        if let (Some(&from_idx), Some(&to_idx)) = (self.id_to_index.get(&from), self.id_to_index.get(&to)) {
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
        // Rough estimate: nodes * avg_label_size + edges * connection_chars + box
        self.nodes.len() * 25 + self.edges.len() * 15 + 200
    }
}
