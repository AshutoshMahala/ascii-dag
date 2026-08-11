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
use super::mem::{PlanBuf, SliceHeap};
use super::plan::{LabelPlan, PlanElement, RenderPlan};
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

    /// First global row of this band.
    pub(crate) fn y0(&self) -> usize {
        self.y0
    }
}

/// One collected horizontal-run interior: `[x0, x1]` inclusive at `row`.
///
/// Interiors are deferred and painted once per final cell (N3.2): the
/// per-cell result of replaying them in order is order-independent for
/// glyphs (per-arm max) and last-writer-wins for colors, so a row sweep
/// reproduces the sequential semantics exactly.
#[derive(Clone, Copy, Default)]
pub(crate) struct Run {
    row: usize,
    x0: usize,
    x1: usize,
    w: Weight,
    /// Paint order (edge list position + 1; 0 is "never written").
    seq: u32,
    color: CellColor,
}

/// Reusable per-render compositing scratch. Heap-backed under alloc or
/// carved from the caller's arena (WDP domain: `Canvas`) — cleared per
/// band, never reallocated.
pub(crate) struct PaintScratch<'a> {
    /// Collected h-run interiors (capacity: plan.run_capacity()).
    runs: PlanBuf<'a, Run>,
    /// Lazy max-heap backing for the flush sweep (capacity: as runs).
    heap: PlanBuf<'a, (u32, usize, u32)>,
    /// Per-cell point-write sequence stamps (width × band_rows;
    /// empty when colors are off).
    seq_plane: PlanBuf<'a, u32>,
    /// Per-band stage worklists (capacities: subgraphs / edges / nodes).
    sgs: PlanBuf<'a, usize>,
    edges: PlanBuf<'a, usize>,
    nodes: PlanBuf<'a, usize>,
    /// Rolling band-sweep state: elements intersecting the current
    /// band, as positions into the plan's y_min-sorted spatial index
    /// (capacity: nodes + edges + subgraphs).
    active: PlanBuf<'a, u32>,
    /// Spatial-index entries the sweep has consumed so far.
    sweep_cursor: usize,
    /// Expected next band start; any other `y0` restarts the sweep.
    sweep_next_y0: usize,
}

impl<'a> PaintScratch<'a> {
    /// Heap-backed scratch (std/alloc path).
    #[cfg(feature = "alloc")]
    pub(crate) fn heap_backed<V: LayoutView>(
        view: &V,
        plan: &RenderPlan<'_>,
        colored: bool,
        band_rows: usize,
    ) -> Self {
        let seq_cells = if colored { plan.width() * band_rows } else { 0 };
        Self {
            runs: PlanBuf::heap(plan.run_capacity()),
            heap: PlanBuf::heap(plan.run_capacity()),
            seq_plane: PlanBuf::heap_zeroed(seq_cells),
            sgs: PlanBuf::heap(view.subgraph_count()),
            edges: PlanBuf::heap(view.edge_count()),
            nodes: PlanBuf::heap(view.node_count()),
            active: PlanBuf::heap(view.node_count() + view.edge_count() + view.subgraph_count()),
            sweep_cursor: 0,
            sweep_next_y0: 0,
        }
    }

    /// Arena-carved scratch (no-alloc path). Exhaustion maps to
    /// `E.Render.Canvas.026`.
    pub(crate) fn carve<V: LayoutView>(
        view: &V,
        plan: &RenderPlan<'_>,
        colored: bool,
        band_rows: usize,
        arena: &crate::graph::arena::Arena<'a>,
    ) -> Result<Self, crate::GraphError> {
        let oom = || crate::GraphError::RenderCanvasTooSmall {
            needed: plan.width() * band_rows,
            got: 0,
        };
        let seq_cells = if colored { plan.width() * band_rows } else { 0 };
        Ok(Self {
            runs: PlanBuf::carve(arena, plan.run_capacity(), oom())?,
            heap: PlanBuf::carve(arena, plan.run_capacity(), oom())?,
            seq_plane: PlanBuf::carve_zeroed(arena, seq_cells, oom())?,
            sgs: PlanBuf::carve(arena, view.subgraph_count(), oom())?,
            edges: PlanBuf::carve(arena, view.edge_count(), oom())?,
            nodes: PlanBuf::carve(arena, view.node_count(), oom())?,
            active: PlanBuf::carve(
                arena,
                view.node_count() + view.edge_count() + view.subgraph_count(),
                oom(),
            )?,
            sweep_cursor: 0,
            sweep_next_y0: 0,
        })
    }

