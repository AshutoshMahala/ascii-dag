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

#[cfg(feature = "ports")]
use crate::algorithms::sugiyama::ports::Port;
use crate::algorithms::sugiyama::ports::PortSide;
use crate::graph::arena::Arena;

/// Node data stride: fields per node
const NODE_STRIDE: usize = 6;
/// Node field offsets
const NODE_ID: usize = 0;
const NODE_LABEL_PTR: usize = 1;
const NODE_LABEL_LEN: usize = 2;
const NODE_WIDTH: usize = 3;
const NODE_HEIGHT: usize = 4;
const NODE_FLAGS: usize = 5;

/// `NODE_FLAGS` bits 1–2: the node's content kind tag (raw
/// `NodeKindTag` value: 0 = simple, 1 = boxed, 2 = custom) — packed
/// into the existing flags word, zero extra stride (D6).
const NODE_TAG_SHIFT: usize = 1;
const NODE_TAG_MASK: usize = 0b11;

/// `NODE_FLAGS` bit: the node was auto-created by an edge reference
/// (`NodeKind::Implicit` in the layout IR — heap parity).
const NODE_FLAG_IMPLICIT: usize = 1;

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

    /// Sparse custom-content entries (graph node index, painter,
    /// payload offset/len into `labels`), sorted by node index. Reuses
    /// the arena-IR entry shape — same fields, different index space.
    custom_nodes: &'a [crate::ir::arena::CustomNodeArena],

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
    /// Declared attachment sides, two encoded bytes per edge
    /// (`[from, to]`), present ONLY for a graph built with ports —
    /// empty otherwise, so an undeclared graph stores nothing.
    edge_ports: &'a [u8],
}

impl<'a> CsrGraph<'a> {
    /// Stored bends the explicit-polyline routes of this graph can
    /// need in the IR output: per ported edge, two per run over the
    /// detour's three runs plus one per jogging level (bounded by the
    /// dummies). Zero without a port table. Mirrors the heap graph's
    /// bound so an estimate made on either side sizes both layouts.
    pub(crate) fn explicit_path_points(&self, dummies: usize) -> usize {
        #[cfg(feature = "ports")]
        {
            if !self.has_ports() {
                return 0;
            }
            let ported = (0..self.edge_ports.len() / 2)
                .filter(|&e| {
                    let (a, b) = self.edge_ports(e);
                    !matches!(a, crate::algorithms::sugiyama::ports::PortSide::Auto)
                        || !matches!(b, crate::algorithms::sugiyama::ports::PortSide::Auto)
                })
                .count();
            ported
                .saturating_mul(6)
                .saturating_add(dummies.saturating_mul(2))
        }
        #[cfg(not(feature = "ports"))]
        {
            let _ = dummies;
            0
        }
    }

    /// Whether this graph carries a port table (built with ports).
    pub fn has_ports(&self) -> bool {
        !self.edge_ports.is_empty()
    }

    /// The declared sides of edge `index` — `Auto`/`Auto` when the
    /// graph carries no port table.
    pub(crate) fn edge_ports(&self, index: usize) -> (PortSide, PortSide) {
        match self.edge_ports.get(index * 2..index * 2 + 2) {
            Some(pair) => (PortSide::from_u8(pair[0]), PortSide::from_u8(pair[1])),
            None => (PortSide::Auto, PortSide::Auto),
        }
    }

    /// Calculate required arena size for a graph with given dimensions.
    ///
    /// This helps users pre-allocate the right arena size.
    #[inline]
    pub fn required_arena_size(node_count: usize, edge_count: usize, label_bytes: usize) -> usize {
        Self::required_arena_size_with_subgraphs(node_count, edge_count, label_bytes, 0)
    }

    /// Like [`required_arena_size_with_content`](Self::required_arena_size_with_content),
    /// plus the port table a builder constructed with ports carries:
    /// two bytes per edge, plus alignment slack.
    #[cfg(feature = "ports")]
    pub fn required_arena_size_with_ports(
        node_count: usize,
        edge_count: usize,
        label_bytes: usize,
        subgraph_count: usize,
        custom_count: usize,
    ) -> usize {
        Self::required_arena_size_with_content(
            node_count,
            edge_count,
            label_bytes,
            subgraph_count,
            custom_count,
        )
        .saturating_add(edge_count.saturating_mul(2))
        .saturating_add(8)
    }

