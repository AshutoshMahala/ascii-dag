//! Semantic canvas cells (temp/06 §3, ruling Q3).
//!
//! A cell stores what it *means*, not which glyph shows it: strokes carry
//! per-arm weights, arrows are markers with a direction, text carries a
//! `char`. Glyphs are produced only at emission by a charset decode table
//! (`engine::charset`), so Unicode and ASCII are equal projections.
//!
//! Merging overlapping strokes is a per-arm maximum — one integer
//! operation, **commutative**, so paint-stage ordering can never change a
//! junction. Precedence between kinds lives in exactly one place (the
//! `painted_*` methods below) instead of scattered guards.
//!
//! # Packing (u32)
//!
//! ```text
//! bits 31..30: tag — 00 empty, 01 text, 10 stroke, 11 marker
//! text:   bits 20..0  = char
//! stroke: bits 11..0  = 4 arms × 3-bit Weight (up, down, left, right)
//! marker: bits  3..0  = kind, bits 5..4 = direction, bit 6 = dashed
//! ```
//!
//! `Cell::EMPTY` is the all-zero pattern, so clearing a canvas is a
//! plain fill with zero.

/// Stroke arm weight.
///
/// Merging picks the **stronger** arm (numeric max), so the ordering is
/// semantic: a dashed stroke loses to a solid one (`┊` crossed by `─`
/// renders `┼`, matching the legacy tables), and anything loses to a
/// double-line cluster border. `Heavy` is reserved for a future weight.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
#[repr(u32)]
pub(crate) enum Weight {
    /// No arm in this direction.
    #[default]
    None = 0,
    /// Dashed stroke (reversed back-edges).
    Dashed = 1,
    /// Light stroke (regular edges).
    Light = 2,
    /// Double stroke (cluster borders).
    Double = 3,
}

impl Weight {
    const fn from_bits(bits: u32) -> Weight {
        match bits & 0b111 {
            1 => Weight::Dashed,
            2 => Weight::Light,
            3 => Weight::Double,
            _ => Weight::None,
        }
    }

    /// The stronger of two arms (merge rule).
    pub(crate) fn max(self, other: Weight) -> Weight {
        if (self as u32) >= (other as u32) { self } else { other }
    }
}

/// Marker kind — glyphs that are endpoints, not path segments.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub(crate) enum MarkerKind {
    /// Arrowhead (direction = where it points).
    Arrow = 1,
    /// Self-loop indicator (`↺`).
    SelfLoop = 2,
    /// Dummy-node marker (`◍`, zigraph parity).
    Dummy = 3,
}

/// Direction, for arrow markers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub(crate) enum Dir {
    /// Pointing up.
    Up = 0,
    /// Pointing down.
    Down = 1,
    /// Pointing left.
    Left = 2,
    /// Pointing right.
    Right = 3,
}

const TAG_SHIFT: u32 = 30;
const TAG_TEXT: u32 = 1;
const TAG_STROKE: u32 = 2;
const TAG_MARKER: u32 = 3;

const ARM_UP_SHIFT: u32 = 0;
const ARM_DOWN_SHIFT: u32 = 3;
const ARM_LEFT_SHIFT: u32 = 6;
const ARM_RIGHT_SHIFT: u32 = 9;

const MARKER_KIND_MASK: u32 = 0b1111;
const MARKER_DIR_SHIFT: u32 = 4;
const MARKER_DASHED_BIT: u32 = 1 << 6;

/// One semantic canvas cell. See module docs for the packing.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct Cell(u32);

impl Cell {
    /// The empty cell — all-zero, so buffers clear with a zero fill.
    pub(crate) const EMPTY: Cell = Cell(0);

    /// A text cell (node labels, edge labels, box labels).
    pub(crate) const fn text(ch: char) -> Cell {
        Cell((TAG_TEXT << TAG_SHIFT) | ch as u32)
    }

