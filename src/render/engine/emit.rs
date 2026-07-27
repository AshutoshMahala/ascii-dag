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
