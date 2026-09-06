//! Edge attachment resolution — where an edge meets a node face.
//!
//! Sides are declared in one of two frames — PHYSICAL compass points
//! (`North` is visually top in every direction — the Graphviz port
//! vocabulary) or FLOW-RELATIVE (`Upstream` is
//! the face edges arrive on, whatever the direction) — and bind to
//! the edge's DECLARED endpoints, never the drawn arrow, so a cycle
//! reversal cannot move a port. The resolver itself works in ROLE
//! space (level axis / cross axis): relative names ARE role faces,
//! physical names reach them through the direction flip, folded in
//! exactly once, here. Lateral flow-relative sides are ROTATIONS of
//! the flow vector (`Clockwise` / `Counterclockwise`, viewer's frame)
//! — equivalently the traveler's right / left when facing downstream,
//! the river-bank convention — so they survive a direction change,
//! which physical compass points cannot.
//!
//! `Auto` is the byte-frozen 0.10 attachment: a source leaves the
//! trailing level face and a target arrives on the leading one, both
//! on the node's cross-axis center line.

use super::geometry::Axis;
use crate::graph::Direction;
use crate::ir::FlowAxis;

/// A declared side of a node for an edge end to attach to — the
/// DECLARATION vocabulary (it has `Auto` and flow-relative names; a
/// resolved side is always physical). One byte by guarantee.
#[non_exhaustive]
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PortSide {
    /// The layout picks: the role's default level face, center line.
    #[default]
    Auto,
    /// The visually top face, whatever the direction (Graphviz `n`).
    North,
    /// The visually right face (`e`).
    East,
    /// The visually bottom face (`s`).
    South,
    /// The visually left face (`w`).
    West,
    /// The face pointing AGAINST the flow — back toward where the
    /// flow comes from; the face edges normally ARRIVE on (an `Auto`
    /// target receives here). Physically North in TopDown, South in
    /// BottomUp, West in LeftRight, East in RightLeft.
    Upstream,
    /// The face pointing WITH the flow — toward where the flow goes;
    /// the face edges normally LEAVE from (an `Auto` source exits
    /// here). The opposite face of `Upstream` in every direction.
    Downstream,
    /// The lateral face reached by turning the flow vector a quarter
    /// turn clockwise (as the viewer sees it) — the traveler's RIGHT
    /// when facing downstream: physically `West` in TopDown, `East`
    /// in BottomUp, `South` in LeftRight, `North` in RightLeft.
    Clockwise,
    /// The lateral face a quarter turn counterclockwise
    /// (anticlockwise) from the flow — the traveler's LEFT facing
    /// downstream: physically `East` in TopDown, `West` in BottomUp,
    /// `North` in LeftRight, `South` in RightLeft.
    Counterclockwise,
}

impl PortSide {
    /// The explicit one-byte encoding the CSR and IR tables store —
    /// a conversion, deliberately not the enum's own representation,
    /// so the public `repr` is never load-bearing for stored data.
    /// `0` is `Auto`, which is why zeroed arena memory reads as "no
    /// declaration".
    #[cfg(feature = "ports")]
    pub(crate) const fn to_u8(self) -> u8 {
        match self {
            PortSide::Auto => 0,
            PortSide::North => 1,
            PortSide::East => 2,
            PortSide::South => 3,
            PortSide::West => 4,
            PortSide::Upstream => 5,
            PortSide::Downstream => 6,
            PortSide::Clockwise => 7,
            PortSide::Counterclockwise => 8,
        }
    }

    /// Inverse of [`to_u8`](Self::to_u8); unknown bytes read as `Auto`.
    pub(crate) const fn from_u8(byte: u8) -> PortSide {
        match byte {
            1 => PortSide::North,
            2 => PortSide::East,
            3 => PortSide::South,
            4 => PortSide::West,
            5 => PortSide::Upstream,
            6 => PortSide::Downstream,
            7 => PortSide::Clockwise,
            8 => PortSide::Counterclockwise,
            _ => PortSide::Auto,
        }
    }
}

/// What a port setter accepts: a side today, a side plus a position
/// later — `#[non_exhaustive]` with private fields so growing it
/// never changes a signature. `PortSide` converts into it, which is
/// how `from_port(PortSide::North)` reads unchanged forever.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Port {
    side: PortSide,
}

#[allow(dead_code)] // consumed by the heap handle today; the CSR handle joins with its port table
impl Port {
    /// A port on `side`, positioned by the router. `const`, so a
    /// declaration can live in a constant.
    pub const fn of(side: PortSide) -> Port {
        Port { side }
    }

    /// The declared side.
    pub const fn side(self) -> PortSide {
        self.side
    }
}

impl From<PortSide> for Port {
    #[inline]
    fn from(side: PortSide) -> Port {
        Port::of(side)
    }
}

// A declaration is compile-time data.
const _: Port = Port::of(PortSide::Upstream);

/// A node face in role space. `Leading` is the smaller coordinate on
/// that axis — for the level axis, the face edges ARRIVE on in flow
/// order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Face {
    LevelLeading,
    LevelTrailing,
    CrossLeading,
    CrossTrailing,
}

impl Face {
    /// A face on the level axis — the two the trunk can leave or
    /// enter head-on (the role's own `Auto` face and its opposite).
    #[cfg_attr(not(feature = "ports"), allow(dead_code))]
    pub(crate) const fn is_level(self) -> bool {
        matches!(self, Face::LevelLeading | Face::LevelTrailing)
    }
}

/// An edge end's role in LAYOUT order: the trunk leaves `Source` and
/// arrives at `Target`. Under a cycle reversal the layout roles swap
/// relative to the declared endpoints — `Auto` binds to the layout
/// role (which is exactly what keeps 0.10's reversed-edge attachment
/// byte-frozen), while an explicit side comes from the DECLARED
/// endpoint, so the caller swaps sides, not roles, when reversing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EndRole {
    Source,
    Target,
}

impl EndRole {
    /// The `Auto` face: sources leave trailing, targets arrive leading.
    pub(crate) const fn auto_face(self) -> Face {
        match self {
            EndRole::Source => Face::LevelTrailing,
            EndRole::Target => Face::LevelLeading,
        }
    }
}

/// A physical side of a node — where an edge end actually attached.
/// The RESOLUTION vocabulary: compass names only (a declaration is a
/// [`PortSide`], which also has `Auto` and the flow-relative names).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PhysicalSide {
    /// The top face.
    North,
    /// The right face.
    East,
    /// The bottom face.
    South,
    /// The left face.
    West,
}

impl PhysicalSide {
    /// The lowercase name (`"north"`, `"east"`, `"south"`, `"west"`) —
    /// the JSON spelling.
    pub const fn name(self) -> &'static str {
        match self {
            PhysicalSide::North => "north",
            PhysicalSide::East => "east",
            PhysicalSide::South => "south",
            PhysicalSide::West => "west",
        }
    }
}

impl PortSide {
    /// The lowercase name — the JSON spelling of a declaration
    /// (`"auto"`, `"north"`, … `"upstream"`, `"downstream"`,
    /// `"clockwise"`, `"counterclockwise"`).
    pub const fn name(self) -> &'static str {
        match self {
            PortSide::Auto => "auto",
            PortSide::North => "north",
            PortSide::East => "east",
            PortSide::South => "south",
            PortSide::West => "west",
            PortSide::Upstream => "upstream",
            PortSide::Downstream => "downstream",
            PortSide::Clockwise => "clockwise",
            PortSide::Counterclockwise => "counterclockwise",
        }
    }
}

/// Which declared end of an edge a condition is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EdgeEnd {
    /// The end at the edge's declared source.
    Source,
    /// The end at the edge's declared target.
    Target,
}

/// How one end of a routed edge is attached: what was declared and
/// the physical side it landed on. `requested` is `Auto` for an
/// undeclared end; `side` is always the truth of the drawing — an end
/// that could not route off its role's face reports that face here,
/// and the run's diagnostics say why.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PortAttachment {
    /// The declared side.
    pub requested: PortSide,
    /// The side the end attached on.
    pub side: PhysicalSide,
}

impl PortAttachment {
    /// An undeclared end that attached on `side` — what every edge of a
    /// port-free layout reports.
    pub const fn auto(side: PhysicalSide) -> Self {
        PortAttachment {
            requested: PortSide::Auto,
            side,
        }
    }
}

/// How a node places the ends declared on each of its faces. A
/// graph-wide default with a per-node override; `Single` unless set.
/// A face with one cell holds one port whatever the policy. Arrival
/// and departure are the DECLARED ends (`to` and `from`), which a
/// cycle reversal does not change.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum PortPolicy {
    /// One port per face, at its center, shared by every arrival and
    /// departure declared on it — the default, and the drawing every
    /// undeclared fan-in and fan-out already has.
    #[default]
    Single,
    /// One arrival port and one departure port per face, adjacent:
    /// the face's primary direction (arrivals on the face the flow
    /// arrives on, departures on the face it leaves by, arrivals on a
    /// side face) at the center, the other on the next cell along the
    /// face. A one-cell face shares.
    Paired,
    /// Up to the bound's number of ports per face, spread evenly and
    /// centered; ends beyond the bound share round-robin — all in
    /// tangent order, the peer's position along the face.
    Spread(PortBound),
    /// The placer registered on the graph (`set_port_placer`) chooses
    /// every end's cell; it is told the node id, so one function
    /// places every `Custom` node. Refused until a placer is
    /// registered.
    Custom,
}

#[cfg(feature = "ports")]
impl PortPolicy {
    /// The CSR table's one-byte code for "inherit the graph's policy".
    pub(crate) const INHERIT: u8 = 0;
    const CODE_SINGLE: u8 = 1;
    const CODE_PAIRED: u8 = 2;
    const CODE_SPREAD_FACE: u8 = 3;
    const CODE_CUSTOM: u8 = 255;
    /// The largest `Ports(n)` bound: what the byte code carries, and
    /// what larger bounds saturate to on both backends.
    pub const SPREAD_MAX: u8 = Self::CODE_CUSTOM - Self::CODE_SPREAD_FACE - 1;

    /// The one-byte code the CSR port table stores (the graph's
    /// placer travels separately). `Ports(0)` is one port, so it
    /// shares `Ports(1)`'s code.
    pub(crate) const fn to_code(self) -> u8 {
        match self {
            PortPolicy::Single => Self::CODE_SINGLE,
            PortPolicy::Paired => Self::CODE_PAIRED,
            PortPolicy::Spread(PortBound::Face) => Self::CODE_SPREAD_FACE,
            PortPolicy::Spread(PortBound::Ports(n)) => {
                let n = if n == 0 {
                    1
                } else if n > Self::SPREAD_MAX {
                    Self::SPREAD_MAX
                } else {
                    n
                };
                Self::CODE_SPREAD_FACE + n
            }
            PortPolicy::Custom => Self::CODE_CUSTOM,
        }
    }

    /// Inverse of [`to_code`](Self::to_code): `None` for the inherit
    /// code.
    pub(crate) fn from_code(code: u8) -> Option<PortPolicy> {
        Some(match code {
            Self::INHERIT => return None,
            Self::CODE_SINGLE => PortPolicy::Single,
            Self::CODE_PAIRED => PortPolicy::Paired,
            Self::CODE_SPREAD_FACE => PortPolicy::Spread(PortBound::Face),
            Self::CODE_CUSTOM => PortPolicy::Custom,
            n => PortPolicy::Spread(PortBound::Ports(n - Self::CODE_SPREAD_FACE)),
        })
    }
}

/// The upper bound of a [`PortPolicy::Spread`] face.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortBound {
    /// As many ports as the face has cells.
    Face,
    /// At most this many ports. `0` is one port (a shared face;
    /// [`Face`](Self::Face) is the unbounded form); bounds above 251
    /// saturate to 251 on both backends (the CSR table's byte code
    /// carries that much).
    Ports(u8),
}

/// A caller's placement rule: the cell offset along the face for one
/// end, clamped to the face by the layout. A plain `fn`, so it runs
/// on the no-alloc pipeline and both backends place identically.
pub type PortPlacer = fn(PortSlot) -> usize;

/// One end to place, as a [`PortPlacer`] sees it.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PortSlot {
    /// The node's id.
    pub node: usize,
    /// The face, physically.
    pub face: PhysicalSide,
    /// Cells along the face.
    pub cells: usize,
    /// Arrivals declared on this face.
    pub arrivals: usize,
    /// Departures declared on this face.
    pub departures: usize,
    /// This end's index among the face's ends, in tangent order.
    pub index: usize,
    /// Whether this end is an arrival.
    pub arrival: bool,
}

impl Face {
    /// The physical side this role-space face is under a profile and
    /// its level flip: level faces follow the flow (leading is North
    /// under TopDown, South under BottomUp, West under LeftRight, East
    /// under RightLeft); cross faces never flip.
    pub(crate) const fn physical(self, axis: FlowAxis, level_flipped: bool) -> PhysicalSide {
        match (axis, self) {
            (FlowAxis::Y, Face::LevelLeading) => {
                if level_flipped {
                    PhysicalSide::South
                } else {
                    PhysicalSide::North
                }
            }
            (FlowAxis::Y, Face::LevelTrailing) => {
                if level_flipped {
                    PhysicalSide::North
                } else {
                    PhysicalSide::South
                }
            }
            (FlowAxis::Y, Face::CrossLeading) => PhysicalSide::West,
            (FlowAxis::Y, Face::CrossTrailing) => PhysicalSide::East,
            (FlowAxis::X, Face::LevelLeading) => {
                if level_flipped {
                    PhysicalSide::East
                } else {
                    PhysicalSide::West
                }
            }
            (FlowAxis::X, Face::LevelTrailing) => {
                if level_flipped {
                    PhysicalSide::West
                } else {
                    PhysicalSide::East
                }
            }
            (FlowAxis::X, Face::CrossLeading) => PhysicalSide::North,
            (FlowAxis::X, Face::CrossTrailing) => PhysicalSide::South,
        }
    }
}

/// Whether the direction mirrors the LEVEL axis after layout
/// (`BottomUp` flips y under the vertical profile, `RightLeft` flips x
/// under the horizontal one). Physical leading/trailing level faces
/// swap under a flip; cross faces are untouched.
pub(crate) const fn level_flipped<A: Axis>(direction: Direction) -> bool {
    match A::FLOW_AXIS {
        #[cfg(feature = "layout-vertical")]
        FlowAxis::Y => matches!(direction, Direction::BottomUp),
        #[cfg(feature = "layout-horizontal")]
        FlowAxis::X => matches!(direction, Direction::RightLeft),
        #[allow(unreachable_patterns)]
        _ => false,
    }
}

impl Face {
    /// Map a declared side to the role-space face under `flow_axis`,
    /// with the direction flip folded in. `Auto` yields the role's
    /// default face. A `const fn`: with a literal side this folds to
    /// a constant at the call site, and the table is checked at
    /// compile time below.
    pub(crate) const fn of(
        side: PortSide,
        flow_axis: FlowAxis,
        flipped: bool,
        role: EndRole,
    ) -> Face {
        let unflipped = match (side, flow_axis) {
            (PortSide::Auto, _) => return role.auto_face(),
            // Flow-relative level names are role faces already — no flip.
            (PortSide::Upstream, _) => return Face::LevelLeading,
            (PortSide::Downstream, _) => return Face::LevelTrailing,
            // Rotations of the flow vector, viewer's frame (y grows
            // downward): a DOWNWARD flow turned clockwise faces left,
            // a RIGHTWARD flow turned clockwise faces down — the two
            // profiles have opposite chirality in role space. A level
            // flip reverses the flow vector, hence the rotation too.
            (PortSide::Clockwise, _) | (PortSide::Counterclockwise, _) => {
                let clockwise = matches!(side, PortSide::Clockwise);
                let leading = match flow_axis {
                    FlowAxis::Y => clockwise,
                    FlowAxis::X => !clockwise,
                };
                return if leading != flipped {
                    Face::CrossLeading
                } else {
                    Face::CrossTrailing
                };
            }
            (PortSide::North, FlowAxis::Y) | (PortSide::West, FlowAxis::X) => Face::LevelLeading,
            (PortSide::South, FlowAxis::Y) | (PortSide::East, FlowAxis::X) => Face::LevelTrailing,
            (PortSide::West, FlowAxis::Y) | (PortSide::North, FlowAxis::X) => Face::CrossLeading,
            (PortSide::East, FlowAxis::Y) | (PortSide::South, FlowAxis::X) => Face::CrossTrailing,
        };
        match (unflipped, flipped) {
            (Face::LevelLeading, true) => Face::LevelTrailing,
            (Face::LevelTrailing, true) => Face::LevelLeading,
            (f, _) => f,
        }
    }
}