    /// A stroke cell from per-arm weights.
    pub(crate) const fn stroke(up: Weight, down: Weight, left: Weight, right: Weight) -> Cell {
        Cell(
            (TAG_STROKE << TAG_SHIFT)
                | ((up as u32) << ARM_UP_SHIFT)
                | ((down as u32) << ARM_DOWN_SHIFT)
                | ((left as u32) << ARM_LEFT_SHIFT)
                | ((right as u32) << ARM_RIGHT_SHIFT),
        )
    }

    /// A marker cell.
    pub(crate) const fn marker(kind: MarkerKind, dir: Dir, dashed: bool) -> Cell {
        Cell(
            (TAG_MARKER << TAG_SHIFT)
                | (kind as u32)
                | ((dir as u32) << MARKER_DIR_SHIFT)
                | if dashed { MARKER_DASHED_BIT } else { 0 },
        )
    }

    // ── Accessors ────────────────────────────────────────────────────────

    #[inline]
    pub(crate) fn is_empty(self) -> bool {
        self.0 >> TAG_SHIFT == 0
    }

    #[inline]
    pub(crate) fn is_text(self) -> bool {
        self.0 >> TAG_SHIFT == TAG_TEXT
    }

    #[inline]
    pub(crate) fn is_stroke(self) -> bool {
        self.0 >> TAG_SHIFT == TAG_STROKE
    }

    #[inline]
    pub(crate) fn is_marker(self) -> bool {
        self.0 >> TAG_SHIFT == TAG_MARKER
    }

    /// The character of a text cell (undefined for other tags).
    #[inline]
    pub(crate) fn text_char(self) -> char {
        char::from_u32(self.0 & 0x001F_FFFF).unwrap_or(' ')
    }

    /// Per-arm weights of a stroke cell: (up, down, left, right).
    #[inline]
    pub(crate) fn arms(self) -> (Weight, Weight, Weight, Weight) {
        (
            Weight::from_bits(self.0 >> ARM_UP_SHIFT),
            Weight::from_bits(self.0 >> ARM_DOWN_SHIFT),
            Weight::from_bits(self.0 >> ARM_LEFT_SHIFT),
            Weight::from_bits(self.0 >> ARM_RIGHT_SHIFT),
        )
    }

    /// Marker kind of a marker cell (undefined for other tags).
    #[inline]
    pub(crate) fn marker_kind(self) -> MarkerKind {
        match self.0 & MARKER_KIND_MASK {
            2 => MarkerKind::SelfLoop,
            3 => MarkerKind::Dummy,
            _ => MarkerKind::Arrow,
        }
    }

    /// Marker direction of a marker cell.
    #[inline]
    pub(crate) fn marker_dir(self) -> Dir {
        match (self.0 >> MARKER_DIR_SHIFT) & 0b11 {
            0 => Dir::Up,
            1 => Dir::Down,
            2 => Dir::Left,
            _ => Dir::Right,
        }
    }

    /// Whether a marker cell is dashed.
    #[inline]
    pub(crate) fn marker_dashed(self) -> bool {
        self.0 & MARKER_DASHED_BIT != 0
    }

    /// A stroke with vertical arms only (edge labels may replace these).
    /// Test-only since RW6: the planner replicates this rule
    /// geometrically (plan.rs `span_blocked`); the tests below keep the
    /// cell-level statement of R1.8 pinned.
    #[cfg(test)]
    pub(crate) fn is_pure_vertical(self) -> bool {
        if !self.is_stroke() {
            return false;
        }
        let (up, down, left, right) = self.arms();
        left == Weight::None
            && right == Weight::None
            && (up != Weight::None || down != Weight::None)
    }

    /// Whether an edge label may occupy this cell — empty, or the edge's
    /// own vertical line (the legacy `can_place_label` rule, R1.8).
    #[cfg(test)]
    pub(crate) fn accepts_label(self) -> bool {
        self.is_empty() || self.is_pure_vertical()
    }

    // ── Paint operations (ALL precedence rules live here) ────────────────

