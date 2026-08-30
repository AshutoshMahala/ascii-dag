//! `NodeRegion` — the safe, clipped writer custom node painters draw
//! through.
//!
//! A node declares an area (`width × height`); layout routes edges
//! around it; at render time a painter fills it. `NodeRegion` is the
//! painter's only pen: coordinates are **node-local** (`(0, 0)` is the
//! node's top-left), and every write outside the declared area — or
//! outside the band currently being composited — is a silent no-op.
//! Escaping the region is structurally impossible at the **canvas**
//! level: painters need no `unsafe` and cannot write to neighboring
//! cells. That isolation is per logical cell — control characters,
//! escape sequences, combining marks, and wide glyphs can still affect
//! terminal output beyond one cell, exactly as they can in node labels
//! (the engine passes text through untranslated; validating painter
//! text is the caller's concern, same as label text today).
//!
//! Banding note: a node spanning a band boundary is replayed once per
//! band; the painter runs again with identical inputs and the canvas
//! clips rows outside the current band. Determinism under replay —
//! drawing the same content each call — is therefore a **documented
//! caller contract**: a plain `fn` cannot capture locals, but it can
//! still read global state, panic (propagating to the render caller),
//! or vary per call, none of which the type system prevents. Use
//! [`NodePaintCtx::visible_rows`] to skip content outside the current
//! band — tall nodes replay per band, and re-drawing everything each
//! time is wasted work (clipping keeps it *correct* either way).

use super::color::CellColor;
use super::compose::{BandCanvas, Paint};

/// What a node painter knows about the node it is filling.
///
/// Engine-created (`#[non_exhaustive]`): painters read fields; only
/// the engine constructs one. New fields arrive without breakage.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct NodePaintCtx<'a> {
    /// The node's id.
    pub node_id: usize,
    /// The node's label text.
    pub label: &'a str,
    /// Declared width in cells (the region's horizontal extent).
    pub width: usize,
    /// Declared height in rows (the region's vertical extent).
    pub height: usize,
    /// The node-local, half-open row range visible in the band being
    /// composited. Rendering is banded: a tall node's painter runs
    /// once per band it spans, and writes outside this range are
    /// clipped. Painters MAY skip work outside it (tall content need
    /// not be re-parsed and re-drawn every band); ignoring it stays
    /// correct.
    pub visible_rows: (usize, usize),
    /// The node's declared payload — the **data** half of the
    /// template/data pair, set at `add_node` and parsed by the painter
    /// as it draws (empty when the node declared none).
    pub payload: &'a str,
}

/// A clipped, node-local view onto the render canvas.
pub struct NodeRegion<'r, 'a> {
    canvas: &'r mut BandCanvas<'a>,
    x0: usize,
    y0: usize,
    width: usize,
    height: usize,
    /// Default ink color (the node's resolved text color).
    color: CellColor,
}

impl<'r, 'a> NodeRegion<'r, 'a> {
    pub(crate) fn new(
        canvas: &'r mut BandCanvas<'a>,
        x0: usize,
        y0: usize,
        width: usize,
        height: usize,
        color: CellColor,
    ) -> Self {
        Self {
            canvas,
            x0,
            y0,
            width,
            height,
            color,
        }
    }

    /// The region's width in cells.
    pub fn width(&self) -> usize {
        self.width
    }

    /// The region's height in rows.
    pub fn height(&self) -> usize {
        self.height
    }

    /// Put one character at node-local `(x, y)` in the node's text
    /// color. Out-of-region coordinates are ignored.
    pub fn set(&mut self, x: usize, y: usize, ch: char) {
        self.set_colored(x, y, ch, self.color);
    }

    /// Put one character with an explicit color. Out-of-region
    /// coordinates are ignored.
    pub fn set_colored(&mut self, x: usize, y: usize, ch: char, color: CellColor) {
        if x >= self.width || y >= self.height {
            return;
        }
        self.canvas
            .text(self.x0 + x, self.y0 + y, ch, Paint::Color(color));
    }

    /// Write a string starting at node-local `(x, y)`; characters that
    /// would leave the region are dropped, and a start outside the
    /// region writes nothing (no arithmetic on out-of-range starts —
    /// the iteration is bounded before any offset is computed).
    pub fn write_str(&mut self, x: usize, y: usize, text: &str) {
        if x >= self.width || y >= self.height {
            return;
        }
        for (i, ch) in text.chars().take(self.width - x).enumerate() {
            self.set(x + i, y, ch);
        }
    }
}

