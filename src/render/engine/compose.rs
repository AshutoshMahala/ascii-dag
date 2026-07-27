//! Band compositor — paints semantic cells in Z-order (temp/06 §6).
//!
//! Geometry-driven paint primitives: orientation derives from the
//! coordinates (physical IR, S3) — there is no assumed flow direction
//! anywhere in this file. Stage order (M7): subgraph borders and edges
//! (stroke merging is commutative, so their relative order cannot change
//! a junction), then edge labels, nodes, and subgraph labels (text, in
//! z-order).
//!
//! Fidelity note: stroke overlaps merge semantically (per-arm max) —
//! the behavior of the legacy *colored* path (`merge_chars`), which the
//! RW0 tests pin exhaustively. The legacy *plain* path was lossier in a
//! few overlap cases (it overwrote corners with plain verticals); where
//! the two legacy paths disagreed with each other, the engine renders
//! the junction-preserving variant. The dual-run harness quantifies
//! every such cell.

use super::cell::{Cell, Dir, MarkerKind, Weight};
use super::config::RenderOptions;
use super::plan::{LabelPlan, RenderPlan};
use super::view::{LayoutView, PathRef};
use crate::ir::NodeKind;

/// A band-sized semantic canvas over caller-provided cells.
pub(crate) struct BandCanvas<'a> {
    cells: &'a mut [Cell],
    width: usize,
    /// First global row of this band.
    y0: usize,
    rows: usize,
}