    /// Like [`required_arena_size_with_subgraphs`](Self::required_arena_size_with_subgraphs),
    /// plus capacity for declared node content: `label_bytes` must
    /// already include custom payload bytes (payloads ride the label
    /// storage), and `custom_count` sizes the sparse entry array —
    /// the number of nodes declaring a painter or non-empty payload.
    pub fn required_arena_size_with_content(
        node_count: usize,
        edge_count: usize,
        label_bytes: usize,
        subgraph_count: usize,
        custom_count: usize,
    ) -> usize {
        Self::required_arena_size_with_subgraphs(
            node_count,
            edge_count,
            label_bytes,
            subgraph_count,
        )
        .saturating_add(
            custom_count.saturating_mul(core::mem::size_of::<crate::ir::arena::CustomNodeArena>()),
        )
        .saturating_add(if custom_count == 0 { 0 } else { 16 })
    }

    /// Calculate required arena size including subgraph storage.
    #[inline]
    pub fn required_arena_size_with_subgraphs(
        node_count: usize,
        edge_count: usize,
        label_bytes: usize,
        subgraph_count: usize,
    ) -> usize {
        // Saturating arithmetic: absurd inputs yield a huge (allocation
        // will fail cleanly) size rather than a wrapped-around small one.
        let nodes_size = node_count
            .saturating_mul(NODE_STRIDE)
            .saturating_mul(core::mem::size_of::<usize>());
        let edges_size = edge_count
            .saturating_mul(EDGE_STRIDE)
            .saturating_mul(core::mem::size_of::<u32>());
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
            .saturating_add(edges_size)
            .saturating_add(children_offsets_size)
            .saturating_add(children_data_size)
            .saturating_add(parents_offsets_size)
            .saturating_add(parents_data_size)
            .saturating_add(sg_data_size)
            .saturating_add(node_sg_size)
            .saturating_add(label_bytes)
            .saturating_add(padding)
            .saturating_add(256) // Extra buffer
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

        // SAFETY: we store valid UTF-8 label offsets
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

    /// Whether the node was auto-created by an edge reference (renders
    /// as `NodeKind::Implicit`, matching the heap pipeline).
    #[inline]
    pub fn node_is_implicit(&self, index: usize) -> bool {
        self.nodes[index * NODE_STRIDE + NODE_FLAGS] & NODE_FLAG_IMPLICIT != 0
    }

    /// The node's content kind tag (raw `NodeKindTag` value: 0 =
    /// simple, 1 = boxed, 2 = custom).
    pub fn node_content_tag(&self, index: usize) -> u8 {
        ((self.nodes[index * NODE_STRIDE + NODE_FLAGS] >> NODE_TAG_SHIFT) & NODE_TAG_MASK) as u8
    }

    /// Sparse custom-content entries, sorted by graph node index.
    pub(crate) fn custom_nodes(&self) -> &[crate::ir::arena::CustomNodeArena] {
        self.custom_nodes
    }

    /// A custom entry's payload text.
    pub(crate) fn custom_payload(&self, entry: &crate::ir::arena::CustomNodeArena) -> &str {
        let bytes = &self.labels[entry.payload_offset..entry.payload_offset + entry.payload_len];
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
        (0..self.edge_count).any(|i| self.edges[i * EDGE_STRIDE + EDGE_LABEL_LEN] != 0)
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
            if v == u32::MAX {
                None
            } else {
                Some(v as usize)
            }
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
            if idx >= self.subgraph_count {
                break;
            }
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
    ///
    /// When the CSR came from a `Graph`, size both arenas with
    /// `Graph::estimate_layout_arena_size_with(&config)` (or the
    /// no-argument form for the standard config) — the estimate is an
    /// upper bound, so an exactly-sized buffer always suffices.
    ///
    /// Those estimators live on `Graph` and therefore need the `alloc`
    /// feature. Building directly on `CsrGraphBuilder` in a pure
    /// no-alloc build, there is no equivalent estimator yet: provision
    /// generously and treat `ArenaOom` as the signal to grow, the way
    /// `examples/lean_render.rs` and `examples/longan_nano` do.
    ///
    /// ```
    /// use ascii_dag::{Graph, LayoutConfig, RenderOptions};
    /// use ascii_dag::graph::arena::Arena;
    ///
    /// let g = Graph::from_edges(&[(1, "A"), (2, "B")], &[(1, 2)]);
    /// let config = LayoutConfig::standard();
    ///
    /// let mut csr_buf = vec![0u8; g.estimate_csr_arena_size()];
    /// let mut csr_arena = Arena::new(&mut csr_buf);
    /// let csr = g.to_csr(&mut csr_arena).unwrap();
    ///
    /// let size = g.estimate_layout_arena_size_with(&config);
    /// let (mut t, mut o) = (vec![0u8; size], vec![0u8; size]);
    /// let (mut ta, mut oa) = (Arena::new(&mut t), Arena::new(&mut o));
    ///
    /// let ir = csr.compute_layout_arena(&config, &mut ta, &mut oa).unwrap();
    /// assert!(ir.render_string(&RenderOptions::plain()).contains("[A]"));
    /// ```
    pub fn compute_layout_arena<'b>(
        &self,
        config: &crate::algorithms::sugiyama::config::LayoutConfig<'_>,
        temp_arena: &mut Arena<'_>,
        output_arena: &'b mut Arena<'b>,
    ) -> Result<crate::ir::arena::LayoutIRArena<'b>, crate::errors::GraphError> {
        crate::algorithms::sugiyama::arena_csr::compute_layout_arena_csr(
            self,
            config,
            temp_arena,
            output_arena,
        )
    }

