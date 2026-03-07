//! Compressed Sparse Row (CSR) graph format for arena/embedded mode.
//!
//! This module provides an arena-friendly graph representation that eliminates
//! heap allocations by using contiguous memory layouts.
//!
//! # Memory Layout
//!
//! ```text
//! ┌─────────────────────────────────────────┐
//! │ Node Data: [id, label_offset, level]... │
//! ├─────────────────────────────────────────┤
//! │ Edge Data: [from_idx, to_idx]...        │
//! ├─────────────────────────────────────────┤
//! │ Children Offsets: [0, c0, c0+c1, ...]   │
//! ├─────────────────────────────────────────┤
//! │ Children Data: [child_idx, ...]         │
//! ├─────────────────────────────────────────┤
//! │ Parents Offsets: [0, p0, p0+p1, ...]    │
//! ├─────────────────────────────────────────┤
//! │ Parents Data: [parent_idx, ...]         │
//! └─────────────────────────────────────────┘
//! ```

use crate::graph::arena::Arena;

/// Node data stride: fields per node
const NODE_STRIDE: usize = 5;
/// Node field offsets
const NODE_ID: usize = 0;
const NODE_LABEL_PTR: usize = 1;
const NODE_LABEL_LEN: usize = 2;
const NODE_WIDTH: usize = 3;
const NODE_HEIGHT: usize = 4;

/// Edge data stride: fields per edge
const EDGE_STRIDE: usize = 4;
/// Edge field offsets
const EDGE_FROM: usize = 0;
const EDGE_TO: usize = 1;
const EDGE_LABEL_PTR: usize = 2;
const EDGE_LABEL_LEN: usize = 3;

/// Subgraph data stride: fields per subgraph
const SUBGRAPH_STRIDE: usize = 4;
/// Subgraph field offsets
const SG_ID: usize = 0;
const SG_PARENT_PLUS1: usize = 1; // 0 = no parent, N = parent index N-1
const SG_LABEL_PTR: usize = 2;
const SG_LABEL_LEN: usize = 3;

/// CSR (Compressed Sparse Row) graph representation.
///
/// This is an arena-friendly alternative to the heap-based DAG.
/// All data is stored in contiguous slices backed by the arena.
#[derive(Debug)]
pub struct CsrGraph<'a> {
    /// Node data: [id(usize), label_ptr(u32), label_len(u32)] per node.
    /// Note: First field is standard usize ID, others are packed.
    /// To simplify alignment, we keep nodes as usize for now but pack other arrays.
    nodes: &'a mut [usize],
    /// Number of nodes
    node_count: usize,

    /// Edge data: [from_idx, to_idx] per edge (u32 indices)
    edges: &'a mut [u32],
    /// Number of edges
    edge_count: usize,

    /// Children adjacency offsets: children of node i are at data[offsets[i]..offsets[i+1]]
    children_offsets: &'a [u32],
    /// Children adjacency data: indices of child nodes
    children_data: &'a [u32],

    /// Parents adjacency offsets
    parents_offsets: &'a [u32],
    /// Parents adjacency data: indices of parent nodes
    parents_data: &'a [u32],

    /// Label storage (raw bytes)
    labels: &'a [u8],

    /// Subgraph metadata: [id, parent+1, label_ptr, label_len] × subgraph_count
    subgraph_data: &'a [usize],
    /// Number of subgraphs
    subgraph_count: usize,
    /// Per-node subgraph index (u32::MAX = not in any subgraph)
    node_subgraph: &'a [u32],
}

impl<'a> CsrGraph<'a> {
    /// Calculate required arena size for a graph with given dimensions.
    ///
    /// This helps users pre-allocate the right arena size.
    #[inline]
    pub fn required_arena_size(node_count: usize, edge_count: usize, label_bytes: usize) -> usize {
        Self::required_arena_size_with_subgraphs(node_count, edge_count, label_bytes, 0)
    }

    /// Calculate required arena size including subgraph storage.
    #[inline]
    pub fn required_arena_size_with_subgraphs(
        node_count: usize,
        edge_count: usize,
        label_bytes: usize,
        subgraph_count: usize,
    ) -> usize {
        let nodes_size = node_count * NODE_STRIDE * core::mem::size_of::<usize>();
        let edges_size = edge_count * EDGE_STRIDE * core::mem::size_of::<u32>();
        let children_offsets_size = (node_count + 1) * core::mem::size_of::<u32>();
        let children_data_size = edge_count * core::mem::size_of::<u32>();
        let parents_offsets_size = (node_count + 1) * core::mem::size_of::<u32>();
        let parents_data_size = edge_count * core::mem::size_of::<u32>();

        // Subgraph storage
        let sg_data_size = subgraph_count * SUBGRAPH_STRIDE * core::mem::size_of::<usize>();
        let node_sg_size = if subgraph_count > 0 {
            node_count * core::mem::size_of::<u32>()
        } else {
            0
        };

        // Add alignment padding (estimate 8 bytes per allocation)
        let num_allocs = 6 + if subgraph_count > 0 { 2 } else { 0 };
        let padding = num_allocs * 8;

        nodes_size
            + edges_size
            + children_offsets_size
            + children_data_size
            + parents_offsets_size
            + parents_data_size
            + sg_data_size
            + node_sg_size
            + label_bytes
            + padding
            + 256 // Extra buffer
    }

