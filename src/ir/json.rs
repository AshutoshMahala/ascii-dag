//! JSON serialization for Layout IR (schema v1.3).
//!
//! Produces JSON output extending zigraph's JSON IR schema: v1.3 adds
//! the per-edge `flow_axis` tag, the per-node `self_loop_at` cell, and
//! renames the path fields axis-neutrally (`bend_at`, `channel_at`,
//! `span_start`/`span_end` — a v1.2 `horizontal_y` is a v1.3 `bend_at`
//! on a `flow_axis: "y"` edge).
//!
//! # Schema
//!
//! - `version`: always `"1.3"`
//! - `width`, `height`, `level_count`: top-level dimensions
//! - `nodes[]`: positioned node objects with `id`, `label`, `x`, `y`, etc.
//! - `edges[]`: routed edge objects with `from`, `to`, path info, etc.
//! - `subgraphs[]`: bounding boxes (omitted when empty)
//!
//! # Features
//!
//! - **`alloc`**: Enables `LayoutIR::to_json() -> String`
//! - **Always available**: `serialize_json_to_buffer()` for `LayoutIRArena`

/// JSON schema version (zigraph v1.2 + the direction extensions).
pub(crate) const VERSION: &str = "1.3";

// ── Minimal no-alloc JSON writer ─────────────────────────────────────────

/// A tiny JSON writer that appends to a `&mut [u8]` buffer.
/// Returns `None` on overflow.
struct JsonWriter<'b> {
    buf: &'b mut [u8],
    pos: usize,
}