// The direction table is evaluated during the BUILD: a wrong entry
// is a compile error, not a test failure. One probe per frame.
const _: () = {
    assert!(matches!(
        Face::of(PortSide::North, FlowAxis::Y, false, EndRole::Source),
        Face::LevelLeading
    ));
    assert!(matches!(
        Face::of(PortSide::Upstream, FlowAxis::X, true, EndRole::Target),
        Face::LevelLeading
    ));
    assert!(matches!(
        Face::of(PortSide::Clockwise, FlowAxis::X, true, EndRole::Source),
        Face::CrossLeading
    ));
    assert!(matches!(
        Face::of(PortSide::Auto, FlowAxis::Y, true, EndRole::Target),
        Face::LevelLeading
    ));
};
#[cfg(feature = "layout-vertical")]
const _: () = assert!(level_flipped::<super::geometry::Vertical>(
    Direction::BottomUp
));
#[cfg(feature = "layout-horizontal")]
const _: () = assert!(!level_flipped::<super::geometry::Horizontal>(
    Direction::LeftRight
));

/// The boundaries of a face of `capacity` cells split into `n` equal
/// sub-spans: yields `(start, end)` for k in `0..n`, every boundary
/// being `floor(k · capacity / n)` — produced by quotient/remainder
/// accumulation, so there is no multiplication, no wide integer, and
/// no overflow on ANY target (every value stays `≤ capacity` or
/// `< 2n`). Requires `n ≤ capacity` for non-empty spans; the callers
/// switch to round-robin above capacity.
#[cfg(feature = "ports")]
#[derive(Debug, Clone)]
pub(crate) struct SubSpans {
    quotient: usize,
    remainder: usize,
    n: usize,
    carry: usize,
    next_start: usize,
    k: usize,
}

#[cfg(feature = "ports")]
impl SubSpans {
    pub(crate) fn new(n: usize, capacity: usize) -> SubSpans {
        let n = n.max(1);
        SubSpans {
            quotient: capacity / n,
            remainder: capacity % n,
            n,
            carry: 0,
            next_start: 0,
            k: 0,
        }
    }
}

#[cfg(feature = "ports")]
impl Iterator for SubSpans {
    type Item = (usize, usize);
    fn next(&mut self) -> Option<(usize, usize)> {
        if self.k >= self.n {
            return None;
        }
        let start = self.next_start;
        let mut end = start + self.quotient;
        self.carry += self.remainder;
        if self.carry >= self.n {
            end += 1;
            self.carry -= self.n;
        }
        self.next_start = end;
        self.k += 1;
        Some((start, end))
    }
}

/// The reference formulation of one request's cell — the assignment
/// pass walks [`SubSpans`] once instead of calling this per request,
/// so it exists for the pins that compare the two.
///
/// Offset along a LEVEL face of `capacity` cells for request `k` of
/// `n`, in tangent order. Under capacity: the center — by the
/// profile's own center rule — of the k-th of `n` equal sub-spans, so
/// ONE request lands exactly where `Auto` does and `n` requests
/// spread evenly and centered. Beyond capacity: round-robin
/// `k mod capacity`, sharing cells evenly. A zero-extent face has only
/// its center line (offset 0). O(k) through [`SubSpans`] — the
/// assignment pass walks the iterator once instead.
#[cfg(all(test, feature = "ports"))]
pub(crate) fn face_offset<A: Axis>(k: usize, n: usize, capacity: usize) -> usize {
    spread_offset::<A>(k, n, capacity, PortBound::Face, true)
}

/// One explicit request for a position on a node face.
#[derive(Debug, Clone, Copy)]
#[cfg_attr(not(feature = "ports"), allow(dead_code))] // the arena temporaries name it; only the ports pass builds it
pub(crate) struct FaceRequest {
    /// The node whose face is requested.
    pub node: usize,
    /// The requested face (role space).
    pub face: Face,
    /// Ordering key along the face's tangent: the peer's position on
    /// that axis (ties break by `edge`, the input index).
    pub key: usize,
    /// The requesting edge's input index.
    pub edge: usize,
    /// Which end of that edge this is (layout role).
    pub end: EndRole,
    /// Whether this is the edge's DECLARED target end — the identity
    /// the policies see, which a cycle reversal does not change.
    pub arrival: bool,
}

/// Tangent order: per (node, face), by key, then input index. Not
/// generic over the axis profile, so the sort is instantiated once.
#[cfg(feature = "ports")]
fn sort_requests(requests: &mut [FaceRequest]) {
    requests.sort_unstable_by_key(|r| (r.node, r.face as u8, r.key, r.edge));
}

/// Assign every LEVEL-face request its cell under the node's port
/// policy: requests are grouped per (node, face), ordered along the
/// tangent by `key` then input index, and placed as the policy says.
/// `face_span(node)` yields the node's `(base, extent)` along the
/// CROSS axis — the tangent of a level face; `policy(node)` the node's
/// policy (its override, else the graph's) and its id;
/// `place(edge, end, coordinate)` receives each result. O(R log R)
/// over the requests only — Auto edges never enter — plus the spread
/// walk (bounded by the face's cells) per shared end. Slice-based, so
/// the arena backend runs it on carved scratch.
///
/// Level faces only — a lateral face's tangent is the LEVEL axis and
/// its centering the profile's level rule:
/// [`assign_cross_face_positions`] is its twin.
#[cfg(feature = "ports")]
pub(crate) fn assign_level_face_positions<A: Axis>(
    requests: &mut [FaceRequest],
    face_span: impl FnMut(usize) -> (usize, usize),
    policy: impl FnMut(usize) -> (PortPolicy, usize),
    placer: Option<PortPlacer>,
    flipped: bool,
    place: impl FnMut(usize, EndRole, usize),
) {
    debug_assert!(
        requests
            .iter()
            .all(|r| matches!(r.face, Face::LevelLeading | Face::LevelTrailing)),
        "level faces only"
    );
    assign_face_positions::<A>(requests, face_span, policy, placer, flipped, place, true);
}

/// The lateral twin of [`assign_level_face_positions`]: requests on a
/// node's CROSS faces are placed along the LEVEL axis — `face_span(node)`
/// yields `(0, level extent)` and the result is the row offset within
/// the node — centered by the profile's level rule (`level_center`),
/// which is where `Single` lands and where `Auto` would.
#[cfg(feature = "ports")]
pub(crate) fn assign_cross_face_positions<A: Axis>(
    requests: &mut [FaceRequest],
    face_span: impl FnMut(usize) -> (usize, usize),
    policy: impl FnMut(usize) -> (PortPolicy, usize),
    placer: Option<PortPlacer>,
    flipped: bool,
    place: impl FnMut(usize, EndRole, usize),
) {
    debug_assert!(
        requests
            .iter()
            .all(|r| matches!(r.face, Face::CrossLeading | Face::CrossTrailing)),
        "cross faces only"
    );
    assign_face_positions::<A>(requests, face_span, policy, placer, flipped, place, false);
}

/// The shared pass: per (node, face) group in tangent order, the
/// policy's cell for each end; the center line for a zero-extent
/// face whatever the policy. O(n) per group over its `n` ends plus
/// one walk of the face's sub-spans per round of a `Spread`.
#[cfg(feature = "ports")]
fn assign_face_positions<A: Axis>(
    requests: &mut [FaceRequest],
    mut face_span: impl FnMut(usize) -> (usize, usize),
    mut policy: impl FnMut(usize) -> (PortPolicy, usize),
    placer: Option<PortPlacer>,
    flipped: bool,
    mut place: impl FnMut(usize, EndRole, usize),
    level: bool,
) {
    sort_requests(requests);
    let center_of = |start: usize, len: usize| {
        if level {
            A::cross_center(start, len)
        } else {
            A::level_center(start, len)
        }
    };
    let mut i = 0;
    while i < requests.len() {
        let (node, face) = (requests[i].node, requests[i].face);
        let mut j = i;
        while j < requests.len() && requests[j].node == node && requests[j].face == face {
            j += 1;
        }
        let (base, capacity) = face_span(node);
        let group = &requests[i..j];
        i = j;
        if capacity == 0 {
            for r in group {
                place(r.edge, r.end, base);
            }
            continue;
        }
        let (policy, id) = policy(node);
        let n = group.len();
        let center = center_of(0, capacity);
        if let PortPolicy::Spread(bound) = policy {
            // `m` ports at sub-span centers; end k takes port k mod m.
            // One walk of the sub-spans per round of `m` ends.
            let m = n.min(bound_ports(bound, capacity)).max(1);
            let mut k = 0;
            while k < n {
                for (start, end) in SubSpans::new(m, capacity) {
                    let Some(r) = group.get(k) else {
                        break;
                    };
                    place(r.edge, r.end, base + center_of(start, end - start));
                    k += 1;
                }
            }
            continue;
        }
        let arrivals = group.iter().filter(|r| r.arrival).count();
        for (k, r) in group.iter().enumerate() {
            let offset = match policy {
                PortPolicy::Single | PortPolicy::Spread(_) => center,
                PortPolicy::Paired => paired_offset(face, r.arrival, center, capacity),
                // The setters refuse `Custom` without a registered
                // placer, so `None` here is unreachable; the center is
                // the honest answer if it ever were.
                PortPolicy::Custom => placer.map_or(center, |f| {
                    f(PortSlot {
                        node: id,
                        face: face.physical(A::FLOW_AXIS, flipped),
                        cells: capacity,
                        arrivals,
                        departures: n - arrivals,
                        index: k,
                        arrival: r.arrival,
                    })
                    .min(capacity - 1)
                }),
            };
            place(r.edge, r.end, base + offset);
        }
    }
}

/// The ports a `Spread` bound allows on a face of `capacity` cells:
/// the face's cells for `Face`, else the bound, at least one and
/// saturated at [`PortPolicy::SPREAD_MAX`] — the same number on both
/// backends, whatever the byte code can carry.
#[cfg(feature = "ports")]
pub(crate) fn bound_ports(bound: PortBound, capacity: usize) -> usize {
    match bound {
        PortBound::Face => capacity,
        PortBound::Ports(p) => (p.clamp(1, PortPolicy::SPREAD_MAX) as usize).min(capacity),
    }
}

/// `Paired`: the face's primary direction — arrivals on the arrive
/// face, departures on the leave face, arrivals on a side face — at
/// the center; the other on the next cell along the tangent, the
/// previous when the center is the last cell; a one-cell face shares.
#[cfg(feature = "ports")]
pub(crate) fn paired_offset(face: Face, arrival: bool, center: usize, capacity: usize) -> usize {
    let primary = match face {
        Face::LevelLeading => arrival,
        Face::LevelTrailing => !arrival,
        Face::CrossLeading | Face::CrossTrailing => arrival,
    };
    if primary || capacity < 2 {
        center
    } else if center + 1 < capacity {
        center + 1
    } else {
        center - 1
    }
}

/// `Spread(bound)`: `m = min(ends, bound, cells)` ports at the centers
/// — by the profile's own rule for the face's tangent — of `m` equal
/// sub-spans; end `k` takes port `k mod m`. One end lands exactly
/// where `Auto` does; `n ≤ m` ends spread evenly and centered; beyond
/// the bound they share evenly. O(m) through [`SubSpans`].
#[cfg(all(test, feature = "ports"))]
pub(crate) fn spread_offset<A: Axis>(
    k: usize,
    n: usize,
    capacity: usize,
    bound: PortBound,
    level: bool,
) -> usize {
    if capacity == 0 {
        return 0;
    }
    let m = n.min(bound_ports(bound, capacity)).max(1);
    let (start, end) = SubSpans::new(m, capacity)
        .nth(k % m)
        .unwrap_or((0, capacity));
    if level {
        A::cross_center(start, end - start)
    } else {
        A::level_center(start, end - start)
    }
}

/// One resolved edge end: the face and the cross-axis line it meets
/// the node on. The level-axis line is the backend's (it depends on
/// the level band tables), keyed on the face.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Attachment {
    pub face: Face,
    pub cross: usize,
}