    /// Get the number of nodes.
    #[inline]
    pub fn node_count(&self) -> usize {
        self.node_count
    }

    /// Get the number of edges.
    #[inline]
    pub fn edge_count(&self) -> usize {
        self.edge_count
    }

    /// Get node ID by index.
    #[inline]
    pub fn node_id(&self, index: usize) -> usize {
        self.nodes[index * NODE_STRIDE + NODE_ID]
    }

    /// Get node label by index.
    #[inline]
    pub fn node_label(&self, index: usize) -> &str {
        let ptr = self.nodes[index * NODE_STRIDE + NODE_LABEL_PTR];
        let len = self.nodes[index * NODE_STRIDE + NODE_LABEL_LEN];

        // Safety: we store valid UTF-8 label offsets
        let bytes = &self.labels[ptr..ptr + len];
        core::str::from_utf8(bytes).unwrap_or("")
    }

    /// Get node display width by index.
    #[inline]
    pub fn node_width(&self, index: usize) -> usize {
        self.nodes[index * NODE_STRIDE + NODE_WIDTH]
    }

    /// Get node display height by index.
    #[inline]
    pub fn node_height(&self, index: usize) -> usize {
        self.nodes[index * NODE_STRIDE + NODE_HEIGHT]
    }

    /// Get children of a node by index.
    #[inline]
    pub fn children(&self, node_index: usize) -> &[u32] {
        let start = self.children_offsets[node_index] as usize;
        let end = self.children_offsets[node_index + 1] as usize;
        &self.children_data[start..end]
    }

    /// Get parents of a node by index.
    #[inline]
    pub fn parents(&self, node_index: usize) -> &[u32] {
        let start = self.parents_offsets[node_index] as usize;
        let end = self.parents_offsets[node_index + 1] as usize;
        &self.parents_data[start..end]
    }

    /// Get edge endpoints by index.
    #[inline]
    pub fn edge(&self, index: usize) -> (usize, usize) {
        let from = self.edges[index * EDGE_STRIDE + EDGE_FROM] as usize;
        let to = self.edges[index * EDGE_STRIDE + EDGE_TO] as usize;
        (from, to)
    }

    /// Get edge label by index. Returns empty string if no label.
    #[inline]
    pub fn edge_label(&self, index: usize) -> &str {
        let ptr = self.edges[index * EDGE_STRIDE + EDGE_LABEL_PTR] as usize;
        let len = self.edges[index * EDGE_STRIDE + EDGE_LABEL_LEN] as usize;
        if len == 0 {
            return "";
        }
        let bytes = &self.labels[ptr..ptr + len];
        core::str::from_utf8(bytes).unwrap_or("")
    }

    /// Check if any edge has a label.
    #[inline]
    pub fn has_edge_labels(&self) -> bool {
        (0..self.edge_count).any(|i| {
            self.edges[i * EDGE_STRIDE + EDGE_LABEL_LEN] != 0
        })
    }