// ── Semantic painter primitives ──────────────────────────────────────────
//
// Painters that draw structure through these emit SEMANTIC stroke and
// marker cells, decoded per charset at emission like all engine ink —
// one painter is byte-correct under every charset, with no charset in
// sight. Stroke cells also merge per-arm with each other (a rule
// flush against a frame becomes a tee, a crossing becomes a
// junction), which raw `char` writes can never do.
impl NodeRegion<'_, '_> {
    fn stroke_at(
        &mut self,
        x: usize,
        y: usize,
        up: super::cell::Weight,
        down: super::cell::Weight,
        left: super::cell::Weight,
        right: super::cell::Weight,
    ) {
        if x >= self.width || y >= self.height {
            return;
        }
        self.canvas.stroke(
            self.x0 + x,
            self.y0 + y,
            up,
            down,
            left,
            right,
            Paint::Color(self.color),
        );
    }

    /// Node-local rows of this region visible in the band being
    /// composited, as a half-open range — the iteration bound for
    /// every primitive below. A tall painter is replayed once per
    /// band; looping only the visible rows keeps that replay
    /// O(band), not O(full extent), and bounds arbitrarily large
    /// caller endpoints.
    fn visible_rows(&self) -> (usize, usize) {
        let band_lo = self.canvas.y0();
        let band_hi = band_lo + self.canvas.rows();
        let lo = self.y0.max(band_lo);
        let hi = (self.y0 + self.height).min(band_hi);
        if lo < hi {
            (lo - self.y0, hi - self.y0)
        } else {
            (0, 0)
        }
    }

    /// Light horizontal rule spanning `x0..=x1` at row `y`. End cells
    /// carry only the inward arm: a standalone rule still decodes to a
    /// plain line at every cell, and a rule flush with a frame or
    /// another rule merges into the right tee or junction. Iteration
    /// is clipped to the region and the current band BEFORE looping;
    /// arm semantics follow the ORIGINAL endpoints.
    pub fn hrule(&mut self, x0: usize, x1: usize, y: usize) {
        use super::cell::Weight::{Light, None as No};
        let (row_lo, row_hi) = self.visible_rows();
        if x0 > x1 || y >= self.height || y < row_lo || y >= row_hi {
            return;
        }
        for x in x0..=x1.min(self.width - 1) {
            let left = if x > x0 || x0 == x1 { Light } else { No };
            let right = if x < x1 || x0 == x1 { Light } else { No };
            self.stroke_at(x, y, No, No, left, right);
        }
    }

    /// Light vertical rule spanning `y0..=y1` at column `x` (end-arm
    /// semantics and clipping as [`hrule`](Self::hrule)).
    pub fn vrule(&mut self, x: usize, y0: usize, y1: usize) {
        use super::cell::Weight::{Light, None as No};
        let (row_lo, row_hi) = self.visible_rows();
        if y0 > y1 || x >= self.width || row_lo == row_hi {
            return;
        }
        for y in y0.max(row_lo)..=y1.min(row_hi - 1) {
            let up = if y > y0 || y0 == y1 { Light } else { No };
            let down = if y < y1 || y0 == y1 { Light } else { No };
            self.stroke_at(x, y, up, down, No, No);
        }
    }

    /// Light border around the full region (the boxed-node arm
    /// conventions), clipped to the current band like the boxed-node
    /// painter. No-op on regions too small for a border.
    pub fn frame(&mut self) {
        use super::cell::Weight::{Light as L, None as No};
        if self.width < 2 || self.height < 2 {
            return;
        }
        let (row_lo, row_hi) = self.visible_rows();
        if row_lo == row_hi {
            return;
        }
        let (r, b) = (self.width - 1, self.height - 1);
        if row_lo == 0 {
            self.stroke_at(0, 0, No, L, No, L);
            self.stroke_at(r, 0, No, L, L, No);
            for x in 1..r {
                self.stroke_at(x, 0, No, No, L, L);
            }
        }
        if b >= row_lo && b < row_hi {
            self.stroke_at(0, b, L, No, No, L);
            self.stroke_at(r, b, L, No, L, No);
            for x in 1..r {
                self.stroke_at(x, b, No, No, L, L);
            }
        }
        for y in row_lo.max(1)..row_hi.min(b) {
            self.stroke_at(0, y, L, L, No, No);
            self.stroke_at(r, y, L, L, No, No);
        }
    }