impl<'b> JsonWriter<'b> {
    fn new(buf: &'b mut [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    /// Bytes written so far.
    #[inline]
    fn len(&self) -> usize {
        self.pos
    }

    #[inline]
    fn write_byte(&mut self, b: u8) -> Option<()> {
        if self.pos < self.buf.len() {
            self.buf[self.pos] = b;
            self.pos += 1;
            Some(())
        } else {
            None
        }
    }

    #[inline]
    fn write_bytes(&mut self, bytes: &[u8]) -> Option<()> {
        let end = self.pos + bytes.len();
        if end <= self.buf.len() {
            self.buf[self.pos..end].copy_from_slice(bytes);
            self.pos = end;
            Some(())
        } else {
            None
        }
    }

    /// Write a JSON string (with escaping for `"`, `\`, and control chars).
    fn write_str(&mut self, s: &str) -> Option<()> {
        self.write_byte(b'"')?;
        for &b in s.as_bytes() {
            match b {
                b'"' => {
                    self.write_byte(b'\\')?;
                    self.write_byte(b'"')?;
                }
                b'\\' => {
                    self.write_byte(b'\\')?;
                    self.write_byte(b'\\')?;
                }
                b'\n' => {
                    self.write_byte(b'\\')?;
                    self.write_byte(b'n')?;
                }
                b'\r' => {
                    self.write_byte(b'\\')?;
                    self.write_byte(b'r')?;
                }
                b'\t' => {
                    self.write_byte(b'\\')?;
                    self.write_byte(b't')?;
                }
                0..=0x1f => {
                    // \u00XX for other control chars
                    self.write_bytes(b"\\u00")?;
                    let hi = b >> 4;
                    let lo = b & 0x0f;
                    self.write_byte(if hi < 10 { b'0' + hi } else { b'a' + hi - 10 })?;
                    self.write_byte(if lo < 10 { b'0' + lo } else { b'a' + lo - 10 })?;
                }
                _ => {
                    self.write_byte(b)?;
                }
            }
        }
        self.write_byte(b'"')
    }

    /// Write a `usize` as decimal digits.
    fn write_usize(&mut self, n: usize) -> Option<()> {
        if n == 0 {
            return self.write_byte(b'0');
        }
        // Max usize digits: 20
        let mut tmp = [0u8; 20];
        let mut i = 20;
        let mut val = n;
        while val > 0 {
            i -= 1;
            tmp[i] = b'0' + (val % 10) as u8;
            val /= 10;
        }
        self.write_bytes(&tmp[i..])
    }

    fn write_bool(&mut self, b: bool) -> Option<()> {
        if b {
            self.write_bytes(b"true")
        } else {
            self.write_bytes(b"false")
        }
    }

    fn write_null(&mut self) -> Option<()> {
        self.write_bytes(b"null")
    }

    /// `"key":value` (no space after colon for compactness).
    fn write_key(&mut self, key: &str) -> Option<()> {
        self.write_str(key)?;
        self.write_byte(b':')
    }
}

// ── Arena path: serialize to byte buffer ─────────────────────────────────

use super::NodeKind;
use super::arena::{
    EdgePathArena, LayoutEdgeArena, LayoutIRArena, LayoutNodeArena, SubgraphInfoArena,
};

impl<'a> LayoutIRArena<'a> {
    /// Serialize the layout IR to JSON (schema v1.3 — zigraph v1.2
    /// plus the direction extensions; see the module docs).
    ///
    /// Writes into `buffer` and returns the number of bytes written,
    /// or `None` if the buffer is too small.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # #[cfg(feature = "arena")]
    /// # fn example(ir: &ascii_dag::ir::arena::LayoutIRArena<'_>) {
    /// let mut buf = [0u8; 4096];
    /// if let Some(len) = ir.serialize_json(&mut buf) {
    ///     let json = core::str::from_utf8(&buf[..len]).unwrap();
    ///     // json follows the v1.3 schema
    /// }
    /// # }
    /// ```
    pub fn serialize_json(&self, buffer: &mut [u8]) -> Option<usize> {
        let mut w = JsonWriter::new(buffer);
        w.write_byte(b'{')?;

        // version
        w.write_key("version")?;
        w.write_str(VERSION)?;
        w.write_byte(b',')?;

        // dimensions
        w.write_key("width")?;
        w.write_usize(self.width())?;
        w.write_byte(b',')?;
        w.write_key("height")?;
        w.write_usize(self.height())?;
        w.write_byte(b',')?;
        w.write_key("level_count")?;
        w.write_usize(self.level_count())?;
        w.write_byte(b',')?;

        // nodes
        w.write_key("nodes")?;
        w.write_byte(b'[')?;
        for (i, node) in self.nodes().iter().enumerate() {
            if i > 0 {
                w.write_byte(b',')?;
            }
            let payload = if node.content_tag == 2 {
                Some(
                    match self
                        .custom_nodes()
                        .binary_search_by_key(&i, |entry| entry.node_idx)
                    {
                        Ok(pos) => self.custom_payload(&self.custom_nodes()[pos]),
                        Err(_) => "",
                    },
                )
            } else {
                None
            };
            write_node_arena(&mut w, node, self.node_label(i), payload)?;
        }
        w.write_byte(b']')?;
        w.write_byte(b',')?;

        // edges
        w.write_key("edges")?;
        w.write_byte(b'[')?;
        for (i, edge) in self.edges().iter().enumerate() {
            if i > 0 {
                w.write_byte(b',')?;
            }
            let label = if edge.label_len > 0 {
                Some(self.edge_label(i))
            } else {
                None
            };
            write_edge_arena(&mut w, edge, label, self)?;
        }
        w.write_byte(b']')?;

        // subgraphs (omit when empty, per v1.2 spec)
        if self.subgraph_count() > 0 {
            w.write_byte(b',')?;
            w.write_key("subgraphs")?;
            w.write_byte(b'[')?;
            for (i, sg) in self.subgraphs().iter().enumerate() {
                if i > 0 {
                    w.write_byte(b',')?;
                }
                write_subgraph_arena(&mut w, sg, self.subgraph_label(i))?;
            }
            w.write_byte(b']')?;
        }

        w.write_byte(b'}')?;
        Some(w.len())
    }