impl Attachment {
    /// Resolve a declared side. `Auto` is the 0.10 rule: the layout
    /// role's default level face at the node's cross-axis center line
    /// (the profile's `cross_center` formula, which matches the IR
    /// center fields exactly). A positioned explicit request overrides
    /// this (the backends carry positions along the face separately);
    /// an end that could not route off its role's face falls back to
    /// exactly this.
    pub(crate) fn resolve<A: Axis>(
        side: PortSide,
        flipped: bool,
        role: EndRole,
        cross_base: usize,
        cross_extent: usize,
    ) -> Self {
        Attachment {
            face: Face::of(side, A::FLOW_AXIS, flipped, role),
            cross: A::cross_center(cross_base, cross_extent),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Physical sides map to role faces per profile, and the level
    /// flip swaps only the level faces.
    #[test]
    fn physical_sides_map_to_role_faces() {
        use EndRole::Source;
        // Vertical, TopDown.
        assert_eq!(
            Face::of(PortSide::North, FlowAxis::Y, false, Source),
            Face::LevelLeading
        );
        assert_eq!(
            Face::of(PortSide::South, FlowAxis::Y, false, Source),
            Face::LevelTrailing
        );
        assert_eq!(
            Face::of(PortSide::West, FlowAxis::Y, false, Source),
            Face::CrossLeading
        );
        assert_eq!(
            Face::of(PortSide::East, FlowAxis::Y, false, Source),
            Face::CrossTrailing
        );
        // Vertical, BottomUp: top/bottom swap, left/right hold.
        assert_eq!(
            Face::of(PortSide::North, FlowAxis::Y, true, Source),
            Face::LevelTrailing
        );
        assert_eq!(
            Face::of(PortSide::South, FlowAxis::Y, true, Source),
            Face::LevelLeading
        );
        assert_eq!(
            Face::of(PortSide::West, FlowAxis::Y, true, Source),
            Face::CrossLeading
        );
        // Horizontal, LeftRight.
        assert_eq!(
            Face::of(PortSide::West, FlowAxis::X, false, Source),
            Face::LevelLeading
        );
        assert_eq!(
            Face::of(PortSide::East, FlowAxis::X, false, Source),
            Face::LevelTrailing
        );
        assert_eq!(
            Face::of(PortSide::North, FlowAxis::X, false, Source),
            Face::CrossLeading
        );
        assert_eq!(
            Face::of(PortSide::South, FlowAxis::X, false, Source),
            Face::CrossTrailing
        );
        // Horizontal, RightLeft: left/right swap, top/bottom hold.
        assert_eq!(
            Face::of(PortSide::West, FlowAxis::X, true, Source),
            Face::LevelTrailing
        );
        assert_eq!(
            Face::of(PortSide::North, FlowAxis::X, true, Source),
            Face::CrossLeading
        );
    }

    /// Flow-relative names are role faces in every direction — the
    /// flip never touches them.
    #[test]
    fn relative_sides_ignore_the_flip() {
        for flipped in [false, true] {
            for axis in [FlowAxis::Y, FlowAxis::X] {
                for role in [EndRole::Source, EndRole::Target] {
                    assert_eq!(
                        Face::of(PortSide::Upstream, axis, flipped, role),
                        Face::LevelLeading
                    );
                    assert_eq!(
                        Face::of(PortSide::Downstream, axis, flipped, role),
                        Face::LevelTrailing
                    );
                }
            }
        }
    }

    /// Rotation sides, all four directions, spelled as the physical
    /// face each equals (viewer's frame, y downward). A direction
    /// change keeps the relative side and moves the physical one —
    /// the whole point of the frame.
    #[test]
    fn rotation_sides_follow_the_flow_vector() {
        use EndRole::Source;
        use PortSide::{Clockwise, Counterclockwise};
        // TopDown: flow down. Clockwise → West, counterclockwise → East.
        assert_eq!(
            Face::of(Clockwise, FlowAxis::Y, false, Source),
            Face::CrossLeading
        );
        assert_eq!(
            Face::of(Counterclockwise, FlowAxis::Y, false, Source),
            Face::CrossTrailing
        );
        // BottomUp: flow up. Clockwise → East, counterclockwise → West.
        assert_eq!(
            Face::of(Clockwise, FlowAxis::Y, true, Source),
            Face::CrossTrailing
        );
        assert_eq!(
            Face::of(Counterclockwise, FlowAxis::Y, true, Source),
            Face::CrossLeading
        );
        // LeftRight: flow right. Clockwise → South, counterclockwise → North.
        assert_eq!(
            Face::of(Clockwise, FlowAxis::X, false, Source),
            Face::CrossTrailing
        );
        assert_eq!(
            Face::of(Counterclockwise, FlowAxis::X, false, Source),
            Face::CrossLeading
        );
        // RightLeft: flow left. Clockwise → North, counterclockwise → South.
        assert_eq!(
            Face::of(Clockwise, FlowAxis::X, true, Source),
            Face::CrossLeading
        );
        assert_eq!(
            Face::of(Counterclockwise, FlowAxis::X, true, Source),
            Face::CrossTrailing
        );
        // Physical lateral sides do NOT rotate with the flip (pinned
        // above for West under BottomUp) — that is the difference
        // between the two frames.
        assert_eq!(
            Face::of(Clockwise, FlowAxis::Y, true, Source),
            Face::of(PortSide::East, FlowAxis::Y, true, Source)
        );
    }

    /// One request lands exactly on the profile's own center (where
    /// `Auto` is); several spread evenly and centered; beyond capacity
    /// they wrap; a zero-extent face has only its center line.
    #[cfg(feature = "ports")]
    #[test]
    fn face_offset_rules() {
        #[cfg(feature = "layout-vertical")]
        {
            use super::super::geometry::Vertical as V;
            assert_eq!(
                face_offset::<V>(0, 1, 6),
                V::cross_center(0, 6),
                "one = Auto's cell"
            );
            assert_eq!(face_offset::<V>(0, 1, 5), 2);
            assert_eq!(
                (0..3)
                    .map(|k| face_offset::<V>(k, 3, 6))
                    .collect::<Vec<_>>(),
                vec![1, 3, 5],
                "three on six: sub-span centers"
            );
            assert_eq!(
                (0..2)
                    .map(|k| face_offset::<V>(k, 2, 4))
                    .collect::<Vec<_>>(),
                vec![1, 3],
                "two straddle the center"
            );
            assert_eq!(
                (0..5)
                    .map(|k| face_offset::<V>(k, 5, 3))
                    .collect::<Vec<_>>(),
                vec![0, 1, 2, 0, 1],
                "beyond capacity: round-robin"
            );
            assert_eq!(face_offset::<V>(0, 3, 0), 0, "zero extent: the center line");
        }
        #[cfg(feature = "layout-horizontal")]
        {
            use super::super::geometry::Horizontal as H;
            assert_eq!(
                face_offset::<H>(0, 1, 4),
                H::cross_center(0, 4),
                "one = Auto's cell"
            );
            assert_eq!(
                face_offset::<H>(0, 1, 4),
                1,
                "the horizontal profile's (h-1)/2 rule"
            );
        }
    }

    /// Sub-span boundaries are exactly `floor(k·C/n)` — checked against
    /// wide arithmetic for small values — and overflow-proof for the
    /// values that would wrap a wide multiply on 64-bit.
    #[cfg(feature = "ports")]
    #[test]
    fn sub_spans_are_exact_and_overflow_free() {
        for n in 1..=9usize {
            for c in n..=40usize {
                let spans: Vec<(usize, usize)> = SubSpans::new(n, c).collect();
                assert_eq!(spans.len(), n);
                for (k, &(start, end)) in spans.iter().enumerate() {
                    let k128 = k as u128;
                    assert_eq!(
                        start as u128,
                        k128 * c as u128 / n as u128,
                        "n={n} c={c} k={k}"
                    );
                    assert_eq!(end as u128, (k128 + 1) * c as u128 / n as u128);
                    assert!(end > start, "non-empty under capacity");
                }
                assert_eq!(spans.last().unwrap().1, c, "the last span ends at capacity");
            }
        }
        // `(k+1)·capacity` would overflow a 64-bit multiply here.
        let huge = usize::MAX / 2 + 3;
        let spans: Vec<(usize, usize)> = SubSpans::new(5, huge).collect();
        assert_eq!(spans[4].1, huge);
        assert!(spans.windows(2).all(|w| w[0].1 == w[1].0), "contiguous");
    }

    /// `Auto` is layout-role-defined and flip-independent: a source
    /// always leaves the trailing level face, a target arrives leading.
    #[test]
    fn auto_follows_the_layout_role() {
        for flipped in [false, true] {
            for axis in [FlowAxis::Y, FlowAxis::X] {
                assert_eq!(
                    Face::of(PortSide::Auto, axis, flipped, EndRole::Source),
                    Face::LevelTrailing
                );
                assert_eq!(
                    Face::of(PortSide::Auto, axis, flipped, EndRole::Target),
                    Face::LevelLeading
                );
            }
        }
    }
}

/// The seam end to end: explicit sides that name the SAME physical
/// faces `Auto` resolves to must render byte-identically — in every
/// direction, through the real layout, including a reversed cycle
/// edge (whose declared sides swap onto the layout roles).
#[cfg(all(
    test,
    feature = "std",
    feature = "ports",
    any(feature = "layout-vertical", feature = "layout-horizontal")
))]
mod layout_tests {
    use super::{PortBound, PortPolicy, PortSide};
    use crate::graph::Graph;
    use crate::render::engine::RenderOptions;

    /// Four forward edges and a reversed cycle edge, all `Auto`;
    /// tests declare sides by input index on top of it.
    fn fixture() -> Graph<'static> {
        let mut g = Graph::new();
        g.add_node(1usize, "Start");
        g.add_node(2usize, "Middle");
        g.add_node(3usize, "Wide node");
        g.add_node(4usize, "End");
        g.add_edge(1usize, 2usize, Some("go"));
        g.add_edge(1usize, 3usize, None);
        g.add_edge(2usize, 4usize, None);
        g.add_edge(3usize, 4usize, None);
        g.add_edge(4usize, 1usize, Some("again")); // cycle → reversed
        g
    }

    /// (direction, side a layout-source leaves from, side a
    /// layout-target arrives on) — Auto's faces named explicitly, in
    /// the physical frame AND the flow-relative one, for every
    /// direction this build enables.
    fn auto_equivalents() -> Vec<(crate::graph::Direction, PortSide, PortSide)> {
        use crate::graph::Direction;
        let mut cases = Vec::new();
        #[cfg(feature = "layout-vertical")]
        {
            cases.push((Direction::TopDown, PortSide::South, PortSide::North));
            cases.push((Direction::BottomUp, PortSide::North, PortSide::South));
            cases.push((Direction::TopDown, PortSide::Downstream, PortSide::Upstream));
            cases.push((
                Direction::BottomUp,
                PortSide::Downstream,
                PortSide::Upstream,
            ));
        }
        #[cfg(feature = "layout-horizontal")]
        {
            cases.push((Direction::LeftRight, PortSide::East, PortSide::West));
            cases.push((Direction::RightLeft, PortSide::West, PortSide::East));
            cases.push((
                Direction::LeftRight,
                PortSide::Downstream,
                PortSide::Upstream,
            ));
            cases.push((
                Direction::RightLeft,
                PortSide::Downstream,
                PortSide::Upstream,
            ));
        }
        cases
    }

    /// A graph that never declares a port stores nothing per edge;
    /// the first non-Auto declaration materializes the table — and
    /// only then.
    #[test]
    fn undeclared_ports_cost_nothing() {
        let mut g = Graph::new();
        g.add_node(1usize, "A");
        g.add_node(2usize, "B");
        g.add_edge(1usize, 2usize, None);
        g.add_edge(2usize, 1usize, None);
        assert!(g.edge_ports.is_empty(), "no declaration, no table");
        assert_eq!(g.edge_ports.capacity(), 0, "no allocation either");
        g.add_edge(1usize, 2usize, Some("x"))
            .to_port(PortSide::Auto);
        assert!(
            g.edge_ports.is_empty(),
            "explicit Auto is what undeclared means"
        );

        g.add_edge(1usize, 2usize, Some("y"))
            .to_port(PortSide::North);
        assert_eq!(g.edge_ports.len(), 4, "materialized to the edge count");
        assert_eq!(g.edge_ports[0], (PortSide::Auto, PortSide::Auto));
        assert_eq!(g.edge_ports[3].1, PortSide::North);
        g.add_edge(2usize, 2usize, None);
        assert_eq!(g.edge_ports.len(), 5, "stays parallel once it exists");
        assert!(
            !g.set_edge_ports(99, PortSide::Auto, PortSide::Auto),
            "unknown edge"
        );
    }

    /// ONE explicit request per face on Auto's own face lands exactly
    /// where Auto does — so the render is byte-identical — in every
    /// direction and both frames, declared through the fluent handle.
    #[test]
    fn single_explicit_request_equals_auto() {
        for (direction, leave, arrive) in auto_equivalents() {
            let mut auto = fixture();
            auto.set_direction(direction);
            let expected = auto.compute_layout().render_string(&RenderOptions::plain());

            let mut declared = Graph::new();
            declared.set_direction(direction);
            declared.add_node(1usize, "Start");
            declared.add_node(2usize, "Middle");
            declared.add_node(3usize, "Wide node");
            declared.add_node(4usize, "End");
            declared
                .add_edge(1usize, 2usize, Some("go"))
                .from_port(leave)
                .to_port(arrive);
            declared.add_edge(1usize, 3usize, None);
            declared.add_edge(2usize, 4usize, None);
            declared.add_edge(3usize, 4usize, None);
            declared.add_edge(4usize, 1usize, Some("again"));
            let got = declared
                .compute_layout()
                .render_string(&RenderOptions::plain());
            assert_eq!(got, expected, "{direction:?} {leave:?}/{arrive:?}");
        }
    }

    /// The reversed cycle edge: its DECLARED source is the layout
    /// target, so mirrored declared sides name Auto's faces — and
    /// render byte-identically. Logical-end binding, end to end.
    #[test]
    fn reversed_edge_binds_sides_to_declared_endpoints() {
        for (direction, leave, arrive) in auto_equivalents() {
            let mut auto = fixture();
            auto.set_direction(direction);
            let expected = auto.compute_layout().render_string(&RenderOptions::plain());
            let mut declared = fixture();
            declared.set_direction(direction);
            assert!(declared.set_edge_ports(4, arrive, leave));
            let got = declared
                .compute_layout()
                .render_string(&RenderOptions::plain());
            assert_eq!(got, expected, "{direction:?} reversed {arrive:?}/{leave:?}");
        }
    }

    /// Several explicit requests on one face spread along it — the
    /// sub-span centers, in tangent order (the child furthest west
    /// gets the westmost cell) — and the trunks leave from their own
    /// columns, so the output differs from Auto by design.
    #[cfg(feature = "layout-vertical")]
    #[test]
    fn explicit_requests_spread_along_the_face_in_tangent_order() {
        use super::super::geometry::Vertical;
        use super::face_offset;
        use crate::render::engine::BoxedNode;
        // A boxed node has three rows, so a policy other than `Single`
        // applies; its box is 8 cells wide.
        let mut g = Graph::new();
        g.set_port_policy(PortPolicy::Spread(PortBound::Face));
        g.add_node(0usize, BoxedNode("Root"));
        for i in 1..=3usize {
            g.add_node(i, "x");
            g.add_edge(0usize, i, None).from_port(PortSide::South);
        }
        let auto = {
            let mut a = Graph::new();
            a.add_node(0usize, BoxedNode("Root"));
            for i in 1..=3usize {
                a.add_node(i, "x");
                a.add_edge(0usize, i, None);
            }
            a.compute_layout().render_string(&RenderOptions::plain())
        };
        let ir = g.compute_layout();
        let root = ir.node_by_id(0).unwrap();
        assert_eq!(root.width, 8);
        // Tangent order: by the child's x.
        let mut edges: Vec<_> = ir.edges().iter().collect();
        edges.sort_by_key(|e| ir.node_by_id(e.to_id).unwrap().x);
        let cells: Vec<usize> = edges.iter().map(|e| e.from_x - root.x).collect();
        assert_eq!(
            cells,
            (0..3)
                .map(|k| face_offset::<Vertical>(k, 3, 8))
                .collect::<Vec<_>>()
        );
        assert_eq!(cells, vec![1, 3, 6]);
        let rendered = ir.render_string(&RenderOptions::plain());
        assert_ne!(rendered, auto, "positions change the drawing");
        // The default policy shares the one port: the Auto drawing.
        g.set_port_policy(PortPolicy::Single);
        assert_eq!(
            g.compute_layout().render_string(&RenderOptions::plain()),
            auto,
            "Single is the Auto drawing"
        );
    }

    /// A resolved port on a one-cell `Fixed` custom node in a two-node
    /// cycle stays on the node: the cycle separation shift is skipped
    /// when it would leave either face (and the pair shares the cell).
    #[cfg(feature = "layout-vertical")]
    #[test]
    fn two_node_cycle_shift_never_leaves_a_narrow_face() {
        use crate::render::engine::CustomNode;
        let mut g = Graph::new();
        g.add_node(
            1usize,
            CustomNode {
                label: "n",
                width: 1,
                height: 1,
                painter: None,
                payload: "",
            },
        );
        g.add_node(2usize, "B");
        g.add_edge(1usize, 2usize, None).from_port(PortSide::South);
        g.add_edge(2usize, 1usize, None).to_port(PortSide::South); // reversed
        let ir = g.compute_layout();
        let narrow = ir.node_by_id(1).unwrap();
        assert_eq!(narrow.width, 1);
        for e in ir.edges() {
            let (x, node) = if e.from_id == 1 {
                (e.from_x, narrow)
            } else {
                (e.to_x, narrow)
            };
            assert!(
                x >= node.x && x < node.x + node.width,
                "edge {}→{} attaches at x={x}, face spans {}..{}",
                e.from_id,
                e.to_id,
                node.x,
                node.x + node.width
            );
        }
    }

    /// Beyond a face's capacity the requests wrap round-robin in the
    /// same tangent order, sharing cells evenly — never spilling to
    /// another side.
    #[cfg(feature = "layout-vertical")]
    #[test]
    fn over_capacity_wraps_round_robin_in_tangent_order() {
        use crate::render::engine::BoxedNode;
        let mut g = Graph::new();
        g.set_port_policy(PortPolicy::Spread(PortBound::Face));
        g.add_node(0usize, BoxedNode("A")); // 5 cells wide, 3 rows
        for i in 1..=7usize {
            g.add_node(i, "x");
            g.add_edge(0usize, i, None).from_port(PortSide::South);
        }
        let ir = g.compute_layout();
        let root = ir.node_by_id(0).unwrap();
        assert_eq!(root.width, 5);
        let mut edges: Vec<_> = ir.edges().iter().collect();
        edges.sort_by_key(|e| ir.node_by_id(e.to_id).unwrap().x);
        let cells: Vec<usize> = edges.iter().map(|e| e.from_x - root.x).collect();
        assert_eq!(cells, vec![0, 1, 2, 3, 4, 0, 1]);
        // A numeric bound caps the ports: three ports for seven ends.
        g.set_port_policy(PortPolicy::Spread(PortBound::Ports(3)));
        let ir = g.compute_layout();
        let root = ir.node_by_id(0).unwrap();
        let mut edges: Vec<_> = ir.edges().iter().collect();
        edges.sort_by_key(|e| ir.node_by_id(e.to_id).unwrap().x);
        let cells: Vec<usize> = edges.iter().map(|e| e.from_x - root.x).collect();
        assert_eq!(cells, vec![0, 2, 4, 0, 2, 4, 0]);
    }

    /// `Paired`: on a level face the head-on direction keeps the center
    /// and the other direction takes the next cell; on a side face the
    /// arrival keeps the center row and the departure the next. Under
    /// `Single` every end of a face shares its center.
    #[cfg(feature = "layout-vertical")]
    #[test]
    fn paired_gives_a_face_an_arrival_and_a_departure_port() {
        use crate::ir::EdgePath;
        use crate::render::engine::BoxedNode;
        // A hub wide enough that its neighbors' trunks stay inside its
        // span, so a lane beside it is free: 17 cells, 3 rows, center
        // x+8, row 1.
        let level_faces = |policy: PortPolicy| {
            let mut g = Graph::new();
            g.set_port_policy(policy);
            g.add_node(0usize, BoxedNode("Hub with room"));
            g.add_node(1usize, "In");
            g.add_node(2usize, "Out");
            g.add_edge(1usize, 0usize, None).to_port(PortSide::Upstream);
            g.add_edge(0usize, 2usize, None)
                .from_port(PortSide::Upstream);
            let ir = g.compute_layout();
            let hub = ir.node_by_id(0).unwrap();
            let arrival = ir.edges().iter().find(|e| e.to_id == 0).unwrap();
            let departure = ir.edges().iter().find(|e| e.from_id == 0).unwrap();
            assert!(
                matches!(departure.path, EdgePath::Orthogonal { .. }),
                "the departure detours"
            );
            (arrival.to_x - hub.x, departure.from_x - hub.x)
        };
        assert_eq!(level_faces(PortPolicy::Paired), (8, 9));
        assert_eq!(level_faces(PortPolicy::Single), (8, 8));
        let side_faces = |policy: PortPolicy| {
            let mut g = Graph::new();
            g.set_port_policy(policy);
            g.add_node(0usize, BoxedNode("Hub with room"));
            g.add_node(3usize, "Side");
            g.add_node(4usize, "Back");
            g.add_edge(3usize, 0usize, None).to_port(PortSide::East);
            g.add_edge(0usize, 4usize, None).from_port(PortSide::East);
            let ir = g.compute_layout();
            let hub = ir.node_by_id(0).unwrap();
            let arrival = ir.edges().iter().find(|e| e.to_id == 0).unwrap();
            let departure = ir.edges().iter().find(|e| e.from_id == 0).unwrap();
            for e in [arrival, departure] {
                assert!(matches!(e.path, EdgePath::Orthogonal { .. }), "both route");
            }
            (arrival.to_y - hub.y, departure.from_y - hub.y)
        };
        assert_eq!(side_faces(PortPolicy::Paired), (1, 2));
        assert_eq!(side_faces(PortPolicy::Single), (1, 1));
    }

    /// `Custom`: the placer sees the node, the physical face, its cells
    /// and its ends, and its answer is clamped to the face.
    #[cfg(feature = "layout-vertical")]
    #[test]
    fn a_custom_placer_places_every_end_of_a_face() {
        use crate::render::engine::BoxedNode;
        use crate::{PhysicalSide, PortSlot};
        fn ends_apart(slot: PortSlot) -> usize {
            assert_eq!(slot.node, 0);
            assert_eq!(slot.face, PhysicalSide::North);
            assert_eq!(slot.cells, 7);
            assert_eq!((slot.arrivals, slot.departures), (1, 1));
            assert!(slot.index < 2);
            if slot.arrival { 0 } else { slot.cells - 1 }
        }
        fn past_the_face(_: PortSlot) -> usize {
            usize::MAX
        }
        let mut g = Graph::new();
        g.add_node(0usize, BoxedNode("Hub"));
        // `Custom` refers to the graph's registered placer: refused
        // until one is registered.
        assert!(!g.set_node_port_policy(0usize, PortPolicy::Custom));
        assert!(!g.set_port_policy(PortPolicy::Custom));
        assert!(g.port_placer().is_none());
        g.set_port_placer(ends_apart);
        assert!(g.port_placer().is_some());
        assert!(g.set_node_port_policy(0usize, PortPolicy::Custom));
        g.add_node(1usize, "In");
        g.add_node(2usize, "Out");
        g.add_edge(1usize, 0usize, None).to_port(PortSide::Upstream);
        g.add_edge(0usize, 2usize, None)
            .from_port(PortSide::Upstream);
        let ir = g.compute_layout();
        let hub = ir.node_by_id(0).unwrap();
        let arrival = ir.edges().iter().find(|e| e.to_id == 0).unwrap();
        let departure = ir.edges().iter().find(|e| e.from_id == 0).unwrap();
        assert_eq!(arrival.to_x, hub.x);
        assert_eq!(departure.from_x, hub.x + 6);
        // Registering another placer replaces it for every `Custom`
        // node; its answer is clamped to the face.
        g.set_port_placer(past_the_face);
        let ir = g.compute_layout();
        let hub = ir.node_by_id(0).unwrap();
        let arrival = ir.edges().iter().find(|e| e.to_id == 0).unwrap();
        assert_eq!(arrival.to_x, hub.x + 6, "clamped to the face's last cell");
    }

    /// A face with one cell holds one port whatever the policy, while
    /// the same node's wide faces take the policy: on a one-row
    /// `[Hub node]` the east arrival and departure share the side cell
    /// under `Paired`, and the top face pairs.
    #[cfg(feature = "layout-vertical")]
    #[test]
    fn a_one_cell_face_holds_one_port_under_every_policy() {
        let mut g = Graph::new();
        g.add_node(0usize, "Hub node"); // "[Hub node]": one row, 10 cells, center x+5
        assert!(
            !g.set_node_port_policy(9usize, PortPolicy::Paired),
            "unknown node"
        );
        assert!(g.set_node_port_policy(0usize, PortPolicy::Paired));
        assert!(matches!(g.node_port_policy(0usize), PortPolicy::Paired));
        assert!(matches!(g.node_port_policy(1usize), PortPolicy::Single));
        g.add_node(1usize, "In");
        g.add_node(2usize, "Out");
        g.add_node(3usize, "Side");
        g.add_node(4usize, "Back");
        g.add_edge(1usize, 0usize, None).to_port(PortSide::Upstream);
        g.add_edge(0usize, 2usize, None)
            .from_port(PortSide::Upstream);
        g.add_edge(3usize, 0usize, None).to_port(PortSide::East);
        g.add_edge(0usize, 4usize, None).from_port(PortSide::East);
        assert_eq!(g.layout().reported().warnings().count(), 0);
        let ir = g.compute_layout();
        let hub = ir.node_by_id(0).unwrap();
        let edge = |from: usize, to: usize| {
            ir.edges()
                .iter()
                .find(|e| e.from_id == from && e.to_id == to)
                .unwrap()
        };
        assert_eq!(edge(1, 0).to_x - hub.x, 5, "top arrival at the center");
        assert_eq!(edge(0, 2).from_x - hub.x, 6, "top departure beside it");
        assert_eq!(edge(3, 0).to_y, hub.y, "one east cell, shared");
        assert_eq!(edge(0, 4).from_y, hub.y, "one east cell, shared");
        // Clearing the override restores inheritance, now and later.
        assert!(g.clear_node_port_policy(0usize));
        assert!(!g.clear_node_port_policy(0usize));
        assert!(matches!(g.node_port_policy(0usize), PortPolicy::Single));
        g.set_port_policy(PortPolicy::Paired);
        assert!(matches!(g.node_port_policy(0usize), PortPolicy::Paired));
    }

    /// Arrival and departure are the DECLARED ends: on a cycle-reversed
    /// edge the declared target is the layout source, and the policies
    /// still see an arrival — `Paired` gives it the arrival port and a
    /// custom placer is told so.
    #[cfg(feature = "layout-vertical")]
    #[test]
    fn policies_see_declared_ends_on_reversed_edges() {
        use crate::render::engine::BoxedNode;
        use crate::{PhysicalSide, PortSlot};
        fn expects_an_arrival(slot: PortSlot) -> usize {
            assert!(slot.arrival, "the declared target end is an arrival");
            assert_eq!((slot.arrivals, slot.departures), (1, 0));
            assert_eq!(slot.face, PhysicalSide::North);
            0
        }
        let build = |policy: PortPolicy| {
            let mut g = Graph::new();
            g.set_port_placer(expects_an_arrival);
            g.add_node(1usize, BoxedNode("Hub with room")); // 17 wide, center x+8
            g.add_node(2usize, "B");
            g.add_edge(1usize, 2usize, None);
            // Declared B → A, reversed by cycle breaking: its declared
            // target A becomes the layout source, and `Upstream` at A
            // is A's top face.
            g.add_edge(2usize, 1usize, None).to_port(PortSide::Upstream);
            assert!(g.set_node_port_policy(1usize, policy));
            g
        };
        let g = build(PortPolicy::Custom);
        let ir = g.compute_layout();
        let hub = ir.node_by_id(1).unwrap();
        let reversed = ir.edges().iter().find(|e| e.reversed).unwrap();
        assert_eq!(reversed.from_x, hub.x, "the placer's cell 0");
        let g = build(PortPolicy::Paired);
        let ir = g.compute_layout();
        let hub = ir.node_by_id(1).unwrap();
        let reversed = ir.edges().iter().find(|e| e.reversed).unwrap();
        assert_eq!(reversed.from_x, hub.x + 8, "the arrival port: the center");
    }
}