    /// Paint a stroke onto this cell.
    ///
    /// - onto empty → the stroke
    /// - onto stroke → per-arm max (commutative)
    /// - onto text → text wins for light/dashed strokes (labels stay
    ///   readable); a border stroke (any `Double` arm) replaces text
    ///   (legacy border tables' fallback behavior)
    /// - onto marker → marker wins for light/dashed strokes (the legacy
    ///   `is_arrow` guards); a border stroke converts an arrow into a
    ///   light arm in its pointing direction and merges (legacy `↓`→`╤`,
    ///   `⇡`→`╧`, `→`→`╞` …); non-arrow markers are replaced
    pub(crate) fn painted_stroke(
        self,
        up: Weight,
        down: Weight,
        left: Weight,
        right: Weight,
    ) -> Cell {
        let new = Cell::stroke(up, down, left, right);
        let has_double = up == Weight::Double
            || down == Weight::Double
            || left == Weight::Double
            || right == Weight::Double;

        match self.0 >> TAG_SHIFT {
            TAG_STROKE => {
                let (eu, ed, el, er) = self.arms();
                Cell::stroke(eu.max(up), ed.max(down), el.max(left), er.max(right))
            }
            TAG_TEXT => {
                if has_double {
                    new
                } else {
                    self
                }
            }
            TAG_MARKER => {
                if !has_double {
                    return self;
                }
                if self.marker_kind() == MarkerKind::Arrow {
                    // Arrow becomes a light stem in its pointing direction.
                    let (mut au, mut ad, mut al, mut ar) =
                        (Weight::None, Weight::None, Weight::None, Weight::None);
                    match self.marker_dir() {
                        Dir::Up => au = Weight::Light,
                        Dir::Down => ad = Weight::Light,
                        Dir::Left => al = Weight::Light,
                        Dir::Right => ar = Weight::Light,
                    }
                    Cell::stroke(au.max(up), ad.max(down), al.max(left), ar.max(right))
                } else {
                    new
                }
            }
            _ => new,
        }
    }

    /// Paint a marker onto this cell.
    ///
    /// Markers overwrite empty cells, strokes, and other markers
    /// (last-wins; the legacy plain path behaves the same). Text wins
    /// over markers — labels and node text are painted by later z-order
    /// stages and must stay readable.
    pub(crate) fn painted_marker(self, kind: MarkerKind, dir: Dir, dashed: bool) -> Cell {
        if self.is_text() {
            self
        } else {
            Cell::marker(kind, dir, dashed)
        }
    }

    /// Paint text onto this cell — unconditional (z-order stages decide
    /// who paints last; text is always the later stage today).
    pub(crate) fn painted_text(self, ch: char) -> Cell {
        let _ = self;
        Cell::text(ch)
    }
}

