//! ASCII decode table — semantic cells → plain-ASCII glyphs.
//!
//! An equal projection of the composed canvas (the charset ruling):
//! ASCII output never passes through Unicode glyphs first. Semantics
//! match zigraph's `toAscii` so both libraries produce identical ASCII:
//!
//! - arrows → `v ^ > <`, self-loop → `@`, dummy marker → `o`
//! - vertical strokes (any weight) → `|`
//! - horizontal strokes → `-`, or `=` when double-weight
//! - anything with both a vertical and a horizontal arm → `+`
//! - text passes through untouched (user content is not transliterated)

use super::super::cell::{Cell, Dir, MarkerKind, Weight};

/// Decode one cell to its ASCII glyph.
pub(crate) fn decode(cell: Cell) -> char {
    if cell.is_empty() {
        return ' ';
    }
    if cell.is_text() {
        return cell.text_char();
    }
    if cell.is_marker() {
        return match cell.marker_kind() {
            MarkerKind::SelfLoop => '@',
            MarkerKind::Dummy => 'o',
            MarkerKind::Arrow => match cell.marker_dir() {
                Dir::Down => 'v',
                Dir::Up => '^',
                Dir::Right => '>',
                Dir::Left => '<',
            },
        };
    }

    let (up, down, left, right) = cell.arms();
    let vertical = up != Weight::None || down != Weight::None;
    let horizontal = left != Weight::None || right != Weight::None;
    match (vertical, horizontal) {
        (true, true) => '+',
        (true, false) => '|',
        (false, true) => {
            if left == Weight::Double || right == Weight::Double {
                '='
            } else {
                '-'
            }
        }
        (false, false) => ' ',
    }
}

#[cfg(test)]
mod tests {
    use super::super::super::cell::{Cell, Dir, MarkerKind, Weight};
    use super::decode;

    #[test]
    fn matches_zigraph_to_ascii_semantics() {
        use Weight::{Dashed, Double, Light, None as N};
        // Arrows and markers.
        assert_eq!(
            decode(Cell::marker(MarkerKind::Arrow, Dir::Down, false)),
            'v'
        );
        assert_eq!(decode(Cell::marker(MarkerKind::Arrow, Dir::Up, true)), '^');
        assert_eq!(
            decode(Cell::marker(MarkerKind::Arrow, Dir::Right, false)),
            '>'
        );
        assert_eq!(
            decode(Cell::marker(MarkerKind::Arrow, Dir::Left, false)),
            '<'
        );
        assert_eq!(
            decode(Cell::marker(MarkerKind::SelfLoop, Dir::Up, false)),
            '@'
        );
        assert_eq!(decode(Cell::marker(MarkerKind::Dummy, Dir::Up, false)), 'o');
        // Strokes.
        assert_eq!(decode(Cell::stroke(Light, Light, N, N)), '|');
        assert_eq!(decode(Cell::stroke(Double, Double, N, N)), '|');
        assert_eq!(decode(Cell::stroke(Dashed, Dashed, N, N)), '|');
        assert_eq!(decode(Cell::stroke(N, N, Light, Light)), '-');
        assert_eq!(decode(Cell::stroke(N, N, Dashed, Dashed)), '-');
        assert_eq!(decode(Cell::stroke(N, N, Double, Double)), '=');
        assert_eq!(decode(Cell::stroke(Light, Light, Light, Light)), '+');
        assert_eq!(decode(Cell::stroke(Light, Light, Double, Double)), '+');
        assert_eq!(decode(Cell::stroke(N, Light, N, Light)), '+');
        // Text passthrough, including non-ASCII user content.
        assert_eq!(decode(Cell::text('A')), 'A');
        assert_eq!(decode(Cell::text('Ω')), 'Ω');
        assert_eq!(decode(Cell::EMPTY), ' ');
    }
}