/// The CSR twin: declared sides travel through `to_csr` and the CSR
/// builder under the preallocation contract, and Auto-equivalent
/// declarations render identically to `Auto` on the arena backend —
/// which must also match the heap backend byte for byte.
#[cfg(all(
    test,
    feature = "std",
    feature = "ports",
    any(feature = "layout-vertical", feature = "layout-horizontal")
))]
mod csr_tests {
    use super::{Port, PortSide};
    use crate::LayoutConfig;
    use crate::graph::Graph;
    use crate::graph::arena::Arena;
    use crate::graph::csr::{CsrGraph, CsrGraphBuilder};
    use crate::render::engine::RenderOptions;

    fn render_csr(csr: &CsrGraph<'_>, config: &LayoutConfig<'_>) -> String {
        let mut temp = vec![0u8; 256 * 1024];
        let mut out = vec![0u8; 256 * 1024];
        let mut ta = Arena::new(&mut temp);
        let mut oa = Arena::new(&mut out);
        let ir = csr.compute_layout_arena(config, &mut ta, &mut oa).unwrap();
        let mut s = String::new();
        ir.render_with(&RenderOptions::plain(), &mut s).unwrap();
        s
    }

    fn heap_fixture() -> Graph<'static> {
        let mut g = Graph::new();
        g.add_node(1usize, "Start");
        g.add_node(2usize, "Middle");
        g.add_node(3usize, "Wide node");
        g.add_node(4usize, "End");
        g.add_edge(1usize, 2usize, Some("go"));
        g.add_edge(1usize, 3usize, None);
        g.add_edge(2usize, 4usize, None);
        g.add_edge(3usize, 4usize, None);
        g.add_edge(4usize, 1usize, Some("again")); // reversed
        g
    }

    /// `to_csr` copies declared sides; the arena layout honors them
    /// through the same seam; heap and arena stay byte-identical. One
    /// request per face (positions reach the arena backend with its
    /// scratch accounting, not here), on a forward edge and — mirrored
    /// — on the reversed one.
    #[test]
    fn to_csr_carries_declared_sides_with_backend_parity() {
        use crate::graph::Direction;
        let mut cases: Vec<(Direction, PortSide, PortSide)> = Vec::new();
        #[cfg(feature = "layout-vertical")]
        {
            cases.push((Direction::TopDown, PortSide::South, PortSide::North));
            cases.push((Direction::TopDown, PortSide::Downstream, PortSide::Upstream));
            cases.push((Direction::BottomUp, PortSide::North, PortSide::South));
            cases.push((
                Direction::BottomUp,
                PortSide::Downstream,
                PortSide::Upstream,
            ));
        }
        #[cfg(feature = "layout-horizontal")]
        {
            cases.push((Direction::LeftRight, PortSide::East, PortSide::West));
            cases.push((
                Direction::LeftRight,
                PortSide::Downstream,
                PortSide::Upstream,
            ));
            cases.push((Direction::RightLeft, PortSide::West, PortSide::East));
            cases.push((
                Direction::RightLeft,
                PortSide::Downstream,
                PortSide::Upstream,
            ));
        }
        for (direction, leave, arrive) in cases {
            let mut config = LayoutConfig::standard();
            config.direction = direction;
            let expected = heap_fixture()
                .compute_layout_with_config(&config)
                .render_string(&RenderOptions::plain());
            // (edge, declared sides): a forward edge, then the reversed
            // edge with its declared sides mirrored.
            for (edge, sides) in [(0usize, (leave, arrive)), (4usize, (arrive, leave))] {
                let mut declared = heap_fixture();
                assert!(declared.set_edge_ports(edge, sides.0, sides.1));
                let heap = declared
                    .compute_layout_with_config(&config)
                    .render_string(&RenderOptions::plain());
                assert_eq!(heap, expected, "{direction:?} edge {edge}: heap");
                let mut buf = vec![0u8; declared.estimate_csr_arena_size()];
                let mut arena = Arena::new(&mut buf);
                let csr = declared
                    .to_csr(&mut arena)
                    .expect("exact estimate fits, ports included");
                assert!(csr.has_ports());
                assert_eq!(csr.edge_ports(edge), sides, "declared sides survive to_csr");
                assert_eq!(
                    render_csr(&csr, &config),
                    expected,
                    "{direction:?} edge {edge}: arena"
                );
            }
        }
    }

    /// Several requests per face — the spread ACTIVE — position
    /// identically on both backends, from arenas sized EXACTLY by the
    /// estimates (the port scratch is counted), in every direction
    /// and both frames; and the drawing differs from Auto by design.
    #[test]
    fn positions_reach_the_arena_backend_with_exact_estimates() {
        use crate::graph::Direction;
        let mut cases: Vec<(Direction, PortSide, PortSide)> = Vec::new();
        #[cfg(feature = "layout-vertical")]
        {
            cases.push((Direction::TopDown, PortSide::South, PortSide::North));
            cases.push((
                Direction::BottomUp,
                PortSide::Downstream,
                PortSide::Upstream,
            ));
        }
        #[cfg(feature = "layout-horizontal")]
        {
            cases.push((Direction::LeftRight, PortSide::East, PortSide::West));
            cases.push((
                Direction::RightLeft,
                PortSide::Downstream,
                PortSide::Upstream,
            ));
        }
        for (direction, leave, arrive) in cases {
            let mut config = LayoutConfig::standard();
            config.direction = direction;
            let auto = heap_fixture()
                .compute_layout_with_config(&config)
                .render_string(&RenderOptions::plain());
            let mut declared = heap_fixture();
            for e in 0..4 {
                assert!(declared.set_edge_ports(e, leave, arrive));
            }
            assert!(declared.set_edge_ports(4, arrive, leave));
            let ir = declared.compute_layout_with_config(&config);
            let heap = ir.render_string(&RenderOptions::plain());
            // Two requests share node 1's leaving face. Where the face has
            // room (vertical profile: the node's WIDTH) they take distinct
            // cells and the drawing changes; under the horizontal profile
            // nodes are one row tall — capacity 1 — so they wrap onto the
            // single center cell, which is Auto's, by the round-robin rule.
            let starts: Vec<(usize, usize)> = ir
                .edges()
                .iter()
                .filter(|e| e.from_id == 1)
                .map(|e| (e.from_x, e.from_y))
                .collect();
            assert_eq!(starts.len(), 2);
            if starts[0] != starts[1] {
                assert_ne!(
                    heap, auto,
                    "{direction:?}: distinct cells change the drawing"
                );
            } else {
                assert_eq!(
                    heap, auto,
                    "{direction:?}: a capacity-1 face shares Auto's cell"
                );
            }

            let mut csr_buf = vec![0u8; declared.estimate_csr_arena_size()];
            let mut csr_arena = Arena::new(&mut csr_buf);
            let csr = declared.to_csr(&mut csr_arena).expect("exact CSR estimate");
            let bytes = declared.estimate_layout_arena_size_with(&config);
            let mut temp = vec![0u8; bytes];
            let mut out = vec![0u8; bytes];
            let mut ta = Arena::new(&mut temp);
            let mut oa = Arena::new(&mut out);
            let ir = csr
                .compute_layout_arena(&config, &mut ta, &mut oa)
                .expect("exact layout estimate covers the port scratch");
            let mut arena_render = String::new();
            ir.render_with(&RenderOptions::plain(), &mut arena_render)
                .unwrap();
            assert_eq!(
                arena_render, heap,
                "{direction:?}: arena positions match heap"
            );
        }
    }

    /// An undeclared graph converts without a table and the estimate
    /// does not grow; a declared one grows by exactly the table.
    /// Policies travel through `to_csr` (node codes, the graph's code
    /// and its one placer) and through the builder's setters, and the
    /// arena backend places exactly as the heap backend does.
    #[cfg(feature = "layout-vertical")]
    #[test]
    fn policies_travel_to_the_arena_backend() {
        use crate::render::engine::BoxedNode;
        use crate::{PortBound, PortPolicy, PortSlot};
        fn ends_apart(slot: PortSlot) -> usize {
            if slot.arrival { 0 } else { slot.cells - 1 }
        }
        let mut g = Graph::new();
        g.set_port_policy(PortPolicy::Spread(PortBound::Ports(2)));
        g.add_node(1usize, BoxedNode("Spread"));
        g.add_node(2usize, BoxedNode("Paired"));
        g.add_node(3usize, BoxedNode("Custom"));
        g.set_node_port_policy(2usize, PortPolicy::Paired);
        assert!(
            !g.set_node_port_policy(3usize, PortPolicy::Custom),
            "no placer registered yet"
        );
        g.set_port_placer(ends_apart);
        assert!(g.set_node_port_policy(3usize, PortPolicy::Custom));
        // A bound past what the byte code carries saturates on both.
        g.add_node(4usize, BoxedNode("Saturated bound node"));
        g.set_node_port_policy(4usize, PortPolicy::Spread(PortBound::Ports(255)));
        for hub in 1usize..=4 {
            let (a, b, c) = (hub * 10, hub * 10 + 1, hub * 10 + 2);
            g.add_node(a, "In");
            g.add_node(b, "In");
            g.add_node(c, "Out");
            g.add_edge(a, hub, None).to_port(PortSide::Upstream);
            g.add_edge(b, hub, None).to_port(PortSide::Upstream);
            g.add_edge(hub, c, None).from_port(PortSide::Upstream);
        }
        let config = LayoutConfig::standard();
        let heap = g.compute_layout().render_string(&RenderOptions::plain());
        let mut buf = vec![0u8; g.estimate_csr_arena_size()];
        let mut arena = Arena::new(&mut buf);
        let csr = g.to_csr(&mut arena).unwrap();
        assert_eq!(
            render_csr(&csr, &config),
            heap,
            "to_csr carries the policies"
        );
        // The same graph on the builder.
        let nodes = g.nodes.len();
        let edges = g.edges.len();
        let need = CsrGraph::required_arena_size_with_ports(nodes, edges, 160, 0, 0);
        let mut buf = vec![0u8; need];
        let mut arena = Arena::new(&mut buf);
        let mut b = CsrGraphBuilder::new_with_ports(&mut arena, nodes, edges, 160, 0, 0).unwrap();
        assert!(
            b.set_port_policy(PortPolicy::Spread(PortBound::Ports(2)))
                .is_some()
        );
        let mut index = std::collections::HashMap::new();
        for &(id, label) in &g.nodes {
            let boxed = id <= 4;
            let idx = if boxed {
                b.add_node(id, BoxedNode(label)).unwrap()
            } else {
                b.add_node(id, label).unwrap()
            };
            index.insert(id, idx);
        }
        assert!(
            b.set_node_port_policy(index[&2], PortPolicy::Paired)
                .is_some()
        );
        assert!(
            b.set_node_port_policy(index[&3], PortPolicy::Custom)
                .is_none(),
            "no placer registered yet"
        );
        assert!(b.set_port_placer(ends_apart).is_some());
        assert!(
            b.set_node_port_policy(index[&3], PortPolicy::Custom)
                .is_some()
        );
        assert!(
            b.set_node_port_policy(index[&4], PortPolicy::Spread(PortBound::Ports(255)))
                .is_some()
        );
        assert!(
            b.set_node_port_policy(index[&1], PortPolicy::Paired)
                .is_some()
        );
        assert!(
            b.clear_node_port_policy(index[&1]).is_some(),
            "back to the graph's policy"
        );
        assert!(b.clear_node_port_policy(nodes).is_none(), "unknown node");
        assert!(
            b.set_node_port_policy(nodes, PortPolicy::Paired).is_none(),
            "unknown node"
        );
        for (ei, &(from, to, _)) in g.edges.iter().enumerate() {
            let (src, dst) = g.edge_ports[ei];
            b.add_edge(index[&from], index[&to])
                .unwrap()
                .from_port(src)
                .unwrap()
                .to_port(dst)
                .unwrap();
        }
        let csr = b.build().unwrap();
        assert_eq!(
            render_csr(&csr, &config),
            heap,
            "the builder carries the policies"
        );
        // A builder without a port table refuses.
        let mut buf = vec![0u8; CsrGraph::required_arena_size(4, 4, 16)];
        let mut arena = Arena::new(&mut buf);
        let mut plain = CsrGraphBuilder::new(&mut arena, 4, 4, 16, 0).unwrap();
        plain.add_node(1, "A").unwrap();
        assert!(plain.set_port_policy(PortPolicy::Paired).is_none());
        assert!(plain.set_node_port_policy(0, PortPolicy::Paired).is_none());
        assert!(plain.set_port_placer(ends_apart).is_none());
    }

    /// The no-alloc pipeline reports the same port conditions as the
    /// heap run, through `compute_layout_arena_reporting`: a side on a
    /// self-loop before the layout, an unroutable side after it.
    #[cfg(feature = "layout-vertical")]
    #[test]
    fn the_arena_entry_reports_port_conditions() {
        use crate::diagnostics::{DiagnosticKind, DiagnosticRun, VecDiagnostics};
        let mut g = Graph::new();
        g.add_node(0usize, "Root");
        for (id, label) in [(1usize, "L"), (2, "M"), (3, "R")] {
            g.add_node(id, label);
            g.add_edge(0usize, id, None);
        }
        g.add_node(4usize, "T");
        g.add_edge(2usize, 4usize, None).from_port(PortSide::East);
        g.add_edge(4usize, 4usize, None).from_port(PortSide::East);
        let tight = LayoutConfig {
            node_spacing: 0,
            ..LayoutConfig::standard()
        };
        let heap: Vec<DiagnosticKind> = g
            .layout()
            .with_config(&tight)
            .reported()
            .warnings()
            .map(|d| *d.kind())
            .collect();
        assert_eq!(heap.len(), 2, "{heap:?}");
        let mut buf = vec![0u8; g.estimate_csr_arena_size()];
        let mut arena = Arena::new(&mut buf);
        let csr = g.to_csr(&mut arena).unwrap();
        let size = g.estimate_layout_arena_size_with(&tight);
        let (mut t, mut o) = (vec![0u8; size], vec![0u8; size]);
        let (mut ta, mut oa) = (Arena::new(&mut t), Arena::new(&mut o));
        let mut run = DiagnosticRun::new(VecDiagnostics::default());
        let ir = {
            let mut cx = run.context();
            csr.compute_layout_arena_reporting(&tight, &mut ta, &mut oa, &mut cx)
                .unwrap()
        };
        let report = run.finish(Ok::<_, crate::errors::GraphError>(ir));
        let arena_kinds: Vec<DiagnosticKind> = report.warnings().map(|d| *d.kind()).collect();
        assert_eq!(arena_kinds, heap, "same conditions, same order");
        assert!(matches!(
            arena_kinds[0],
            DiagnosticKind::PortIgnoredOnSelfLoop { edge: 4 }
        ));
        assert!(matches!(
            arena_kinds[1],
            DiagnosticKind::PortUnroutable { edge: 3, .. }
        ));
    }

    #[test]
    fn undeclared_graphs_carry_no_csr_table() {
        let auto = heap_fixture();
        let mut declared = heap_fixture();
        assert!(declared.set_edge_ports(0, PortSide::South, PortSide::North));
        assert_eq!(
            declared.estimate_csr_arena_size(),
            auto.estimate_csr_arena_size() + 5 * 2 + auto.nodes.len() + 16,
            "two bytes per edge, one policy byte per node, plus slack — only when declared"
        );
        let mut buf = vec![0u8; auto.estimate_csr_arena_size()];
        let mut arena = Arena::new(&mut buf);
        let csr = auto.to_csr(&mut arena).unwrap();
        assert!(!csr.has_ports());
        assert_eq!(csr.edge_ports(0), (PortSide::Auto, PortSide::Auto));
        // Declaring `Auto` explicitly is what an undeclared edge already
        // means — it materializes nothing.
        assert!(auto.edge_ports.is_empty(), "explicit Auto costs nothing");
    }

    /// The exact with-ports estimate holds with EVERY capacity nonzero
    /// — nodes, edges, labels, a subgraph, custom content, and the
    /// port table — and the graph lays out and renders from it.
    #[test]
    fn exact_with_ports_estimate_covers_subgraphs_and_custom_content() {
        use crate::render::engine::CustomNode;
        // Labels: "A" + "B" + "C" + payload "p" + subgraph "S" = 5 bytes.
        let size = CsrGraph::required_arena_size_with_ports(3, 2, 5, 1, 1);
        let mut buf = vec![0u8; size];
        let mut arena = Arena::new(&mut buf);
        let mut b = CsrGraphBuilder::new_with_ports(&mut arena, 3, 2, 5, 1, 1)
            .expect("exact estimate with every capacity nonzero");
        b.add_node(1, "A").unwrap();
        b.add_node(2, "B").unwrap();
        b.add_node(
            3,
            CustomNode {
                label: "C",
                width: 3,
                height: 1,
                painter: None,
                payload: "p",
            },
        )
        .unwrap();
        let sg = b.add_subgraph(9, "S").unwrap();
        b.set_node_subgraph(0, sg).unwrap();
        b.set_node_subgraph(1, sg).unwrap();
        b.add_edge(0, 1)
            .unwrap()
            .from_port(PortSide::South)
            .unwrap()
            .to_port(PortSide::North)
            .unwrap();
        b.add_edge(1, 2)
            .unwrap()
            .from_port(PortSide::Downstream)
            .unwrap();
        let csr = b.build().expect("builds within the exact estimate");
        assert!(csr.has_ports());
        assert_eq!(csr.edge_ports(1), (PortSide::Downstream, PortSide::Auto));
        let out = render_csr(&csr, &LayoutConfig::standard());
        // A painterless custom node reserves its area without painting
        // it, so `C` is (correctly) absent; the boxed label and the
        // cluster prove the render went through.
        assert!(
            out.contains("[A]") && out.contains("[B]") && out.contains('S'),
            "renders: {out}"
        );
    }

    /// The builder contract: a with-ports builder's setters never fail
    /// and never allocate (the exact `_with_ports` estimate fits); a
    /// builder without the table reports `None` — never a silent
    /// discard.
    #[test]
    fn csr_builder_handle_follows_the_preallocation_contract() {
        let size = CsrGraph::required_arena_size_with_ports(3, 3, 16, 0, 0);
        let mut buf = vec![0u8; size];
        let mut arena = Arena::new(&mut buf);
        let mut b = CsrGraphBuilder::new_with_ports(&mut arena, 3, 3, 16, 0, 0)
            .expect("exact with-ports estimate");
        b.add_node(1, "A").unwrap();
        b.add_node(2, "B").unwrap();
        b.add_node(3, "C").unwrap();
        // Flow-relative names for Auto's own faces — Auto-equivalent
        // under every direction this build lays out in.
        let h = b
            .add_edge(0, 1)
            .unwrap()
            .from_port(PortSide::Downstream)
            .expect("preallocated")
            .to_port(Port::of(PortSide::Upstream))
            .expect("preallocated");
        assert_eq!(h.edge(), 0);
        b.add_edge(1, 2).unwrap();
        assert!(
            b.set_edge_ports(1, PortSide::Downstream, PortSide::Upstream)
                .is_some()
        );
        assert!(
            b.set_edge_ports(7, PortSide::Auto, PortSide::Auto)
                .is_none(),
            "unknown edge"
        );
        let csr = b.build().unwrap();
        assert!(csr.has_ports());
        assert_eq!(
            csr.edge_ports(0),
            (PortSide::Downstream, PortSide::Upstream)
        );
        assert_eq!(
            csr.edge_ports(1),
            (PortSide::Downstream, PortSide::Upstream)
        );
        assert_eq!(
            csr.edge_ports(2),
            (PortSide::Auto, PortSide::Auto),
            "beyond the edges: Auto"
        );

        // Auto-equivalent declarations render exactly like Auto.
        let mut plain_buf = vec![0u8; CsrGraph::required_arena_size(3, 3, 16)];
        let mut plain_arena = Arena::new(&mut plain_buf);
        let mut p = CsrGraphBuilder::new(&mut plain_arena, 3, 3, 16, 0).unwrap();
        p.add_node(1, "A").unwrap();
        p.add_node(2, "B").unwrap();
        p.add_node(3, "C").unwrap();
        let h = p.add_edge(0, 1).unwrap();
        assert!(
            h.from_port(PortSide::Downstream).is_none(),
            "no table: refused, not discarded"
        );
        p.add_edge(1, 2).unwrap();
        assert!(
            p.set_edge_ports(1, PortSide::Auto, PortSide::Auto)
                .is_none()
        );
        let plain = p.build().unwrap();
        assert!(!plain.has_ports());
        let config = LayoutConfig::standard();
        assert_eq!(render_csr(&csr, &config), render_csr(&plain, &config));
    }
}