    /// Estimate the buffer size needed for JSON serialization.
    ///
    /// Returns a conservative upper bound. The actual output is usually smaller.
    pub fn estimate_json_size(&self) -> usize {
        // Base: {"version":"1.3","width":N,"height":N,"level_count":N,"nodes":[...],"edges":[...]}
        let base: usize = 100;
        // Each node: fixed fields incl. `content_kind` and the v1.3
        // `self_loop_at` cell (~210 bytes) plus its label at
        // worst-case JSON escaping (6 bytes per input byte — \u00XX
        // form).
        let nodes = self.node_count().saturating_mul(210);
        let node_labels: usize = (0..self.node_count())
            .map(|i| self.node_label(i).len().saturating_mul(6))
            .fold(0usize, |a, b| a.saturating_add(b));
        // Custom payloads: key/quotes overhead + escaped bytes.
        let payloads: usize = self
            .custom_nodes()
            .iter()
            .map(|entry| entry.payload_len.saturating_mul(6).saturating_add(16))
            .fold(0usize, |a, b| a.saturating_add(b));
        // Each edge: fixed fields + path + the v1.3 `flow_axis` tag
        // (~220 bytes) plus its label at worst-case escaping.
        let edges = self.edge_count().saturating_mul(220);
        let edge_labels: usize = (0..self.edge_count())
            .map(|i| self.edge_label(i).len().saturating_mul(6))
            .fold(0usize, |a, b| a.saturating_add(b));
        // Each subgraph: fixed fields (~120 bytes) plus escaped label.
        let sgs = self.subgraph_count().saturating_mul(120);
        let sg_labels: usize = (0..self.subgraph_count())
            .map(|i| self.subgraph_label(i).len().saturating_mul(6))
            .fold(0usize, |a, b| a.saturating_add(b));
        // Waypoints: ~20 bytes each
        let wps: usize = self
            .edges()
            .iter()
            .map(|e| match e.path {
                EdgePathArena::MultiSegment { waypoints_len, .. } => {
                    waypoints_len.saturating_mul(20)
                }
                _ => 0,
            })
            .fold(0usize, |a, b| a.saturating_add(b));
        base.saturating_add(nodes)
            .saturating_add(node_labels)
            .saturating_add(payloads)
            .saturating_add(edges)
            .saturating_add(edge_labels)
            .saturating_add(sgs)
            .saturating_add(sg_labels)
            .saturating_add(wps)
    }
}

fn write_node_arena(
    w: &mut JsonWriter<'_>,
    node: &LayoutNodeArena,
    label: &str,
    payload: Option<&str>,
) -> Option<()> {
    w.write_byte(b'{')?;

    w.write_key("id")?;
    w.write_usize(node.id)?;
    w.write_byte(b',')?;

    w.write_key("label")?;
    w.write_str(label)?;
    w.write_byte(b',')?;

    w.write_key("x")?;
    w.write_usize(node.x)?;
    w.write_byte(b',')?;
    w.write_key("y")?;
    w.write_usize(node.y)?;
    w.write_byte(b',')?;
    w.write_key("width")?;
    w.write_usize(node.width)?;
    w.write_byte(b',')?;
    w.write_key("height")?;
    w.write_usize(node.height)?;
    w.write_byte(b',')?;
    w.write_key("center_x")?;
    w.write_usize(node.center_x)?;
    w.write_byte(b',')?;
    w.write_key("center_y")?;
    w.write_usize(node.center_y)?;
    w.write_byte(b',')?;
    w.write_key("level")?;
    w.write_usize(node.level)?;
    w.write_byte(b',')?;
    w.write_key("level_position")?;
    w.write_usize(node.level_position)?;
    w.write_byte(b',')?;

    w.write_key("kind")?;
    match node.kind {
        NodeKind::Explicit => w.write_str("explicit")?,
        NodeKind::Implicit => w.write_str("implicit")?,
        NodeKind::Dummy => w.write_str("dummy")?,
    }
    w.write_byte(b',')?;

    // Declared content kind — a separate field from `kind`, which is
    // the layout classification (explicit/implicit/dummy). `payload`
    // is emitted only for custom nodes (their painter is code and
    // never serializes — data only).
    w.write_key("content_kind")?;
    w.write_str(match node.content_tag {
        1 => "boxed",
        2 => "custom",
        _ => "simple",
    })?;
    w.write_byte(b',')?;
    if let Some(payload) = payload {
        w.write_key("payload")?;
        w.write_str(payload)?;
        w.write_byte(b',')?;
    }

    // v1.3: the self-loop marker cell (only when the node has one).
    if node.self_loop_at != (usize::MAX, usize::MAX) {
        w.write_key("self_loop_at")?;
        w.write_byte(b'[')?;
        w.write_usize(node.self_loop_at.0)?;
        w.write_byte(b',')?;
        w.write_usize(node.self_loop_at.1)?;
        w.write_byte(b']')?;
        w.write_byte(b',')?;
    }

    w.write_key("edge_index")?;
    if node.edge_index != usize::MAX {
        w.write_usize(node.edge_index)?;
    } else {
        w.write_null()?;
    }

    w.write_byte(b'}')
}

fn write_edge_arena(
    w: &mut JsonWriter<'_>,
    edge: &LayoutEdgeArena,
    label: Option<&str>,
    ir: &LayoutIRArena<'_>,
) -> Option<()> {
    w.write_byte(b'{')?;

    w.write_key("from")?;
    w.write_usize(edge.from_id)?;
    w.write_byte(b',')?;
    w.write_key("to")?;
    w.write_usize(edge.to_id)?;
    w.write_byte(b',')?;

    w.write_key("from_x")?;
    w.write_usize(edge.from_x)?;
    w.write_byte(b',')?;
    w.write_key("from_y")?;
    w.write_usize(edge.from_y)?;
    w.write_byte(b',')?;
    w.write_key("to_x")?;
    w.write_usize(edge.to_x)?;
    w.write_byte(b',')?;
    w.write_key("to_y")?;
    w.write_usize(edge.to_y)?;
    w.write_byte(b',')?;

    w.write_key("edge_index")?;
    w.write_usize(edge.edge_index)?;
    w.write_byte(b',')?;

    w.write_key("directed")?;
    w.write_bool(edge.directed)?;
    w.write_byte(b',')?;

    // v1.3: which physical axis the trunk runs along.
    w.write_key("flow_axis")?;
    w.write_str(match edge.flow_axis {
        crate::ir::FlowAxis::Y => "y",
        crate::ir::FlowAxis::X => "x",
    })?;

    // reversed: only emit when true (per v1.2 spec)
    if edge.reversed {
        w.write_byte(b',')?;
        w.write_key("reversed")?;
        w.write_bool(true)?;
    }

    // path
    w.write_byte(b',')?;
    w.write_key("path")?;
    write_edge_path_arena(w, edge, ir)?;

    // label (optional)
    if let Some(lbl) = label {
        w.write_byte(b',')?;
        w.write_key("label")?;
        w.write_str(lbl)?;
        w.write_byte(b',')?;
        w.write_key("label_x")?;
        w.write_usize(edge.label_x)?;
        w.write_byte(b',')?;
        w.write_key("label_y")?;
        w.write_usize(edge.label_y)?;
    }

    w.write_byte(b'}')
}

fn write_edge_path_arena(
    w: &mut JsonWriter<'_>,
    edge: &LayoutEdgeArena,
    ir: &LayoutIRArena<'_>,
) -> Option<()> {
    match edge.path {
        EdgePathArena::Direct => w.write_bytes(b"{\"type\":\"direct\"}"),
        EdgePathArena::Corner { bend_at } => {
            w.write_bytes(b"{\"type\":\"corner\",")?;
            w.write_key("bend_at")?;
            w.write_usize(bend_at)?;
            w.write_byte(b'}')
        }
        EdgePathArena::SideChannel {
            channel_at,
            span_start,
            span_end,
        } => {
            w.write_bytes(b"{\"type\":\"side_channel\",")?;
            w.write_key("channel_at")?;
            w.write_usize(channel_at)?;
            w.write_byte(b',')?;
            w.write_key("span_start")?;
            w.write_usize(span_start)?;
            w.write_byte(b',')?;
            w.write_key("span_end")?;
            w.write_usize(span_end)?;
            w.write_byte(b'}')
        }
        EdgePathArena::MultiSegment {
            waypoints_start,
            waypoints_len,
            ..
        } => {
            w.write_bytes(b"{\"type\":\"multi_segment\",")?;
            w.write_key("waypoints")?;
            w.write_byte(b'[')?;
            let wps = ir.edge_waypoints_raw(waypoints_start, waypoints_len);
            for (i, &(x, y)) in wps.iter().enumerate() {
                if i > 0 {
                    w.write_byte(b',')?;
                }
                w.write_byte(b'[')?;
                w.write_usize(x)?;
                w.write_byte(b',')?;
                w.write_usize(y)?;
                w.write_byte(b']')?;
            }
            w.write_byte(b']')?;
            w.write_byte(b'}')
        }
        EdgePathArena::Spline {
            cp1_x,
            cp1_y,
            cp2_x,
            cp2_y,
        } => {
            w.write_bytes(b"{\"type\":\"spline\",")?;
            w.write_key("cp1_x")?;
            w.write_usize(cp1_x)?;
            w.write_byte(b',')?;
            w.write_key("cp1_y")?;
            w.write_usize(cp1_y)?;
            w.write_byte(b',')?;
            w.write_key("cp2_x")?;
            w.write_usize(cp2_x)?;
            w.write_byte(b',')?;
            w.write_key("cp2_y")?;
            w.write_usize(cp2_y)?;
            w.write_byte(b'}')
        }
    }
}

fn write_subgraph_arena(w: &mut JsonWriter<'_>, sg: &SubgraphInfoArena, label: &str) -> Option<()> {
    w.write_byte(b'{')?;

    w.write_key("id")?;
    w.write_usize(sg.id)?;
    w.write_byte(b',')?;

    w.write_key("label")?;
    w.write_str(label)?;
    w.write_byte(b',')?;

    w.write_key("parent_id")?;
    if sg.parent_idx == usize::MAX {
        w.write_null()?;
    } else {
        w.write_usize(sg.parent_idx)?;
    }
    w.write_byte(b',')?;

    w.write_key("x")?;
    w.write_usize(sg.x)?;
    w.write_byte(b',')?;
    w.write_key("y")?;
    w.write_usize(sg.y)?;
    w.write_byte(b',')?;
    w.write_key("width")?;
    w.write_usize(sg.width)?;
    w.write_byte(b',')?;
    w.write_key("height")?;
    w.write_usize(sg.height)?;

    w.write_byte(b'}')
}

// ── Heap path: serialize to String ───────────────────────────────────────

#[cfg(feature = "alloc")]
mod heap_json {
    use super::VERSION;
    use crate::ir::{EdgePath, LayoutEdge, LayoutIR, LayoutNode, NodeKind, SubgraphInfo};
    use alloc::string::String;