    /// Iterate over all edges as (from_index, to_index) pairs.
    pub fn edges_iter(&self) -> impl Iterator<Item = (usize, usize)> + '_ {
        (0..self.edge_count).map(move |i| self.edge(i))
    }

    // ── Subgraph accessors ───────────────────────────────────────────────

    /// Get the number of subgraphs.
    #[inline]
    pub fn subgraph_count(&self) -> usize {
        self.subgraph_count
    }

    /// Check if this graph has any subgraphs.
    #[inline]
    pub fn has_subgraphs(&self) -> bool {
        self.subgraph_count > 0
    }

    /// Get subgraph ID by subgraph index.
    #[inline]
    pub fn subgraph_id(&self, sg_idx: usize) -> usize {
        self.subgraph_data[sg_idx * SUBGRAPH_STRIDE + SG_ID]
    }

    /// Get subgraph parent index, or `None` for top-level subgraphs.
    #[inline]
    pub fn subgraph_parent(&self, sg_idx: usize) -> Option<usize> {
        let v = self.subgraph_data[sg_idx * SUBGRAPH_STRIDE + SG_PARENT_PLUS1];
        if v == 0 { None } else { Some(v - 1) }
    }

    /// Get subgraph label by subgraph index.
    #[inline]
    pub fn subgraph_label(&self, sg_idx: usize) -> &str {
        let ptr = self.subgraph_data[sg_idx * SUBGRAPH_STRIDE + SG_LABEL_PTR];
        let len = self.subgraph_data[sg_idx * SUBGRAPH_STRIDE + SG_LABEL_LEN];
        if len == 0 {
            return "";
        }
        let bytes = &self.labels[ptr..ptr + len];
        core::str::from_utf8(bytes).unwrap_or("")
    }

    /// Get the subgraph index a node belongs to, or `None`.
    #[inline]
    pub fn node_subgraph(&self, node_idx: usize) -> Option<usize> {
        if node_idx < self.node_subgraph.len() {
            let v = self.node_subgraph[node_idx];
            if v == u32::MAX { None } else { Some(v as usize) }
        } else {
            None
        }
    }

    /// Walk the ancestry chain for a subgraph and return nesting depth
    /// (0 = no subgraph, 1 = top-level, 2 = child of top-level, …).
    pub fn sg_chain_depth(&self, sg_idx: Option<usize>) -> usize {
        let mut depth = 0usize;
        let mut cur = sg_idx;
        while let Some(idx) = cur {
            if idx >= self.subgraph_count { break; }
            depth += 1;
            cur = self.subgraph_parent(idx);
        }
        depth
    }

    /// Render graph summary to a pre-allocated buffer.
    ///
    /// This is a simple text-based representation showing nodes and edges.
    /// Returns the number of bytes written.
    ///
    /// # Arguments
    /// * `buffer` - Output buffer (must be large enough)
    ///
    /// # Returns
    /// Number of bytes written, or None if buffer too small
    pub fn render_to_buffer(&self, buffer: &mut [u8]) -> Option<usize> {
        let mut writer = BufferWriter::new(buffer);

        // Write header
        writer.write_str("CSR Graph: ")?;
        writer.write_num(self.node_count)?;
        writer.write_str(" nodes, ")?;
        writer.write_num(self.edge_count)?;
        writer.write_str(" edges\n")?;

        // Write nodes
        writer.write_str("Nodes:\n")?;
        for i in 0..self.node_count {
            writer.write_str("  [")?;
            writer.write_num(i)?;
            writer.write_str("] ")?;
            writer.write_str(self.node_label(i))?;

            // Show children
            let children = self.children(i);
            if !children.is_empty() {
                writer.write_str(" -> ")?;
                for (j, &child) in children.iter().enumerate() {
                    if j > 0 {
                        writer.write_str(", ")?;
                    }
                    writer.write_num(child as usize)?;
                }
            }
            writer.write_str("\n")?;
        }

        Some(writer.pos)
    }

    /// Estimate buffer size needed for render_to_buffer.
    pub fn estimate_render_size(&self) -> usize {
        // Header: ~50 bytes
        // Per node: label + " -> " + children list ~ 50 bytes avg
        50 + self.node_count * 50
    }

    /// Compute the layout of the graph using the provided arenas.
    ///
    /// This is the entry point for the no-alloc layout algorithm.
    /// It consumes memory from `temp_arena` for calculation scratch space
    /// and populates `output_arena` with the final `LayoutIRArena`.
    pub fn compute_layout_arena<'b>(
        &self,
        config: &crate::algorithms::sugiyama::config::LayoutConfig<'_>,
        temp_arena: &mut Arena<'_>,
        output_arena: &'b mut Arena<'b>,
    ) -> Result<crate::ir::arena::LayoutIRArena<'b>, crate::errors::GraphError> {
        crate::algorithms::sugiyama::arena_csr::compute_layout_arena_csr(self, config, temp_arena, output_arena)
    }
}

/// Helper struct for writing to a byte buffer without allocation.
struct BufferWriter<'a> {
    buffer: &'a mut [u8],
    pos: usize,
}

impl<'a> BufferWriter<'a> {
    fn new(buffer: &'a mut [u8]) -> Self {
        Self { buffer, pos: 0 }
    }

    fn write_str(&mut self, s: &str) -> Option<()> {
        let bytes = s.as_bytes();
        if self.pos + bytes.len() > self.buffer.len() {
            return None;
        }
        self.buffer[self.pos..self.pos + bytes.len()].copy_from_slice(bytes);
        self.pos += bytes.len();
        Some(())
    }

    fn write_num(&mut self, n: usize) -> Option<()> {
        if n == 0 {
            return self.write_str("0");
        }
        let mut digits = [0u8; 20];
        let mut i = 0;
        let mut num = n;
        while num > 0 {
            digits[i] = b'0' + (num % 10) as u8;
            num /= 10;
            i += 1;
        }
        // Reverse and write
        while i > 0 {
            i -= 1;
            if self.pos >= self.buffer.len() {
                return None;
            }
            self.buffer[self.pos] = digits[i];
            self.pos += 1;
        }
        Some(())
    }
}

/// Builder for constructing a CSR graph from arena memory.
pub struct CsrGraphBuilder<'a> {
    arena: &'a mut Arena<'a>,
    // Data slices being populated
    nodes: &'a mut [usize],
    edges: &'a mut [u32],
    children_offsets: &'a mut [u32],
    children_data: &'a mut [u32],
    parents_offsets: &'a mut [u32],
    parents_data: &'a mut [u32],
    labels: &'a mut [u8],

    // Subgraph data (empty slices if no subgraphs)
    subgraph_data: &'a mut [usize],
    node_subgraph: &'a mut [u32],

    // Tracking current progress
    current_node_count: usize,
    current_edge_count: usize,
    current_label_offset: usize,
    current_subgraph_count: usize,

    // Limits
    max_nodes: usize,
    max_edges: usize,
    max_subgraphs: usize,
}