    /// Bytes an arena needs for this scratch (estimate companion).
    pub(crate) fn estimate_bytes(
        run_capacity: usize,
        subgraphs: usize,
        edges: usize,
        nodes: usize,
        colored: bool,
        width: usize,
        band_rows: usize,
    ) -> usize {
        let seq = if colored { width * band_rows } else { 0 };
        run_capacity * (core::mem::size_of::<Run>() + core::mem::size_of::<(u32, usize, u32)>())
            + seq * core::mem::size_of::<u32>()
            + (subgraphs + edges + nodes)
                * (core::mem::size_of::<usize>() + core::mem::size_of::<u32>())
    }

    /// Advance the rolling active set to the band `[y0, y0 + rows)`.
    ///
    /// Bands arrive in ascending, contiguous order (the partition tiles
    /// `0..height`), so each element enters the set once when the sweep
    /// reaches its `y_min` and is dropped once a band start passes its
    /// `y_max` — O(elements) total across all bands, where re-deriving
    /// membership per band from the sorted index would re-scan its
    /// whole prefix every time. A `y0` that is not the expected next
    /// band start restarts the sweep from the top, so any call order
    /// still yields exactly the intersecting elements.
    fn sweep_band(&mut self, elements: &[PlanElement], y0: usize, rows: usize) {
        if y0 != self.sweep_next_y0 {
            self.active.clear();
            self.sweep_cursor = 0;
        }
        let last = y0 + rows.saturating_sub(1);
        self.active.retain(|&i| elements[i as usize].y_max >= y0);
        while self.sweep_cursor < elements.len() && elements[self.sweep_cursor].y_min <= last {
            if elements[self.sweep_cursor].y_max >= y0 {
                self.active.push(self.sweep_cursor as u32);
            }
            self.sweep_cursor += 1;
        }
        self.sweep_next_y0 = last + 1;
    }
}

/// Edge-stage paint sink: point writes (verticals, corners, markers) go
/// straight to the canvas — stamping their paint order into a per-cell
/// plane when colors are on — while h-run interiors are collected and
/// painted by [`EdgePainter::flush`] as merged runs.
struct EdgePainter<'p, 'c, 'a, 's> {
    canvas: &'c mut BandCanvas<'a>,
    scratch: &'p mut PaintScratch<'s>,
    /// Current edge's paint order (list index + 1).
    seq: u32,
    /// Current edge's resolved color.
    color: CellColor,
}

impl<'p, 'c, 'a, 's> EdgePainter<'p, 'c, 'a, 's> {
    fn new(canvas: &'c mut BandCanvas<'a>, scratch: &'p mut PaintScratch<'s>) -> Self {
        scratch.runs.clear();
        scratch.seq_plane.refill_default();
        Self {
            canvas,
            scratch,
            seq: 0,
            color: CellColor::DEFAULT,
        }
    }

    /// Begin painting the edge at list position `index`.
    fn begin_edge(&mut self, index: usize, color: CellColor) {
        self.seq = index as u32 + 1;
        self.color = color;
    }

    #[inline]
    fn stamp(&mut self, x: usize, y: usize) {
        if self.scratch.seq_plane.len() > 0 {
            if let Some(i) = self.canvas.idx(x, y) {
                self.scratch.seq_plane.as_mut_slice()[i] = self.seq;
            }
        }
    }

    #[inline]
    fn stroke(
        &mut self,
        x: usize,
        y: usize,
        up: Weight,
        down: Weight,
        left: Weight,
        right: Weight,
    ) {
        self.canvas
            .stroke(x, y, up, down, left, right, Paint::Color(self.color));
        self.stamp(x, y);
    }

    #[inline]
    fn marker(&mut self, x: usize, y: usize, kind: MarkerKind, dir: Dir, dashed: bool) {
        self.canvas
            .marker(x, y, kind, dir, dashed, Paint::Color(self.color));
        self.stamp(x, y);
    }

    /// Collect a horizontal-run interior (`x0..=x1`, both-arm weight
    /// `w`). Rows outside the band are dropped here — the flush would
    /// clip them anyway, and skipping keeps scratch bounded per band.
    #[inline]
    fn run(&mut self, row: usize, x0: usize, x1: usize, w: Weight) {
        if x0 > x1 || row < self.canvas.y0() || row >= self.canvas.y0() + self.canvas.rows() {
            return;
        }
        self.scratch.runs.push(Run {
            row,
            x0,
            x1,
            w,
            seq: self.seq,
            color: self.color,
        });
    }

    /// Paint every collected interior, one write per final cell.
    fn flush(self) {
        self.scratch
            .runs
            .as_mut_slice()
            .sort_unstable_by_key(|r| (r.row, r.x0));
        let runs = self.scratch.runs.as_slice();
        let mut i = 0;
        while i < runs.len() {
            let row = runs[i].row;
            let end = runs[i..]
                .iter()
                .position(|r| r.row != row)
                .map_or(runs.len(), |p| i + p);
            flush_row(
                &runs[i..end],
                row,
                self.scratch.seq_plane.as_slice(),
                &mut self.scratch.heap,
                self.canvas,
            );
            i = end;
        }
    }
}

