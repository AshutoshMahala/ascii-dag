//! Band compositor — paints semantic cells in Z-order (temp/06 §6).
//!
//! Geometry-driven paint primitives: orientation derives from the
//! coordinates (physical IR, S3) — there is no assumed flow direction
//! anywhere in this file. Stage order (M7): subgraph borders and edges
//! (stroke merging is commutative, so their relative order cannot change
//! a junction), then edge labels, nodes, and subgraph labels (text, in
//! z-order).
//!
//! Colors follow the legacy colored renderer's semantics exactly: the
//! last element to touch a cell owns its color (even where glyph
//! precedence keeps the older glyph — an arrow crossed by a later edge
//! keeps its shape but takes the newer color); subgraph borders and box
//! labels never write colors (junction cells keep the crossing edge's
//! color); node text explicitly resets to the terminal default.
//!
//! Fidelity note: stroke overlaps merge semantically (per-arm max) —
//! the behavior of the legacy *colored* path (`merge_chars`), which the
//! RW0 tests pin exhaustively. The legacy *plain* path was lossier in a
//! few overlap cases; where the two legacy paths disagreed with each
//! other, the engine renders the junction-preserving variant (ruled
//! canonical for 0.10.0).

use super::cell::{Cell, Dir, MarkerKind, Weight};
use super::color::{CellColor, ColorMode};
use super::config::RenderOptions;
use super::plan::{LabelPlan, RenderPlan};
use super::view::{LayoutView, PathRef};
use crate::ir::NodeKind;

/// Color effect of a paint op on a cell.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Paint {
    /// Set the cell's color (legacy: `colors[x] = color`).
    Color(CellColor),
    /// Leave the cell's color untouched (legacy border/box-label ops).
    KeepColor,
}

/// A band-sized semantic canvas over caller-provided cells, with an
/// optional parallel color plane (present iff colors are enabled).
pub(crate) struct BandCanvas<'a> {
    cells: &'a mut [Cell],
    colors: Option<&'a mut [CellColor]>,
    width: usize,
    /// First global row of this band.
    y0: usize,
    rows: usize,
}

impl<'a> BandCanvas<'a> {
    /// Wrap `cells` (must hold `width × rows`) as a band starting at
    /// global row `y0`. Both planes are cleared here.
    pub(crate) fn new(
        cells: &'a mut [Cell],
        colors: Option<&'a mut [CellColor]>,
        width: usize,
        y0: usize,
        rows: usize,
    ) -> Self {
        for c in cells[..width * rows].iter_mut() {
            *c = Cell::EMPTY;
        }
        if let Some(plane) = &colors {
            debug_assert!(plane.len() >= width * rows);
        }
        let mut colors = colors;
        if let Some(p) = &mut colors {
            for c in p[..width * rows].iter_mut() {
                *c = CellColor::DEFAULT;
            }
        }
        Self {
            cells,
            colors,
            width,
            y0,
            rows,
        }
    }

    #[inline]
    fn idx(&self, x: usize, y: usize) -> Option<usize> {
        if x < self.width && y >= self.y0 && y < self.y0 + self.rows {
            Some((y - self.y0) * self.width + x)
        } else {
            None
        }
    }

    #[inline]
    fn apply_paint(&mut self, i: usize, paint: Paint) {
        if let (Paint::Color(c), Some(plane)) = (paint, &mut self.colors) {
            plane[i] = c;
        }
    }

    #[inline]
    pub(crate) fn stroke(
        &mut self,
        x: usize,
        y: usize,
        up: Weight,
        down: Weight,
        left: Weight,
        right: Weight,
        paint: Paint,
    ) {
        if let Some(i) = self.idx(x, y) {
            self.cells[i] = self.cells[i].painted_stroke(up, down, left, right);
            self.apply_paint(i, paint);
        }
    }

    #[inline]
    pub(crate) fn marker(
        &mut self,
        x: usize,
        y: usize,
        kind: MarkerKind,
        dir: Dir,
        dashed: bool,
        paint: Paint,
    ) {
        if let Some(i) = self.idx(x, y) {
            self.cells[i] = self.cells[i].painted_marker(kind, dir, dashed);
            self.apply_paint(i, paint);
        }
    }

    #[inline]
    pub(crate) fn text(&mut self, x: usize, y: usize, ch: char, paint: Paint) {
        if let Some(i) = self.idx(x, y) {
            self.cells[i] = self.cells[i].painted_text(ch);
            self.apply_paint(i, paint);
        }
    }

