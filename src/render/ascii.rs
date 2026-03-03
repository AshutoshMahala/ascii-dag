//! ASCII rendering implementation for DAG visualization.

use crate::graph::{Graph, RenderMode};
use alloc::{string::String, vec::Vec};
use core::fmt::Write;

// Type alias for connection groups to reduce complexity
pub(super) type ConnectionGroup = (usize, Vec<(usize, usize, usize)>);

// Box drawing characters (Unicode)
pub(super) const ARROW_RIGHT: char = '→';
pub(crate) const CYCLE_ARROW: char = '⇄'; // For cycle detection

/// Packed bitmap for memory-efficient boolean storage.
/// Stores 64 booleans per u64, reducing memory by 64x compared to Vec<bool>.
/// This is particularly beneficial for no-alloc environments with fixed-size arrays.
pub(super) struct BitSet {
    bits: Vec<u64>,
    len: usize,
}

impl BitSet {
    pub(super) fn new() -> Self {
        Self {
            bits: Vec::new(),
            len: 0,
        }
    }

    /// Prepare the bitmap for a given size (clear and resize)
    #[inline]
    pub(super) fn prepare(&mut self, size: usize) {
        self.len = size;
        let words_needed = size.div_ceil(64);
        self.bits.clear();
        self.bits.resize(words_needed, 0);
    }

    /// Set a bit to true
    #[inline]
    pub(super) fn set(&mut self, idx: usize) {
        debug_assert!(idx < self.len);
        let word = idx / 64;
        let bit = idx % 64;
        self.bits[word] |= 1u64 << bit;
    }

    /// Get a bit value
    #[inline]
    pub(super) fn get(&self, idx: usize) -> bool {
        debug_assert!(idx < self.len);
        let word = idx / 64;
        let bit = idx % 64;
        (self.bits[word] >> bit) & 1 != 0
    }
}

/// Reusable buffers for rendering to avoid repeated allocations.
/// Uses packed bitmaps (64 bools per u64) for 64x memory reduction on boolean arrays.
pub(super) struct RenderBuffers {
    /// Packed bitmap for source positions
    pub(super) is_source: BitSet,
    /// Packed bitmap for real target positions (for arrows)
    pub(super) is_target: BitSet,
    /// Packed bitmap for all target positions (for routing, includes dummy nodes)
    pub(super) all_targets: BitSet,
    /// Additional packed bitmap (pass-through, straight, etc.)
    pub(super) bitmap_aux: BitSet,
    /// Character line buffer
    pub(super) line_chars: Vec<char>,
}

impl RenderBuffers {
    pub(super) fn new() -> Self {
        Self {
            is_source: BitSet::new(),
            is_target: BitSet::new(),
            all_targets: BitSet::new(),
            bitmap_aux: BitSet::new(),
            line_chars: Vec::new(),
        }
    }

    /// Resize and fill a char buffer with spaces
    #[inline]
    pub(super) fn prepare_chars(&mut self, size: usize) {
        self.line_chars.clear();
        self.line_chars.resize(size, ' ');
    }

    /// Prepare source and target bitmaps
    pub(super) fn prepare_bitmaps(&mut self, size: usize) {
        self.is_source.prepare(size);
        self.is_target.prepare(size);
        self.all_targets.prepare(size);
    }

    /// Prepare aux bitmap
    pub(super) fn prepare_aux(&mut self, size: usize) {
        self.bitmap_aux.prepare(size);
    }
}

/// A virtual node in the layout - either a real node or a dummy for edge routing.
/// Memory: 8 bytes using tagged pointer (high bit = is_dummy flag)
/// This is 3x smaller than the naive enum representation.
#[derive(Clone, Copy, Debug)]
pub(super) struct VirtualNode(usize);

impl VirtualNode {
    /// High bit indicates dummy node
    const DUMMY_FLAG: usize = 1 << (usize::BITS - 1);

    #[inline]
    pub(super) fn real(idx: usize) -> Self {
        debug_assert!(idx & Self::DUMMY_FLAG == 0, "index too large");
        Self(idx)
    }

    #[inline]
    pub(super) fn dummy(edge_idx: usize) -> Self {
        debug_assert!(edge_idx & Self::DUMMY_FLAG == 0, "edge index too large");
        Self(edge_idx | Self::DUMMY_FLAG)
    }

    #[inline]
    pub(super) fn is_real(&self) -> bool {
        self.0 & Self::DUMMY_FLAG == 0
    }

    #[inline]
    pub(super) fn is_dummy(&self) -> bool {
        self.0 & Self::DUMMY_FLAG != 0
    }

    #[inline]
    pub(super) fn real_index(&self) -> Option<usize> {
        if self.is_real() { Some(self.0) } else { None }
    }

    #[inline]
    pub(super) fn index(&self) -> usize {
        self.0 & !Self::DUMMY_FLAG
    }
}

/// Virtual layout with dummy nodes for proper edge routing.
/// Memory cost: O(N + E*D) where N=nodes, E=skip edges, D=avg level span
pub(super) struct VirtualLayout {
    /// Virtual nodes at each level
    pub(super) levels: Vec<Vec<VirtualNode>>,
    /// X-coordinate for each virtual node (indexed by level, then position in level)
    pub(super) x_coords: Vec<Vec<usize>>,
    /// Edges grouped by source level for O(1) lookup: edges_by_level[level] = [(from_pos, to_pos), ...]
    pub(super) edges_by_level: Vec<Vec<(usize, usize)>>,
}

impl VirtualLayout {
    /// Get width of a virtual node: 1 for dummies, node_width for real nodes
    #[inline]
    pub(super) fn get_width<'a>(&self, dag: &Graph<'a>, level: usize, pos: usize) -> usize {
        let vnode = &self.levels[level][pos];
        if vnode.is_real() {
            dag.get_node_width(vnode.index())
        } else {
            1 // Dummy nodes are always width 1
        }
    }
}

impl<'a> Graph<'a> {
    /// Render the DAG to an ASCII string.
    ///
    /// # Examples
    ///
    /// ```
    /// use ascii_dag::graph::Graph;
    ///
    /// let dag = Graph::from_edges(
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

    /// Render using the classic renderer (pre-0.6 behavior).
    /// This renderer has more sophisticated edge routing but is slower.
    pub fn render_classic(&self) -> String {
        let mut buf = String::with_capacity(self.estimate_size());
        self.render_classic_to(&mut buf);
        buf
    }

    /// Render the DAG into a provided buffer (zero-allocation).
    ///
    /// # Examples
    ///
    /// ```
    /// use ascii_dag::graph::Graph;
    ///
    /// let dag = Graph::from_edges(
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

        // Check for cycles - use classic renderer for this special case
        if self.has_cycle() {
            self.render_cycle(output);
            return;
        }

        // Determine render mode
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

        // Use classic renderer for horizontal mode (scanline doesn't support it yet)
        if mode == RenderMode::Horizontal {
            self.render_horizontal(output);
            return;
        }

        // Use fast scanline renderer for vertical mode
        let ir = self.compute_layout();
        ir.render_scanline_to(output);
    }

    /// Render using the classic renderer into a buffer.
    pub fn render_classic_to(&self, output: &mut String) {
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
}