/// Whether an explicit side takes an end OFF its layout role's own
/// face: onto the opposite level face (a source leaving through its
/// arrive face, a target arriving through its leave face — routed
/// around the node) or onto a lateral face (a stub beside the node,
/// then the turn onto the flow axis). Such an end gets a lane and an
/// explicit path instead of attaching head-on.
#[cfg_attr(not(feature = "ports"), allow(dead_code))]
pub(crate) const fn detours(
    side: PortSide,
    axis: crate::ir::FlowAxis,
    level_flipped: bool,
    role: EndRole,
) -> bool {
    if matches!(side, PortSide::Auto) {
        return false;
    }
    let face = Face::of(side, axis, level_flipped, role);
    let opposite = match role {
        EndRole::Source => Face::LevelLeading,
        EndRole::Target => Face::LevelTrailing,
    };
    !face.is_level()
        || matches!(
            (face, opposite),
            (Face::LevelLeading, Face::LevelLeading) | (Face::LevelTrailing, Face::LevelTrailing)
        )
}

/// The lane a LATERAL end's stub turns onto: the cell just past the
/// node on that face's side — no fallback to the other side, the face
/// was named. `usize::MAX` when that cell is blocked (a neighbor at
/// `node_spacing == 0`, a dummy chain, a marker cell) or off the
/// canvas; the end then attaches head-on.
#[cfg_attr(not(feature = "ports"), allow(dead_code))]
pub(crate) fn lateral_lane(
    base: usize,
    extent: usize,
    face: Face,
    cross_limit: usize,
    blocked: &dyn Fn(usize) -> bool,
) -> usize {
    let lane = if matches!(face, Face::CrossTrailing) {
        base + extent
    } else {
        match base.checked_sub(1) {
            Some(b) => b,
            None => return usize::MAX,
        }
    };
    if lane < cross_limit && !blocked(lane) {
        lane
    } else {
        usize::MAX
    }
}