    impl<'a> LayoutIR<'a> {
        /// Serialize the layout IR to a JSON string (schema v1.3 —
        /// zigraph v1.2 plus the direction extensions).
        ///
        /// # Example
        ///
        /// ```
        /// use ascii_dag::Graph;
        ///
        /// let dag = Graph::from_edges(
        ///     &[(1, "A"), (2, "B")],
        ///     &[(1, 2)]
        /// );
        /// let ir = dag.compute_layout();
        /// let json = ir.to_json();
        /// assert!(json.contains("\"version\":\"1.3\""));
        /// assert!(json.contains("\"nodes\":["));
        /// ```
        pub fn to_json(&self) -> String {
            let mut out = String::with_capacity(self.estimate_json_size());
            self.write_json(&mut out);
            out
        }

        /// Write JSON to an existing String buffer.
        pub fn write_json(&self, out: &mut String) {
            out.push('{');

            push_key(out, "version");
            push_json_str(out, VERSION);
            out.push(',');

            push_key(out, "width");
            push_usize(out, self.width());
            out.push(',');
            push_key(out, "height");
            push_usize(out, self.height());
            out.push(',');
            push_key(out, "level_count");
            push_usize(out, self.level_count());
            out.push(',');

            // nodes
            push_key(out, "nodes");
            out.push('[');
            for (i, node) in self.nodes().iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                let payload = if node.content_tag == 2 {
                    Some(
                        match self.custom_nodes.binary_search_by_key(&i, |entry| entry.0) {
                            Ok(pos) => self.custom_nodes[pos].2,
                            Err(_) => "",
                        },
                    )
                } else {
                    None
                };
                write_node_heap(out, node, payload);
            }
            out.push(']');
            out.push(',');

