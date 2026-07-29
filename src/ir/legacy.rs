//! Deprecated arena-IR render entry points — thin wrappers over the
//! unified engine (temp/06 §9, Q7/R6.1). Removed in 0.11.
//!
//! The plain byte surface keeps its documented contract: size buffers
//! with [`LayoutIRArena::estimate_render_size`] (which now returns
//! engine-adequate sizes) and `None` still means "buffers too small".
//! The engine's working memory is carved from the caller's scratch
//! buffer, so the zero-allocation guarantee is preserved.

use crate::graph::arena::Arena;
use crate::ir::arena::LayoutIRArena;
use crate::render::engine::RenderOptions;

/// Reinterpret a `usize` scratch buffer as arena bytes. Sound: every
/// bit pattern is a valid `usize`, so scribbling raw bytes into it and
/// letting the caller reuse it later stays defined behavior. (The same
/// is NOT true of `char`/`bool` buffers — those are never reused here.)
fn scratch_as_bytes(scratch: &mut [usize]) -> &mut [u8] {
    let len = core::mem::size_of_val(scratch);
    // Safety: same allocation, alignment 8 ≥ 1, len covers the slice
    // exactly, and u8 has no validity constraints.
    unsafe { core::slice::from_raw_parts_mut(scratch.as_mut_ptr().cast::<u8>(), len) }
}

/// Recover the `Palette` a caller-supplied color slice came from
/// (documented legacy flow: `Palette::X.colors()`); unknown slices fall
/// back to the default palette.
#[cfg(feature = "alloc")]
fn palette_from_slice(palette: &[u8]) -> crate::render::colors::Palette {
    use crate::render::colors::Palette;
    for p in [Palette::Ansi, Palette::AnsiDark, Palette::AnsiLight] {
        if p.colors() == palette {
            return p;
        }
    }
    Palette::Ansi
}

impl LayoutIRArena<'_> {
    /// Plain-text render into a byte buffer, zero allocation.
    #[deprecated(
        since = "0.10.0",
        note = "use `render_to_bytes(&RenderOptions::plain(), &arena, buffer)` with `estimate_render_arena_size`"
    )]
    pub fn render_to_buffer(
        &self,
        buffer: &mut [u8],
        _line_buffer: &mut [char],
        scratch_buffer: &mut [usize],
    ) -> Option<usize> {
        if self.is_empty() {
            return Some(0);
        }
        let arena = Arena::new(scratch_as_bytes(scratch_buffer));
        self.render_to_bytes(&RenderOptions::plain(), &arena, buffer)
            .ok()
    }

    /// Buffer sizes for [`Self::render_to_buffer`]:
    /// `(output_bytes, scratch_len_in_usize)`.
    #[deprecated(
        since = "0.10.0",
        note = "use `estimate_render_output_size` and `estimate_render_arena_size`"
    )]
    pub fn estimate_render_size(&self) -> (usize, usize) {
        let options = RenderOptions::plain();
        let output = self.estimate_render_output_size(&options);
        let scratch_len = self
            .estimate_render_arena_size(&options)
            .div_ceil(core::mem::size_of::<usize>())
            + 8;
        (output, scratch_len)
    }

    /// Modulo edge coloring into a caller buffer (matches the heap
    /// `LayoutIR::compute_edge_colors`). Returns the colors used, or
    /// `None` if `color_buffer` is too small.
    #[deprecated(
        since = "0.10.0",
        note = "the engine assigns palette colors internally; use `RenderOptions::colored` or an `edge_style_fn`"
    )]
    pub fn compute_edge_colors(
        &self,
        color_buffer: &mut [usize],
        palette_size: usize,
    ) -> Option<usize> {
        let n = self.edge_count();
        if n == 0 {
            return Some(0);
        }
        if color_buffer.len() < n || palette_size == 0 {
            return None;
        }
        for (i, c) in color_buffer[..n].iter_mut().enumerate() {
            *c = i % palette_size;
        }
        Some(palette_size.min(n))
    }

    /// ANSI-colored render (no legend) into a byte buffer.
    #[cfg(feature = "alloc")]
    #[deprecated(
        since = "0.10.0",
        note = "use `render_to_bytes(&RenderOptions::colored(palette), ...)` (set `legend = false` to match)"
    )]
    pub fn render_to_buffer_colored(
        &self,
        buffer: &mut [u8],
        _line_buffer: &mut [char],
        _color_buffer: &mut [u8],
        _edge_colors: &[usize],
        palette: &[u8],
    ) -> Option<usize> {
        if self.is_empty() {
            return Some(0);
        }
        let mut options = RenderOptions::colored(palette_from_slice(palette));
        options.legend = false;
        let mut sink = crate::render::engine::emit::ByteSink::new(buffer);
        crate::render::engine::render_into(self, &options, &mut sink).ok()?;
        Some(sink.written())
    }

    /// ANSI-colored render with the skipped-label legend.
    #[cfg(feature = "alloc")]
    #[deprecated(
        since = "0.10.0",
        note = "use `render_to_bytes(&RenderOptions::colored(palette), ...)`"
    )]
    #[allow(clippy::too_many_arguments)]
    pub fn render_to_buffer_colored_with_legend(
        &self,
        buffer: &mut [u8],
        _line_buffer: &mut [char],
        _color_buffer: &mut [u8],
        _edge_colors: &[usize],
        palette: &[u8],
        _skipped_buffer: &mut [bool],
    ) -> Option<usize> {
        if self.is_empty() {
            return Some(0);
        }
        let options = RenderOptions::colored(palette_from_slice(palette));
        let mut sink = crate::render::engine::emit::ByteSink::new(buffer);
        crate::render::engine::render_into(self, &options, &mut sink).ok()?;
        Some(sink.written())
    }
}