/// The cross-axis lane a detour runs along beside its node: the cell
/// just past the node on the side facing the peer, falling back to
/// the other side; `usize::MAX` when neither is free. `after` sits one
/// further out on a self-loop node (the `↺` cell is at the node's
/// trailing edge on its leading line). Lanes live in the packing gap
/// — nothing is reserved — so a caller-blocked column (a neighbor at
/// `node_spacing == 0`, a dummy chain, a marker cell) or the canvas
/// edge means no lane on that side. Both backends decide this way.
#[cfg_attr(not(feature = "ports"), allow(dead_code))]
pub(crate) fn choose_lane(
    base: usize,
    extent: usize,
    self_loop: bool,
    toward_after: bool,
    cross_limit: usize,
    blocked: &dyn Fn(usize) -> bool,
) -> usize {
    let after = base + extent + usize::from(self_loop);
    let after_ok = after < cross_limit && !blocked(after);
    let before = base.checked_sub(1).filter(|&b| !blocked(b));
    match (toward_after, after_ok, before) {
        (true, true, _) | (false, true, None) => after,
        (_, _, Some(b)) => b,
        _ => usize::MAX,
    }
}

/// The flip and axis a direction lays out under — the same facts
/// `level_flipped::<A>` and `A::FLOW_AXIS` give the layout, for callers
/// that are not generic over the profile (the size estimate).
#[cfg_attr(not(all(feature = "ports", feature = "alloc")), allow(dead_code))]
pub(crate) const fn frame(direction: Direction) -> (FlowAxis, bool) {
    match direction {
        #[cfg(feature = "layout-vertical")]
        Direction::TopDown => (FlowAxis::Y, false),
        #[cfg(feature = "layout-vertical")]
        Direction::BottomUp => (FlowAxis::Y, true),
        #[cfg(feature = "layout-horizontal")]
        Direction::LeftRight => (FlowAxis::X, false),
        #[cfg(feature = "layout-horizontal")]
        Direction::RightLeft => (FlowAxis::X, true),
    }
}

/// Sizes of the detour scratch a layout carves — from the same face
/// decisions the layout makes, so an estimate on either graph type
/// sizes the CSR layout exactly. All zero when no end detours: a
/// declared port that lands on its role's own face costs nothing here.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(not(feature = "ports"), allow(dead_code))]
pub(crate) struct DetourBudget {
    /// Edges with at least one detouring end: plans, slot intervals,
    /// staged bends.
    pub(crate) edges: usize,
    /// Lane blockers on those nodes' levels: node spans, dummy columns,
    /// self-loop marker cells.
    pub(crate) blockers: usize,
    /// Stored bends the explicit polylines can need in the IR output:
    /// six per detouring edge plus two per dummy on it.
    pub(crate) points: usize,
}

impl DetourBudget {
    #[cfg_attr(not(feature = "ports"), allow(dead_code))]
    pub(crate) const NONE: DetourBudget = DetourBudget {
        edges: 0,
        blockers: 0,
        points: 0,
    };

    /// Whether any end detours at all.
    #[cfg_attr(not(feature = "ports"), allow(dead_code))]
    pub(crate) const fn any(&self) -> bool {
        self.edges > 0
    }
}

/// Compute the [`DetourBudget`] from resolved layout facts. `edge`
/// yields DECLARED endpoint indices (`usize::MAX` for an unresolvable
/// one), `sides` the declared `(from, to)` sides, `is_back` the cycle
/// reversal, `level` a node's level; `level_real`/`level_dummy` count
/// the vnodes per level. `node_marks` and `level_marks` are caller
/// scratch, all `false` on entry: on return they mark the detouring
/// nodes and their levels (the layout builds its sparse tables from
/// them). O(E + N) and allocation-free.
#[allow(clippy::too_many_arguments)]
#[cfg_attr(not(feature = "ports"), allow(dead_code))]
pub(crate) fn detour_budget(
    edge_count: usize,
    edge: &dyn Fn(usize) -> (usize, usize),
    sides: &dyn Fn(usize) -> (PortSide, PortSide),
    is_back: &dyn Fn(usize) -> bool,
    level: &dyn Fn(usize) -> usize,
    axis: FlowAxis,
    flipped: bool,
    level_real: &[usize],
    level_dummy: &[usize],
    node_marks: &mut [bool],
    level_marks: &mut [bool],
) -> DetourBudget {
    let mut budget = DetourBudget::NONE;
    for ei in 0..edge_count {
        let (f, t) = edge(ei);
        if f == t || f == usize::MAX || t == usize::MAX {
            continue;
        }
        let (src_side, dst_side) = sides(ei);
        let (src, dst, src_side, dst_side) = if is_back(ei) {
            (t, f, dst_side, src_side)
        } else {
            (f, t, src_side, dst_side)
        };
        let sd = detours(src_side, axis, flipped, EndRole::Source);
        let dd = detours(dst_side, axis, flipped, EndRole::Target);
        if !(sd || dd) {
            continue;
        }
        budget.edges += 1;
        let span = level(src).abs_diff(level(dst));
        budget.points += 6 + 2 * span.saturating_sub(1);
        if sd && src < node_marks.len() {
            node_marks[src] = true;
        }
        if dd && dst < node_marks.len() {
            node_marks[dst] = true;
        }
    }
    for (n, &marked) in node_marks.iter().enumerate() {
        if marked {
            let l = level(n);
            if l < level_marks.len() {
                level_marks[l] = true;
            }
        }
    }
    for ei in 0..edge_count {
        let (f, t) = edge(ei);
        if f == usize::MAX || t == usize::MAX {
            continue;
        }
        if f == t {
            // A self-loop marker cell blocks a lane on its level.
            let l = level(f);
            if l < level_marks.len() && level_marks[l] {
                budget.blockers += 1;
            }
        }
    }
    for (l, &marked) in level_marks.iter().enumerate() {
        if marked {
            budget.blockers +=
                level_real.get(l).copied().unwrap_or(0) + level_dummy.get(l).copied().unwrap_or(0);
        }
    }
    budget
}

/// The plan of edge `ei` in a table sorted by edge index, if any.
/// Inlined with an empty-table fast path: the arena's per-edge loops
/// ask for every edge, and a port-free layout has no table.
#[cfg_attr(not(feature = "ports"), allow(dead_code))]
#[inline]
pub(crate) fn plan_lookup(plans: &[(usize, Detour)], ei: usize) -> Option<Detour> {
    if plans.is_empty() {
        return None;
    }
    plans
        .binary_search_by_key(&ei, |p| p.0)
        .ok()
        .map(|i| plans[i].1)
}

/// One edge's detour plan: the lane beside each detouring end
/// (`usize::MAX` = that end attaches head-on) and the routing-row
/// slots its runs were allocated — the up-run above the source
/// (`up_slot`: in the band above the source's level, or the rows above
/// level 0), the first run below the source (`first_slot`, absent when
/// the descent starts in the lane's own column), and the run under
/// the target (`below_slot`).
#[derive(Clone, Copy, Debug)]
#[cfg_attr(not(feature = "ports"), allow(dead_code))]
pub(crate) struct Detour {
    /// The declared side takes this end off its role's own face.
    pub(crate) src_wants: bool,
    pub(crate) dst_wants: bool,
    /// The resolved face of each end — a LEVEL face routes around the
    /// node (up-run or below-run), a CROSS face is a lateral stub at
    /// the node's own row.
    pub(crate) src_face: Face,
    pub(crate) dst_face: Face,
    pub(crate) src_lane: usize,
    pub(crate) dst_lane: usize,
    pub(crate) up_slot: usize,
    pub(crate) first_slot: usize,
    pub(crate) below_slot: usize,
}

impl Detour {
    #[cfg_attr(not(feature = "ports"), allow(dead_code))]
    pub(crate) const NONE: Detour = Detour {
        src_wants: false,
        dst_wants: false,
        src_face: Face::LevelLeading,
        dst_face: Face::LevelLeading,
        src_lane: usize::MAX,
        dst_lane: usize::MAX,
        up_slot: usize::MAX,
        first_slot: usize::MAX,
        below_slot: usize::MAX,
    };

    #[cfg_attr(not(feature = "ports"), allow(dead_code))]
    pub(crate) const fn active(&self) -> bool {
        self.src_lane != usize::MAX || self.dst_lane != usize::MAX
    }
}

/// Opposite-face detours through the heap layout:
/// an explicit side on the level face opposite the layout role routes
/// AROUND the node — up out of a TopDown source's top face, or in
/// through a target's bottom face — as an explicit polyline.
#[cfg(all(
    test,
    feature = "std",
    feature = "ports",
    any(feature = "layout-vertical", feature = "layout-horizontal")
))]
mod detour_tests {
    use super::{PortBound, PortPolicy, PortSide};
    use crate::graph::{Direction, Graph};
    use crate::ir::{EdgePath, LayoutIR};
    use crate::render::engine::RenderOptions;
    use crate::render::engine::plan::{HitResult, RenderPlan};

    const EDGE_INK: &str = "│─┌┐└┘├┤┬┴┼↓↑←→⇣⇡⇠⇢┆┄";

    fn cell_at(out: &str, x: usize, y: usize) -> char {
        out.lines()
            .nth(y)
            .and_then(|l| l.chars().nth(x))
            .unwrap_or(' ')
    }

    fn node_rect(ir: &LayoutIR<'_>, id: usize) -> (usize, usize, usize, usize) {
        let n = ir.node_by_id(id).unwrap();
        (n.x, n.y, n.width, n.height)
    }

    /// Every bend stays outside every node, and every inked
    /// cell outside the nodes belongs to an edge — hit-testing, which
    /// walks the plan's visitors, agrees with the painter.
    fn assert_routing_invariants(ir: &LayoutIR<'_>, tag: &str) {
        let out = ir.render_string(&RenderOptions::plain());
        let plan = RenderPlan::build(ir, &RenderOptions::plain().plan);
        let rects: Vec<(usize, usize, usize, usize)> = ir
            .nodes()
            .iter()
            .map(|n| (n.x, n.y, n.width, n.height))
            .collect();
        // A node's body must carry no edge ink. A boxed or custom node
        // paints its own border with box glyphs, so for a multi-row
        // node only the interior counts.
        let in_rect = |x: usize, y: usize| {
            rects
                .iter()
                .any(|&(nx, ny, w, h)| (nx..nx + w).contains(&x) && (ny..ny + h).contains(&y))
        };
        let inside = |x: usize, y: usize| {
            rects.iter().any(|&(nx, ny, w, h)| {
                if h > 1 {
                    (nx + 1..nx + w.saturating_sub(1)).contains(&x)
                        && (ny + 1..ny + h.saturating_sub(1)).contains(&y)
                } else {
                    (nx..nx + w).contains(&x) && (ny..ny + h).contains(&y)
                }
            })
        };
        let loop_cells: Vec<(usize, usize)> =
            ir.nodes().iter().filter_map(|n| n.self_loop_at).collect();
        // Two edges share an overlapping horizontal run on one row only
        // when they share a layout end: a fan-out bus (same source — a
        // reversed edge's included, it is drawn from its layout source)
        // or a fan-in into one face cell (same target) — or when the
        // overlap is exactly one cell that is an endpoint of both, a
        // port two ends share under the node's policy. Anything else
        // merges two unrelated edges into one unreadable line.
        type Run = (usize, usize, usize, (usize, usize), [(usize, usize); 2]);
        let mut runs: Vec<Run> = Vec::new();
        for (i, e) in ir.edges().iter().enumerate() {
            let v = crate::render::engine::view::LayoutView::edge(ir, i);
            crate::render::engine::plan::for_each_h_run_all(
                &v.path,
                v.from_x,
                v.from_y,
                v.to_x,
                v.to_y,
                v.flow_axis,
                &mut |row, a, b| {
                    let ends = if e.reversed {
                        (e.to_id, e.from_id)
                    } else {
                        (e.from_id, e.to_id)
                    };
                    let cells = [(e.from_x, e.from_y), (e.to_x, e.to_y)];
                    runs.push((row, a, b, ends, cells))
                },
            );
        }
        for (i, &(row, a0, a1, ends_a, cells_a)) in runs.iter().enumerate() {
            for &(row_b, b0, b1, ends_b, cells_b) in &runs[i + 1..] {
                let shared_end = ends_a.0 == ends_b.0 || ends_a.1 == ends_b.1;
                // Two ends on one port — an endpoint cell both edges
                // own, on this row or the row a side face's stub runs
                // on beside it — run together from that cell until they
                // part; that is what sharing a port draws.
                let (lo, hi) = (a0.max(b0), a1.min(b1));
                let shared_port = cells_a.iter().any(|c| {
                    cells_b.contains(c)
                        && c.1.abs_diff(row) <= 1
                        && (lo.saturating_sub(1)..=hi + 1).contains(&c.0)
                });
                assert!(
                    row != row_b || shared_end || a1 < b0 || b1 < a0 || shared_port,
                    "{tag}: runs of different sources overlap on row {row}: [{a0}, {a1}] vs [{b0}, {b1}] (edges {ends_a:?} at {cells_a:?} vs {ends_b:?} at {cells_b:?}):\n{out}"
                );
            }
        }
        for y in 0..ir.height() {
            for x in 0..ir.width() {
                let ch = cell_at(&out, x, y);
                if in_rect(x, y) {
                    assert!(
                        !inside(x, y) || !EDGE_INK.contains(ch),
                        "{tag}: edge ink {ch:?} inside a node at ({x}, {y}):\n{out}"
                    );
                    continue;
                }
                if loop_cells.contains(&(x, y)) {
                    continue;
                }
                let inked = ch != ' ';
                let owned = matches!(plan.element_at(ir, x, y), HitResult::Edge(_));
                assert_eq!(
                    inked, owned,
                    "{tag}: ({x}, {y}) {ch:?} inked={inked} owned={owned}:\n{out}"
                );
            }
        }
    }

    /// The bends of the routed edge with INPUT index `edge` (self-loops
    /// never enter the routed list, so positions and input indices
    /// can differ).
    fn bends(ir: &LayoutIR<'_>, edge: usize) -> Vec<(usize, usize)> {
        let e = ir
            .edges()
            .iter()
            .find(|e| e.edge_index == edge)
            .unwrap_or_else(|| panic!("edge {edge} is not routed"));
        match &e.path {
            EdgePath::Orthogonal { bends } => bends.clone(),
            other => panic!("edge {edge}: expected an explicit polyline, got {other:?}"),
        }
    }

    #[cfg(feature = "layout-vertical")]
    #[test]
    fn upstream_source_leaves_through_its_arrive_face() {
        let mut auto = Graph::new();
        auto.add_node(1usize, "A");
        auto.add_node(2usize, "B");
        auto.add_edge(1usize, 2usize, None);
        let plain = auto.compute_layout();

        let mut g = Graph::new();
        g.add_node(1usize, "A");
        g.add_node(2usize, "B");
        g.add_edge(1usize, 2usize, None)
            .from_port(PortSide::Upstream);
        let ir = g.compute_layout();
        let out = ir.render_string(&RenderOptions::plain());
        let (ax, ay, aw, _) = node_rect(&ir, 1);
        let (_, by, _, _) = node_rect(&ir, 2);
        let b = bends(&ir, 0);
        // Up out of the top face, around, down into B: four turns, the
        // first above A, the last between the nodes.
        assert_eq!(b.len(), 4, "{b:?}\n{out}");
        assert!(b[0].1 < ay && b[1].1 < ay, "{b:?}\n{out}");
        assert!(b[2].1 > ay && b[3].1 > ay && b[3].1 < by, "{b:?}\n{out}");
        // The lane runs beside A, never through it.
        assert!(b[1].0 < ax || b[1].0 >= ax + aw, "{b:?}\n{out}");
        let e = &ir.edges()[0];
        assert_eq!(e.from_y, ay, "the endpoint is A's own top line");
        assert_eq!(cell_at(&out, e.from_x, ay - 1), '│', "{out}");
        assert_eq!(cell_at(&out, e.to_x, by - 1), '↓', "{out}");
        // Two rows were opened above the first level: one slot row and
        // the clearance line.
        assert_eq!(ir.height(), plain.height() + 2, "{out}");
        assert_routing_invariants(&ir, "upstream source");
    }