            // edges
            push_key(out, "edges");
            out.push('[');
            for (i, edge) in self.edges().iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_edge_heap(out, edge);
            }
            out.push(']');

            // subgraphs
            if !self.subgraphs().is_empty() {
                out.push(',');
                push_key(out, "subgraphs");
                out.push('[');
                for (i, sg) in self.subgraphs().iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    write_subgraph_heap(out, sg);
                }
                out.push(']');
            }

            out.push('}');
        }

        fn estimate_json_size(&self) -> usize {
            100usize
                .saturating_add(self.nodes().len().saturating_mul(180))
                .saturating_add(self.edges().len().saturating_mul(220))
                .saturating_add(self.subgraphs().len().saturating_mul(120))
        }
    }

    fn push_key(out: &mut String, key: &str) {
        push_json_str(out, key);
        out.push(':');
    }

    fn push_json_str(out: &mut String, s: &str) {
        out.push('"');
        for c in s.chars() {
            match c {
                '"' => out.push_str("\\\""),
                '\\' => out.push_str("\\\\"),
                '\n' => out.push_str("\\n"),
                '\r' => out.push_str("\\r"),
                '\t' => out.push_str("\\t"),
                c if (c as u32) < 0x20 => {
                    use core::fmt::Write;
                    let _ = write!(out, "\\u{:04x}", c as u32);
                }
                _ => out.push(c),
            }
        }
        out.push('"');
    }

    fn push_usize(out: &mut String, n: usize) {
        use core::fmt::Write;
        let _ = write!(out, "{}", n);
    }

    fn push_bool(out: &mut String, b: bool) {
        out.push_str(if b { "true" } else { "false" });
    }

    fn write_node_heap(out: &mut String, node: &LayoutNode<'_>, payload: Option<&str>) {
        out.push('{');

        push_key(out, "id");
        push_usize(out, node.id);
        out.push(',');
        push_key(out, "label");
        push_json_str(out, node.label);
        out.push(',');
        push_key(out, "x");
        push_usize(out, node.x);
        out.push(',');
        push_key(out, "y");
        push_usize(out, node.y);
        out.push(',');
        push_key(out, "width");
        push_usize(out, node.width);
        out.push(',');
        push_key(out, "height");
        push_usize(out, node.height);
        out.push(',');
        push_key(out, "center_x");
        push_usize(out, node.center_x);
        out.push(',');
        push_key(out, "center_y");
        push_usize(out, node.center_y);
        out.push(',');
        push_key(out, "level");
        push_usize(out, node.level);
        out.push(',');
        push_key(out, "level_position");
        push_usize(out, node.level_position);
        out.push(',');

        push_key(out, "kind");
        match node.kind {
            NodeKind::Explicit => push_json_str(out, "explicit"),
            NodeKind::Implicit => push_json_str(out, "implicit"),
            NodeKind::Dummy => push_json_str(out, "dummy"),
        }
        out.push(',');

        // Declared content kind — separate from the layout `kind`
        // field; `payload` only for custom nodes (data, never code).
        push_key(out, "content_kind");
        push_json_str(
            out,
            match node.content_tag {
                1 => "boxed",
                2 => "custom",
                _ => "simple",
            },
        );
        out.push(',');
        if let Some(payload) = payload {
            push_key(out, "payload");
            push_json_str(out, payload);
            out.push(',');
        }

        // v1.3: the self-loop marker cell (only when present).
        if let Some((mx, my)) = node.self_loop_at {
            push_key(out, "self_loop_at");
            out.push('[');
            push_usize(out, mx);
            out.push(',');
            push_usize(out, my);
            out.push(']');
            out.push(',');
        }

        push_key(out, "edge_index");
        if let Some(ei) = node.edge_index {
            push_usize(out, ei);
        } else {
            out.push_str("null");
        }

        out.push('}');
    }

    fn write_edge_heap(out: &mut String, edge: &LayoutEdge<'_>) {
        out.push('{');

        push_key(out, "from");
        push_usize(out, edge.from_id);
        out.push(',');
        push_key(out, "to");
        push_usize(out, edge.to_id);
        out.push(',');
        push_key(out, "from_x");
        push_usize(out, edge.from_x);
        out.push(',');
        push_key(out, "from_y");
        push_usize(out, edge.from_y);
        out.push(',');
        push_key(out, "to_x");
        push_usize(out, edge.to_x);
        out.push(',');
        push_key(out, "to_y");
        push_usize(out, edge.to_y);
        out.push(',');
        push_key(out, "edge_index");
        push_usize(out, edge.edge_index);
        out.push(',');
        push_key(out, "directed");
        push_bool(out, edge.directed);
        out.push(',');

        // v1.3: which physical axis the trunk runs along.
        push_key(out, "flow_axis");
        push_json_str(
            out,
            match edge.flow_axis {
                crate::ir::FlowAxis::Y => "y",
                crate::ir::FlowAxis::X => "x",
            },
        );

        if edge.reversed {
            out.push(',');
            push_key(out, "reversed");
            push_bool(out, true);
        }

        out.push(',');
        push_key(out, "path");
        write_edge_path_heap(out, &edge.path);

        if let Some(label) = edge.label {
            out.push(',');
            push_key(out, "label");
            push_json_str(out, label);
            out.push(',');
            push_key(out, "label_x");
            push_usize(out, edge.label_x);
            out.push(',');
            push_key(out, "label_y");
            push_usize(out, edge.label_y);
        }

        out.push('}');
    }

    fn write_edge_path_heap(out: &mut String, path: &EdgePath) {
        match path {
            EdgePath::Direct => {
                out.push_str("{\"type\":\"direct\"}");
            }
            EdgePath::Corner { bend_at } => {
                out.push_str("{\"type\":\"corner\",");
                push_key(out, "bend_at");
                push_usize(out, *bend_at);
                out.push('}');
            }
            EdgePath::SideChannel {
                channel_at,
                span_start,
                span_end,
            } => {
                out.push_str("{\"type\":\"side_channel\",");
                push_key(out, "channel_at");
                push_usize(out, *channel_at);
                out.push(',');
                push_key(out, "span_start");
                push_usize(out, *span_start);
                out.push(',');
                push_key(out, "span_end");
                push_usize(out, *span_end);
                out.push('}');
            }
            EdgePath::MultiSegment { waypoints, .. } => {
                out.push_str("{\"type\":\"multi_segment\",");
                push_key(out, "waypoints");
                out.push('[');
                for (i, &(x, y)) in waypoints.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    out.push('[');
                    push_usize(out, x);
                    out.push(',');
                    push_usize(out, y);
                    out.push(']');
                }
                out.push(']');
                out.push('}');
            }
            EdgePath::Spline {
                cp1_x,
                cp1_y,
                cp2_x,
                cp2_y,
            } => {
                out.push_str("{\"type\":\"spline\",");
                push_key(out, "cp1_x");
                push_usize(out, *cp1_x);
                out.push(',');
                push_key(out, "cp1_y");
                push_usize(out, *cp1_y);
                out.push(',');
                push_key(out, "cp2_x");
                push_usize(out, *cp2_x);
                out.push(',');
                push_key(out, "cp2_y");
                push_usize(out, *cp2_y);
                out.push('}');
            }
        }
    }

    fn write_subgraph_heap(out: &mut String, sg: &SubgraphInfo<'_>) {
        out.push('{');

        push_key(out, "id");
        push_usize(out, sg.id);
        out.push(',');
        push_key(out, "label");
        push_json_str(out, sg.label);
        out.push(',');
        push_key(out, "parent_id");
        match sg.parent_id {
            Some(pid) => push_usize(out, pid),
            None => out.push_str("null"),
        }
        out.push(',');
        push_key(out, "x");
        push_usize(out, sg.x);
        out.push(',');
        push_key(out, "y");
        push_usize(out, sg.y);
        out.push(',');
        push_key(out, "width");
        push_usize(out, sg.width);
        out.push(',');
        push_key(out, "height");
        push_usize(out, sg.height);

        out.push('}');
    }
}