impl<'a> CsrGraphBuilder<'a> {
    /// Create a new builder with known maximum graph dimensions.
    pub fn new(
        arena: &'a mut Arena<'a>,
        max_nodes: usize,
        max_edges: usize,
        max_label_bytes: usize,
    ) -> Option<Self> {
        // Allocate all memory from arena using raw pointers
        let (nodes_ptr, _) = arena.alloc_raw::<usize>(max_nodes * NODE_STRIDE)?;
        let (edges_ptr, _) = arena.alloc_raw::<u32>(max_edges * EDGE_STRIDE)?;
        let (children_offsets_ptr, _) = arena.alloc_raw::<u32>(max_nodes + 1)?;
        let (children_data_ptr, _) = arena.alloc_raw::<u32>(max_edges)?;
        let (parents_offsets_ptr, _) = arena.alloc_raw::<u32>(max_nodes + 1)?;
        let (parents_data_ptr, _) = arena.alloc_raw::<u32>(max_edges)?;
        let (labels_ptr, _) = arena.alloc_raw::<u8>(max_label_bytes)?;

        // Convert to slices
        let (nodes, edges, children_offsets, children_data, parents_offsets, parents_data, labels) = unsafe {
            (
                core::slice::from_raw_parts_mut(nodes_ptr, max_nodes * NODE_STRIDE),
                core::slice::from_raw_parts_mut(edges_ptr, max_edges * EDGE_STRIDE),
                core::slice::from_raw_parts_mut(children_offsets_ptr, max_nodes + 1),
                core::slice::from_raw_parts_mut(children_data_ptr, max_edges),
                core::slice::from_raw_parts_mut(parents_offsets_ptr, max_nodes + 1),
                core::slice::from_raw_parts_mut(parents_data_ptr, max_edges),
                core::slice::from_raw_parts_mut(labels_ptr, max_label_bytes),
            )
        };

        // Initialize offsets to 0
        children_offsets.fill(0);
        parents_offsets.fill(0);

        // No subgraph storage in basic constructor — use empty mutable refs
        // (safe: zero-length slices from a valid, non-null, aligned pointer)
        let subgraph_data: &'a mut [usize] = &mut [];
        let node_subgraph: &'a mut [u32] = &mut [];

        Some(Self {
            arena,
            nodes,
            edges,
            children_offsets,
            children_data,
            parents_offsets,
            parents_data,
            labels,
            subgraph_data,
            node_subgraph,
            current_node_count: 0,
            current_edge_count: 0,
            current_label_offset: 0,
            current_subgraph_count: 0,
            max_nodes,
            max_edges,
            max_subgraphs: 0,
        })
    }

    /// Create a new builder with subgraph support.
    ///
    /// `max_label_bytes` must cover both node/edge labels AND subgraph labels.
    pub fn new_with_subgraphs(
        arena: &'a mut Arena<'a>,
        max_nodes: usize,
        max_edges: usize,
        max_label_bytes: usize,
        max_subgraphs: usize,
    ) -> Option<Self> {
        let (nodes_ptr, _) = arena.alloc_raw::<usize>(max_nodes * NODE_STRIDE)?;
        let (edges_ptr, _) = arena.alloc_raw::<u32>(max_edges * EDGE_STRIDE)?;
        let (children_offsets_ptr, _) = arena.alloc_raw::<u32>(max_nodes + 1)?;
        let (children_data_ptr, _) = arena.alloc_raw::<u32>(max_edges)?;
        let (parents_offsets_ptr, _) = arena.alloc_raw::<u32>(max_nodes + 1)?;
        let (parents_data_ptr, _) = arena.alloc_raw::<u32>(max_edges)?;
        let (labels_ptr, _) = arena.alloc_raw::<u8>(max_label_bytes)?;
        let (sg_data_ptr, _) = arena.alloc_raw::<usize>(max_subgraphs * SUBGRAPH_STRIDE)?;
        let (node_sg_ptr, _) = arena.alloc_raw::<u32>(max_nodes)?;

        let (nodes, edges, children_offsets, children_data, parents_offsets, parents_data, labels, subgraph_data, node_subgraph) = unsafe {
            (
                core::slice::from_raw_parts_mut(nodes_ptr, max_nodes * NODE_STRIDE),
                core::slice::from_raw_parts_mut(edges_ptr, max_edges * EDGE_STRIDE),
                core::slice::from_raw_parts_mut(children_offsets_ptr, max_nodes + 1),
                core::slice::from_raw_parts_mut(children_data_ptr, max_edges),
                core::slice::from_raw_parts_mut(parents_offsets_ptr, max_nodes + 1),
                core::slice::from_raw_parts_mut(parents_data_ptr, max_edges),
                core::slice::from_raw_parts_mut(labels_ptr, max_label_bytes),
                core::slice::from_raw_parts_mut(sg_data_ptr, max_subgraphs * SUBGRAPH_STRIDE),
                core::slice::from_raw_parts_mut(node_sg_ptr, max_nodes),
            )
        };

        children_offsets.fill(0);
        parents_offsets.fill(0);
        node_subgraph.fill(u32::MAX); // no subgraph

        Some(Self {
            arena,
            nodes,
            edges,
            children_offsets,
            children_data,
            parents_offsets,
            parents_data,
            labels,
            subgraph_data,
            node_subgraph,
            current_node_count: 0,
            current_edge_count: 0,
            current_label_offset: 0,
            current_subgraph_count: 0,
            max_nodes,
            max_edges,
            max_subgraphs,
        })
    }

    /// Add a node to the graph with default width (label + 2 for brackets) and height 1.
    /// Returns the node index (0 to N-1).
    pub fn add_node(&mut self, id: usize, label: &str) -> Option<usize> {
        let width = label.len() + 2; // brackets
        self.add_node_with_size(id, label, width, 1)
    }

