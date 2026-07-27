//! Unicode decode table — semantic cells → box-drawing glyphs.
//!
//! Light arm patterns delegate to [`crate::render::chars::mask_to_char`]
//! so the engine and the legacy renderers share one source of truth
//! during the migration. Mixed light/double patterns reproduce the
//! legacy border-merge glyphs **byte-for-byte** (see the pinning notes
//! below) — where the legacy tables were approximate (`┤` + a vertical
//! border shows `╡`, not the typographically exact `╣`), we match the
//! legacy choice, because R2.1 demands byte-identical output.

use super::super::cell::{Cell, Dir, MarkerKind, Weight};
use crate::render::chars::{DIR_DOWN, DIR_LEFT, DIR_RIGHT, DIR_UP, mask_to_char};

/// Decode one cell to its Unicode glyph.
pub(crate) fn decode(cell: Cell) -> char {
    if cell.is_empty() {
        return ' ';
    }
    if cell.is_text() {
        return cell.text_char();
    }
    if cell.is_marker() {
        return decode_marker(cell);
    }
    let (up, down, left, right) = cell.arms();
    decode_stroke(up, down, left, right)
}

fn decode_marker(cell: Cell) -> char {
    match cell.marker_kind() {
        MarkerKind::SelfLoop => '↺',
        MarkerKind::Dummy => '◍', // zigraph parity
        MarkerKind::Arrow => match (cell.marker_dir(), cell.marker_dashed()) {
            (Dir::Down, false) => '↓',
            (Dir::Down, true) => '⇣',
            (Dir::Up, false) => '↑',
            (Dir::Up, true) => '⇡',
            (Dir::Right, false) => '→',
            (Dir::Right, true) => '⇢',
            (Dir::Left, false) => '←',
            (Dir::Left, true) => '⇠',
        },
    }
}

fn decode_stroke(up: Weight, down: Weight, left: Weight, right: Weight) -> char {
    use Weight as W;

    let vertical_only = left == W::None && right == W::None;
    let horizontal_only = up == W::None && down == W::None;

    // Pure dashed lines keep their dashed glyphs; any other pattern
    // folds dashed arms to light (legacy: corners and junctions have no
    // dashed variants — `to_dashed` keeps them solid, and a dashed
    // stroke crossed by anything renders as a solid junction).
    let any_dashed = up == W::Dashed || down == W::Dashed || left == W::Dashed || right == W::Dashed;
    if any_dashed {
        let pure_dashed_v = vertical_only && up != W::Light && down != W::Light;
        if pure_dashed_v && up != W::Double && down != W::Double {
            return '┊';
        }
        let pure_dashed_h = horizontal_only && left != W::Light && right != W::Light;
        if pure_dashed_h && left != W::Double && right != W::Double {
            return '┈';
        }
        let fold = |w: W| if w == W::Dashed { W::Light } else { w };
        return decode_stroke(fold(up), fold(down), fold(left), fold(right));
    }

    let any_double =
        up == W::Double || down == W::Double || left == W::Double || right == W::Double;
    if !any_double {
        // All-light pattern: exact legacy table via the shared bitmask.
        let mut mask = 0u8;
        if up != W::None {
            mask |= DIR_UP;
        }
        if down != W::None {
            mask |= DIR_DOWN;
        }
        if left != W::None {
            mask |= DIR_LEFT;
        }
        if right != W::None {
            mask |= DIR_RIGHT;
        }
        return mask_to_char(mask);
    }

    // Double-bearing patterns. `L` = light arm, `D` = double arm,
    // `N` = none. Ordered: pure doubles, then the mixed junctions the
    // legacy border tables produce (pinned to their glyph choices).
    let key = |w: W| -> u8 {
        match w {
            W::None => 0,
            W::Double => 2,
            _ => 1,
        }
    };
    match (key(up), key(down), key(left), key(right)) {
        // Pure double lines / corners / junctions.
        (2, 2, 0, 0) | (2, 0, 0, 0) | (0, 2, 0, 0) => '║',
        (0, 0, 2, 2) | (0, 0, 2, 0) | (0, 0, 0, 2) => '═',
        (0, 2, 0, 2) => '╔',
        (0, 2, 2, 0) => '╗',
        (2, 0, 0, 2) => '╚',
        (2, 0, 2, 0) => '╝',
        (2, 2, 0, 2) => '╠',
        (2, 2, 2, 0) => '╣',
        (0, 2, 2, 2) => '╦',
        (2, 0, 2, 2) => '╩',
        (2, 2, 2, 2) => '╬',
        // Light vertical × double horizontal (legacy `merge_h_border`).
        (1, 1, 2, 2) | (1, 1, 2, 0) | (1, 1, 0, 2) => '╪',
        (0, 1, 2, 2) | (0, 1, 2, 0) | (0, 1, 0, 2) => '╤',
        (1, 0, 2, 2) | (1, 0, 2, 0) | (1, 0, 0, 2) => '╧',
        // Double vertical × light horizontal (legacy `merge_v_border`;
        // pinned to the legacy glyph choices, see module docs).
        (2, 2, 1, 1) | (2, 0, 1, 1) | (0, 2, 1, 1) => '╫',
        (2, 2, 0, 1) | (2, 0, 0, 1) | (0, 2, 0, 1) => '╞',
        (2, 2, 1, 0) | (2, 0, 1, 0) | (0, 2, 1, 0) => '╡',
        // Double + light on the same axis after a partial merge — treat
        // the light arm as absorbed (closest pure-double glyph).
        (a, b, c, d) => {
            // No exact glyph exists for this mix (e.g. a box corner with
            // a light stroke passing through). Decode the double-arm
            // subset only — borders visually dominate, and the stroke
            // resumes on the neighboring cells (the legacy corner look).
            let fold = |k: u8| -> Weight {
                match k {
                    2 => W::Double,
                    _ => W::None,
                }
            };
            decode_double_fallback(fold(a), fold(b), fold(c), fold(d))
        }
    }
}