    /// One row of cells (local row index).
    pub(crate) fn row(&self, local_row: usize) -> &[Cell] {
        let start = local_row * self.width;
        &self.cells[start..start + self.width]
    }

    /// One row of the color plane, if colors are enabled.
    pub(crate) fn color_row(&self, local_row: usize) -> Option<&[CellColor]> {
        self.colors.as_ref().map(|p| {
            let start = local_row * self.width;
            &p[start..start + self.width]
        })
    }

    pub(crate) fn rows(&self) -> usize {
        self.rows
    }
}

/// Composite every element intersecting the canvas band.
pub(crate) fn composite_band<V: LayoutView>(
    view: &V,
    plan: &RenderPlan,
    options: &RenderOptions,
    canvas: &mut BandCanvas<'_>,
) {
    // Z0+Z1: strokes — subgraph borders and edges (commutative merges).
    for i in 0..view.subgraph_count() {
        paint_subgraph_border(view, i, canvas);
    }
    for i in 0..view.edge_count() {
        paint_edge(view, plan, i, canvas);
    }
    // Z2: edge labels. Placement gate mirrors the three legacy paths:
    // plain and colored-without-legend place geometrically; only the
    // colored-with-legend path additionally vetoes rows hosting nodes.
    let colored = !matches!(options.color_mode, ColorMode::None);
    for label in plan.labels() {
        let place = if colored && options.legend {
            label.placed_colored()
        } else {
            label.placeable
        };
        if place {
            paint_edge_label(view, plan, label, canvas);
        }
    }
    // Z3: nodes.
    for i in 0..view.node_count() {
        paint_node(view, i, options, canvas);
    }
    // Z4: subgraph labels (always readable; colors untouched).
    for i in 0..view.subgraph_count() {
        paint_subgraph_label(view, plan, i, canvas);
    }
}

// ── Edges ────────────────────────────────────────────────────────────────

/// Rows strictly between two anchors, in either order.
#[inline]
fn between(a: usize, b: usize) -> core::ops::Range<usize> {
    (a.min(b) + 1)..a.max(b)
}