    /// Create a `CsrGraph` from node and edge slices (batch construction).
    ///
    /// Edges use **node IDs** (not indices), matching the `Graph::from_edges` API.
    /// Returns `None` if the arena is too small or an edge references an unknown ID.
    ///
    /// # Examples
    ///
    /// ```
    /// use ascii_dag::graph::arena::Arena;
    /// use ascii_dag::graph::csr::CsrGraph;
    ///
    /// let size = CsrGraph::required_arena_size(3, 2, 10);
    /// let mut buf = vec![0u8; size];
    /// let mut arena = Arena::new(&mut buf);
    /// let csr = CsrGraph::from_edges(
    ///     &mut arena,
    ///     &[(1, "A"), (2, "B"), (3, "C")],
    ///     &[(1, 2), (2, 3)],
    /// ).unwrap();
    /// assert_eq!(csr.node_count(), 3);
    /// assert_eq!(csr.edge_count(), 2);
    /// ```
    pub fn from_edges(
        arena: &'a mut Arena<'a>,
        nodes: &[(usize, &str)],
        edges: &[(usize, usize)],
    ) -> Option<CsrGraph<'a>> {
        let label_bytes: usize = nodes.iter().map(|(_, l)| l.len()).sum();
        let mut builder =
            CsrGraphBuilder::new(arena, nodes.len(), edges.len(), label_bytes + 64, 0)?;

        for &(id, label) in nodes {
            builder.add_node(id, label)?;
        }

        // Map IDs to indices for edges
        for &(from_id, to_id) in edges {
            let from_idx = nodes.iter().position(|&(id, _)| id == from_id)?;
            let to_idx = nodes.iter().position(|&(id, _)| id == to_id)?;
            builder.add_edge(from_idx, to_idx)?;
        }