    #[cfg(feature = "layout-vertical")]
    #[test]
    fn downstream_target_arrives_through_its_leave_face() {
        let mut g = Graph::new();
        g.add_node(1usize, "A");
        g.add_node(2usize, "B");
        g.add_edge(1usize, 2usize, None)
            .to_port(PortSide::Downstream);
        let ir = g.compute_layout();
        let out = ir.render_string(&RenderOptions::plain());
        let (bx, by, bw, bh) = node_rect(&ir, 2);
        let b = bends(&ir, 0);
        let bottom = by + bh - 1;
        assert_eq!(b.len(), 4, "{b:?}\n{out}");
        assert!(b[0].1 < by && b[1].1 < by, "{b:?}\n{out}");
        assert!(b[2].1 > bottom && b[3].1 > bottom, "{b:?}\n{out}");
        assert!(b[1].0 < bx || b[1].0 >= bx + bw, "{b:?}\n{out}");
        let e = &ir.edges()[0];
        assert_eq!(e.to_y, bottom, "the endpoint is B's own bottom line");
        assert_eq!(cell_at(&out, e.to_x, bottom + 1), '↑', "{out}");
        assert_routing_invariants(&ir, "downstream target");
    }

    /// In every direction the exit is on the face AGAINST the flow and
    /// the arrival on the face WITH it — physically wherever the
    /// direction puts them.
    #[test]
    fn detours_follow_the_flow_in_every_direction() {
        let mut directions = Vec::new();
        #[cfg(feature = "layout-vertical")]
        directions.extend([Direction::TopDown, Direction::BottomUp]);
        #[cfg(feature = "layout-horizontal")]
        directions.extend([Direction::LeftRight, Direction::RightLeft]);
        for direction in directions {
            let mut g = Graph::new();
            g.set_direction(direction);
            g.add_node(1usize, "A");
            g.add_node(2usize, "B");
            g.add_edge(1usize, 2usize, None)
                .from_port(PortSide::Upstream)
                .to_port(PortSide::Downstream);
            let ir = g.compute_layout();
            let out = ir.render_string(&RenderOptions::plain());
            let (ax, ay, aw, ah) = node_rect(&ir, 1);
            let (bx, by, bw, bh) = node_rect(&ir, 2);
            let b = bends(&ir, 0);
            let (first, last) = (b[0], b[b.len() - 1]);
            let (exit_ok, entry_ok) = match direction {
                #[cfg(feature = "layout-vertical")]
                Direction::TopDown => (first.1 < ay, last.1 > by + bh - 1),
                #[cfg(feature = "layout-vertical")]
                Direction::BottomUp => (first.1 > ay + ah - 1, last.1 < by),
                #[cfg(feature = "layout-horizontal")]
                Direction::LeftRight => (first.0 < ax, last.0 > bx + bw - 1),
                #[cfg(feature = "layout-horizontal")]
                Direction::RightLeft => (first.0 > ax + aw - 1, last.0 < bx),
            };
            assert!(
                exit_ok,
                "{direction:?}: exit {first:?} vs A {:?}\n{out}",
                (ax, ay, aw, ah)
            );
            assert!(
                entry_ok,
                "{direction:?}: entry {last:?} vs B {:?}\n{out}",
                (bx, by, bw, bh)
            );
            assert_routing_invariants(&ir, "direction sweep");
        }
    }

    #[cfg(feature = "layout-vertical")]
    #[test]
    fn skip_edges_detour_and_thread_their_dummy_column() {
        let mut g = Graph::new();
        g.add_node(1usize, "A");
        g.add_node(2usize, "B");
        g.add_node(3usize, "C");
        g.add_edge(1usize, 2usize, None);
        g.add_edge(2usize, 3usize, None);
        g.add_edge(1usize, 3usize, None)
            .from_port(PortSide::Upstream);
        let ir = g.compute_layout();
        let out = ir.render_string(&RenderOptions::plain());
        let b = bends(&ir, 2);
        let (_, ay, _, _) = node_rect(&ir, 1);
        let (_, cy, _, _) = node_rect(&ir, 3);
        assert!(b.len() >= 4, "{b:?}\n{out}");
        assert!(b[0].1 < ay, "{b:?}\n{out}");
        assert!(b.iter().all(|&(_, y)| y < cy), "{b:?}\n{out}");
        assert_routing_invariants(&ir, "skip edge");
    }

    /// A reversed cycle edge binds its declared sides to its declared
    /// ends: `Downstream` on the declared source (the layout TARGET)
    /// is that node's leave face — opposite for a target — so it
    /// detours; `Upstream` there is the arrive face, head-on.
    #[cfg(feature = "layout-vertical")]
    #[test]
    fn a_reversed_edge_detours_from_its_declared_end() {
        for (side, detours) in [(PortSide::Downstream, true), (PortSide::Upstream, false)] {
            let mut g = Graph::new();
            g.add_node(1usize, "A");
            g.add_node(2usize, "B");
            g.add_edge(1usize, 2usize, None);
            g.add_edge(2usize, 1usize, None).from_port(side);
            let ir = g.compute_layout();
            let out = ir.render_string(&RenderOptions::plain());
            let e = &ir.edges()[1];
            assert!(e.reversed, "{out}");
            let explicit = matches!(e.path, EdgePath::Orthogonal { .. });
            assert_eq!(explicit, detours, "{side:?}: {:?}\n{out}", e.path);
            if detours {
                let (_, by, _, bh) = node_rect(&ir, 2);
                let b = bends(&ir, 1);
                assert!(b[b.len() - 1].1 > by + bh - 1, "{b:?}\n{out}");
            }
            assert_routing_invariants(&ir, "reversed");
        }
    }

    /// Spread ports on a skip edge's target: the chain's last jog is
    /// budgeted against the RESOLVED port, not the node center — the
    /// bend to a spread cell gets its row like any other jog.
    #[cfg(feature = "layout-vertical")]
    #[test]
    fn spread_ports_on_skip_edges_keep_a_budgeted_jog_row() {
        use crate::render::engine::BoxedNode;
        let mut g = Graph::new();
        g.set_port_policy(PortPolicy::Spread(PortBound::Face));
        g.add_node(1usize, "A");
        g.add_node(2usize, "B");
        g.add_node(3usize, BoxedNode("Wide target node"));
        g.add_node(4usize, "L");
        g.add_node(5usize, "R");
        g.add_edge(1usize, 2usize, None);
        g.add_edge(2usize, 3usize, None).to_port(PortSide::Upstream);
        g.add_edge(1usize, 3usize, None).to_port(PortSide::Upstream);
        g.add_edge(4usize, 3usize, None).to_port(PortSide::Upstream);
        g.add_edge(5usize, 3usize, None).to_port(PortSide::Upstream);
        let ir = g.compute_layout();
        let out = ir.render_string(&RenderOptions::plain());
        let arrivals = out.lines().map(|l| l.matches('↓').count()).sum::<usize>();
        assert_eq!(arrivals, 5, "{out}");
        assert_routing_invariants(&ir, "spread skip");
    }

    /// No lane on either side (neighbors packed at spacing 0) means
    /// the end attaches head-on after all — the drawing is the Auto
    /// drawing, and nothing panics.
    #[cfg(feature = "layout-vertical")]
    #[test]
    fn no_free_lane_means_head_on_attachment() {
        let build = |declare: bool| {
            let mut g = Graph::new();
            g.add_node(0usize, "Root");
            g.add_node(1usize, "L");
            g.add_node(2usize, "M");
            g.add_node(3usize, "R");
            g.add_node(4usize, "T");
            g.add_edge(0usize, 1usize, None);
            g.add_edge(0usize, 2usize, None);
            g.add_edge(0usize, 3usize, None);
            let h = g.add_edge(2usize, 4usize, None);
            if declare {
                h.from_port(PortSide::Upstream);
            }
            let cfg = crate::algorithms::sugiyama::config::LayoutConfig {
                node_spacing: 0,
                ..crate::algorithms::sugiyama::config::LayoutConfig::standard()
            };
            g.compute_layout_with_config(&cfg)
        };
        let plain = build(false).render_string(&RenderOptions::plain());
        let ir = build(true);
        let out = ir.render_string(&RenderOptions::plain());
        assert!(
            !matches!(ir.edges()[3].path, EdgePath::Orthogonal { .. }),
            "{out}"
        );
        assert_eq!(out, plain);
    }

    /// Both backends route every detour fixture identically — the arena
    /// layout from arenas sized EXACTLY by the estimates (detour scratch
    /// and staged bends counted).
    #[cfg(feature = "arena")]
    #[test]
    fn both_backends_route_detours_identically() {
        use crate::algorithms::sugiyama::config::LayoutConfig;
        use crate::graph::arena::Arena;
        let mut fixtures: Vec<(&str, Graph<'static>, LayoutConfig<'static>)> = Vec::new();
        let mut directions = Vec::new();
        #[cfg(feature = "layout-vertical")]
        directions.extend([Direction::TopDown, Direction::BottomUp]);
        #[cfg(feature = "layout-horizontal")]
        directions.extend([Direction::LeftRight, Direction::RightLeft]);
        for direction in directions {
            let mut cfg = LayoutConfig::standard();
            cfg.direction = direction;
            let mut g = Graph::new();
            g.add_node(1usize, "A");
            g.add_node(2usize, "B");
            g.add_node(3usize, "C");
            g.add_node(4usize, "Wide target node");
            g.add_edge(1usize, 2usize, None)
                .from_port(PortSide::Upstream);
            g.add_edge(2usize, 3usize, None)
                .to_port(PortSide::Downstream);
            g.add_edge(1usize, 3usize, Some("skip"))
                .from_port(PortSide::Upstream)
                .to_port(PortSide::Downstream);
            g.add_edge(3usize, 4usize, None);
            g.add_edge(2usize, 4usize, None).to_port(PortSide::Upstream);
            g.add_edge(1usize, 4usize, None).to_port(PortSide::Upstream);
            g.add_edge(4usize, 1usize, None)
                .from_port(PortSide::Downstream);
            g.add_edge(3usize, 3usize, None);
            fixtures.push(("mixed", g, cfg));
            let mut tight = LayoutConfig::standard();
            tight.direction = direction;
            tight.node_spacing = 0;
            let mut g = Graph::new();
            g.add_node(0usize, "Root");
            g.add_node(1usize, "L");
            g.add_node(2usize, "M");
            g.add_node(3usize, "R");
            g.add_node(4usize, "T");
            g.add_edge(0usize, 1usize, None);
            g.add_edge(0usize, 2usize, None);
            g.add_edge(0usize, 3usize, None);
            g.add_edge(2usize, 4usize, None)
                .from_port(PortSide::Upstream);
            g.add_edge(1usize, 4usize, None)
                .from_port(PortSide::Upstream);
            fixtures.push(("tight", g, tight));
            let mut cfg = LayoutConfig::standard();
            cfg.direction = direction;
            let mut g = Graph::new();
            g.add_node(1usize, "A");
            g.add_node(2usize, "B");
            g.add_node(3usize, "C");
            g.add_node(4usize, "Wide target node");
            g.add_edge(1usize, 2usize, None)
                .from_port(PortSide::Clockwise);
            g.add_edge(2usize, 3usize, None)
                .to_port(PortSide::Counterclockwise);
            g.add_edge(1usize, 3usize, Some("skip"))
                .from_port(PortSide::Counterclockwise)
                .to_port(PortSide::Clockwise);
            g.add_edge(3usize, 4usize, None)
                .from_port(PortSide::Clockwise);
            g.add_edge(2usize, 4usize, None)
                .to_port(PortSide::Counterclockwise);
            g.add_edge(1usize, 4usize, None).to_port(PortSide::Upstream);
            g.add_edge(4usize, 1usize, None)
                .from_port(PortSide::Clockwise);
            g.add_edge(3usize, 3usize, None);
            fixtures.push(("lateral", g, cfg));
            let mut cfg = LayoutConfig::standard();
            cfg.direction = direction;
            let mut g = Graph::new();
            g.set_port_policy(PortPolicy::Spread(PortBound::Ports(2)));
            g.add_node(1usize, crate::render::engine::BoxedNode("Hub"));
            g.add_node(2usize, crate::render::engine::BoxedNode("Pair"));
            g.set_node_port_policy(2usize, PortPolicy::Paired);
            g.add_node(3usize, "In");
            g.add_node(4usize, "Out");
            g.add_node(5usize, "Side");
            g.add_edge(3usize, 1usize, None).to_port(PortSide::Upstream);
            g.add_edge(5usize, 1usize, None).to_port(PortSide::Upstream);
            g.add_edge(1usize, 2usize, None)
                .from_port(PortSide::Upstream)
                .to_port(PortSide::Downstream);
            g.add_edge(1usize, 4usize, None)
                .from_port(PortSide::Clockwise);
            g.add_edge(2usize, 4usize, None)
                .from_port(PortSide::Clockwise);
            g.add_edge(3usize, 2usize, None)
                .to_port(PortSide::Counterclockwise);
            fixtures.push(("policies", g, cfg));
        }
        // A zero-extent neighbor occupies no cell: the lane beside it
        // is free on both backends (the heap's inclusive blocker used
        // to claim the cell).
        #[cfg(feature = "layout-vertical")]
        {
            use crate::render::engine::CustomNode;
            let mut cfg = LayoutConfig::standard();
            cfg.node_spacing = 1;
            let mut g = Graph::new();
            g.add_node(0usize, "Root");
            g.add_node(
                1usize,
                CustomNode {
                    label: "",
                    width: 0,
                    height: 1,
                    painter: None,
                    payload: "",
                },
            );
            g.add_node(2usize, "B");
            g.add_node(3usize, "C");
            g.add_node(4usize, "T");
            g.add_edge(0usize, 1usize, None);
            g.add_edge(0usize, 2usize, None);
            g.add_edge(0usize, 3usize, None);
            g.add_edge(2usize, 4usize, None)
                .from_port(PortSide::Upstream);
            fixtures.push(("zero-extent", g, cfg));
        }
        for (tag, g, cfg) in &fixtures {
            let heap_ir = g.compute_layout_with_config(cfg);
            assert_routing_invariants(&heap_ir, tag);
            let heap = heap_ir.render_string(&RenderOptions::plain());
            let mut csr_buf = vec![0u8; g.estimate_csr_arena_size()];
            let mut csr_arena = Arena::new(&mut csr_buf);
            let csr = g.to_csr(&mut csr_arena).expect("exact CSR estimate");
            let bytes = g.estimate_layout_arena_size_with(cfg);
            let mut temp = vec![0u8; bytes];
            let mut out = vec![0u8; bytes];
            let mut ta = Arena::new(&mut temp);
            let mut oa = Arena::new(&mut out);
            let ir = csr
                .compute_layout_arena(cfg, &mut ta, &mut oa)
                .expect("exact layout estimate covers the detour scratch");
            let mut arena = String::new();
            ir.render_with(&RenderOptions::plain(), &mut arena).unwrap();
            assert_eq!(heap, arena, "{tag} {:?}", cfg.direction);
            // The attachments agree edge for edge.
            for (i, e) in heap_ir.edges().iter().enumerate() {
                let a = crate::ir::arena::LayoutIRArena::edge(&ir, i);
                assert_eq!(
                    (e.from_port, e.to_port),
                    (a.from_port, a.to_port),
                    "{tag} edge {i}"
                );
            }
        }
    }