fn paint_edge<V: LayoutView>(
    view: &V,
    plan: &RenderPlan,
    edge_index: usize,
    canvas: &mut BandCanvas<'_>,
) {
    let e = view.edge(edge_index);
    let w = plan.edge_plan(edge_index).weight.arm();
    let paint = Paint::Color(plan.edge_plan(edge_index).color);
    let rev = e.reversed;

    // Flow derives from the geometry (M4): +1 when the target sits below
    // the source (TopDown layouts), −1 when above (BottomUp layouts).
    // The Direction enum is never consulted here.
    let dir: isize = if e.to_y >= e.from_y { 1 } else { -1 };
    let off = |y: usize, k: isize| (y as isize + k * dir) as usize;
    let fwd = if dir > 0 { Dir::Down } else { Dir::Up };
    let bwd = if dir > 0 { Dir::Up } else { Dir::Down };

    match e.path {
        PathRef::Direct | PathRef::Spline { .. } => {
            for y in between(e.from_y, e.to_y) {
                if !rev && y == off(e.to_y, -1) {
                    canvas.marker(e.from_x, y, MarkerKind::Arrow, fwd, false, paint);
                } else if rev && y == off(e.from_y, 1) {
                    canvas.marker(e.from_x, y, MarkerKind::Arrow, bwd, true, paint);
                } else {
                    canvas.stroke(e.from_x, y, w, w, Weight::None, Weight::None, paint);
                }
            }
        }
        PathRef::Corner { horizontal_y } => {
            for y in between(e.from_y, horizontal_y) {
                if rev && y == off(e.from_y, 1) {
                    canvas.marker(e.from_x, y, MarkerKind::Arrow, bwd, true, paint);
                } else {
                    canvas.stroke(e.from_x, y, w, w, Weight::None, Weight::None, paint);
                }
            }
            let bend_adjacent = (horizontal_y as isize - e.from_y as isize) * dir <= 1;
            h_run_with_corners(
                canvas,
                horizontal_y,
                e.from_x,
                e.to_x,
                w,
                rev && bend_adjacent,
                paint,
                dir,
            );
            for y in between(horizontal_y, e.to_y) {
                if !rev && y == off(e.to_y, -1) {
                    canvas.marker(e.to_x, y, MarkerKind::Arrow, fwd, false, paint);
                } else {
                    canvas.stroke(e.to_x, y, w, w, Weight::None, Weight::None, paint);
                }
            }
        }
        PathRef::SideChannel {
            channel_x,
            start_y,
            end_y,
        } => {
            h_run_with_corners(canvas, start_y, e.from_x, channel_x, w, false, paint, dir);
            for y in between(start_y, end_y) {
                canvas.stroke(channel_x, y, w, w, Weight::None, Weight::None, paint);
            }
            h_run_with_corners(canvas, end_y, channel_x, e.to_x, w, false, paint, dir);
            for y in between(end_y, e.to_y) {
                if !rev && y == off(e.to_y, -1) {
                    canvas.marker(e.to_x, y, MarkerKind::Arrow, fwd, false, paint);
                } else {
                    canvas.stroke(e.to_x, y, w, w, Weight::None, Weight::None, paint);
                }
            }
        }
        PathRef::MultiSegment {
            waypoints,
            start_y_offset,
        } => {
            let mut px = e.from_x;
            let mut py = e.from_y;
            let mut first = true;
            for i in 0..=waypoints.len() {
                let (nx, ny) = if i < waypoints.len() {
                    waypoints[i]
                } else {
                    (e.to_x, e.to_y)
                };
                let last = i == waypoints.len();
                if px == nx {
                    for y in between(py, ny) {
                        if first && rev && y == off(py, 1) {
                            canvas.marker(px, y, MarkerKind::Arrow, bwd, true, paint);
                        } else if last && !rev && y == off(ny, -1) {
                            canvas.marker(px, y, MarkerKind::Arrow, fwd, false, paint);
                        } else {
                            canvas.stroke(px, y, w, w, Weight::None, Weight::None, paint);
                        }
                    }
                    // The waypoint row itself carries the vertical for
                    // non-first segments (legacy gap fill).
                    if !first {
                        canvas.stroke(px, py, w, w, Weight::None, Weight::None, paint);
                    }
                } else {
                    let corner_y = off(py, 1 + if first { start_y_offset as isize } else { 0 });
                    if first && start_y_offset > 0 {
                        for y in between(py, corner_y) {
                            if rev && y == off(py, 1) {
                                canvas.marker(px, y, MarkerKind::Arrow, bwd, true, paint);
                            } else {
                                canvas.stroke(px, y, w, w, Weight::None, Weight::None, paint);
                            }
                        }
                    }
                    if !first {
                        canvas.stroke(px, py, w, w, Weight::None, Weight::None, paint);
                    }
                    let bend_adjacent = (corner_y as isize - py as isize) * dir <= 1;
                    h_run_with_corners(
                        canvas,
                        corner_y,
                        px,
                        nx,
                        w,
                        first && rev && bend_adjacent,
                        paint,
                        dir,
                    );
                    for y in between(corner_y, ny) {
                        if last && !rev && y == off(ny, -1) {
                            canvas.marker(nx, y, MarkerKind::Arrow, fwd, false, paint);
                        } else {
                            canvas.stroke(nx, y, w, w, Weight::None, Weight::None, paint);
                        }
                    }
                }
                px = nx;
                py = ny;
                first = false;
            }
        }
    }
}

/// Horizontal run with corner arms at both ends. The start end's
/// vertical arm points back toward the source (against the flow), the
/// far end's arm continues with the flow toward the target.
/// `reversed_hack` paints the legacy "no room for the reversed arrow,
/// put it at the corner" marker at the start end instead of a corner.
#[allow(clippy::too_many_arguments)]
fn h_run_with_corners(
    canvas: &mut BandCanvas<'_>,
    row: usize,
    x_start: usize,
    x_end: usize,
    w: Weight,
    reversed_hack: bool,
    paint: Paint,
    dir: isize,
) {
    if x_start == x_end {
        return;
    }
    let n = Weight::None;
    let (lo, hi) = (x_start.min(x_end), x_start.max(x_end));
    for x in (lo + 1)..hi {
        canvas.stroke(x, row, n, n, w, w, paint);
    }
    // Vertical arms: anti-flow at the start (toward the source), flow at
    // the end (toward the target).
    let (src_up, src_down) = if dir > 0 { (w, n) } else { (n, w) };
    let (tgt_up, tgt_down) = if dir > 0 { (n, w) } else { (w, n) };
    if reversed_hack {
        let bwd = if dir > 0 { Dir::Up } else { Dir::Down };
        canvas.marker(x_start, row, MarkerKind::Arrow, bwd, true, paint);
    } else if x_start < x_end {
        canvas.stroke(x_start, row, src_up, src_down, n, w, paint);
    } else {
        canvas.stroke(x_start, row, src_up, src_down, w, n, paint);
    }
    if x_start < x_end {
        canvas.stroke(x_end, row, tgt_up, tgt_down, w, n, paint);
    } else {
        canvas.stroke(x_end, row, tgt_up, tgt_down, n, w, paint);
    }
}

