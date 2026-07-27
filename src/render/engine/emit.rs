//! Emission — decode cells through the active charset and write rows
//! (temp/06 §7, R4/R1.13).
//!
//! Plain (uncolored) emission for RW3: per row, decode, right-trim, and
//! stream to any `core::fmt::Write`. Color emission (escape runs, modes)
//! arrives in RW4.

use super::charset::Charset;
use super::compose::BandCanvas;

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