        builder.build()
    }

    /// Create a `CsrGraph` from node and labeled edge slices (batch construction).
    ///
    /// Edges use **node IDs** (not indices), matching the `Graph::from_edges_labeled` API.
    /// Returns `None` if the arena is too small or an edge references an unknown ID.
    ///
    /// # Examples
    ///
    /// ```
    /// use ascii_dag::graph::arena::Arena;
    /// use ascii_dag::graph::csr::CsrGraph;
    ///
    /// let size = CsrGraph::required_arena_size(3, 2, 30);
    /// let mut buf = vec![0u8; size];
    /// let mut arena = Arena::new(&mut buf);
    /// let csr = CsrGraph::from_edges_labeled(
    ///     &mut arena,
    ///     &[(1, "A"), (2, "B"), (3, "C")],
    ///     &[(1, 2, Some("uses")), (2, 3, None)],
    /// ).unwrap();
    /// assert_eq!(csr.node_count(), 3);
    /// assert_eq!(csr.edge_count(), 2);
    /// ```
    pub fn from_edges_labeled(
        arena: &'a mut Arena<'a>,
        nodes: &[(usize, &str)],
        edges: &[(usize, usize, Option<&str>)],
    ) -> Option<CsrGraph<'a>> {
        let node_label_bytes: usize = nodes.iter().map(|(_, l)| l.len()).sum();
        let edge_label_bytes: usize = edges.iter().map(|(_, _, l)| l.map_or(0, |s| s.len())).sum();
        let label_bytes = node_label_bytes + edge_label_bytes;
        let mut builder =
            CsrGraphBuilder::new(arena, nodes.len(), edges.len(), label_bytes + 64, 0)?;

        for &(id, label) in nodes {
            builder.add_node(id, label)?;
        }

        for &(from_id, to_id, label) in edges {
            let from_idx = nodes.iter().position(|&(id, _)| id == from_id)?;
            let to_idx = nodes.iter().position(|&(id, _)| id == to_id)?;
            builder.add_edge_with_label(from_idx, to_idx, label.unwrap_or(""))?;
        }

        builder.build()
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

    // Sparse custom-content entries (graph node index order)
    custom_nodes: &'a mut [crate::ir::arena::CustomNodeArena],

    // Declared attachment sides, two encoded bytes per edge —
    // PREALLOCATED by the with-ports constructor (setters never
    // allocate); empty, with `ports == false`, otherwise.
    edge_ports: &'a mut [u8],
    #[cfg_attr(not(feature = "ports"), allow(dead_code))] // written only by `new_with_ports`
    ports: bool,

    // Tracking current progress
    current_node_count: usize,
    current_edge_count: usize,
    current_label_offset: usize,
    current_subgraph_count: usize,
    current_custom_count: usize,

    // Limits
    max_nodes: usize,
    max_edges: usize,
    max_subgraphs: usize,
}