fn decode_double_fallback(up: Weight, down: Weight, left: Weight, right: Weight) -> char {
    use Weight as W;
    // No exact glyph exists for this mix. Decode the double-arm subset
    // only — borders visually dominate a light stroke passing through
    // (e.g. a box corner crossed by an edge decodes as the corner, the
    // stroke resuming on the next row — the legacy renderers' look).
    let keep = |w: W| if w == W::Double { W::Double } else { W::None };
    let (u, d2, l, r) = (keep(up), keep(down), keep(left), keep(right));
    match (u, d2, l, r) {
        (W::Double, W::None, W::None, W::None) | (W::None, W::Double, W::None, W::None) => '║',
        (W::None, W::None, W::Double, W::None) | (W::None, W::None, W::None, W::Double) => '═',
        (W::Double, W::Double, W::None, W::None) => '║',
        (W::None, W::None, W::Double, W::Double) => '═',
        (W::None, W::Double, W::None, W::Double) => '╔',
        (W::None, W::Double, W::Double, W::None) => '╗',
        (W::Double, W::None, W::None, W::Double) => '╚',
        (W::Double, W::None, W::Double, W::None) => '╝',
        (W::None, W::Double, W::Double, W::Double) => '╦',
        (W::Double, W::None, W::Double, W::Double) => '╩',
        (W::Double, W::Double, W::None, W::Double) => '╠',
        (W::Double, W::Double, W::Double, W::None) => '╣',
        (W::Double, W::Double, W::Double, W::Double) => '╬',
        _ => ' ',
    }
}

#[cfg(test)]
mod tests {
    use super::super::super::cell::{Cell, Dir, MarkerKind, Weight};
    use super::decode;
    use crate::render::chars::{
        DIR_DOWN, DIR_LEFT, DIR_RIGHT, DIR_UP, char_direction_mask, mask_to_char, merge_chars,
    };

    /// Encode a legacy glyph as a semantic cell (test-only bridge).
    fn encode_legacy(c: char) -> Cell {
        use Weight as W;
        match c {
            ' ' => Cell::EMPTY,
            '↓' => Cell::marker(MarkerKind::Arrow, Dir::Down, false),
            '⇣' => Cell::marker(MarkerKind::Arrow, Dir::Down, true),
            '↑' => Cell::marker(MarkerKind::Arrow, Dir::Up, false),
            '⇡' => Cell::marker(MarkerKind::Arrow, Dir::Up, true),
            '→' => Cell::marker(MarkerKind::Arrow, Dir::Right, false),
            '←' => Cell::marker(MarkerKind::Arrow, Dir::Left, false),
            '↺' => Cell::marker(MarkerKind::SelfLoop, Dir::Up, false),
            '┊' => Cell::stroke(W::Dashed, W::Dashed, W::None, W::None),
            '┈' => Cell::stroke(W::None, W::None, W::Dashed, W::Dashed),
            _ => {
                let mask = char_direction_mask(c);
                if mask == 0 {
                    return Cell::text(c);
                }
                let arm = |bit: u8| if mask & bit != 0 { W::Light } else { W::None };
                Cell::stroke(arm(DIR_UP), arm(DIR_DOWN), arm(DIR_LEFT), arm(DIR_RIGHT))
            }
        }
    }

    /// All 15 non-empty light arm combinations decode exactly like the
    /// legacy shared bitmask table.
    #[test]
    fn light_patterns_match_mask_to_char() {
        for mask in 1u8..16 {
            let w = |bit: u8| {
                if mask & bit != 0 {
                    Weight::Light
                } else {
                    Weight::None
                }
            };
            let cell = Cell::stroke(w(DIR_UP), w(DIR_DOWN), w(DIR_LEFT), w(DIR_RIGHT));
            assert_eq!(
                decode(cell),
                mask_to_char(mask),
                "light pattern mask {mask:#06b}"
            );
        }
    }

