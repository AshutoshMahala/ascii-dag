//! Emission — decode cells through the active charset and write rows
//! (temp/06 §7, R4/R1.13).
//!
//! Per band row: decode, right-trim, stream to any `core::fmt::Write`.
//! The colored emitter carries the legacy escape-transition logic over
//! both color modes; the legend writer and [`ByteSink`] serve the
//! no-alloc byte surface (R4.2/R4.3) — nothing here allocates.

use super::charset::Charset;
use super::compose::BandCanvas;
use super::plan::RenderPlan;
use super::view::LayoutView;

/// A `fmt::Write` sink over a caller byte buffer. Overflow is remembered
/// rather than panicking, so the caller can map it to
/// `E.Render.Sink.026` after the write chain unwinds (R7.3: the unit
/// `fmt::Error` itself carries no diagnostic value).
pub(crate) struct ByteSink<'a> {
    buf: &'a mut [u8],
    pos: usize,
    overflowed: bool,
}

impl<'a> ByteSink<'a> {
    pub(crate) fn new(buf: &'a mut [u8]) -> Self {
        Self {
            buf,
            pos: 0,
            overflowed: false,
        }
    }

    /// Bytes written so far.
    pub(crate) fn written(&self) -> usize {
        self.pos
    }

    /// Did any write exceed the buffer?
    pub(crate) fn overflowed(&self) -> bool {
        self.overflowed
    }
}

impl core::fmt::Write for ByteSink<'_> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let bytes = s.as_bytes();
        if self.pos + bytes.len() > self.buf.len() {
            self.overflowed = true;
            return Err(core::fmt::Error);
        }
        self.buf[self.pos..self.pos + bytes.len()].copy_from_slice(bytes);
        self.pos += bytes.len();
        Ok(())
    }
}

/// Write the legacy-format legend for every unplaced label: one line
/// per edge whose endpoints both resolve, colored by the edge's plan
/// color. Allocation-free (labels stream straight from the view).
pub(crate) fn emit_legend<V: LayoutView, W: core::fmt::Write>(
    view: &V,
    plan: &RenderPlan<'_>,
    charset: Charset,
    mode: super::color::ColorMode,
    out: &mut W,
) -> core::fmt::Result {
    if plan.legend_entries().is_empty() {
        return Ok(());
    }
    out.write_str("\nEdge labels:\n")?;

    // Endpoint labels resolve by node id. Under alloc, one pass builds
    // a sorted id→label table so the legend is O((N + E) log N) instead
    // of scanning every node per entry; the no-alloc byte path keeps
    // the linear scan (legends are small and memory is the constraint).
    #[cfg(feature = "alloc")]
    let table: alloc::vec::Vec<(usize, &str)> = {
        let mut t: alloc::vec::Vec<(usize, &str)> = (0..view.node_count())
            .map(|i| view.node(i))
            .filter(|n| !matches!(n.kind, crate::ir::NodeKind::Dummy))
            .map(|n| (n.id, n.label))
            .collect();
        t.sort_unstable_by_key(|&(id, _)| id);
        t
    };
    let find_label = |id: usize| -> Option<&str> {
        #[cfg(feature = "alloc")]
        {
            table
                .binary_search_by_key(&id, |&(nid, _)| nid)
                .ok()
                .map(|i| table[i].1)
        }
        #[cfg(not(feature = "alloc"))]
        {
            (0..view.node_count())
                .map(|i| view.node(i))
                .find(|n| n.id == id && !matches!(n.kind, crate::ir::NodeKind::Dummy))
                .map(|n| n.label)
        }
    };

    for &ei in plan.legend_entries() {
        let e = view.edge(ei);
        let Some(label) = e.label else { continue };
        // Legacy lists an entry only when both endpoints resolve.
        let (Some(from), Some(to)) = (find_label(e.from_id), find_label(e.to_id)) else {
            continue;
        };
        let arrow = charset.legend_arrow();
        write_fg(out, plan.edge_plan(ei).color, mode)?;
        write!(out, "{from} {arrow} {to}: \"{label}\"")?;
        // Plain legends emit no escapes at all (D4) — the reset only
        // closes a color that was actually opened.
        if !matches!(mode, super::color::ColorMode::None) {
            out.write_str("\x1b[0m")?;
        }
        out.write_char('\n')?;
    }
    Ok(())
}