impl<'a> CsrGraphBuilder<'a> {
    /// Create a new builder with known maximum graph dimensions.
    /// `max_label_bytes` must also cover custom payload bytes (payloads
    /// ride the label storage); `max_custom` sizes the sparse
    /// custom-content entry array — pass 0 when no node declares a
    /// painter or payload.
    pub fn new(
        arena: &'a mut Arena<'a>,
        max_nodes: usize,
        max_edges: usize,
        max_label_bytes: usize,
        max_custom: usize,
    ) -> Option<Self> {
        // Allocate all memory from arena using raw pointers. Counts are
        // pre-multiplied with checked arithmetic — adversarial sizes
        // fail the allocation instead of wrapping.
        let (nodes_ptr, _) = arena.alloc_raw::<usize>(max_nodes.checked_mul(NODE_STRIDE)?)?;
        let (edges_ptr, _) = arena.alloc_raw::<u32>(max_edges.checked_mul(EDGE_STRIDE)?)?;
        let (children_offsets_ptr, _) = arena.alloc_raw::<u32>(max_nodes.checked_add(1)?)?;
        let (children_data_ptr, _) = arena.alloc_raw::<u32>(max_edges)?;
        let (parents_offsets_ptr, _) = arena.alloc_raw::<u32>(max_nodes + 1)?;
        let (parents_data_ptr, _) = arena.alloc_raw::<u32>(max_edges)?;
        let (labels_ptr, _) = arena.alloc_raw::<u8>(max_label_bytes)?;
        let custom_ptr = if max_custom > 0 {
            Some(
                arena
                    .alloc_raw::<crate::ir::arena::CustomNodeArena>(max_custom)?
                    .0,
            )
        } else {
            None
        };

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
        // SAFETY: alloc_raw zeroed the memory; all-zero is a valid
        // CustomNodeArena (`Option<fn>` is null-pointer optimized).
        let custom_nodes: &'a mut [crate::ir::arena::CustomNodeArena] = match custom_ptr {
            Some(ptr) => unsafe { core::slice::from_raw_parts_mut(ptr, max_custom) },
            None => &mut [],
        };

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
            custom_nodes,
            edge_ports: &mut [],
            ports: false,
            current_node_count: 0,
            current_edge_count: 0,
            current_label_offset: 0,
            current_subgraph_count: 0,
            current_custom_count: 0,
            max_nodes,
            max_edges,
            max_subgraphs: 0,
        })
    }

    /// Create a builder that will carry PORT declarations: the full
    /// form (subgraphs, custom content) plus a preallocated port table
    /// of two bytes per edge — so `from_port`/`to_port` on the handles
    /// this builder's `add_edge` returns can never fail and never
    /// allocate. Size the arena with
    /// [`CsrGraph::required_arena_size_with_ports`].
    #[cfg(feature = "ports")]
    pub fn new_with_ports(
        arena: &'a mut Arena<'a>,
        max_nodes: usize,
        max_edges: usize,
        max_label_bytes: usize,
        max_subgraphs: usize,
        max_custom: usize,
    ) -> Option<Self> {
        let mut builder = Self::new_with_subgraphs(
            arena,
            max_nodes,
            max_edges,
            max_label_bytes,
            max_subgraphs,
            max_custom,
        )?;
        let (ports_ptr, _) = builder.arena.alloc_raw::<u8>(max_edges.checked_mul(2)?)?;
        // SAFETY: alloc_raw validated and zeroed the bytes; zero encodes
        // `Auto`, so an undeclared edge reads as "no declaration".
        builder.edge_ports = unsafe { core::slice::from_raw_parts_mut(ports_ptr, max_edges * 2) };
        builder.ports = true;
        Some(builder)
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
        max_custom: usize,
    ) -> Option<Self> {
        let (nodes_ptr, _) = arena.alloc_raw::<usize>(max_nodes * NODE_STRIDE)?;
        let (edges_ptr, _) = arena.alloc_raw::<u32>(max_edges * EDGE_STRIDE)?;
        let (children_offsets_ptr, _) = arena.alloc_raw::<u32>(max_nodes + 1)?;
        let (children_data_ptr, _) = arena.alloc_raw::<u32>(max_edges)?;
        let (parents_offsets_ptr, _) = arena.alloc_raw::<u32>(max_nodes + 1)?;
        let (parents_data_ptr, _) = arena.alloc_raw::<u32>(max_edges)?;
        let (labels_ptr, _) = arena.alloc_raw::<u8>(max_label_bytes)?;
        let custom_ptr = if max_custom > 0 {
            Some(
                arena
                    .alloc_raw::<crate::ir::arena::CustomNodeArena>(max_custom)?
                    .0,
            )
        } else {
            None
        };
        let (sg_data_ptr, _) = arena.alloc_raw::<usize>(max_subgraphs * SUBGRAPH_STRIDE)?;
        let (node_sg_ptr, _) = arena.alloc_raw::<u32>(max_nodes)?;

        let (
            nodes,
            edges,
            children_offsets,
            children_data,
            parents_offsets,
            parents_data,
            labels,
            subgraph_data,
            node_subgraph,
        ) = unsafe {
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
        // SAFETY: alloc_raw zeroed the memory; all-zero is a valid
        // CustomNodeArena (`Option<fn>` is null-pointer optimized).
        let custom_nodes: &'a mut [crate::ir::arena::CustomNodeArena] = match custom_ptr {
            Some(ptr) => unsafe { core::slice::from_raw_parts_mut(ptr, max_custom) },
            None => &mut [],
        };

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
            custom_nodes,
            edge_ports: &mut [],
            ports: false,
            current_node_count: 0,
            current_edge_count: 0,
            current_label_offset: 0,
            current_subgraph_count: 0,
            current_custom_count: 0,
            max_nodes,
            max_edges,
            max_subgraphs,
        })
    }

    /// Add a node to the graph. Returns the node index (0 to N-1).
    ///
    /// Accepts anything implementing `NodeContent` — a bare `&str`,
    /// a built-in `SimpleNode`/`BoxedNode`, or a custom declaration
    /// whose painter/payload are carried through the arena pipeline
    /// (payload bytes come out of `max_label_bytes`; entries out of
    /// `max_custom`). Sizing matches `Graph::add_node` exactly —
    /// character-based (NC9 parity: the same declaration renders
    /// byte-identically whether built here or via `Graph → to_csr`).
    ///
    /// Failure-atomic: on `None` (any capacity exhausted) the builder
    /// is unchanged.
    pub fn add_node<'c>(
        &mut self,
        id: usize,
        node: impl crate::render::engine::NodeContent<'c>,
    ) -> Option<usize> {
        let label = node.label();
        let (width, height) = node.size();
        let kind = node.kind();
        let painter = node.painter();
        let payload = node.payload();
        let has_custom = painter.is_some() || !payload.is_empty();
        // Preflight EVERY capacity before committing anything — a
        // failed insertion must leave the builder untouched.
        if self.current_node_count >= self.max_nodes {
            return None;
        }
        if self.current_label_offset + label.len() + payload.len() > self.labels.len() {
            return None;
        }
        if has_custom && self.current_custom_count >= self.custom_nodes.len() {
            return None;
        }
        let idx = self.add_node_with_size(id, label, width, height)?;
        // Overwrite the tag written by add_node_with_size (Simple).
        self.nodes[idx * NODE_STRIDE + NODE_FLAGS] =
            (kind.to_u8() as usize & NODE_TAG_MASK) << NODE_TAG_SHIFT;
        if has_custom {
            let bytes = payload.as_bytes();
            self.labels[self.current_label_offset..self.current_label_offset + bytes.len()]
                .copy_from_slice(bytes);
            self.custom_nodes[self.current_custom_count] = crate::ir::arena::CustomNodeArena {
                node_idx: idx,
                painter,
                payload_offset: self.current_label_offset,
                payload_len: bytes.len(),
            };
            self.current_label_offset += bytes.len();
            self.current_custom_count += 1;
        }
        Some(idx)
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
        // Direct-built CSR graphs declare every node — none is implicit.
        self.nodes[idx * NODE_STRIDE + NODE_FLAGS] = 0;

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
    pub fn add_edge(&mut self, from_idx: usize, to_idx: usize) -> Option<CsrEdgeHandle<'_, 'a>> {
        self.add_edge_with_label(from_idx, to_idx, "")
    }

    /// Add a labeled edge between two node INDICES. The returned handle
    /// carries the edge index and the port declarations for a builder
    /// constructed with ports; `None` when the edge does not fit.
    pub fn add_edge_with_label(
        &mut self,
        from_idx: usize,
        to_idx: usize,
        label: &str,
    ) -> Option<CsrEdgeHandle<'_, 'a>> {
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
        Some(CsrEdgeHandle {
            builder: self,
            edge: idx,
        })
    }

    /// Declare the sides edge `edge` (by index) attaches to. `None`
    /// when the edge does not exist or the builder was constructed
    /// without a port table — never an allocation.
    #[cfg(feature = "ports")]
    #[cfg_attr(not(test), allow(dead_code))] // exercised by the layout tests until the API lands
    pub(crate) fn set_edge_ports(
        &mut self,
        edge: usize,
        source: PortSide,
        target: PortSide,
    ) -> Option<()> {
        if !self.ports || edge >= self.current_edge_count {
            return None;
        }
        self.edge_ports[edge * 2] = source.to_u8();
        self.edge_ports[edge * 2 + 1] = target.to_u8();
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
    /// Consume the builder and produce a finished [`CsrGraph`], or `None` if the arena is exhausted.
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
            custom_nodes,
            edge_ports,
            ports,
            current_node_count,
            current_edge_count,
            current_label_offset,
            current_subgraph_count,
            current_custom_count,
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
            custom_nodes: &custom_nodes[..current_custom_count],
            subgraph_data: &subgraph_data[..current_subgraph_count * SUBGRAPH_STRIDE],
            subgraph_count: current_subgraph_count,
            node_subgraph: if current_subgraph_count > 0 {
                &node_subgraph[..node_count]
            } else {
                &[]
            },
            edge_ports: if ports {
                &edge_ports[..edge_count * 2]
            } else {
                &[]
            },
        })
    }
}

