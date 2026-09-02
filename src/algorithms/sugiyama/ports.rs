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
#[allow(dead_code)] // the explicit sides are constructed by the port API, which lands with side routing
pub(crate) enum PortSide {
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
#[allow(dead_code)] // constructed by the heap handle today; the CSR handle joins with its port table
pub(crate) struct Port {
    side: PortSide,
}

#[allow(dead_code)] // consumed by the heap handle today; the CSR handle joins with its port table
impl Port {
    /// A port on `side`, positioned by the router. `const`, so a
    /// declaration can live in a constant.
    pub(crate) const fn of(side: PortSide) -> Port {
        Port { side }
    }

    /// The declared side.
    pub(crate) const fn side(self) -> PortSide {
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
    /// center fields exactly). Explicit level faces attach on the
    /// center line for now (per-face offsets arrive with capacity
    /// resolution); cross faces carry their face but the same center
    /// cross line until side routing exists — the consumer decides
    /// what it can honor.
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
    any(feature = "layout-vertical", feature = "layout-horizontal")
))]
mod layout_tests {
    use super::PortSide;
    use crate::graph::Graph;
    use crate::render::engine::RenderOptions;

    /// Four forward edges declared through the fluent handle with the
    /// given sides, plus a reversed cycle edge whose DECLARED source
    /// is the layout target — so its declared sides are the mirror.
    fn fixture(leave: PortSide, arrive: PortSide) -> Graph<'static> {
        let mut g = Graph::new();
        g.add_node(1usize, "Start");
        g.add_node(2usize, "Middle");
        g.add_node(3usize, "Wide node");
        g.add_node(4usize, "End");
        g.add_edge(1usize, 2usize, Some("go"))
            .from_port(leave)
            .to_port(arrive);
        g.add_edge(1usize, 3usize, None)
            .from_port(leave)
            .to_port(arrive);
        g.add_edge(2usize, 4usize, None)
            .from_port(leave)
            .to_port(arrive);
        g.add_edge(3usize, 4usize, None)
            .from_port(leave)
            .to_port(arrive);
        g.add_edge(4usize, 1usize, Some("again")); // cycle → reversed
        assert!(g.set_edge_ports(4, arrive, leave));
        assert!(!g.set_edge_ports(99, leave, arrive), "unknown edge");
        g
    }

    /// A graph that never declares a port stores nothing per edge;
    /// the first declaration materializes the table — and only then.
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
            .to_port(PortSide::North);
        assert_eq!(g.edge_ports.len(), 3, "materialized to the edge count");
        assert_eq!(g.edge_ports[0], (PortSide::Auto, PortSide::Auto));
        assert_eq!(g.edge_ports[2].1, PortSide::North);
        g.add_edge(2usize, 2usize, None);
        assert_eq!(g.edge_ports.len(), 4, "stays parallel once it exists");
    }

    #[test]
    fn explicit_auto_equivalent_sides_render_identically() {
        // (direction, side a layout-source leaves from, side a
        // layout-target arrives on) — Auto's faces, named explicitly in
        // the physical frame AND the flow-relative one, for every
        // direction this build enables.
        use crate::graph::Direction;
        let mut cases: Vec<(Direction, PortSide, PortSide)> = Vec::new();
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
        for (direction, leave, arrive) in cases {
            let mut auto = fixture(PortSide::Auto, PortSide::Auto);
            auto.set_direction(direction);
            let expected = auto.compute_layout().render_string(&RenderOptions::plain());

            let mut explicit = fixture(leave, arrive);
            explicit.set_direction(direction);
            let got = explicit
                .compute_layout()
                .render_string(&RenderOptions::plain());
            assert_eq!(
                got, expected,
                "{direction:?} {leave:?}/{arrive:?}: Auto-equivalent sides diverged"
            );
        }
    }
}