#[cfg(test)]
#[cfg(feature = "alloc")]
mod tests {
    use crate::Graph;

    #[test]
    fn heap_json_roundtrip_basic() {
        let dag = Graph::from_edges(&[(1, "A"), (2, "B"), (3, "C")], &[(1, 2), (1, 3), (2, 3)]);
        let ir = dag.compute_layout();
        let json = ir.to_json();

        // Validate structure
        assert!(json.starts_with('{'));
        assert!(json.ends_with('}'));
        assert!(json.contains("\"version\":\"1.3\""));
        assert!(json.contains("\"nodes\":["));
        assert!(json.contains("\"edges\":["));

        // Nodes
        assert!(json.contains("\"label\":\"A\""));
        assert!(json.contains("\"label\":\"B\""));
        assert!(json.contains("\"label\":\"C\""));
        assert!(json.contains("\"kind\":\"explicit\""));
        assert!(json.contains("\"center_y\":"));

        // Edges
        assert!(json.contains("\"from\":1"));
        assert!(json.contains("\"to\":2"));
        assert!(json.contains("\"directed\":true"));
        assert!(json.contains("\"path\":{\"type\":"));

        // No subgraphs → no subgraphs key
        assert!(!json.contains("\"subgraphs\""));
    }

    #[test]
    fn heap_json_with_subgraphs() {
        let mut dag = Graph::new();
        dag.add_node(1, "A");
        dag.add_node(2, "B");
        dag.add_edge(1, 2, None);
        let sg = dag.add_subgraph("cluster");
        dag.put_nodes(&[1, 2]).inside(sg).unwrap();

        let ir = dag.compute_layout();
        let json = ir.to_json();

        assert!(json.contains("\"subgraphs\":["));
        assert!(json.contains("\"label\":\"cluster\""));
        assert!(json.contains("\"parent_id\":null"));
    }