    /// Encode → merge → decode reproduces `merge_chars` for every pair
    /// of stroke glyphs (arrows are covered separately — legacy paths
    /// disagree with each other on arrow-vs-arrow ordering).
    #[test]
    fn stroke_merges_match_merge_chars() {
        let strokes = [
            '│', '─', '└', '┘', '┌', '┐', '┬', '┴', '├', '┤', '┼', '┊', '┈',
        ];
        for &a in &strokes {
            for &b in &strokes {
                let cell_a = encode_legacy(a);
                let (u, d, l, r) = encode_legacy(b).arms();
                let merged = cell_a.painted_stroke(u, d, l, r);
                assert_eq!(
                    decode(merged),
                    merge_chars(a, b),
                    "merging {a:?} + {b:?}"
                );
            }
        }
    }

    /// Strokes never overwrite arrows (the legacy `is_arrow` guards).
    #[test]
    fn stroke_onto_arrow_keeps_arrow() {
        for arrow in ['↓', '⇣', '⇡'] {
            let cell = encode_legacy(arrow);
            let (u, d, l, r) = encode_legacy('─').arms();
            assert_eq!(decode(cell.painted_stroke(u, d, l, r)), arrow);
            let (u, d, l, r) = encode_legacy('│').arms();
            assert_eq!(decode(cell.painted_stroke(u, d, l, r)), arrow);
        }
    }

    /// Every entry of the legacy `merge_h_border` table (scanline.rs):
    /// horizontal double border painted onto an existing cell.
    #[test]
    fn h_border_merges_match_legacy_table() {
        let expected = [
            ('│', '╪'),
            ('┊', '╪'),
            ('┼', '╪'),
            ('├', '╪'),
            ('┤', '╪'),
            ('↓', '╤'),
            ('⇣', '╤'),
            ('┌', '╤'),
            ('┐', '╤'),
            ('┬', '╤'),
            ('↑', '╧'),
            ('⇡', '╧'),
            ('└', '╧'),
            ('┘', '╧'),
            ('┴', '╧'),
            (' ', '═'),
            ('A', '═'), // legacy fallback arm: anything else → '═'
        ];
        for (existing, want) in expected {
            let merged = encode_legacy(existing).painted_stroke(
                Weight::None,
                Weight::None,
                Weight::Double,
                Weight::Double,
            );
            assert_eq!(decode(merged), want, "h-border onto {existing:?}");
        }
    }

    /// Every entry of the legacy `merge_v_border` table (scanline.rs).
    #[test]
    fn v_border_merges_match_legacy_table() {
        let expected = [
            ('─', '╫'),
            ('┈', '╫'),
            ('┼', '╫'),
            ('┬', '╫'),
            ('┴', '╫'),
            ('→', '╞'),
            ('┌', '╞'),
            ('└', '╞'),
            ('├', '╞'),
            ('←', '╡'),
            ('┐', '╡'),
            ('┘', '╡'),
            ('┤', '╡'),
            (' ', '║'),
            ('A', '║'),
        ];
        for (existing, want) in expected {
            let merged = encode_legacy(existing).painted_stroke(
                Weight::Double,
                Weight::Double,
                Weight::None,
                Weight::None,
            );
            assert_eq!(decode(merged), want, "v-border onto {existing:?}");
        }
    }

    /// Box borders themselves decode to the double set.
    #[test]
    fn double_patterns_decode_to_box_glyphs() {
        use Weight::{Double as D, None as N};
        let cases = [
            ((D, D, N, N), '║'),
            ((N, N, D, D), '═'),
            ((N, D, N, D), '╔'),
            ((N, D, D, N), '╗'),
            ((D, N, N, D), '╚'),
            ((D, N, D, N), '╝'),
        ];
        for ((u, d, l, r), want) in cases {
            assert_eq!(decode(Cell::stroke(u, d, l, r)), want);
        }
    }

    #[test]
    fn markers_and_dashed_lines_decode() {
        assert_eq!(decode(encode_legacy('↓')), '↓');
        assert_eq!(decode(encode_legacy('⇡')), '⇡');
        assert_eq!(decode(encode_legacy('↺')), '↺');
        assert_eq!(
            decode(Cell::marker(MarkerKind::Dummy, Dir::Up, false)),
            '◍'
        );
        assert_eq!(decode(encode_legacy('┊')), '┊');
        assert_eq!(decode(encode_legacy('┈')), '┈');
        assert_eq!(decode(Cell::text('Æ')), 'Æ');
        assert_eq!(decode(Cell::EMPTY), ' ');
    }

    /// Dashed + solid on one axis renders solid (legacy: dashed loses).
    #[test]
    fn dashed_loses_to_light() {
        let dashed_v = encode_legacy('┊');
        let (u, d, l, r) = encode_legacy('│').arms();
        assert_eq!(decode(dashed_v.painted_stroke(u, d, l, r)), '│');
        // Dashed cross renders as the solid cross, like merge_chars.
        let (u, d, l, r) = encode_legacy('┈').arms();
        assert_eq!(decode(dashed_v.painted_stroke(u, d, l, r)), '┼');
    }
}