    /// Scale: a wide star with ONE detouring port. Both backends agree,
    /// the arena lays out from exactly estimated arenas, and the
    /// declared port costs the estimate a bounded per-edge amount — the
    /// detour tables are sized by what detours (one edge, one node, the
    /// leaves' level), not by the graph.
    #[cfg(all(feature = "arena", feature = "layout-vertical"))]
    #[test]
    fn a_wide_star_with_one_detouring_port_scales_linearly() {
        use crate::algorithms::sugiyama::config::LayoutConfig;
        use crate::graph::arena::Arena;
        // Sized by the index type in play (`arena-idx-u8` holds 255 nodes).
        let leaves: usize =
            2_000.min(crate::algorithms::sugiyama::idx::MAX_NODES.saturating_sub(2));
        let build = |declare: bool| {
            let mut g = Graph::new();
            g.add_node(0usize, "Root");
            for leaf in 1..=leaves {
                g.add_node(leaf, "L");
                let h = g.add_edge(0usize, leaf, None);
                if declare && leaf == leaves / 2 {
                    h.to_port(PortSide::Downstream);
                }
            }
            g
        };
        let cfg = LayoutConfig::standard();
        let plain = build(false);
        let ported = build(true);
        let plain_bytes = plain.estimate_layout_arena_size_with(&cfg);
        let ported_bytes = ported.estimate_layout_arena_size_with(&cfg);
        // Requests, positions and marks are per edge/node by design; the
        // detour tables must not be.
        assert!(
            ported_bytes - plain_bytes <= 160 * (leaves + 1) + 16_384,
            "one port grew the estimate by {} bytes",
            ported_bytes - plain_bytes
        );
        let heap = ported
            .compute_layout_with_config(&cfg)
            .render_string(&RenderOptions::plain());
        let mut csr_buf = vec![0u8; ported.estimate_csr_arena_size()];
        let mut csr_arena = Arena::new(&mut csr_buf);
        let csr = ported.to_csr(&mut csr_arena).expect("exact CSR estimate");
        let mut temp = vec![0u8; ported_bytes];
        let mut out = vec![0u8; ported_bytes];
        let mut ta = Arena::new(&mut temp);
        let mut oa = Arena::new(&mut out);
        let ir = csr
            .compute_layout_arena(&cfg, &mut ta, &mut oa)
            .expect("exact layout estimate covers the sparse detour scratch");
        let mut arena_render = String::new();
        ir.render_with(&RenderOptions::plain(), &mut arena_render)
            .unwrap();
        assert_eq!(heap, arena_render);
        assert!(
            heap.contains('↑'),
            "the detour reached the leaf's bottom face"
        );
    }

    /// Lateral faces: a source leaves through its side face
    /// with a stub straight onto the lane beside the node, then turns
    /// onto the flow; a target is entered the same way in reverse.
    #[cfg(feature = "layout-vertical")]
    #[test]
    fn side_faces_leave_and_enter_beside_the_node() {
        let mut g = Graph::new();
        g.add_node(1usize, "A");
        g.add_node(2usize, "B");
        g.add_edge(1usize, 2usize, None).from_port(PortSide::East);
        let ir = g.compute_layout();
        let out = ir.render_string(&RenderOptions::plain());
        let (ax, ay, aw, _) = node_rect(&ir, 1);
        let b = bends(&ir, 0);
        let e = &ir.edges()[0];
        // The endpoint is A's own east cell; the first turn is beside
        // it on A's row; the lane runs down from there.
        assert_eq!((e.from_x, e.from_y), (ax + aw - 1, ay), "{out}");
        assert_eq!(b[0].1, ay, "{b:?}\n{out}");
        assert!(b[0].0 >= ax + aw, "{b:?}\n{out}");
        assert_eq!(cell_at(&out, ax + aw, ay), '┐', "{out}");
        assert_routing_invariants(&ir, "east exit");

        let mut g = Graph::new();
        g.add_node(1usize, "A");
        g.add_node(2usize, "B");
        g.add_edge(1usize, 2usize, None).to_port(PortSide::West);
        let ir = g.compute_layout();
        let out = ir.render_string(&RenderOptions::plain());
        let (bx, by, _, _) = node_rect(&ir, 2);
        let b = bends(&ir, 0);
        let e = &ir.edges()[0];
        assert_eq!((e.to_x, e.to_y), (bx, by), "{out}");
        assert_eq!(b[b.len() - 1].1, by, "{b:?}\n{out}");
        assert!(b[b.len() - 1].0 < bx, "{b:?}\n{out}");
        assert_eq!(cell_at(&out, bx - 1, by), '→', "{out}");
        assert_routing_invariants(&ir, "west arrival");
    }

    /// The rotations name a side by the traveler's hand facing
    /// downstream: `Clockwise` is the right hand — West under TopDown,
    /// East under BottomUp, South under LeftRight, North under
    /// RightLeft — and `Counterclockwise` the other.
    #[test]
    fn rotations_pick_the_side_by_direction() {
        let mut directions = Vec::new();
        #[cfg(feature = "layout-vertical")]
        directions.extend([Direction::TopDown, Direction::BottomUp]);
        #[cfg(feature = "layout-horizontal")]
        directions.extend([Direction::LeftRight, Direction::RightLeft]);
        for direction in directions {
            for (side, clockwise) in [
                (PortSide::Clockwise, true),
                (PortSide::Counterclockwise, false),
            ] {
                let mut g = Graph::new();
                g.set_direction(direction);
                g.add_node(1usize, "A");
                g.add_node(2usize, "B");
                g.add_edge(1usize, 2usize, None).from_port(side);
                let ir = g.compute_layout();
                let out = ir.render_string(&RenderOptions::plain());
                let (ax, ay, aw, ah) = node_rect(&ir, 1);
                let first = bends(&ir, 0)[0];
                let west = first.0 < ax;
                let east = first.0 >= ax + aw;
                let south = first.1 >= ay + ah;
                let north = first.1 < ay;
                let _ = (west, east, south, north);
                let ok = match direction {
                    #[cfg(feature = "layout-vertical")]
                    Direction::TopDown => {
                        if clockwise {
                            west
                        } else {
                            east
                        }
                    }
                    #[cfg(feature = "layout-vertical")]
                    Direction::BottomUp => {
                        if clockwise {
                            east
                        } else {
                            west
                        }
                    }
                    #[cfg(feature = "layout-horizontal")]
                    Direction::LeftRight => {
                        if clockwise {
                            south
                        } else {
                            north
                        }
                    }
                    #[cfg(feature = "layout-horizontal")]
                    Direction::RightLeft => {
                        if clockwise {
                            north
                        } else {
                            south
                        }
                    }
                };
                assert!(
                    ok,
                    "{direction:?} {side:?}: first turn {first:?} vs A {:?}\n{out}",
                    (ax, ay, aw, ah)
                );
                assert_routing_invariants(&ir, "rotation");
            }
        }
    }

    /// No lane beside the named side (a neighbor packed at spacing 0)
    /// means the end attaches head-on after all — the Auto drawing; so
    /// does a trailing-side stub that would leave through a one-row
    /// node's `↺` cell.
    #[cfg(feature = "layout-vertical")]
    #[test]
    fn side_faces_fall_back_head_on_without_a_lane() {
        let build = |declare: bool| {
            let mut g = Graph::new();
            g.add_node(0usize, "Root");
            g.add_node(1usize, "L");
            g.add_node(2usize, "M");
            g.add_node(3usize, "R");
            g.add_node(4usize, "T");
            g.add_edge(0usize, 1usize, None);
            g.add_edge(0usize, 2usize, None);
            g.add_edge(0usize, 3usize, None);
            let h = g.add_edge(2usize, 4usize, None);
            if declare {
                h.from_port(PortSide::East);
            }
            let cfg = crate::algorithms::sugiyama::config::LayoutConfig {
                node_spacing: 0,
                ..crate::algorithms::sugiyama::config::LayoutConfig::standard()
            };
            g.compute_layout_with_config(&cfg)
        };
        assert_eq!(
            build(true).render_string(&RenderOptions::plain()),
            build(false).render_string(&RenderOptions::plain())
        );
        let build = |declare: bool| {
            let mut g = Graph::new();
            g.add_node(1usize, "A");
            g.add_node(2usize, "B");
            g.add_edge(1usize, 1usize, None);
            let h = g.add_edge(1usize, 2usize, None);
            if declare {
                h.from_port(PortSide::East);
            }
            g.compute_layout()
        };
        assert_eq!(
            build(true).render_string(&RenderOptions::plain()),
            build(false).render_string(&RenderOptions::plain())
        );
    }

    /// A lane-less end attaches head-on AFTER lanes are known and
    /// BEFORE conflicts are settled: an east exit that finds no lane
    /// (the `↺` cell on a one-row node) lands on the leave face's
    /// center, and a bottom-face arrival that detours on the same node
    /// must not share that cell.
    #[cfg(feature = "layout-vertical")]
    #[test]
    fn a_lane_less_end_attaches_head_on_and_shares_the_port() {
        let mut g = Graph::new();
        g.add_node(0usize, "Root");
        g.add_node(1usize, "M");
        g.add_node(2usize, "T");
        g.add_edge(0usize, 1usize, None)
            .to_port(PortSide::Downstream);
        g.add_edge(1usize, 1usize, None);
        g.add_edge(1usize, 2usize, None).from_port(PortSide::East);
        let ir = g.compute_layout();
        let out = ir.render_string(&RenderOptions::plain());
        // The east exit fell back (Direct or Corner) onto the leave
        // face's port; the arrival still detours into that same port —
        // under `Single` a face has one port and both ends share it.
        let exit = ir.edges().iter().find(|e| e.edge_index == 2).unwrap();
        assert!(!matches!(exit.path, EdgePath::Orthogonal { .. }), "{out}");
        let arrival = ir.edges().iter().find(|e| e.edge_index == 0).unwrap();
        assert!(matches!(arrival.path, EdgePath::Orthogonal { .. }), "{out}");
        assert_eq!(exit.from_x, arrival.to_x, "{out}");
        assert_routing_invariants(&ir, "lane-less east exit");
    }

    /// A `↺` cell blocks a trailing-side stub only on the node's top
    /// row: a centered east exit on a three-row self-loop node routes.
    #[cfg(feature = "layout-vertical")]
    #[test]
    fn a_marker_blocks_only_its_own_row() {
        use crate::render::engine::CustomNode;
        let mut g = Graph::new();
        g.add_node(
            1usize,
            CustomNode {
                label: "A",
                width: 3,
                height: 3,
                painter: None,
                payload: "",
            },
        );
        g.add_node(2usize, "B");
        g.add_edge(1usize, 1usize, None);
        g.add_edge(1usize, 2usize, None).from_port(PortSide::East);
        let ir = g.compute_layout();
        let out = ir.render_string(&RenderOptions::plain());
        let (ax, ay, aw, _) = node_rect(&ir, 1);
        let b = bends(&ir, 1);
        assert_eq!(
            b[0],
            (ax + aw, ay + 1),
            "centered stub beside the node:\n{out}"
        );
        let (mx, my) = ir.node_by_id(1).unwrap().self_loop_at.unwrap();
        assert_eq!(cell_at(&out, mx, my), '↺', "{out}");
        assert_routing_invariants(&ir, "marker row");
    }

    /// The leading cell a west port opens must stay representable: two
    /// nodes at the coordinate type's width fail cleanly on the arena
    /// backend instead of wrapping. A 16-bit canvas only: with `u32`
    /// coordinates the nodes would be four billion cells wide.
    #[cfg(all(
        feature = "arena",
        feature = "layout-vertical",
        any(feature = "arena-idx-u8", feature = "arena-idx-u16")
    ))]
    #[test]
    fn a_leading_cell_past_the_coordinate_type_is_an_error() {
        use crate::algorithms::sugiyama::config::LayoutConfig;
        use crate::algorithms::sugiyama::idx::MAX_COORD;
        use crate::graph::arena::Arena;
        use crate::render::engine::CustomNode;
        let wide = |label: &'static str| CustomNode {
            label,
            width: MAX_COORD - 3,
            height: 1,
            painter: None,
            payload: "",
        };
        let mut g = Graph::new();
        g.add_node(1usize, wide("A"));
        g.add_node(2usize, wide("B"));
        g.add_edge(1usize, 2usize, None).from_port(PortSide::West);
        let cfg = LayoutConfig::standard();
        let mut csr_buf = vec![0u8; g.estimate_csr_arena_size()];
        let mut csr_arena = Arena::new(&mut csr_buf);
        let csr = g.to_csr(&mut csr_arena).expect("exact CSR estimate");
        let bytes = g.estimate_layout_arena_size_with(&cfg);
        let mut temp = vec![0u8; bytes];
        let mut out = vec![0u8; bytes];
        let mut ta = Arena::new(&mut temp);
        let mut oa = Arena::new(&mut out);
        match csr.compute_layout_arena(&cfg, &mut ta, &mut oa) {
            Err(crate::GraphError::ExceedsMaxExtent { extent, max }) => {
                assert!(extent > max, "{extent} vs {max}");
            }
            other => panic!(
                "expected ExceedsMaxExtent, got {:?}",
                other.map(|ir| ir.width())
            ),
        }
    }

    /// Everything is reported back — requested AND resolved: the IR's
    /// attachments, the scene view, and the JSON keys agree, on a
    /// routed side, an undeclared end, and a reversed edge whose
    /// declared side binds to its declared end.
    #[cfg(feature = "layout-vertical")]
    #[test]
    fn attachments_report_the_requested_and_resolved_sides() {
        use crate::{PhysicalSide, PortAttachment};
        let mut g = Graph::new();
        g.add_node(1usize, "A");
        g.add_node(2usize, "B");
        g.add_edge(1usize, 2usize, None).from_port(PortSide::East);
        g.add_edge(2usize, 1usize, None)
            .from_port(PortSide::Downstream);
        let ir = g.compute_layout();
        let forward = &ir.edges()[0];
        assert_eq!(
            forward.from_port,
            PortAttachment {
                requested: PortSide::East,
                side: PhysicalSide::East
            }
        );
        assert_eq!(forward.to_port, PortAttachment::auto(PhysicalSide::North));
        let back = &ir.edges()[1];
        assert!(back.reversed);
        assert_eq!(
            back.from_port,
            PortAttachment {
                requested: PortSide::Downstream,
                side: PhysicalSide::South
            },
            "declared on B, the layout target: its bottom face"
        );
        assert_eq!(back.to_port, PortAttachment::auto(PhysicalSide::South));
        let json = ir.to_json();
        assert!(
            json.contains("\"from_side\":\"east\",\"to_side\":\"north\",\"from_port\":\"east\""),
            "{json}"
        );
        assert!(
            json.contains(
                "\"from_side\":\"south\",\"to_side\":\"south\",\"from_port\":\"downstream\""
            ),
            "{json}"
        );
        assert!(!json.contains("\"to_port\""), "{json}");
        let mut planner = crate::render::engine::ScenePlanner::new();
        let scene = planner
            .plan(&ir, &RenderOptions::plain().plan)
            .quiet()
            .unwrap();
        let view = scene.edge(0).unwrap();
        assert_eq!(view.from_port, forward.from_port);
        assert_eq!(view.to_port, forward.to_port);
    }

    /// A self-loop node keeps its `↺` cell: the lane on that side sits
    /// one further out.
    #[cfg(feature = "layout-vertical")]
    #[test]
    fn self_loop_nodes_keep_their_marker_beside_a_lane() {
        let mut g = Graph::new();
        g.add_node(1usize, "A");
        g.add_node(2usize, "B");
        g.add_node(3usize, "C");
        g.add_edge(1usize, 1usize, None);
        g.add_edge(1usize, 2usize, None);
        g.add_edge(1usize, 3usize, None)
            .from_port(PortSide::Upstream);
        let ir = g.compute_layout();
        let out = ir.render_string(&RenderOptions::plain());
        let a = ir.node_by_id(1).unwrap();
        let (mx, my) = a.self_loop_at.unwrap();
        assert_eq!(cell_at(&out, mx, my), '↺', "{out}");
        let b = bends(&ir, 2);
        assert!(
            b.iter().all(|&(x, _)| x != mx),
            "lane on the marker column: {b:?}\n{out}"
        );
        assert_routing_invariants(&ir, "self-loop");
    }
}