    #[test]
    fn heap_json_label_escaping() {
        let dag = Graph::from_edges(&[(1, "say \"hello\""), (2, "line\nnewline")], &[(1, 2)]);
        let ir = dag.compute_layout();
        let json = ir.to_json();

        assert!(json.contains("say \\\"hello\\\""));
        assert!(json.contains("line\\nnewline"));
    }

    #[test]
    fn heap_json_reversed_edge() {
        let dag = Graph::from_edges(
            &[(1, "A"), (2, "B")],
            &[(1, 2), (2, 1)], // cycle
        );
        let ir = dag.compute_layout();
        let json = ir.to_json();

        // At least one edge should have reversed:true
        assert!(json.contains("\"reversed\":true"));
    }

    #[test]
    fn heap_json_dimensions() {
        let dag = Graph::from_edges(&[(1, "X"), (2, "Y")], &[(1, 2)]);
        let ir = dag.compute_layout();
        let json = ir.to_json();

        let width = ir.width();
        let height = ir.height();
        assert!(json.contains(&format!("\"width\":{}", width)));
        assert!(json.contains(&format!("\"height\":{}", height)));
    }
}

#[cfg(test)]
#[cfg(feature = "arena")]
mod arena_tests {
    use crate::algorithms::sugiyama::config::LayoutConfig;
    use crate::graph::arena::Arena;
    use crate::graph::csr::CsrGraph;

