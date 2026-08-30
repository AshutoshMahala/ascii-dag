//! The public cell vocabulary — decoded views over the composed
//! canvas.
//!
//! [`SceneComposer::visit_cells`](super::composer::SceneComposer::visit_cells)
//! yields one [`CellView`] per canvas cell: what the cell **means**
//! (per-arm stroke weights, marker kind and direction, a text char —
//! never a pre-decoded glyph or the packed internal representation),
//! its resolved color, and its hit/pick owner. Unicode and ASCII
//! terminal output are two projections of exactly this vocabulary; an
//! SVG or TUI consumer projects it however it likes.

use super::cell::{Cell, Dir, MarkerKind, Weight};
use super::color::CellColor;
use super::plan::HitResult;

/// One composed canvas cell: meaning, color, owner.
#[non_exhaustive]
#[derive(Debug, Clone, Copy)]
pub struct CellView<'s> {
    /// What the cell means.
    pub kind: CellKind,
    /// Resolved color (plain emitters ignore it; always present —
    /// the composer is color-complete regardless of emission mode).
    pub color: CellColor,
    /// The HIT/PICK owner — the element
    /// [`Scene::hit_test`](super::scene::Scene::hit_test) reports for
    /// this cell — NOT paint provenance. A blank cell inside a node's
    /// or cluster's region still has an owner; a cell painted by one
    /// edge but hit-owned by another (merge tie-breaks) reports the
    /// hit-test winner. Same vocabulary, same resolution, pinned by
    /// test.
    pub owner: HitResult,
    /// Reserved for future borrowed payloads (e.g. custom-node ink).
    pub(crate) _reserved: core::marker::PhantomData<&'s ()>,
}

/// What a composed cell means.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CellKind {
    /// Nothing painted here.
    Empty,
    /// One character of node/label/legend text (passes through
    /// untranslated — user content is never transliterated).
    Text {
        /// The character.
        ch: char,
    },
    /// A stroke junction: per-arm weights INSTEAD of a pre-decoded
    /// box-drawing character.
    Stroke {
        /// The four arm weights.
        arms: ArmWeights,
    },
    /// A marker cell — its own vocabulary, distinct from the per-edge
    /// [`MarkerShape`](super::style::MarkerShape) endpoint style.
    Marker {
        /// The marker.
        marker: CellMarker,
    },
}

/// Per-arm stroke weights of one junction cell. Merging is a per-arm
/// maximum, so any consumer can reconstruct the exact glyph the
/// terminal emitters would pick — or draw real vector strokes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArmWeights {
    /// Arm toward the row above.
    pub up: ArmWeight,
    /// Arm toward the row below.
    pub down: ArmWeight,
    /// Arm toward the previous column.
    pub left: ArmWeight,
    /// Arm toward the next column.
    pub right: ArmWeight,
}

/// One stroke arm's weight.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArmWeight {
    /// No arm in this direction.
    None,
    /// Dashed stroke (reversed back-edges).
    Dashed,
    /// Light stroke (regular edges).
    Light,
    /// Double stroke (cluster borders).
    Double,
    // Heavy joins here if the extended edge vocabulary ships.
}

/// A composed marker cell. Distinct from the per-edge
/// [`MarkerShape`](super::style::MarkerShape) endpoint declaration:
/// the cell grid also contains self-loop and dummy markers, which are
/// not endpoint shapes.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CellMarker {
    /// An arrowhead.
    Arrow {
        /// Which way it points.
        direction: MarkerDirection,
        /// Dashed variant (reversed back-edges).
        dashed: bool,
    },
    /// The `↺` self-loop indicator.
    SelfLoop,
    /// The `◍` dummy-node marker (painted only when the scene shows
    /// dummy nodes).
    Dummy,
}

/// A marker's direction (also the direction vocabulary for
/// [`NodeRegion::arrow`](super::region::NodeRegion::arrow)).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkerDirection {
    /// Pointing up.
    Up,
    /// Pointing down.
    Down,
    /// Pointing left.
    Left,
    /// Pointing right.
    Right,
}

impl CellKind {
    /// Decode the packed internal cell into the public vocabulary.
    pub(crate) fn from_cell(cell: Cell) -> CellKind {
        if cell.is_empty() {
            CellKind::Empty
        } else if cell.is_text() {
            CellKind::Text {
                ch: cell.text_char(),
            }
        } else if cell.is_marker() {
            CellKind::Marker {
                marker: match cell.marker_kind() {
                    MarkerKind::SelfLoop => CellMarker::SelfLoop,
                    MarkerKind::Dummy => CellMarker::Dummy,
                    MarkerKind::Arrow => CellMarker::Arrow {
                        direction: MarkerDirection::from_dir(cell.marker_dir()),
                        dashed: cell.marker_dashed(),
                    },
                },
            }
        } else {
            let (up, down, left, right) = cell.arms();
            CellKind::Stroke {
                arms: ArmWeights {
                    up: ArmWeight::from_weight(up),
                    down: ArmWeight::from_weight(down),
                    left: ArmWeight::from_weight(left),
                    right: ArmWeight::from_weight(right),
                },
            }
        }
    }
}

impl ArmWeight {
    pub(crate) fn from_weight(w: Weight) -> ArmWeight {
        match w {
            Weight::None => ArmWeight::None,
            Weight::Dashed => ArmWeight::Dashed,
            Weight::Light => ArmWeight::Light,
            Weight::Double => ArmWeight::Double,
        }
    }
}

impl MarkerDirection {
    pub(crate) fn from_dir(d: Dir) -> MarkerDirection {
        match d {
            Dir::Up => MarkerDirection::Up,
            Dir::Down => MarkerDirection::Down,
            Dir::Left => MarkerDirection::Left,
            Dir::Right => MarkerDirection::Right,
        }
    }

    /// The internal paint direction (painter primitives).
    pub(crate) fn to_dir(self) -> Dir {
        match self {
            MarkerDirection::Up => Dir::Up,
            MarkerDirection::Down => Dir::Down,
            MarkerDirection::Left => Dir::Left,
            MarkerDirection::Right => Dir::Right,
        }
    }
}