impl core::fmt::Debug for Cell {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        if self.is_empty() {
            write!(f, "Cell::EMPTY")
        } else if self.is_text() {
            write!(f, "Cell::text({:?})", self.text_char())
        } else if self.is_stroke() {
            let (u, d, l, r) = self.arms();
            write!(f, "Cell::stroke({u:?}, {d:?}, {l:?}, {r:?})")
        } else {
            write!(
                f,
                "Cell::marker({:?}, {:?}, dashed={})",
                self.marker_kind(),
                self.marker_dir(),
                self.marker_dashed()
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_is_all_zero() {
        assert_eq!(Cell::EMPTY.0, 0);
        assert!(Cell::EMPTY.is_empty());
        assert!(Cell::EMPTY.accepts_label());
    }

    #[test]
    fn packing_roundtrips() {
        let t = Cell::text('Ω');
        assert!(t.is_text());
        assert_eq!(t.text_char(), 'Ω');

        let s = Cell::stroke(Weight::Light, Weight::Double, Weight::None, Weight::Dashed);
        assert!(s.is_stroke());
        assert_eq!(
            s.arms(),
            (Weight::Light, Weight::Double, Weight::None, Weight::Dashed)
        );

        let m = Cell::marker(MarkerKind::Arrow, Dir::Left, true);
        assert!(m.is_marker());
        assert_eq!(m.marker_kind(), MarkerKind::Arrow);
        assert_eq!(m.marker_dir(), Dir::Left);
        assert!(m.marker_dashed());

        let d = Cell::marker(MarkerKind::Dummy, Dir::Up, false);
        assert_eq!(d.marker_kind(), MarkerKind::Dummy);
        assert!(!d.marker_dashed());
    }

    #[test]
    fn stroke_merge_is_commutative_and_max() {
        let a = (Weight::Light, Weight::None, Weight::Dashed, Weight::None);
        let b = (Weight::Dashed, Weight::Double, Weight::None, Weight::Light);
        let ab = Cell::EMPTY
            .painted_stroke(a.0, a.1, a.2, a.3)
            .painted_stroke(b.0, b.1, b.2, b.3);
        let ba = Cell::EMPTY
            .painted_stroke(b.0, b.1, b.2, b.3)
            .painted_stroke(a.0, a.1, a.2, a.3);
        assert_eq!(ab, ba);
        assert_eq!(
            ab.arms(),
            (Weight::Light, Weight::Double, Weight::Dashed, Weight::Light)
        );
    }

    #[test]
    fn light_stroke_never_beats_text_or_marker() {
        let text = Cell::text('x');
        assert_eq!(
            text.painted_stroke(Weight::Light, Weight::Light, Weight::None, Weight::None),
            text,
        );
        let arrow = Cell::marker(MarkerKind::Arrow, Dir::Down, false);
        assert_eq!(
            arrow.painted_stroke(Weight::None, Weight::None, Weight::Light, Weight::Light),
            arrow,
        );
    }

    #[test]
    fn border_converts_arrow_to_stem() {
        // Legacy `merge_h_border`: '↓' → '╤' (down stem + double horizontal).
        let arrow = Cell::marker(MarkerKind::Arrow, Dir::Down, false);
        let merged =
            arrow.painted_stroke(Weight::None, Weight::None, Weight::Double, Weight::Double);
        assert_eq!(
            merged.arms(),
            (Weight::None, Weight::Light, Weight::Double, Weight::Double)
        );
        // Legacy: '↺' merged with a border is replaced by the border.
        let loops = Cell::marker(MarkerKind::SelfLoop, Dir::Up, false);
        let replaced =
            loops.painted_stroke(Weight::None, Weight::None, Weight::Double, Weight::Double);
        assert_eq!(
            replaced.arms(),
            (Weight::None, Weight::None, Weight::Double, Weight::Double)
        );
    }

    #[test]
    fn marker_last_wins_except_text() {
        let stroke = Cell::stroke(Weight::Light, Weight::Light, Weight::None, Weight::None);
        let m = stroke.painted_marker(MarkerKind::Arrow, Dir::Down, false);
        assert!(m.is_marker());
        assert!(
            Cell::text('a')
                .painted_marker(MarkerKind::Arrow, Dir::Down, false)
                .is_text()
        );
    }

    #[test]
    fn label_placement_rule_matches_legacy() {
        // Legacy can_place_label: space or the edge's own '│' only.
        assert!(Cell::EMPTY.accepts_label());
        assert!(
            Cell::stroke(Weight::Light, Weight::Light, Weight::None, Weight::None)
                .accepts_label()
        );
        assert!(
            Cell::stroke(Weight::Dashed, Weight::Dashed, Weight::None, Weight::None)
                .accepts_label()
        );
        assert!(
            !Cell::stroke(Weight::Light, Weight::Light, Weight::Light, Weight::None)
                .accepts_label()
        );
        assert!(!Cell::text('q').accepts_label());
        assert!(
            !Cell::marker(MarkerKind::Arrow, Dir::Down, false).accepts_label()
        );
    }
}