/// Sweep one row's interiors left-to-right. Per cell: arms = per-weight
/// max over covering runs (tracked as running per-weight extents — a
/// weight is active at `x` iff some activated run of that weight reaches
/// `x`); color = the covering run painted latest (lazy max-heap by seq),
/// unless a later point write already stamped the cell.
fn flush_row(
    runs: &[Run],
    row: usize,
    seq_plane: &[u32],
    heap_buf: &mut PlanBuf<'_, (u32, usize, u32)>,
    canvas: &mut BandCanvas<'_>,
) {
    // Extent per weight tier (edge interiors are Light or Dashed; keep
    // all four tiers so future weights need no rework).
    let mut extent = [None::<usize>; 4];
    // (seq, x1, color) ordered by seq; expired tops are popped lazily.
    let mut heap = SliceHeap::new(heap_buf);
    let colored = canvas.colors.is_some();

    let mut i = 0;
    let mut x = runs[0].x0;
    while i < runs.len() {
        // Activate every run starting at or before `x`.
        while i < runs.len() && runs[i].x0 <= x {
            let r = &runs[i];
            let t = r.w as usize;
            extent[t] = Some(extent[t].map_or(r.x1, |e: usize| e.max(r.x1)));
            if colored {
                heap.push((r.seq, r.x1, r.color.raw()));
            }
            i += 1;
        }
        let next_start = runs.get(i).map(|r| r.x0);
        // Paint until coverage runs out or the next run activates.
        let cover_end = extent.iter().flatten().copied().max().unwrap_or(0);
        let stop = match next_start {
            Some(ns) => cover_end.min(ns - 1),
            None => cover_end,
        };
        while x <= stop {
            let w = if extent[Weight::Double as usize].is_some_and(|e| e >= x) {
                Weight::Double
            } else if extent[Weight::Light as usize].is_some_and(|e| e >= x) {
                Weight::Light
            } else if extent[Weight::Dashed as usize].is_some_and(|e| e >= x) {
                Weight::Dashed
            } else {
                x += 1;
                continue;
            };
            if let Some(ci) = canvas.idx(x, row) {
                canvas.cells[ci] =
                    canvas.cells[ci].painted_stroke(Weight::None, Weight::None, w, w);
                if colored {
                    while let Some(&(_, x1, _)) = heap.peek() {
                        if x1 < x {
                            heap.pop();
                        } else {
                            break;
                        }
                    }
                    if let Some(&(seq, _, color)) = heap.peek() {
                        if seq > seq_plane[ci] {
                            if let Some(plane) = &mut canvas.colors {
                                plane[ci] = CellColor::from_raw(color);
                            }
                        }
                    }
                }
            }
            x += 1;
        }
        match next_start {
            Some(ns) => x = x.max(ns),
            None => break,
        }
    }
}

