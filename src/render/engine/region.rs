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