impl<'a> BandCanvas<'a> {
    /// Wrap `cells` (must hold `width × rows`) as a band starting at
    /// global row `y0`. Cells are cleared here.
    pub(crate) fn new(cells: &'a mut [Cell], width: usize, y0: usize, rows: usize) -> Self {
        debug_assert!(cells.len() >= width * rows);
        for c in cells[..width * rows].iter_mut() {
            *c = Cell::EMPTY;
        }
        Self {
            cells,
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

    /// The cell at global (x, y); `EMPTY` outside the band (free clip).
    #[inline]
    pub(crate) fn get(&self, x: usize, y: usize) -> Cell {
        self.idx(x, y).map_or(Cell::EMPTY, |i| self.cells[i])
    }

    #[inline]
    pub(crate) fn stroke(&mut self, x: usize, y: usize, up: Weight, down: Weight, left: Weight, right: Weight) {
        if let Some(i) = self.idx(x, y) {
            self.cells[i] = self.cells[i].painted_stroke(up, down, left, right);
        }
    }

    #[inline]
    pub(crate) fn marker(&mut self, x: usize, y: usize, kind: MarkerKind, dir: Dir, dashed: bool) {
        if let Some(i) = self.idx(x, y) {
            self.cells[i] = self.cells[i].painted_marker(kind, dir, dashed);
        }
    }

    #[inline]
    pub(crate) fn text(&mut self, x: usize, y: usize, ch: char) {
        if let Some(i) = self.idx(x, y) {
            self.cells[i] = self.cells[i].painted_text(ch);
        }
    }

    /// One decoded row of the band (local row index).
    pub(crate) fn row(&self, local_row: usize) -> &[Cell] {
        let start = local_row * self.width;
        &self.cells[start..start + self.width]
    }

    pub(crate) fn rows(&self) -> usize {
        self.rows
    }

    pub(crate) fn y0(&self) -> usize {
        self.y0
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
    // Z2: edge labels (plain-path decision; colored applies its stricter
    // rule in the colored emitter, RW4).
    for label in plan.labels() {
        if label.placeable {
            paint_edge_label(view, label, canvas);
        }
    }
    // Z3: nodes.
    for i in 0..view.node_count() {
        paint_node(view, i, options, canvas);
    }
    // Z4: subgraph labels (always readable).
    for i in 0..view.subgraph_count() {
        paint_subgraph_label(view, plan, i, canvas);
    }
}

// ── Edges ────────────────────────────────────────────────────────────────

fn paint_edge<V: LayoutView>(
    view: &V,
    plan: &RenderPlan,
    edge_index: usize,
    canvas: &mut BandCanvas<'_>,
) {
    let e = view.edge(edge_index);
    let w = plan.edge_plan(edge_index).weight.arm();
    let rev = e.reversed;

    match e.path {
        PathRef::Direct | PathRef::Spline { .. } => {
            // Vertical from below the source to above the target;
            // forward arrow above the target, reversed arrow below the
            // source (legacy Direct semantics).
            vertical_run(canvas, e.from_x, e.from_y, e.to_y, w, rev, true, true);
        }
        PathRef::Corner { horizontal_y } => {
            // Vertical from source down to the bend…
            if horizontal_y > e.from_y {
                for y in (e.from_y + 1)..horizontal_y {
                    if rev && y == e.from_y + 1 {
                        canvas.marker(e.from_x, y, MarkerKind::Arrow, Dir::Up, true);
                    } else {
                        canvas.stroke(e.from_x, y, w, w, Weight::None, Weight::None);
                    }
                }
            }
            // …the bend row…
            h_run_with_corners(
                canvas,
                horizontal_y,
                e.from_x,
                e.to_x,
                w,
                rev && horizontal_y <= e.from_y + 1,
            );
            // …vertical from the bend to the target.
            for y in (horizontal_y + 1)..e.to_y {
                if !rev && y == e.to_y - 1 {
                    canvas.marker(e.to_x, y, MarkerKind::Arrow, Dir::Down, false);
                } else {
                    canvas.stroke(e.to_x, y, w, w, Weight::None, Weight::None);
                }
            }
        }
        PathRef::SideChannel {
            channel_x,
            start_y,
            end_y,
        } => {
            h_run_with_corners(canvas, start_y, e.from_x, channel_x, w, false);
            for y in (start_y + 1)..end_y {
                canvas.stroke(channel_x, y, w, w, Weight::None, Weight::None);
            }
            h_run_with_corners(canvas, end_y, channel_x, e.to_x, w, false);
            for y in (end_y + 1)..e.to_y {
                if !rev && y == e.to_y - 1 {
                    canvas.marker(e.to_x, y, MarkerKind::Arrow, Dir::Down, false);
                } else {
                    canvas.stroke(e.to_x, y, w, w, Weight::None, Weight::None);
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
                    // Pure vertical segment.
                    let start = if first { py + 1 } else { py };
                    for y in start..ny {
                        if first && rev && y == py + 1 {
                            canvas.marker(px, y, MarkerKind::Arrow, Dir::Up, true);
                        } else if last && !rev && y == ny - 1 {
                            canvas.marker(px, y, MarkerKind::Arrow, Dir::Down, false);
                        } else {
                            canvas.stroke(px, y, w, w, Weight::None, Weight::None);
                        }
                    }
                } else {
                    // Bend: vertical to the corner row, horizontal run,
                    // vertical down to the next stop.
                    let corner_y = py + 1 + if first { start_y_offset } else { 0 };
                    if first && start_y_offset > 0 {
                        for y in (py + 1)..corner_y {
                            if rev && y == py + 1 {
                                canvas.marker(px, y, MarkerKind::Arrow, Dir::Up, true);
                            } else {
                                canvas.stroke(px, y, w, w, Weight::None, Weight::None);
                            }
                        }
                    }
                    // Waypoint-row gap fill (legacy: non-first segments
                    // draw a vertical through their own waypoint row).
                    if !first {
                        canvas.stroke(px, py, w, w, Weight::None, Weight::None);
                    }
                    h_run_with_corners(
                        canvas,
                        corner_y,
                        px,
                        nx,
                        w,
                        first && rev && corner_y <= py + 1,
                    );
                    for y in (corner_y + 1)..ny {
                        if last && !rev && y == ny - 1 {
                            canvas.marker(nx, y, MarkerKind::Arrow, Dir::Down, false);
                        } else {
                            canvas.stroke(nx, y, w, w, Weight::None, Weight::None);
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

/// Vertical run between two node anchors with endpoint markers
/// (geometry-driven: works for either y ordering).
#[allow(clippy::too_many_arguments)]
fn vertical_run(
    canvas: &mut BandCanvas<'_>,
    x: usize,
    from_y: usize,
    to_y: usize,
    w: Weight,
    reversed: bool,
    arrow_at_target: bool,
    arrow_at_source: bool,
) {
    let (lo, hi) = (from_y.min(to_y), from_y.max(to_y));
    for y in (lo + 1)..hi {
        let at_target_arrow = arrow_at_target && !reversed && y == to_y.wrapping_sub(1) && to_y > from_y;
        let at_source_arrow = arrow_at_source && reversed && y == from_y + 1;
        if at_target_arrow {
            canvas.marker(x, y, MarkerKind::Arrow, Dir::Down, false);
        } else if at_source_arrow {
            canvas.marker(x, y, MarkerKind::Arrow, Dir::Up, true);
        } else {
            canvas.stroke(x, y, w, w, Weight::None, Weight::None);
        }
    }
}

/// Horizontal run with corner arms at both ends. `reversed_hack` paints
/// the legacy "no room for ⇡, put it at the corner" marker at the start
/// end instead of a corner.
fn h_run_with_corners(
    canvas: &mut BandCanvas<'_>,
    row: usize,
    x_start: usize,
    x_end: usize,
    w: Weight,
    reversed_hack: bool,
) {
    if x_start == x_end {
        return;
    }
    let (lo, hi) = (x_start.min(x_end), x_start.max(x_end));
    for x in (lo + 1)..hi {
        canvas.stroke(x, row, Weight::None, Weight::None, w, w);
    }
    // Start end: the edge arrives from above (up arm) and turns toward
    // the run; far end: the edge continues downward (down arm).
    if reversed_hack {
        canvas.marker(x_start, row, MarkerKind::Arrow, Dir::Up, true);
    } else if x_start < x_end {
        canvas.stroke(x_start, row, w, Weight::None, Weight::None, w);
    } else {
        canvas.stroke(x_start, row, w, Weight::None, w, Weight::None);
    }
    if x_start < x_end {
        canvas.stroke(x_end, row, Weight::None, w, w, Weight::None);
    } else {
        canvas.stroke(x_end, row, Weight::None, w, Weight::None, w);
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
    let top = sg.y;
    let bottom = sg.y + sg.height - 1;
    let left = sg.x;
    let right = sg.x + sg.width - 1;

    canvas.stroke(left, top, n, d, n, d);
    canvas.stroke(right, top, n, d, d, n);
    canvas.stroke(left, bottom, d, n, n, d);
    canvas.stroke(right, bottom, d, n, d, n);
    for x in (left + 1)..right {
        canvas.stroke(x, top, n, n, d, d);
        canvas.stroke(x, bottom, n, n, d, d);
    }
    for y in (top + 1)..bottom {
        canvas.stroke(left, y, d, d, n, n);
        canvas.stroke(right, y, d, d, n, n);
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
        canvas.text(x, label_y, ch);
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
    if matches!(n.kind, NodeKind::Dummy) {
        if options.show_dummy_nodes {
            canvas.marker(n.x, n.y, MarkerKind::Dummy, Dir::Up, false);
        }
        return;
    }
    // Bracket row at the node's top row (content atomicity, D4). The
    // closing bracket sits at the node's declared width (arena widths
    // can exceed label+2 — the legacy arena renderer pads; heap widths
    // are always exactly label+2, so both agree). Cells between the
    // label and the bracket are left untouched, like both legacy paths.
    let close_x = n.x + n.width.saturating_sub(1);
    let mut x = n.x;
    canvas.text(x, n.y, '[');
    x += 1;
    for ch in n.label.chars() {
        if x >= close_x {
            break;
        }
        canvas.text(x, n.y, ch);
        x += 1;
    }
    if n.width >= 2 {
        canvas.text(close_x, n.y, ']');
    }
    if n.has_self_loop {
        canvas.marker(close_x + 1, n.y, MarkerKind::SelfLoop, Dir::Up, false);
    }
}

fn paint_edge_label<V: LayoutView>(view: &V, label: &LabelPlan, canvas: &mut BandCanvas<'_>) {
    let e = view.edge(label.edge_index);
    let Some(text) = e.label else { return };
    let mut x = label.x;
    canvas.text(x, label.y, '"');
    x += 1;
    for ch in text.chars() {
        canvas.text(x, label.y, ch);
        x += 1;
    }
    canvas.text(x, label.y, '"');
}
