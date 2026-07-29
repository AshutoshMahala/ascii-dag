//! Deprecated heap-IR render entry points — thin wrappers over the
//! unified engine (temp/06 §9, Q7/R6.1).
//!
//! Every function here delegates to the engine and is removed in 0.11;
//! each deprecation note names its replacement. Output follows the
//! engine's canonical rendering (the 0.10.0 rulings): junction cells
//! merge instead of last-edge-wins, so a handful of corner glyphs
//! differ from 0.9.x in graphs with overlapping edge routes.

use crate::ir::LayoutIR;
use crate::render::colors::Palette;
use crate::render::engine::RenderOptions;
use alloc::string::String;

/// Colored options matching `render_scanline_colored` (no legend).
fn colored_no_legend(palette: Palette) -> RenderOptions {
    let mut options = RenderOptions::colored(palette);
    options.legend = false;
    options
}

impl LayoutIR<'_> {
    /// Plain-text render.
    #[deprecated(
        since = "0.10.0",
        note = "use `render_string(&RenderOptions::plain())`"
    )]
    pub fn render_scanline(&self) -> String {
        self.render_string(&RenderOptions::plain())
    }

    /// Plain-text render into an existing `String`.
    #[deprecated(
        since = "0.10.0",
        note = "use `render_with(&RenderOptions::plain(), output)`"
    )]
    pub fn render_scanline_to(&self, output: &mut String) {
        let _ = self.render_with(&RenderOptions::plain(), output);
    }

    /// Plain-text render; `line_buffer` is no longer needed.
    #[deprecated(
        since = "0.10.0",
        note = "use `render_with(&RenderOptions::plain(), output)` — no line buffer needed"
    )]
    pub fn render_scanline_with_buffer(&self, _line_buffer: &mut [char], output: &mut String) {
        let _ = self.render_with(&RenderOptions::plain(), output);
    }

    /// Plain-text render into a byte buffer. Returns the bytes written;
    /// output that does not fit is truncated (size the buffer with
    /// `estimate_render_output_size`).
    #[deprecated(
        since = "0.10.0",
        note = "use `render_to_bytes` with `estimate_render_arena_size`/`estimate_render_output_size`"
    )]
    pub fn render_scanline_to_bytes(&self, _line_buffer: &mut [char], output: &mut [u8]) -> usize {
        let mut sink = crate::render::engine::emit::ByteSink::new(output);
        let _ = crate::render::engine::render_into(self, &RenderOptions::plain(), &mut sink);
        sink.written()
    }

    /// ANSI-colored render (no legend).
    #[deprecated(
        since = "0.10.0",
        note = "use `render_string(&RenderOptions::colored(palette))` (set `legend = false` to match)"
    )]
    pub fn render_scanline_colored(&self, palette: Palette) -> String {
        self.render_string(&colored_no_legend(palette))
    }

    /// ANSI-colored render (no legend) into an existing `String`.
    #[deprecated(
        since = "0.10.0",
        note = "use `render_with(&RenderOptions::colored(palette), output)` (set `legend = false` to match)"
    )]
    pub fn render_scanline_colored_to(&self, output: &mut String, palette: Palette) {
        let _ = self.render_with(&colored_no_legend(palette), output);
    }

    /// ANSI-colored render with the skipped-label legend.
    #[deprecated(
        since = "0.10.0",
        note = "use `render_string(&RenderOptions::colored(palette))`"
    )]
    pub fn render_scanline_colored_with_legend(&self, palette: Palette) -> String {
        self.render_string(&RenderOptions::colored(palette))
    }
}