    /// Add a node with explicit display dimensions.
    /// Returns the node index (0 to N-1).
    pub fn add_node_with_size(
        &mut self,
        id: usize,
        label: &str,
        width: usize,
        height: usize,
    ) -> Option<usize> {
        if self.current_node_count >= self.max_nodes {
            return None;
        }

        let label_len = label.len();
        if self.current_label_offset + label_len > self.labels.len() {
            return None;
        }

        let idx = self.current_node_count;

        // Store node data
        self.nodes[idx * NODE_STRIDE + NODE_ID] = id;
        self.nodes[idx * NODE_STRIDE + NODE_LABEL_PTR] = self.current_label_offset;
        self.nodes[idx * NODE_STRIDE + NODE_LABEL_LEN] = label_len;
        self.nodes[idx * NODE_STRIDE + NODE_WIDTH] = width;
        self.nodes[idx * NODE_STRIDE + NODE_HEIGHT] = height;

        // Store label bytes
        self.labels[self.current_label_offset..self.current_label_offset + label_len]
            .copy_from_slice(label.as_bytes());

        self.current_node_count += 1;
        self.current_label_offset += label_len;

        Some(idx)
    }

    /// Add an edge between two node INDICES (not IDs).
    /// To get the index, use the return value of add_node.
    /// This is safer and faster than looking up IDs.
    pub fn add_edge(&mut self, from_idx: usize, to_idx: usize) -> Option<()> {
        self.add_edge_with_label(from_idx, to_idx, "")
    }

    /// Add a labeled edge between two node INDICES.
    pub fn add_edge_with_label(&mut self, from_idx: usize, to_idx: usize, label: &str) -> Option<()> {
        if self.current_edge_count >= self.max_edges {
            return None;
        }

        if from_idx >= self.current_node_count || to_idx >= self.current_node_count {
            return None;
        }

        let idx = self.current_edge_count;

        // Store edge data
        self.edges[idx * EDGE_STRIDE + EDGE_FROM] = from_idx as u32;
        self.edges[idx * EDGE_STRIDE + EDGE_TO] = to_idx as u32;

        // Store edge label (shares node label storage)
        if !label.is_empty() {
            let label_len = label.len();
            if self.current_label_offset + label_len > self.labels.len() {
                return None;
            }
            self.labels[self.current_label_offset..self.current_label_offset + label_len]
                .copy_from_slice(label.as_bytes());
            self.edges[idx * EDGE_STRIDE + EDGE_LABEL_PTR] = self.current_label_offset as u32;
            self.edges[idx * EDGE_STRIDE + EDGE_LABEL_LEN] = label_len as u32;
            self.current_label_offset += label_len;
        } else {
            self.edges[idx * EDGE_STRIDE + EDGE_LABEL_PTR] = 0;
            self.edges[idx * EDGE_STRIDE + EDGE_LABEL_LEN] = 0;
        }

        self.current_edge_count += 1;
        Some(())
    }

    /// Add a subgraph. Returns the subgraph index (0 to S-1).
    /// Label bytes are shared with node/edge labels.
    pub fn add_subgraph(&mut self, id: usize, label: &str) -> Option<usize> {
        if self.current_subgraph_count >= self.max_subgraphs {
            return None;
        }

        let label_len = label.len();
        if self.current_label_offset + label_len > self.labels.len() {
            return None;
        }

        let sg_idx = self.current_subgraph_count;
        self.subgraph_data[sg_idx * SUBGRAPH_STRIDE + SG_ID] = id;
        self.subgraph_data[sg_idx * SUBGRAPH_STRIDE + SG_PARENT_PLUS1] = 0; // no parent
        self.subgraph_data[sg_idx * SUBGRAPH_STRIDE + SG_LABEL_PTR] = self.current_label_offset;
        self.subgraph_data[sg_idx * SUBGRAPH_STRIDE + SG_LABEL_LEN] = label_len;

        if label_len > 0 {
            self.labels[self.current_label_offset..self.current_label_offset + label_len]
                .copy_from_slice(label.as_bytes());
            self.current_label_offset += label_len;
        }

        self.current_subgraph_count += 1;
        Some(sg_idx)
    }

    /// Set a subgraph's parent (for nesting).
    pub fn set_subgraph_parent(&mut self, sg_idx: usize, parent_sg_idx: usize) -> Option<()> {
        if sg_idx >= self.current_subgraph_count || parent_sg_idx >= self.current_subgraph_count {
            return None;
        }
        self.subgraph_data[sg_idx * SUBGRAPH_STRIDE + SG_PARENT_PLUS1] = parent_sg_idx + 1;
        Some(())
    }

    /// Assign a node to a subgraph.
    pub fn set_node_subgraph(&mut self, node_idx: usize, sg_idx: usize) -> Option<()> {
        if node_idx >= self.current_node_count || sg_idx >= self.current_subgraph_count {
            return None;
        }
        if node_idx >= self.node_subgraph.len() {
            return None;
        }
        self.node_subgraph[node_idx] = sg_idx as u32;
        Some(())
    }
    pub fn build(self) -> Option<CsrGraph<'a>> {
        let CsrGraphBuilder {
            arena,
            nodes,
            edges,
            children_offsets,
            children_data,
            parents_offsets,
            parents_data,
            labels,
            subgraph_data,
            node_subgraph,
            current_node_count,
            current_edge_count,
            current_label_offset,
            current_subgraph_count,
            ..
        } = self;

        let node_count = current_node_count;
        let edge_count = current_edge_count;