/// Composite every element intersecting the canvas band. Elements come
/// from the plan's spatial index, so per-band work is proportional to
/// what the band shows, not to the whole graph.
pub(crate) fn composite_band<V: LayoutView>(
    view: &V,
    plan: &RenderPlan<'_>,
    options: &RenderOptions,
    canvas: &mut BandCanvas<'_>,
    scratch: &mut PaintScratch<'_>,
) {
    use super::plan::ElementKind;
    let (y0, rows) = (canvas.y0(), canvas.rows());
    // The sweep's active set is sorted by row; painting must happen in
    // LIST order (colors are last-writer-wins, marker overlap is
    // last-wins), so each stage's band subset is re-sorted by element
    // index.
    scratch.sgs.clear();
    scratch.edges.clear();
    scratch.nodes.clear();
    let elements = plan.elements();
    scratch.sweep_band(elements, y0, rows);
    for &ei in scratch.active.as_slice() {
        let el = &elements[ei as usize];
        match el.kind {
            ElementKind::Subgraph => scratch.sgs.push(el.index),
            ElementKind::Edge => scratch.edges.push(el.index),
            ElementKind::Node => scratch.nodes.push(el.index),
        }
    }
    scratch.sgs.as_mut_slice().sort_unstable();
    scratch.edges.as_mut_slice().sort_unstable();
    scratch.nodes.as_mut_slice().sort_unstable();
    // Z0+Z1: strokes — subgraph borders and edges (commutative merges).
    for i in 0..scratch.sgs.len() {
        paint_subgraph_border(view, plan, scratch.sgs.as_slice()[i], canvas);
    }
    let mut painter = EdgePainter::new(canvas, scratch);
    for pos in 0.. {
        let i = match painter.scratch.edges.as_slice().get(pos) {
            Some(&i) => i,
            None => break,
        };
        painter.begin_edge(i, plan.edge_plan(i).color);
        paint_edge(view, plan, i, &mut painter);
    }
    painter.flush();
    // Z2: edge labels. Placement gate mirrors the three legacy paths:
    // plain and colored-without-legend place geometrically; only the
    // colored-with-legend path additionally vetoes rows hosting nodes.
    let colored = !matches!(options.color_mode, ColorMode::None);
    let band_has = |y: usize| y >= y0 && y < y0 + rows;
    for label in plan.labels() {
        if !band_has(label.y) {
            continue;
        }
        if label.paints(colored, options.legend) {
            paint_edge_label(view, plan, label, canvas);
        }
    }
    // Z3: nodes.
    for i in 0..scratch.nodes.len() {
        paint_node(view, scratch.nodes.as_slice()[i], options, canvas);
    }
    // Z4: subgraph labels (always readable; colors untouched).
    for i in 0..scratch.sgs.len() {
        paint_subgraph_label(view, plan, scratch.sgs.as_slice()[i], canvas);
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
    p: &mut EdgePainter<'_, '_, '_, '_>,
) {
    let e = view.edge(edge_index);
    // Horizontal trunks paint through the X mirror; the Y path below
    // is the byte-frozen legacy compositor.
    if matches!(e.flow_axis, crate::ir::FlowAxis::X) {
        return paint_edge_x(view, plan, edge_index, p);
    }
    let ep = plan.edge_plan(edge_index);
    let w = ep.weight.arm();
    let rev = e.reversed;
    // Which style marker each geometric side carries — the shared
    // resolution placement also uses, so both agree cell-for-cell.
    let (from_m, to_m) = ep.resolved_markers(rev);

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
                if to_m && y == off(e.to_y, -1) {
                    p.marker(e.from_x, y, MarkerKind::Arrow, fwd, rev);
                } else if from_m && y == off(e.from_y, 1) {
                    p.marker(e.from_x, y, MarkerKind::Arrow, bwd, rev);
                } else {
                    p.stroke(e.from_x, y, w, w, Weight::None, Weight::None);
                }
            }
        }
        PathRef::Corner { bend_at } => {
            for y in between(e.from_y, bend_at) {
                if from_m && y == off(e.from_y, 1) {
                    p.marker(e.from_x, y, MarkerKind::Arrow, bwd, rev);
                } else {
                    p.stroke(e.from_x, y, w, w, Weight::None, Weight::None);
                }
            }
            let bend_adjacent = (bend_at as isize - e.from_y as isize) * dir <= 1;
            h_run_with_corners(
                p,
                bend_at,
                e.from_x,
                e.to_x,
                w,
                from_m && bend_adjacent,
                rev,
                dir,
            );
            for y in between(bend_at, e.to_y) {
                if to_m && y == off(e.to_y, -1) {
                    p.marker(e.to_x, y, MarkerKind::Arrow, fwd, rev);
                } else {
                    p.stroke(e.to_x, y, w, w, Weight::None, Weight::None);
                }
            }
        }
        PathRef::SideChannel {
            channel_at,
            span_start,
            span_end,
        } => {
            h_run_with_corners(p, span_start, e.from_x, channel_at, w, false, rev, dir);
            for y in between(span_start, span_end) {
                p.stroke(channel_at, y, w, w, Weight::None, Weight::None);
            }
            h_run_with_corners(p, span_end, channel_at, e.to_x, w, false, rev, dir);
            for y in between(span_end, e.to_y) {
                if to_m && y == off(e.to_y, -1) {
                    p.marker(e.to_x, y, MarkerKind::Arrow, fwd, rev);
                } else {
                    p.stroke(e.to_x, y, w, w, Weight::None, Weight::None);
                }
            }
        }
        PathRef::MultiSegment {
            waypoints,
            start_offset,
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
                        if first && from_m && y == off(py, 1) {
                            p.marker(px, y, MarkerKind::Arrow, bwd, rev);
                        } else if last && to_m && y == off(ny, -1) {
                            p.marker(px, y, MarkerKind::Arrow, fwd, rev);
                        } else {
                            p.stroke(px, y, w, w, Weight::None, Weight::None);
                        }
                    }
                    // The waypoint row itself carries the vertical for
                    // non-first segments (legacy gap fill).
                    if !first {
                        p.stroke(px, py, w, w, Weight::None, Weight::None);
                    }
                } else {
                    let corner_y = off(py, 1 + if first { start_offset as isize } else { 0 });
                    if first && start_offset > 0 {
                        for y in between(py, corner_y) {
                            if from_m && y == off(py, 1) {
                                p.marker(px, y, MarkerKind::Arrow, bwd, rev);
                            } else {
                                p.stroke(px, y, w, w, Weight::None, Weight::None);
                            }
                        }
                    }
                    if !first {
                        p.stroke(px, py, w, w, Weight::None, Weight::None);
                    }
                    let bend_adjacent = (corner_y as isize - py as isize) * dir <= 1;
                    h_run_with_corners(
                        p,
                        corner_y,
                        px,
                        nx,
                        w,
                        first && from_m && bend_adjacent,
                        rev,
                        dir,
                    );
                    for y in between(corner_y, ny) {
                        if last && to_m && y == off(ny, -1) {
                            p.marker(nx, y, MarkerKind::Arrow, fwd, rev);
                        } else {
                            p.stroke(nx, y, w, w, Weight::None, Weight::None);
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

/// The X-flow mirror of the Y paint path (temp/08 P3): trunks run
/// horizontally, the bend scalar in `Corner`/`MultiSegment` is a
/// COLUMN, markers point `→`/`←`, and vertical segments are the
/// cross-axis distribution runs. Formulas mirror `paint_edge` with
/// the axes swapped, including the adjacent-bend source-marker
/// fallback (a bend one cell past the source face leaves the marker
/// no trunk cell of its own, so it takes the corner cell).
fn paint_edge_x<V: LayoutView>(
    view: &V,
    plan: &RenderPlan,
    edge_index: usize,
    p: &mut EdgePainter<'_, '_, '_, '_>,
) {
    let e = view.edge(edge_index);
    let ep = plan.edge_plan(edge_index);
    let w = ep.weight.arm();
    let rev = e.reversed;
    let (from_m, to_m) = ep.resolved_markers(rev);

    // Flow sign from the geometry: +1 rightward, −1 leftward.
    let dir: isize = if e.to_x >= e.from_x { 1 } else { -1 };
    let off = |x: usize, k: isize| (x as isize + k * dir) as usize;
    let fwd = if dir > 0 { Dir::Right } else { Dir::Left };
    let bwd = if dir > 0 { Dir::Left } else { Dir::Right };
    let n = Weight::None;

    match e.path {
        PathRef::Direct | PathRef::Spline { .. } => {
            for x in between(e.from_x, e.to_x) {
                if to_m && x == off(e.to_x, -1) {
                    p.marker(x, e.from_y, MarkerKind::Arrow, fwd, rev);
                } else if from_m && x == off(e.from_x, 1) {
                    p.marker(x, e.from_y, MarkerKind::Arrow, bwd, rev);
                } else {
                    p.stroke(x, e.from_y, n, n, w, w);
                }
            }
        }
        PathRef::Corner { bend_at: bend_x } => {
            for x in between(e.from_x, bend_x) {
                if from_m && x == off(e.from_x, 1) {
                    p.marker(x, e.from_y, MarkerKind::Arrow, bwd, rev);
                } else {
                    p.stroke(x, e.from_y, n, n, w, w);
                }
            }
            let bend_adjacent = bend_x == off(e.from_x, 1);
            v_run_with_corners(
                p,
                bend_x,
                e.from_y,
                e.to_y,
                w,
                from_m && bend_adjacent,
                rev,
                dir,
            );
            for x in between(bend_x, e.to_x) {
                if to_m && x == off(e.to_x, -1) {
                    p.marker(x, e.to_y, MarkerKind::Arrow, fwd, rev);
                } else {
                    p.stroke(x, e.to_y, n, n, w, w);
                }
            }
        }
        // Not produced by the layout today, but public and
        // round-trippable through JSON — the X mirror of the Y arm:
        // `channel_at` is the far ROW, `span_start`/`span_end` the
        // COLUMNS where the flow enters and leaves it.
        PathRef::SideChannel {
            channel_at,
            span_start,
            span_end,
        } => {
            for x in between(e.from_x, span_start) {
                if from_m && x == off(e.from_x, 1) {
                    p.marker(x, e.from_y, MarkerKind::Arrow, bwd, rev);
                } else {
                    p.stroke(x, e.from_y, n, n, w, w);
                }
            }
            let source_adjacent = span_start == off(e.from_x, 1);
            v_run_with_corners(
                p,
                span_start,
                e.from_y,
                channel_at,
                w,
                from_m && source_adjacent,
                rev,
                dir,
            );
            for x in between(span_start, span_end) {
                p.stroke(x, channel_at, n, n, w, w);
            }
            v_run_with_corners(p, span_end, channel_at, e.to_y, w, false, rev, dir);
            for x in between(span_end, e.to_x) {
                if to_m && x == off(e.to_x, -1) {
                    p.marker(x, e.to_y, MarkerKind::Arrow, fwd, rev);
                } else {
                    p.stroke(x, e.to_y, n, n, w, w);
                }
            }
        }
        PathRef::MultiSegment {
            waypoints,
            start_offset,
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
                if py == ny {
                    for x in between(px, nx) {
                        if first && from_m && x == off(px, 1) {
                            p.marker(x, py, MarkerKind::Arrow, bwd, rev);
                        } else if last && to_m && x == off(nx, -1) {
                            p.marker(x, py, MarkerKind::Arrow, fwd, rev);
                        } else {
                            p.stroke(x, py, n, n, w, w);
                        }
                    }
                    // The waypoint column carries the trunk for
                    // non-first segments (mirror of the legacy gap fill).
                    if !first {
                        p.stroke(px, py, n, n, w, w);
                    }
                } else {
                    let bend_x = off(px, 1 + if first { start_offset as isize } else { 0 });
                    if first && start_offset > 0 {
                        for x in between(px, bend_x) {
                            if from_m && x == off(px, 1) {
                                p.marker(x, py, MarkerKind::Arrow, bwd, rev);
                            } else {
                                p.stroke(x, py, n, n, w, w);
                            }
                        }
                    }
                    if !first {
                        p.stroke(px, py, n, n, w, w);
                    }
                    let bend_adjacent = bend_x == off(px, 1);
                    v_run_with_corners(
                        p,
                        bend_x,
                        py,
                        ny,
                        w,
                        first && from_m && bend_adjacent,
                        rev,
                        dir,
                    );
                    for x in between(bend_x, nx) {
                        if last && to_m && x == off(nx, -1) {
                            p.marker(x, ny, MarkerKind::Arrow, fwd, rev);
                        } else {
                            p.stroke(x, ny, n, n, w, w);
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

/// Vertical run with corner arms at both ends — the X-flow mirror of
/// [`h_run_with_corners`]. The start end's horizontal arm points back
/// toward the source (against the flow), the far end's arm continues
/// with the flow toward the target.
#[allow(clippy::too_many_arguments)]
fn v_run_with_corners(
    p: &mut EdgePainter<'_, '_, '_, '_>,
    col: usize,
    y_start: usize,
    y_end: usize,
    w: Weight,
    marker_hack: bool,
    dashed: bool,
    dir: isize,
) {
    if y_start == y_end {
        return;
    }
    let n = Weight::None;
    for y in between(y_start, y_end) {
        p.stroke(col, y, w, w, n, n);
    }
    // Horizontal arms: anti-flow at the start (toward the source),
    // flow at the end (toward the target).
    let (src_left, src_right) = if dir > 0 { (w, n) } else { (n, w) };
    let (tgt_left, tgt_right) = if dir > 0 { (n, w) } else { (w, n) };
    let (toward_end_up, toward_end_down) = if y_start < y_end { (n, w) } else { (w, n) };
    if marker_hack {
        // The bend sits immediately past the source face, so the
        // source-side marker has no trunk cell of its own — it takes
        // the corner cell instead (the X mirror of the Y path's
        // no-room-for-the-arrow rule).
        let bwd = if dir > 0 { Dir::Left } else { Dir::Right };
        p.marker(col, y_start, MarkerKind::Arrow, bwd, dashed);
    } else {
        p.stroke(
            col,
            y_start,
            toward_end_up,
            toward_end_down,
            src_left,
            src_right,
        );
    }
    p.stroke(
        col,
        y_end,
        toward_end_down,
        toward_end_up,
        tgt_left,
        tgt_right,
    );
}

/// Horizontal run with corner arms at both ends. The start end's
/// vertical arm points back toward the source (against the flow), the
/// far end's arm continues with the flow toward the target.
/// `reversed_hack` paints the legacy "no room for the reversed arrow,
/// put it at the corner" marker at the start end instead of a corner.
#[allow(clippy::too_many_arguments)]
fn h_run_with_corners(
    p: &mut EdgePainter<'_, '_, '_, '_>,
    row: usize,
    x_start: usize,
    x_end: usize,
    w: Weight,
    marker_hack: bool,
    dashed: bool,
    dir: isize,
) {
    if x_start == x_end {
        return;
    }
    let n = Weight::None;
    let (lo, hi) = (x_start.min(x_end), x_start.max(x_end));
    if lo + 1 < hi {
        p.run(row, lo + 1, hi - 1, w);
    }
    // Vertical arms: anti-flow at the start (toward the source), flow at
    // the end (toward the target).
    let (src_up, src_down) = if dir > 0 { (w, n) } else { (n, w) };
    let (tgt_up, tgt_down) = if dir > 0 { (n, w) } else { (w, n) };
    if marker_hack {
        let bwd = if dir > 0 { Dir::Up } else { Dir::Down };
        p.marker(x_start, row, MarkerKind::Arrow, bwd, dashed);
    } else if x_start < x_end {
        p.stroke(x_start, row, src_up, src_down, n, w);
    } else {
        p.stroke(x_start, row, src_up, src_down, w, n);
    }
    if x_start < x_end {
        p.stroke(x_end, row, tgt_up, tgt_down, w, n);
    } else {
        p.stroke(x_end, row, tgt_up, tgt_down, n, w);
    }
}

// ── Subgraphs ────────────────────────────────────────────────────────────

fn paint_subgraph_border<V: LayoutView>(
    view: &V,
    plan: &RenderPlan<'_>,
    index: usize,
    canvas: &mut BandCanvas<'_>,
) {
    let sg = view.subgraph(index);
    if sg.width < 2 || sg.height < 2 {
        return;
    }
    let sp = plan.subgraph_plan(index);
    // `SubgraphBorder::None` groups without ink.
    let Some(d) = sp.border.arm() else { return };
    let n = Weight::None;
    // Legacy borders never write color; an explicit style color does.
    let k = if sp.color.is_set() {
        Paint::Color(sp.color)
    } else {
        Paint::KeepColor
    };
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
    use super::node_content::NodeKindTag;
    let n = view.node(index);
    if matches!(n.kind, NodeKind::Dummy) {
        if options.show_dummy_nodes {
            let default = Paint::Color(CellColor::DEFAULT);
            canvas.marker(n.x, n.y, MarkerKind::Dummy, Dir::Up, false, default);
        }
        return;
    }
    // Node ink explicitly resets to the terminal default — the legacy
    // behavior (nodes are never colored by edge ink bleeding through).
    let paint = Paint::Color(CellColor::DEFAULT);
    let height = n.height.max(1);

    // Every node fills its layout-reserved area according to its
    // DECLARED kind — the content channel is the only steering (there
    // is no style override). A box needs a border row above and below
    // the label and both side columns, so a boxed declaration on a
    // too-small area falls back to the simple look. A custom
    // declaration without a painter is a blank node: the area stays
    // reserved and unpainted.
    match NodeKindTag::from_u8(n.content_tag) {
        NodeKindTag::Boxed if height >= 3 && n.width >= 2 => paint_node_boxed(&n, canvas, paint),
        NodeKindTag::Custom => {
            let (painter, payload) = view.node_custom(index);
            if let Some(painter) = painter {
                // Node-local rows visible in this band — tall nodes
                // replay per band; the ctx range lets painters skip
                // clipped rows.
                let band_lo = canvas.y0();
                let band_hi = band_lo + canvas.rows();
                let g_lo = n.y.max(band_lo);
                let g_hi = (n.y + height).min(band_hi);
                let visible_rows = if g_lo < g_hi {
                    (g_lo - n.y, g_hi - n.y)
                } else {
                    (0, 0)
                };
                let mut region = super::region::NodeRegion::new(
                    canvas,
                    n.x,
                    n.y,
                    n.width,
                    height,
                    CellColor::DEFAULT,
                );
                painter(
                    &mut region,
                    super::region::NodePaintCtx {
                        node_id: n.id,
                        label: n.label,
                        width: n.width,
                        height,
                        charset: options.charset,
                        visible_rows,
                        payload,
                    },
                );
            }
            // No painter → blank: skip painting entirely.
        }
        _ => paint_node_simple(&n, canvas, paint),
    }

    // The self-loop marker is engine ink at the IR-computed cell
    // (temp/08 D5) — outside every painter's region by design. For
    // vertical flows the cell equals the legacy right-of-top-row
    // position, byte-for-byte.
    if let Some((mx, my)) = n.self_loop_at {
        canvas.marker(mx, my, MarkerKind::SelfLoop, Dir::Up, false, paint);
    }
}

/// The classic `[label]` painter: delimiters + label on the top row;
/// any extra reserved rows stay blank.
fn paint_node_simple(n: &super::view::NodeRef<'_>, canvas: &mut BandCanvas<'_>, paint: Paint) {
    let (open, close) = ('[', ']');
    // Border row at the node's top row (content atomicity, D4). The
    // closing delimiter sits at the node's declared width (arena widths
    // can exceed label+2 — the padded interior cells are left
    // untouched, like the legacy renderers).
    let close_x = n.x + n.width.saturating_sub(1);
    let mut x = n.x;
    canvas.text(x, n.y, open, paint);
    x += 1;
    for ch in n.label.chars() {
        if x >= close_x {
            break;
        }
        canvas.text(x, n.y, ch, paint);
        x += 1;
    }
    if n.width >= 2 {
        canvas.text(close_x, n.y, close, paint);
    }
}

/// A light-stroke box spanning the node's full `width × height`, label
/// inside. Strokes merge with crossing edge ink into proper junctions
/// and decode to `+ - |` under the ASCII charset.
fn paint_node_boxed(n: &super::view::NodeRef<'_>, canvas: &mut BandCanvas<'_>, paint: Paint) {
    let l = Weight::Light;
    let no = Weight::None;
    let top = n.y;
    let bottom = n.y + n.height.max(1) - 1;
    let left = n.x;
    let right = n.x + n.width - 1;

    // A tall node is replayed once per band it spans; every loop below
    // is clipped to the current band so total work across all bands is
    // O(width + height), not O(spans × extent) — the canvas would clip
    // out-of-band writes anyway, but iterating to be clipped is the
    // cost this avoids.
    let band_lo = canvas.y0();
    let band_hi = band_lo + canvas.rows();
    let in_band = |y: usize| y >= band_lo && y < band_hi;

    if in_band(top) {
        canvas.stroke(left, top, no, l, no, l, paint);
        canvas.stroke(right, top, no, l, l, no, paint);
        for x in (left + 1)..right {
            canvas.stroke(x, top, no, no, l, l, paint);
        }
    }
    if in_band(bottom) {
        canvas.stroke(left, bottom, l, no, no, l, paint);
        canvas.stroke(right, bottom, l, no, l, no, paint);
        for x in (left + 1)..right {
            canvas.stroke(x, bottom, no, no, l, l, paint);
        }
    }
    let side_lo = (top + 1).max(band_lo);
    let side_hi = bottom.min(band_hi);
    for y in side_lo..side_hi {
        canvas.stroke(left, y, l, l, no, no, paint);
        canvas.stroke(right, y, l, l, no, no, paint);
    }
    // Label inside, one row below the top border, one pad column in.
    let label_y = top + 1;
    if in_band(label_y) {
        let max_len = n.width.saturating_sub(4);
        let mut x = left + 2;
        for ch in n.label.chars().take(max_len) {
            canvas.text(x, label_y, ch, paint);
            x += 1;
        }
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

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;
    use crate::graph::Graph;

    /// Composite one band and return its emitted text.
    fn band_text<V: LayoutView>(
        view: &V,
        plan: &RenderPlan<'_>,
        options: &RenderOptions,
        scratch: &mut PaintScratch<'_>,
        cells: &mut [Cell],
        y0: usize,
        rows: usize,
    ) -> alloc::string::String {
        let mut canvas = BandCanvas::new(cells, None, plan.width(), y0, rows);
        composite_band(view, plan, options, &mut canvas, scratch);
        let mut out = alloc::string::String::new();
        super::super::emit::emit_plain_band(&canvas, options.charset, &mut out).unwrap();
        out
    }

    /// The rolling sweep serves the driver loops' ascending band order
    /// incrementally; any other order must restart it and still yield
    /// exactly the band's elements. Reverse order (plus a repeated
    /// band) hits the reset path on every call.
    #[test]
    fn sweep_is_call_order_independent() {
        let mut g = Graph::new();
        for i in 0..12 {
            g.add_node(i, "N");
        }
        for i in 0..11 {
            g.add_edge(i, i + 1, None);
        }
        let ir = g.compute_layout();
        let mut options = RenderOptions::plain();
        options.band_rows_cap = 3;
        let plan = RenderPlan::build(&ir, &options);
        assert!(plan.band_count() > 2, "corpus must actually band");
        let mut cells = alloc::vec![Cell::EMPTY; plan.width() * plan.max_band_rows()];

        let mut fwd = PaintScratch::heap_backed(&ir, &plan, false, plan.max_band_rows());
        let ascending: alloc::vec::Vec<_> = plan
            .band_ranges()
            .iter()
            .map(|&(y0, rows)| band_text(&ir, &plan, &options, &mut fwd, &mut cells, y0, rows))
            .collect();

        let mut rev = PaintScratch::heap_backed(&ir, &plan, false, plan.max_band_rows());
        for (i, &(y0, rows)) in plan.band_ranges().iter().enumerate().rev() {
            let got = band_text(&ir, &plan, &options, &mut rev, &mut cells, y0, rows);
            assert_eq!(got, ascending[i], "band {i} (y0={y0}) reverse order");
            let again = band_text(&ir, &plan, &options, &mut rev, &mut cells, y0, rows);
            assert_eq!(again, ascending[i], "band {i} (y0={y0}) repeated");
        }
    }
}