    /// Arrowhead marker pointing `direction`, in the node's ink
    /// color — a real marker cell, decoded per charset at emission
    /// (`↓`/`v`, `→`/`>`, …).
    pub fn arrow(&mut self, x: usize, y: usize, direction: super::cells::MarkerDirection) {
        if x >= self.width || y >= self.height {
            return;
        }
        self.canvas.marker(
            self.x0 + x,
            self.y0 + y,
            super::cell::MarkerKind::Arrow,
            direction.to_dir(),
            false,
            Paint::Color(self.color),
        );
    }
}

#[cfg(all(test, feature = "std", feature = "layout-vertical"))]
mod tests {
    use super::super::cells::MarkerDirection;
    use super::*;
    use crate::graph::Graph;
    use crate::render::engine::CustomNode;
    use crate::{Charset, RenderOptions};

    /// Structure-heavy card drawn entirely through the semantic
    /// primitives — no charset anywhere in the painter.
    fn semantic_card(region: &mut NodeRegion<'_, '_>, ctx: NodePaintCtx<'_>) {
        let right = region.width() - 1;
        region.frame();
        region.write_str(2, 1, ctx.label);
        region.hrule(0, right, 2);
        region.hrule(0, right, 4);
        region.vrule(7, 2, 6);
        for (i, line) in ctx.payload.lines().enumerate() {
            region.write_str(1, 3 + i, line);
        }
        region.arrow(3, 5, MarkerDirection::Down);
        region.write_str(1, 6, "ok");
    }

    fn card_graph() -> Graph<'static> {
        let mut g = Graph::new();
        g.add_node(1usize, "A");
        g.add_node(
            10usize,
            CustomNode {
                label: "Server",
                width: 12,
                height: 8,
                painter: Some(semantic_card),
                payload: "cpu: 4",
            },
        );
        g.add_edge(1usize, 10usize, None);
        g
    }

    fn options(charset: Charset) -> RenderOptions {
        let mut o = RenderOptions::plain();
        o.emit.charset = charset;
        o
    }

    /// One charset-blind painter, byte-correct under both charsets:
    /// its rules meet its frame as tees (`├ ┬ ┤`), cross as `┼`, and
    /// its arrow is a real marker (`↓`/`v`) — all decoded per charset
    /// at emission. Raw `char` writes could produce none of those
    /// junctions.
    #[test]
    fn semantic_painter_is_byte_correct_under_both_charsets() {
        let g = card_graph();
        let ir = g.compute_layout();
        assert_eq!(
            ir.render_string(&options(Charset::Unicode)),
            "      [A]\n       └┐\n        ↓\n  \
             ┌──────────┐\n  \
             │ Server   │\n  \
             ├──────┬───┤\n  \
             │cpu: 4│   │\n  \
             ├──────┼───┤\n  \
             │  ↓   │   │\n  \
             │ok    │   │\n  \
             └──────────┘\n\n\n"
        );
        assert_eq!(
            ir.render_string(&options(Charset::Ascii)),
            "      [A]\n       ++\n        v\n  \
             +----------+\n  \
             | Server   |\n  \
             +------+---+\n  \
             |cpu: 4|   |\n  \
             +------+---+\n  \
             |  v   |   |\n  \
             |ok    |   |\n  \
             +----------+\n\n\n"
        );
    }

    /// The arena backend serves the same painter path byte-for-byte,
    /// both charsets.
    #[test]
    fn arena_backend_serves_painters_byte_identically() {
        let g = card_graph();
        let heap_ir = g.compute_layout();
        for charset in [Charset::Unicode, Charset::Ascii] {
            let opts = options(charset);
            let g = card_graph();
            let mut csr_buf = vec![0u8; g.estimate_csr_arena_size() * 2];
            let mut csr_arena = crate::graph::arena::Arena::new(&mut csr_buf);
            let csr = g.to_csr(&mut csr_arena).unwrap();
            let size = (g.estimate_layout_arena_size() * 2).max(256 * 1024);
            let mut temp_buf = vec![0u8; size];
            let mut out_buf = vec![0u8; size];
            let mut temp_arena = crate::graph::arena::Arena::new(&mut temp_buf);
            let mut out_arena = crate::graph::arena::Arena::new(&mut out_buf);
            let arena_ir = csr
                .compute_layout_arena(
                    &crate::LayoutConfig::standard(),
                    &mut temp_arena,
                    &mut out_arena,
                )
                .unwrap();
            let mut arena_out = String::new();
            arena_ir.render_with(&opts, &mut arena_out).unwrap();
            assert_eq!(
                heap_ir.render_string(&opts),
                arena_out,
                "backend divergence under {charset:?}"
            );
        }
    }
}