// ── Subgraphs ────────────────────────────────────────────────────────────

fn paint_subgraph_border<V: LayoutView>(view: &V, index: usize, canvas: &mut BandCanvas<'_>) {
    let sg = view.subgraph(index);
    if sg.width < 2 || sg.height < 2 {
        return;
    }
    let d = Weight::Double;
    let n = Weight::None;
    let k = Paint::KeepColor;
    let top = sg.y;
    let bottom = sg.y + sg.height - 1;
    let left = sg.x;
    let right = sg.x + sg.width - 1;

    canvas.stroke(left, top, n, d, n, d, k);
    canvas.stroke(right, top, n, d, d, n, k);
    canvas.stroke(left, bottom, d, n, n, d, k);
    canvas.stroke(right, bottom, d, n, d, n, k);
    for x in (left + 1)..right {
        canvas.stroke(x, top, n, n, d, d, k);
        canvas.stroke(x, bottom, n, n, d, d, k);
    }
    for y in (top + 1)..bottom {
        canvas.stroke(left, y, d, d, n, n, k);
        canvas.stroke(right, y, d, d, n, n, k);
    }
}

fn paint_subgraph_label<V: LayoutView>(
    view: &V,
    plan: &RenderPlan,
    index: usize,
    canvas: &mut BandCanvas<'_>,
) {
    let sg = view.subgraph(index);
    if sg.label.is_empty() || sg.width < 4 || sg.height < 3 {
        return;
    }
    let label_y = match plan.subgraph_plan(index).label_pos {
        super::style::LabelPosition::InsideTop => sg.y + 1,
        super::style::LabelPosition::InsideBottom => (sg.y + sg.height).saturating_sub(2),
    };
    let max_len = sg.width.saturating_sub(4);
    let mut x = sg.x + 2;
    for ch in sg.label.chars().take(max_len) {
        canvas.text(x, label_y, ch, Paint::KeepColor);
        x += 1;
    }
}

// ── Nodes & edge labels ──────────────────────────────────────────────────

fn paint_node<V: LayoutView>(
    view: &V,
    index: usize,
    options: &RenderOptions,
    canvas: &mut BandCanvas<'_>,
) {
    let n = view.node(index);
    let default = Paint::Color(CellColor::DEFAULT);
    if matches!(n.kind, NodeKind::Dummy) {
        if options.show_dummy_nodes {
            canvas.marker(n.x, n.y, MarkerKind::Dummy, Dir::Up, false, default);
        }
        return;
    }
    // Bracket row at the node's top row (content atomicity, D4). The
    // closing bracket sits at the node's declared width (arena widths
    // can exceed label+2 — the padded interior cells are left
    // untouched, like the legacy renderers). Node text explicitly
    // resets colors to the terminal default (legacy colored behavior).
    let close_x = n.x + n.width.saturating_sub(1);
    let mut x = n.x;
    canvas.text(x, n.y, '[', default);
    x += 1;
    for ch in n.label.chars() {
        if x >= close_x {
            break;
        }
        canvas.text(x, n.y, ch, default);
        x += 1;
    }
    if n.width >= 2 {
        canvas.text(close_x, n.y, ']', default);
    }
    if n.has_self_loop {
        canvas.marker(close_x + 1, n.y, MarkerKind::SelfLoop, Dir::Up, false, default);
    }
}

fn paint_edge_label<V: LayoutView>(
    view: &V,
    plan: &RenderPlan,
    label: &LabelPlan,
    canvas: &mut BandCanvas<'_>,
) {
    let e = view.edge(label.edge_index);
    let Some(text) = e.label else { return };
    let paint = Paint::Color(plan.edge_plan(label.edge_index).label_color);
    let mut x = label.x;
    canvas.text(x, label.y, '"', paint);
    x += 1;
    for ch in text.chars() {
        canvas.text(x, label.y, ch, paint);
        x += 1;
    }
    canvas.text(x, label.y, '"', paint);
}