/// What [`CsrGraphBuilder::add_edge`] returns: the edge's index plus
/// eager port declarations — the CSR twin of the heap graph's
/// `EdgeHandle`, with the same names. Setters return `None` only for
/// a builder constructed WITHOUT a port table (the builder's uniform
/// `Option` idiom for a configuration failure); on a builder
/// constructed with ports they cannot fail, because the table was
/// preallocated and no setter ever allocates.
pub struct CsrEdgeHandle<'b, 'a> {
    #[cfg_attr(not(feature = "ports"), allow(dead_code))] // read only by the port setters
    builder: &'b mut CsrGraphBuilder<'a>,
    edge: usize,
}

impl core::fmt::Debug for CsrEdgeHandle<'_, '_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("CsrEdgeHandle")
            .field("edge", &self.edge)
            .finish_non_exhaustive()
    }
}

impl CsrEdgeHandle<'_, '_> {
    /// The inserted edge's index.
    pub fn edge(&self) -> usize {
        self.edge
    }

    /// Declare the side the edge leaves its `from` node from.
    #[cfg(feature = "ports")]
    #[allow(clippy::wrong_self_convention)] // mirrors add_edge(from, to); not a constructor
    #[cfg_attr(not(test), allow(dead_code))] // exercised by the layout tests until the API lands
    pub(crate) fn from_port(self, port: impl Into<Port>) -> Option<Self> {
        if !self.builder.ports {
            return None;
        }
        self.builder.edge_ports[self.edge * 2] = port.into().side().to_u8();
        Some(self)
    }