/// Upper bound on the bytes a render can emit, computed without a
/// plan. Right-trimming only shrinks rows, so `width + 1` per row
/// bounds plain output; colored rows add at most one escape per cell
/// (worst case truecolor, 19 bytes) plus a reset. The legend bound
/// assumes every labeled edge lands in the legend.
pub(crate) fn estimate_output_size<V: LayoutView>(view: &V, colored: bool, legend: bool) -> usize {
    let per_cell = if colored { 4 + 19 } else { 4 };
    let mut size = view
        .height()
        .saturating_mul(view.width().saturating_mul(per_cell).saturating_add(8));
    if legend {
        // Endpoint labels print in full — bound by the widest node
        // label (computed once), never a fixed guess.
        let max_node_label = (0..view.node_count())
            .map(|i| view.node(i).label.len())
            .max()
            .unwrap_or(0);
        for i in 0..view.edge_count() {
            let label_len = view.edge(i).label.map_or(0, |l| l.len());
            // Worst-case per-line overhead: truecolor escape (19) +
            // indent/spacing/colon (6) + arrow (≤3) + quotes (2) +
            // reset (4) + newline (1) = 35; rounded up for headroom.
            size += label_len + 2 * max_node_label + 40;
        }
        size += 16; // header
    }
    size
}

/// Emit every row of a band, right-trimmed, `\n`-terminated.
pub(crate) fn emit_plain_band<W: core::fmt::Write>(
    canvas: &BandCanvas<'_>,
    charset: Charset,
    out: &mut W,
) -> core::fmt::Result {
    for r in 0..canvas.rows() {
        let row = canvas.row(r);
        // Right-trim on the *decoded* glyphs (a text cell holding a
        // space trims exactly like an empty cell — legacy behavior).
        let mut end = row.len();
        while end > 0 && charset.decode(row[end - 1]) == ' ' {
            end -= 1;
        }
        for cell in &row[..end] {
            out.write_char(charset.decode(*cell))?;
        }
        out.write_char('\n')?;
    }
    Ok(())
}

/// Emit a band with ANSI color escapes — the legacy colored emitter's
/// exact transition logic (escape on color change, reset when returning
/// to default, reset at end of a colored line), generalized over the
/// color mode.
pub(crate) fn emit_colored_band<W: core::fmt::Write>(
    canvas: &BandCanvas<'_>,
    charset: Charset,
    mode: super::color::ColorMode,
    out: &mut W,
) -> core::fmt::Result {
    use super::color::CellColor;
    for r in 0..canvas.rows() {
        let row = canvas.row(r);
        let color_row = canvas.color_row(r);
        let mut end = row.len();
        while end > 0 && charset.decode(row[end - 1]) == ' ' {
            end -= 1;
        }
        let mut last = CellColor::DEFAULT;
        for (i, cell) in row[..end].iter().enumerate() {
            let color = color_row.map_or(CellColor::DEFAULT, |cr| cr[i]);
            if color.is_set() && color != last {
                write_fg(out, color, mode)?;
                last = color;
            } else if !color.is_set() && last.is_set() {
                out.write_str(crate::render::colors::RESET)?;
                last = CellColor::DEFAULT;
            }
            out.write_char(charset.decode(*cell))?;
        }
        if last.is_set() {
            out.write_str(crate::render::colors::RESET)?;
        }
        out.write_char('\n')?;
    }
    Ok(())
}

/// Write a foreground escape for `color` in the given mode.
fn write_fg<W: core::fmt::Write>(
    out: &mut W,
    color: super::color::CellColor,
    mode: super::color::ColorMode,
) -> core::fmt::Result {
    use super::color::ColorMode;
    match mode {
        ColorMode::None => Ok(()),
        ColorMode::Ansi256 => {
            write!(out, "\x1b[38;5;{}m", color.as_ansi256().unwrap_or(0))
        }
        ColorMode::TrueColor => {
            let (r, g, b) = color.as_rgb().unwrap_or((0, 0, 0));
            write!(out, "\x1b[38;2;{r};{g};{b}m")
        }
    }
}
