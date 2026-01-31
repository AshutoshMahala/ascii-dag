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

use crate::arena::Arena;

/// Node data stride: fields per node
const NODE_STRIDE: usize = 3;
/// Node field offsets
const NODE_ID: usize = 0;
const NODE_LABEL_PTR: usize = 1;
const NODE_LABEL_LEN: usize = 2;

/// Edge data stride: fields per edge
const EDGE_STRIDE: usize = 2;
/// Edge field offsets
const EDGE_FROM: usize = 0;
const EDGE_TO: usize = 1;

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
}

impl<'a> CsrGraph<'a> {
    /// Calculate required arena size for a graph with given dimensions.
    ///
    /// This helps users pre-allocate the right arena size.
    #[inline]
    pub fn required_arena_size(node_count: usize, edge_count: usize, label_bytes: usize) -> usize {
        let nodes_size = node_count * NODE_STRIDE * core::mem::size_of::<usize>();
        let edges_size = edge_count * EDGE_STRIDE * core::mem::size_of::<u32>();
        let children_offsets_size = (node_count + 1) * core::mem::size_of::<u32>();
        let children_data_size = edge_count * core::mem::size_of::<u32>();
        let parents_offsets_size = (node_count + 1) * core::mem::size_of::<u32>();
        let parents_data_size = edge_count * core::mem::size_of::<u32>();

        // Add alignment padding (estimate 8 bytes per allocation)
        let padding = 6 * 8;

        nodes_size
            + edges_size
            + children_offsets_size
            + children_data_size
            + parents_offsets_size
            + parents_data_size
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

    /// Iterate over all edges as (from_index, to_index) pairs.
    pub fn edges_iter(&self) -> impl Iterator<Item = (usize, usize)> + '_ {
        (0..self.edge_count).map(move |i| self.edge(i))
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
        temp_arena: &mut Arena<'_>,
        output_arena: &'b mut Arena<'b>,
    ) -> Option<crate::ir::arena::LayoutIRArena<'b>> {
        crate::layout::arena::compute_layout_arena_csr(self, temp_arena, output_arena)
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

    // Tracking current progress
    current_node_count: usize,
    current_edge_count: usize,
    current_label_offset: usize,

    // Limits
    max_nodes: usize,
    max_edges: usize,
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

        Some(Self {
            arena,
            nodes,
            edges,
            children_offsets,
            children_data,
            parents_offsets,
            parents_data,
            labels,
            current_node_count: 0,
            current_edge_count: 0,
            current_label_offset: 0,
            max_nodes,
            max_edges,
        })
    }

    /// Add a node to the graph.
    /// Returns the node index (0 to N-1).
    pub fn add_node(&mut self, id: usize, label: &str) -> Option<usize> {
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
        if self.current_edge_count >= self.max_edges {
            return None;
        }

        if from_idx >= self.current_node_count || to_idx >= self.current_node_count {
            return None;
        }

        let idx = self.current_edge_count;

        // Store edge data temporarily in the edges array
        // We cast to u32 here - this panics if user has > 4 billion nodes, which is acceptable
        self.edges[idx * EDGE_STRIDE + EDGE_FROM] = from_idx as u32;
        self.edges[idx * EDGE_STRIDE + EDGE_TO] = to_idx as u32;

        self.current_edge_count += 1;
        Some(())
    }

    /// Finalize the graph construction.
    /// This sorts edges and builds the adjacency lists (children/parents).
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
            current_node_count,
            current_edge_count,
            current_label_offset,
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
        })
    }
}

#[cfg(feature = "alloc")]
impl<'a> crate::DAG<'a> {
    /// Convert heap-based DAG to CSR format using the provided arena.
    ///
    /// This is useful for transitioning to arena mode or for
    /// repeated operations where you want to avoid re-allocation.
    ///
    /// # Example
    ///
    /// ```
    /// use ascii_dag::{DAG, arena::Arena};
    ///
    /// let dag = DAG::from_edges(&[(1, "A"), (2, "B")], &[(1, 2)]);
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
    /// use ascii_dag::{DAG, arena::Arena};
    ///
    /// let dag = DAG::from_edges(&[(1, "A"), (2, "B")], &[(1, 2)]);
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

        // Calculate total label bytes needed
        let total_label_bytes: usize = self.nodes.iter().map(|(_, label)| label.len()).sum();

        // Allocate all memory from arena using raw pointers
        // This avoids the borrow checker issue with multiple mutable borrows
        let (nodes_ptr, _) = arena.alloc_raw::<usize>(node_count * NODE_STRIDE)?;
        let (edges_ptr, _) = arena.alloc_raw::<u32>(edge_count * EDGE_STRIDE)?;
        let (children_offsets_ptr, _) = arena.alloc_raw::<u32>(node_count + 1)?;
        let (children_data_ptr, _) = arena.alloc_raw::<u32>(edge_count)?;
        let (parents_offsets_ptr, _) = arena.alloc_raw::<u32>(node_count + 1)?;
        let (parents_data_ptr, _) = arena.alloc_raw::<u32>(edge_count)?;
        let (labels_ptr, _) = arena.alloc_raw::<u8>(total_label_bytes)?;

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

        for (edge_idx, &(from_id, to_id, _)) in self.edges.iter().enumerate() {
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
        })
    }

    /// Estimate the arena size needed for CSR conversion.
    pub fn estimate_csr_arena_size(&self) -> usize {
        let label_bytes: usize = self.nodes.iter().map(|(_, label)| label.len()).sum();
        CsrGraph::required_arena_size(self.nodes.len(), self.edges.len(), label_bytes)
    }
}