    /// Declare the side the edge arrives at its `to` node on.
    #[cfg(feature = "ports")]
    #[allow(clippy::wrong_self_convention)] // mirrors add_edge(from, to); not a conversion
    #[cfg_attr(not(test), allow(dead_code))] // exercised by the layout tests until the API lands
    pub(crate) fn to_port(self, port: impl Into<Port>) -> Option<Self> {
        if !self.builder.ports {
            return None;
        }
        self.builder.edge_ports[self.edge * 2 + 1] = port.into().side().to_u8();
        Some(self)
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
        let edge_label_bytes: usize = self
            .edges
            .iter()
            .filter_map(|(_, _, label)| label.map(|l| l.len()))
            .sum();
        let sg_label_bytes: usize = self.subgraphs.iter().map(|sg| sg.label.len()).sum();
        // Custom payloads ride the label storage, like labels.
        let payload_bytes: usize = self.node_custom.iter().map(|entry| entry.2.len()).sum();
        let total_label_bytes: usize =
            node_label_bytes + edge_label_bytes + sg_label_bytes + payload_bytes;

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
        let custom_count = self.node_custom.len();
        let custom_ptr = if custom_count > 0 {
            Some(
                arena
                    .alloc_raw::<crate::ir::arena::CustomNodeArena>(custom_count)?
                    .0,
            )
        } else {
            None
        };

        // Convert raw pointers to slices
        // SAFETY: alloc_raw has validated the allocations and zeroed the memory
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
            nodes[idx * NODE_STRIDE + NODE_FLAGS] = {
                let implicit = if self.auto_created.contains(&id) {
                    NODE_FLAG_IMPLICIT
                } else {
                    0
                };
                let tag = (self.node_kind_tag[idx] as usize & NODE_TAG_MASK) << NODE_TAG_SHIFT;
                implicit | tag
            };

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
                    labels[label_offset..label_offset + lbl_bytes.len()].copy_from_slice(lbl_bytes);
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

        // Port table: only a graph that declared ports carries one
        // (the heap table is empty until the first declaration).
        #[cfg(not(feature = "ports"))]
        let edge_ports_slice: &[u8] = &[];
        #[cfg(feature = "ports")]
        let edge_ports_slice: &[u8] = if self.edge_ports.is_empty() {
            &[]
        } else {
            let (ports_ptr, _) = arena.alloc_raw::<u8>(edge_count * 2)?;
            // SAFETY: alloc_raw validated and zeroed `edge_count * 2` bytes.
            let table = unsafe { core::slice::from_raw_parts_mut(ports_ptr, edge_count * 2) };
            for (edge_idx, &(src, dst)) in self.edge_ports.iter().enumerate() {
                table[edge_idx * 2] = src.to_u8();
                table[edge_idx * 2 + 1] = dst.to_u8();
            }
            table
        };

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
            let node_sg =
                unsafe { core::slice::from_raw_parts_mut(node_sg_ptr.unwrap(), node_count) };
            node_sg.fill(u32::MAX);

            // Build heap-subgraph-ID → CSR-index mapping
            // Since we're in the alloc feature, we can use Vec
            use alloc::vec::Vec;
            let id_to_idx: Vec<(usize, usize)> = self
                .subgraphs
                .iter()
                .enumerate()
                .map(|(i, sg)| (sg.id, i))
                .collect();

            for (sg_idx, sg) in self.subgraphs.iter().enumerate() {
                sg_data[sg_idx * SUBGRAPH_STRIDE + SG_ID] = sg.id;
                sg_data[sg_idx * SUBGRAPH_STRIDE + SG_PARENT_PLUS1] = match sg.parent_id {
                    None => 0,
                    Some(pid) => {
                        // Find parent's CSR index
                        id_to_idx
                            .iter()
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

        // Copy custom payloads into label storage and record entries
        // (sorted: node_custom is sorted by node index already).
        let custom_nodes: &[crate::ir::arena::CustomNodeArena] = if custom_count > 0 {
            // SAFETY: alloc_raw zeroed the memory, and all-zero is a
            // valid CustomNodeArena (`Option<fn>` is null-pointer
            // optimized: None = 0); every element is overwritten below.
            let entries =
                unsafe { core::slice::from_raw_parts_mut(custom_ptr.unwrap(), custom_count) };
            for (i, &(node_idx, painter, payload)) in self.node_custom.iter().enumerate() {
                let bytes = payload.as_bytes();
                labels[label_offset..label_offset + bytes.len()].copy_from_slice(bytes);
                entries[i] = crate::ir::arena::CustomNodeArena {
                    node_idx,
                    painter,
                    payload_offset: label_offset,
                    payload_len: bytes.len(),
                };
                label_offset += bytes.len();
            }
            entries
        } else {
            &[]
        };
        let _ = label_offset;
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
            custom_nodes,
            subgraph_data,
            subgraph_count: sg_count,
            node_subgraph: node_subgraph_slice,
            edge_ports: edge_ports_slice,
        })
    }

    /// Estimate the arena size needed for CSR conversion.
    pub fn estimate_csr_arena_size(&self) -> usize {
        let node_label_bytes: usize = self.nodes.iter().map(|(_, label)| label.len()).sum();
        let edge_label_bytes: usize = self
            .edges
            .iter()
            .filter_map(|(_, _, label)| label.map(|l| l.len()))
            .sum();
        let sg_label_bytes: usize = self.subgraphs.iter().map(|sg| sg.label.len()).sum();
        let payload_bytes: usize = self.node_custom.iter().map(|entry| entry.2.len()).sum();
        CsrGraph::required_arena_size_with_subgraphs(
            self.nodes.len(),
            self.edges.len(),
            node_label_bytes + edge_label_bytes + sg_label_bytes + payload_bytes,
            self.subgraphs.len(),
        )
        .saturating_add(
            self.node_custom
                .len()
                .saturating_mul(core::mem::size_of::<crate::ir::arena::CustomNodeArena>()),
        )
        .saturating_add(if self.node_custom.is_empty() { 0 } else { 16 })
        .saturating_add(self.csr_port_table_bytes())
    }

    /// The port table's arena bytes — only when something was declared
    /// (and only with the `ports` feature at all).
    fn csr_port_table_bytes(&self) -> usize {
        #[cfg(feature = "ports")]
        {
            if self.edge_ports.is_empty() {
                0
            } else {
                self.edges.len().saturating_mul(2).saturating_add(8)
            }
        }
        #[cfg(not(feature = "ports"))]
        {
            0
        }
    }
}