        // 1. Count children per node to build offsets
        // We use the already-zeroed children_offsets array as counters
        for i in 0..edge_count {
            let from_idx = edges[i * EDGE_STRIDE + EDGE_FROM] as usize;
            children_offsets[from_idx + 1] += 1;
        }

        // 2. Prefix sum for children offsets
        for i in 1..=node_count {
            children_offsets[i] += children_offsets[i - 1];
        }

        // 3. Fill children_data
        // Reuse parents_offsets as temporary counters
        {
            let child_fill_counts = &mut parents_offsets[..=node_count];
            child_fill_counts.fill(0);

            for i in 0..edge_count {
                let from_idx = edges[i * EDGE_STRIDE + EDGE_FROM] as usize;
                let to_idx = edges[i * EDGE_STRIDE + EDGE_TO]; // already u32

                let offset = (children_offsets[from_idx] + child_fill_counts[from_idx]) as usize;
                children_data[offset] = to_idx;
                child_fill_counts[from_idx] += 1;
            }
        }

        // 4. Now do the same for parents
        // Clear the counters we just used
        parents_offsets[..=node_count].fill(0);

        // Count parents per node
        for i in 0..edge_count {
            let to_idx = edges[i * EDGE_STRIDE + EDGE_TO] as usize;
            parents_offsets[to_idx + 1] += 1;
        }

        // Prefix sum for parents offsets
        for i in 1..=node_count {
            parents_offsets[i] += parents_offsets[i - 1];
        }

        // Fill parents_data
        // We need new temp counters. Allocate from arena.
        // These counters are just small temporary usize values, but let's use u32 to match offsets type if possible
        // Actually, parents_offsets is u32 now.
        let (counters_ptr, _) = arena.alloc_raw::<u32>(node_count)?;
        let parent_fill_counts =
            unsafe { core::slice::from_raw_parts_mut(counters_ptr, node_count) };
        parent_fill_counts.fill(0);

        for i in 0..edge_count {
            let from_idx = edges[i * EDGE_STRIDE + EDGE_FROM]; // already u32
            let to_idx = edges[i * EDGE_STRIDE + EDGE_TO] as usize;

            let offset = (parents_offsets[to_idx] + parent_fill_counts[to_idx]) as usize;
            parents_data[offset] = from_idx;
            parent_fill_counts[to_idx] += 1;
        }

        // Shrink slices to actual used size
        // We need to create NEW slices from the original mutable references.

        Some(CsrGraph {
            nodes: &mut nodes[..node_count * NODE_STRIDE],
            node_count,
            edges: &mut edges[..edge_count * EDGE_STRIDE],
            edge_count,
            children_offsets: &children_offsets[..node_count + 1],
            children_data: &children_data[..edge_count],
            parents_offsets: &parents_offsets[..node_count + 1],
            parents_data: &parents_data[..edge_count],
            labels: &labels[..current_label_offset],
            subgraph_data: &subgraph_data[..current_subgraph_count * SUBGRAPH_STRIDE],
            subgraph_count: current_subgraph_count,
            node_subgraph: if current_subgraph_count > 0 {
                &node_subgraph[..node_count]
            } else {
                &[]
            },
        })
    }
}

