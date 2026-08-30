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
    /// The active output charset. Painter text passes through
    /// untranslated, so charset-faithful painters should pick their
    /// own glyphs (e.g. `-` vs `─`) from this.
    pub charset: super::charset::Charset,
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

// ── Semantic painter primitives (0.11 prototype, test-gated) ─────────────
//
// Painters that draw structure through these emit SEMANTIC stroke and
// marker cells, decoded per charset at emission like all engine ink —
// so `NodePaintCtx.charset` becomes unnecessary and one painter is
// byte-correct under every charset. Stroke cells also merge per-arm
// with each other (a rule flush against a frame becomes a tee, a
// crossing becomes a junction), which raw `char` writes can never do.
// Test-only while the spike runs; the real 0.11 painter API promotes
// these and removes `charset` from the context. See
// temp/spike-4.0d-findings.md.
#[cfg(all(test, feature = "std", feature = "layout-vertical"))]
impl NodeRegion<'_, '_> {
    fn spike_stroke(
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
    pub(crate) fn spike_hrule(&mut self, x0: usize, x1: usize, y: usize) {
        use super::cell::Weight::{Light, None as No};
        let (row_lo, row_hi) = self.visible_rows();
        if x0 > x1 || y >= self.height || y < row_lo || y >= row_hi {
            return;
        }
        for x in x0..=x1.min(self.width - 1) {
            let left = if x > x0 || x0 == x1 { Light } else { No };
            let right = if x < x1 || x0 == x1 { Light } else { No };
            self.spike_stroke(x, y, No, No, left, right);
        }
    }

    /// Light vertical rule spanning `y0..=y1` at column `x` (end-arm
    /// semantics and clipping as [`spike_hrule`](Self::spike_hrule)).
    pub(crate) fn spike_vrule(&mut self, x: usize, y0: usize, y1: usize) {
        use super::cell::Weight::{Light, None as No};
        let (row_lo, row_hi) = self.visible_rows();
        if y0 > y1 || x >= self.width || row_lo == row_hi {
            return;
        }
        for y in y0.max(row_lo)..=y1.min(row_hi - 1) {
            let up = if y > y0 || y0 == y1 { Light } else { No };
            let down = if y < y1 || y0 == y1 { Light } else { No };
            self.spike_stroke(x, y, up, down, No, No);
        }
    }

    /// Light border around the full region (the boxed-node arm
    /// conventions), clipped to the current band like the boxed-node
    /// painter. No-op on regions too small for a border.
    pub(crate) fn spike_frame(&mut self) {
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
            self.spike_stroke(0, 0, No, L, No, L);
            self.spike_stroke(r, 0, No, L, L, No);
            for x in 1..r {
                self.spike_stroke(x, 0, No, No, L, L);
            }
        }
        if b >= row_lo && b < row_hi {
            self.spike_stroke(0, b, L, No, No, L);
            self.spike_stroke(r, b, L, No, L, No);
            for x in 1..r {
                self.spike_stroke(x, b, No, No, L, L);
            }
        }
        for y in row_lo.max(1)..row_hi.min(b) {
            self.spike_stroke(0, y, L, L, No, No);
            self.spike_stroke(r, y, L, L, No, No);
        }
    }

    /// Arrowhead marker pointing `dir`, in the node's ink color.
    pub(crate) fn spike_arrow(&mut self, x: usize, y: usize, dir: super::cell::Dir) {
        if x >= self.width || y >= self.height {
            return;
        }
        self.canvas.marker(
            self.x0 + x,
            self.y0 + y,
            super::cell::MarkerKind::Arrow,
            dir,
            false,
            Paint::Color(self.color),
        );
    }
}