    fn make_ir_json(
        nodes: &[(usize, &str)],
        edges: &[(usize, usize)],
        json_buf: &mut [u8],
    ) -> usize {
        let mut graph_backing = [0u8; 16384];
        let mut graph_arena = Arena::new(&mut graph_backing);
        let graph = CsrGraph::from_edges(&mut graph_arena, nodes, edges).unwrap();
        let config = LayoutConfig::standard();
        let mut temp_backing = [0u8; 65536];
        let mut temp_arena = Arena::new(&mut temp_backing);
        let mut out_backing = [0u8; 65536];
        let mut output_arena = Arena::new(&mut out_backing);
        let ir = graph
            .compute_layout_arena(&config, &mut temp_arena, &mut output_arena)
            .unwrap();
        ir.serialize_json(json_buf).unwrap()
    }

    #[test]
    fn arena_json_basic() {
        let mut json_buf = [0u8; 8192];
        let len = make_ir_json(
            &[(1, "A"), (2, "B"), (3, "C")],
            &[(1, 2), (1, 3), (2, 3)],
            &mut json_buf,
        );
        let json = core::str::from_utf8(&json_buf[..len]).unwrap();

        assert!(json.starts_with('{'));
        assert!(json.ends_with('}'));
        assert!(json.contains("\"version\":\"1.3\""));
        assert!(json.contains("\"nodes\":["));
        assert!(json.contains("\"edges\":["));
        assert!(json.contains("\"label\":\"A\""));
        assert!(json.contains("\"label\":\"B\""));
        assert!(json.contains("\"label\":\"C\""));
        assert!(json.contains("\"directed\":true"));
        assert!(json.contains("\"center_y\":"));
    }

    #[test]
    fn arena_json_buffer_too_small() {
        let mut graph_backing = [0u8; 16384];
        let mut graph_arena = Arena::new(&mut graph_backing);
        let graph =
            CsrGraph::from_edges(&mut graph_arena, &[(1, "A"), (2, "B")], &[(1, 2)]).unwrap();
        let config = LayoutConfig::standard();
        let mut temp_backing = [0u8; 65536];
        let mut temp_arena = Arena::new(&mut temp_backing);
        let mut out_backing = [0u8; 65536];
        let mut output_arena = Arena::new(&mut out_backing);
        let ir = graph
            .compute_layout_arena(&config, &mut temp_arena, &mut output_arena)
            .unwrap();

        let mut tiny_buf = [0u8; 10];
        assert!(ir.serialize_json(&mut tiny_buf).is_none());
    }

    #[test]
    fn arena_json_estimate_size() {
        let mut graph_backing = [0u8; 16384];
        let mut graph_arena = Arena::new(&mut graph_backing);
        let graph = CsrGraph::from_edges(
            &mut graph_arena,
            &[(1, "A"), (2, "B"), (3, "C")],
            &[(1, 2), (1, 3), (2, 3)],
        )
        .unwrap();
        let config = LayoutConfig::standard();
        let mut temp_backing = [0u8; 65536];
        let mut temp_arena = Arena::new(&mut temp_backing);
        let mut out_backing = [0u8; 65536];
        let mut output_arena = Arena::new(&mut out_backing);
        let ir = graph
            .compute_layout_arena(&config, &mut temp_arena, &mut output_arena)
            .unwrap();

        let estimate = ir.estimate_json_size();
        let mut buf = vec![0u8; estimate];
        let actual = ir.serialize_json(&mut buf).unwrap();
        assert!(
            actual <= estimate,
            "actual {} > estimate {}",
            actual,
            estimate
        );
    }
}