#[cfg(feature = "alloc")]
impl<'a> super::Graph<'a> {
    /// Convert heap-based DAG to CSR format using the provided arena.
    ///
    /// This is useful for transitioning to arena mode or for
    /// repeated operations where you want to avoid re-allocation.
    ///
    /// # Example
    ///
    /// ```
    /// use ascii_dag::Graph;
    /// use ascii_dag::graph::arena::Arena;
    ///
    /// let dag = Graph::from_edges(&[(1, "A"), (2, "B")], &[(1, 2)]);
    ///
    /// // Calculate required arena size
    /// let size = dag.estimate_csr_arena_size();
    /// let mut buffer = vec![0u8; size];
    /// let mut arena = Arena::new(&mut buffer);
    ///
    /// // Convert to CSR
    /// let csr = dag.to_csr(&mut arena).unwrap();
    /// assert_eq!(csr.node_count(), 2);
    /// ```
    /// Convert heap-based DAG to CSR format using the provided arena.
    ///
    /// This is useful for transitioning to arena mode or for
    /// repeated operations where you want to avoid re-allocation.
    ///
    /// # Example
    ///
    /// ```
    /// use ascii_dag::Graph;
    /// use ascii_dag::graph::arena::Arena;
    ///
    /// let dag = Graph::from_edges(&[(1, "A"), (2, "B")], &[(1, 2)]);
    ///
    /// // Calculate required arena size
    /// let size = dag.estimate_csr_arena_size();
    /// let mut buffer = vec![0u8; size];
    /// let mut arena = Arena::new(&mut buffer);
    ///
    /// // Convert to CSR
    /// let csr = dag.to_csr(&mut arena).unwrap();
    /// assert_eq!(csr.node_count(), 2);
    /// ```
    pub fn to_csr<'b>(&self, arena: &'b mut Arena<'b>) -> Option<CsrGraph<'b>> {
        let node_count = self.nodes.len();
        let edge_count = self.edges.len();
        let sg_count = self.subgraphs.len();

        // Calculate total label bytes needed (node + edge + subgraph labels share storage)
        let node_label_bytes: usize = self.nodes.iter().map(|(_, label)| label.len()).sum();
        let edge_label_bytes: usize = self.edges.iter()
            .filter_map(|(_, _, label)| label.map(|l| l.len()))
            .sum();
        let sg_label_bytes: usize = self.subgraphs.iter().map(|sg| sg.label.len()).sum();
        let total_label_bytes: usize = node_label_bytes + edge_label_bytes + sg_label_bytes;

        // Allocate all memory from arena using raw pointers
        // This avoids the borrow checker issue with multiple mutable borrows
        let (nodes_ptr, _) = arena.alloc_raw::<usize>(node_count * NODE_STRIDE)?;
        let (edges_ptr, _) = arena.alloc_raw::<u32>(edge_count * EDGE_STRIDE)?;
        let (children_offsets_ptr, _) = arena.alloc_raw::<u32>(node_count + 1)?;
        let (children_data_ptr, _) = arena.alloc_raw::<u32>(edge_count)?;
        let (parents_offsets_ptr, _) = arena.alloc_raw::<u32>(node_count + 1)?;
        let (parents_data_ptr, _) = arena.alloc_raw::<u32>(edge_count)?;
        let (labels_ptr, _) = arena.alloc_raw::<u8>(total_label_bytes)?;

        // Subgraph allocations (only if subgraphs present)
        let sg_data_ptr = if sg_count > 0 {
            Some(arena.alloc_raw::<usize>(sg_count * SUBGRAPH_STRIDE)?.0)
        } else {
            None
        };
        let node_sg_ptr = if sg_count > 0 {
            Some(arena.alloc_raw::<u32>(node_count)?.0)
        } else {
            None
        };

        // Convert raw pointers to slices
        // Safety: alloc_raw has validated the allocations and zeroed the memory
        let (nodes, edges, children_offsets, children_data, parents_offsets, parents_data, labels) = unsafe {
            (
                core::slice::from_raw_parts_mut(nodes_ptr, node_count * NODE_STRIDE),
                core::slice::from_raw_parts_mut(edges_ptr, edge_count * EDGE_STRIDE),
                core::slice::from_raw_parts_mut(children_offsets_ptr, node_count + 1),
                core::slice::from_raw_parts_mut(children_data_ptr, edge_count),
                core::slice::from_raw_parts_mut(parents_offsets_ptr, node_count + 1),
                core::slice::from_raw_parts_mut(parents_data_ptr, edge_count),
                core::slice::from_raw_parts_mut(labels_ptr, total_label_bytes),
            )
        };

        // Copy labels and set up node data
        let mut label_offset = 0;
        for (idx, &(id, label)) in self.nodes.iter().enumerate() {
            nodes[idx * NODE_STRIDE + NODE_ID] = id;
            nodes[idx * NODE_STRIDE + NODE_LABEL_PTR] = label_offset;
            nodes[idx * NODE_STRIDE + NODE_LABEL_LEN] = label.len();
            nodes[idx * NODE_STRIDE + NODE_WIDTH] = self.get_node_width(idx);
            nodes[idx * NODE_STRIDE + NODE_HEIGHT] = self.get_node_height(idx);

            // Copy label bytes
            labels[label_offset..label_offset + label.len()].copy_from_slice(label.as_bytes());
            label_offset += label.len();
        }

        // Count children per node for offsets
        for &(from_id, _to_id, _) in &self.edges {
            if let Some(from_idx) = self.node_index(from_id) {
                children_offsets[from_idx + 1] += 1;
            }
        }

        // Convert counts to offsets (prefix sum)
        for i in 1..=node_count {
            children_offsets[i] += children_offsets[i - 1];
        }

        // Fill children data (use stack array for temporary counts)
        // Note: for very large graphs, this would need a different approach
        // We really need an arena temp buffer here... but for now, let's just re-calculate offsets?
        // Or assume max 1024 nodes for this simplified method?
        // Let's use re-allocation from arena temporarily if possible?
        // No, 'arena' is borrowed mutably.
        // We can just iterate the edges again, but that's O(E).
        // Let's use a small stack buffer and panic if too large? No, dangerous.
        // We can reuse `parents_offsets` as temp storage again! It's u32.

        // Zero parents_offsets to use as counters
        parents_offsets.fill(0);

        for (edge_idx, &(from_id, to_id, edge_label)) in self.edges.iter().enumerate() {
            if let (Some(from_idx), Some(to_idx)) =
                (self.node_index(from_id), self.node_index(to_id))
            {
                let offset = (children_offsets[from_idx] + parents_offsets[from_idx]) as usize;
                // Safe if offset logic is correct
                if offset < children_data.len() {
                    children_data[offset] = to_idx as u32;
                    parents_offsets[from_idx] += 1;
                }

                // Also copy edge data
                edges[edge_idx * EDGE_STRIDE + EDGE_FROM] = from_idx as u32;
                edges[edge_idx * EDGE_STRIDE + EDGE_TO] = to_idx as u32;

                // Copy edge label if present
                if let Some(lbl) = edge_label {
                    let lbl_bytes = lbl.as_bytes();
                    labels[label_offset..label_offset + lbl_bytes.len()]
                        .copy_from_slice(lbl_bytes);
                    edges[edge_idx * EDGE_STRIDE + EDGE_LABEL_PTR] = label_offset as u32;
                    edges[edge_idx * EDGE_STRIDE + EDGE_LABEL_LEN] = lbl_bytes.len() as u32;
                    label_offset += lbl_bytes.len();
                } else {
                    edges[edge_idx * EDGE_STRIDE + EDGE_LABEL_PTR] = 0;
                    edges[edge_idx * EDGE_STRIDE + EDGE_LABEL_LEN] = 0;
                }
            }
        }

        // Reset parents_offsets (it was used as child_counts)
        parents_offsets.fill(0);

        // Build parents (reverse of children)
        for &(_from_id, to_id, _) in &self.edges {
            if let Some(to_idx) = self.node_index(to_id) {
                parents_offsets[to_idx + 1] += 1;
            }
        }

        for i in 1..=node_count {
            parents_offsets[i] += parents_offsets[i - 1];
        }

        // Fill parents_data
        // We need temp storage again. Since we finished with children setup, can we reuse something?
        // We can reuse children_offsets ONLY if we are careful not to destroy its prefix sums.
        // Actually, we can't reuse children_offsets, we need it for the result.
        // We can alloc a small temp buffer from the arena.
        // We have to use raw pointers again because of borrow checker.
        let (counters_ptr, _) = arena.alloc_raw::<u32>(node_count)?;
        let parent_counts = unsafe { core::slice::from_raw_parts_mut(counters_ptr, node_count) };
        parent_counts.fill(0);

        for &(from_id, to_id, _) in &self.edges {
            if let (Some(from_idx), Some(to_idx)) =
                (self.node_index(from_id), self.node_index(to_id))
            {
                let offset = (parents_offsets[to_idx] + parent_counts[to_idx]) as usize;
                if offset < parents_data.len() {
                    parents_data[offset] = from_idx as u32;
                    parent_counts[to_idx] += 1;
                }
            }
        }

        // Convert to immutable references for the result struct
        let children_offsets: &[u32] = children_offsets;
        let children_data: &[u32] = children_data;
        let parents_offsets: &[u32] = parents_offsets;
        let parents_data: &[u32] = parents_data;

        // Copy subgraph data (labels still mutable for subgraph label copying)
        let (subgraph_data, node_subgraph_slice): (&[usize], &[u32]) = if sg_count > 0 {
            let sg_data = unsafe {
                core::slice::from_raw_parts_mut(sg_data_ptr.unwrap(), sg_count * SUBGRAPH_STRIDE)
            };
            let node_sg = unsafe {
                core::slice::from_raw_parts_mut(node_sg_ptr.unwrap(), node_count)
            };
            node_sg.fill(u32::MAX);

            // Build heap-subgraph-ID → CSR-index mapping
            // Since we're in the alloc feature, we can use Vec
            use alloc::vec::Vec;
            let id_to_idx: Vec<(usize, usize)> = self.subgraphs.iter()
                .enumerate()
                .map(|(i, sg)| (sg.id, i))
                .collect();

            for (sg_idx, sg) in self.subgraphs.iter().enumerate() {
                sg_data[sg_idx * SUBGRAPH_STRIDE + SG_ID] = sg.id;
                sg_data[sg_idx * SUBGRAPH_STRIDE + SG_PARENT_PLUS1] = match sg.parent_id {
                    None => 0,
                    Some(pid) => {
                        // Find parent's CSR index
                        id_to_idx.iter()
                            .find(|&&(id, _)| id == pid)
                            .map(|&(_, idx)| idx + 1)
                            .unwrap_or(0)
                    }
                };
                sg_data[sg_idx * SUBGRAPH_STRIDE + SG_LABEL_PTR] = label_offset;
                sg_data[sg_idx * SUBGRAPH_STRIDE + SG_LABEL_LEN] = sg.label.len();
                if !sg.label.is_empty() {
                    labels[label_offset..label_offset + sg.label.len()]
                        .copy_from_slice(sg.label.as_bytes());
                    label_offset += sg.label.len();
                }
            }

            // Copy node → subgraph mapping
            for (node_idx, &(id, _)) in self.nodes.iter().enumerate() {
                if let Some(&sg_id) = self.node_subgraph.get(&id) {
                    if let Some(&(_, sg_idx)) = id_to_idx.iter().find(|&&(sid, _)| sid == sg_id) {
                        node_sg[node_idx] = sg_idx as u32;
                    }
                }
            }

            (sg_data as &[usize], node_sg as &[u32])
        } else {
            (&[], &[])
        };

        let labels: &[u8] = labels;

        Some(CsrGraph {
            nodes,
            node_count,
            edges,
            edge_count,
            children_offsets,
            children_data,
            parents_offsets,
            parents_data,
            labels,
            subgraph_data,
            subgraph_count: sg_count,
            node_subgraph: node_subgraph_slice,
        })
    }

    /// Estimate the arena size needed for CSR conversion.
    pub fn estimate_csr_arena_size(&self) -> usize {
        let node_label_bytes: usize = self.nodes.iter().map(|(_, label)| label.len()).sum();
        let edge_label_bytes: usize = self.edges.iter()
            .filter_map(|(_, _, label)| label.map(|l| l.len()))
            .sum();
        let sg_label_bytes: usize = self.subgraphs.iter().map(|sg| sg.label.len()).sum();
        CsrGraph::required_arena_size_with_subgraphs(
            self.nodes.len(),
            self.edges.len(),
            node_label_bytes + edge_label_bytes + sg_label_bytes,
            self.subgraphs.len(),
        )
    }
}