/// The CSR twin: declared sides travel through `to_csr` and the CSR
/// builder under the preallocation contract, and Auto-equivalent
/// declarations render identically to `Auto` on the arena backend —
/// which must also match the heap backend byte for byte.
#[cfg(all(
    test,
    feature = "std",
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

    fn heap_fixture(leave: PortSide, arrive: PortSide) -> Graph<'static> {
        let mut g = Graph::new();
        g.add_node(1usize, "Start");
        g.add_node(2usize, "Middle");
        g.add_node(3usize, "Wide node");
        g.add_node(4usize, "End");
        g.add_edge(1usize, 2usize, Some("go"))
            .from_port(leave)
            .to_port(arrive);
        g.add_edge(1usize, 3usize, None)
            .from_port(leave)
            .to_port(arrive);
        g.add_edge(2usize, 4usize, None)
            .from_port(leave)
            .to_port(arrive);
        g.add_edge(3usize, 4usize, None)
            .from_port(leave)
            .to_port(arrive);
        g.add_edge(4usize, 1usize, Some("again")); // reversed: sides mirrored
        assert!(g.set_edge_ports(4, arrive, leave));
        g
    }

    /// `to_csr` copies declared sides; the arena layout honors them
    /// through the same seam; heap and arena stay byte-identical.
    #[test]
    fn to_csr_carries_declared_sides_with_backend_parity() {
        use crate::graph::Direction;
        // Every enabled direction, in BOTH frames: the physical sides
        // Auto resolves to, and the flow-relative spelling of them.
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

            let auto = heap_fixture(PortSide::Auto, PortSide::Auto);
            let expected = auto
                .compute_layout_with_config(&config)
                .render_string(&RenderOptions::plain());

            let declared = heap_fixture(leave, arrive);
            let heap = declared
                .compute_layout_with_config(&config)
                .render_string(&RenderOptions::plain());
            assert_eq!(heap, expected, "{direction:?}: heap");

            let mut buf = vec![0u8; declared.estimate_csr_arena_size()];
            let mut arena = Arena::new(&mut buf);
            let csr = declared
                .to_csr(&mut arena)
                .expect("exact estimate fits, ports included");
            assert!(csr.has_ports());
            assert_eq!(
                csr.edge_ports(0),
                (leave, arrive),
                "declared sides survive to_csr"
            );
            assert_eq!(
                csr.edge_ports(4),
                (arrive, leave),
                "mirrored pair on the reversed edge"
            );
            assert_eq!(render_csr(&csr, &config), expected, "{direction:?}: arena");
        }
    }

    /// An undeclared graph converts without a table and the estimate
    /// does not grow; a declared one grows by exactly the table.
    #[test]
    fn undeclared_graphs_carry_no_csr_table() {
        let auto = heap_fixture(PortSide::Auto, PortSide::Auto);
        let declared = heap_fixture(PortSide::South, PortSide::North);
        assert_eq!(
            declared.estimate_csr_arena_size(),
            auto.estimate_csr_arena_size() + 5 * 2 + 8,
            "two bytes per edge plus slack, only when declared"
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
        let h = b
            .add_edge(0, 1)
            .unwrap()
            .from_port(PortSide::South)
            .expect("preallocated")
            .to_port(Port::of(PortSide::Upstream))
            .expect("preallocated");
        assert_eq!(h.edge(), 0);
        b.add_edge(1, 2).unwrap();
        assert!(
            b.set_edge_ports(1, PortSide::Downstream, PortSide::North)
                .is_some()
        );
        assert!(
            b.set_edge_ports(7, PortSide::Auto, PortSide::Auto)
                .is_none(),
            "unknown edge"
        );
        let csr = b.build().unwrap();
        assert!(csr.has_ports());
        assert_eq!(csr.edge_ports(0), (PortSide::South, PortSide::Upstream));
        assert_eq!(csr.edge_ports(1), (PortSide::Downstream, PortSide::North));
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
            h.from_port(PortSide::South).is_none(),
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
