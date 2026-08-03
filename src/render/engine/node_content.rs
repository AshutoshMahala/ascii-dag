//! `NodeContent` — nodes as objects (temp/07).
//!
//! A node's kind belongs to the node, at construction:
//!
//! ```
//! use ascii_dag::{Graph, AUTO, BoxedNode, NodeContent};
//!
//! let mut g = Graph::new();
//! let client = g.add_node(AUTO, "Client");            // &str → simple
//! let server = g.add_node(AUTO, BoxedNode("Server")); // built-in kind
//! g.add_edge(client, server, None);
//! ```
//!
//! The trait is **sugar, not storage** (NC7b): implementors are
//! resolved once at insertion into plain data — label, size, kind tag,
//! optional painter fn, payload — so the object may be a temporary and
//! no `&dyn` is stored anywhere. All five fields travel both IRs
//! (including graphs built directly on `CsrGraphBuilder`) and drive
//! rendering; custom nodes serialize their kind and payload to JSON.
//! Rich typed state must flatten into `payload`; the painter parses it
//! back as it draws (zero-alloc iteration — see the trait docs).

use super::style::NodePaintFn;

/// The resolved kind tag a [`NodeContent`] implementor declares.
///
/// Hidden from docs: user impls normally keep the default (`Custom`);
/// built-ins override it so both backends can render them as pure
/// data. Overriding it to a built-in tag in a user impl is harmless —
/// the node simply renders through that built-in painter.
#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
#[repr(u8)]
pub enum NodeKindTag {
    /// `[label]` on the top row — today's default look.
    Simple = 0,
    /// A light-stroke box spanning the reserved area.
    Boxed = 1,
    /// User-declared content: painter + payload (or blank, if no
    /// painter — the area stays reserved but unpainted).
    Custom = 2,
}

impl NodeKindTag {
    /// Stable storage value (CSR `NODE_FLAGS` bits / heap tag array).
    ///
    /// Exhaustive on purpose — the CSR-tag-drift guard: a new kind
    /// added here fails to compile until every conversion handles it.
    #[inline]
    // Consumers: Graph resolution (alloc) and CsrGraphBuilder (arena).
    #[cfg_attr(not(any(feature = "alloc", feature = "arena")), allow(dead_code))]
    pub(crate) fn to_u8(self) -> u8 {
        match self {
            NodeKindTag::Simple => 0,
            NodeKindTag::Boxed => 1,
            NodeKindTag::Custom => 2,
        }
    }

    /// Rebuild from a storage value; unknown values fall back to
    /// `Simple` (renders the label — nothing is silently invisible).
    #[inline]
    pub(crate) fn from_u8(value: u8) -> Self {
        match value {
            1 => NodeKindTag::Boxed,
            2 => NodeKindTag::Custom,
            _ => NodeKindTag::Simple,
        }
    }
}

/// What a node *is*: label, size, and — for custom kinds — the
/// template/data pair that fills its reserved area at render time.
///
/// Implementors are resolved **at insertion** (the object may be a
/// temporary); `label()`/`payload()` return `&'a str` so the strings
/// outlive the object — the same borrow discipline node labels have
/// always had. Accessors should be cheap and pure: resolution may
/// call them more than once (the default `size()` reads `label()`),
/// and nothing is stored until every accessor has returned.
///
/// - [`label`](Self::label) feeds legends, JSON, debug, and the
///   default sizing.
/// - [`size`](Self::size) feeds layout: the reserved `width × height`
///   area edges route around. Default: `(label chars + 2, 1)` — the
///   classic `[label]` footprint.
/// - [`painter`](Self::painter) is the **template**: a plain `fn` that
///   draws through the clipped [`NodeRegion`](super::region::NodeRegion). `None` on a custom kind
///   means **blank**: the area stays reserved, the canvas untouched,
///   while the label still flows to legends/JSON and hit-testing.
/// - [`payload`](Self::payload) is the **data**: delivered to the
///   painter via [`NodePaintCtx`](super::region::NodePaintCtx), parsed as it draws.
///
/// A minimal impl (just `label()`) is a *blank* node — pass `&str` or
/// [`SimpleNode`] instead when you want the classic `[label]` look.
///
/// ```
/// use ascii_dag::{BoxedNode, CustomNode, Graph, RenderOptions};
/// use ascii_dag::render::engine::{NodePaintCtx, NodeRegion};
///
/// // A painter is a plain `fn`: one template, many nodes.
/// fn card(region: &mut NodeRegion<'_, '_>, ctx: NodePaintCtx<'_>) {
///     region.write_str(0, 0, ctx.label);
///     for (i, line) in ctx.payload.lines().enumerate() {
///         region.write_str(0, 1 + i, line);
///     }
/// }
///
/// let mut g = Graph::new();
/// g.add_node(1, "Client");                       // [Client]
/// g.add_node(2, BoxedNode("Database"));          // boxed label
/// g.add_node(3, CustomNode {                     // your painter + its data
///     label: "Server",
///     width: 14,
///     height: 3,
///     painter: Some(card),
///     payload: "cpu: 4\nram: 16G",
/// });
/// g.add_edge(1, 3, None);
/// g.add_edge(3, 2, None);
///
/// let text = g.compute_layout().render_string(&RenderOptions::plain());
/// assert!(text.contains("cpu: 4"));
/// ```
pub trait NodeContent<'a> {
    /// The node's label text.
    fn label(&self) -> &'a str;

    /// Reserved `(width, height)` in cells/rows. Layout routes edges
    /// around this area; the painter fills it.
    fn size(&self) -> (usize, usize) {
        (self.label().chars().count() + 2, 1)
    }

    /// The template fn that fills the reserved area, if any.
    fn painter(&self) -> Option<NodePaintFn> {
        None
    }

    /// Data for the painter: travels like a second label, delivered
    /// via [`NodePaintCtx`](super::region::NodePaintCtx) at paint time
    /// and exported to JSON as `"payload"` on custom nodes.
    fn payload(&self) -> &'a str {
        ""
    }

    /// The resolved kind tag — built-ins override this; user impls
    /// keep the default.
    #[doc(hidden)]
    fn kind(&self) -> NodeKindTag {
        NodeKindTag::Custom
    }

    /// Size provenance: `true` only for impls whose `size()` is the
    /// provided default formula (`&str`, `&String`, [`SimpleNode`]) —
    /// the graph then applies the legacy `⟨id⟩` placeholder width to
    /// empty labels (an id-dependent value `size()` cannot compute).
    /// Every impl that overrides `size()` keeps `false`: its declared
    /// size is authoritative, empty label or not.
    #[doc(hidden)]
    fn size_is_implicit(&self) -> bool {
        false
    }
}

/// The classic `[label]` node as an explicit object — identical to
/// passing the `&str` itself.
#[derive(Debug, Clone, Copy)]
pub struct SimpleNode<'a>(pub &'a str);

impl<'a> NodeContent<'a> for SimpleNode<'a> {
    fn label(&self) -> &'a str {
        self.0
    }

    fn kind(&self) -> NodeKindTag {
        NodeKindTag::Simple
    }

    fn size_is_implicit(&self) -> bool {
        true
    }
}

/// A light-stroke box with the label inside, sized
/// `(label chars + 4, 3)` — a real box out of the box (D4). For a
/// box of any other size, declare a [`CustomNode`] (or your own
/// `NodeContent` impl) with a box-drawing painter.
#[derive(Debug, Clone, Copy)]
pub struct BoxedNode<'a>(pub &'a str);

impl<'a> NodeContent<'a> for BoxedNode<'a> {
    fn label(&self) -> &'a str {
        self.0
    }

    fn size(&self) -> (usize, usize) {
        (self.0.chars().count() + 4, 3)
    }

    fn kind(&self) -> NodeKindTag {
        NodeKindTag::Boxed
    }
}

/// The explicit five-field custom-node form, for callers who want no
/// trait impl of their own.
#[derive(Debug, Clone, Copy)]
pub struct CustomNode<'a> {
    /// Label (legends, JSON, debug).
    pub label: &'a str,
    /// Reserved width in cells.
    pub width: usize,
    /// Reserved height in rows.
    pub height: usize,
    /// Template fn; `None` = blank (area reserved, nothing painted).
    pub painter: Option<NodePaintFn>,
    /// Data delivered to the painter via [`NodePaintCtx`](super::region::NodePaintCtx).
    pub payload: &'a str,
}

impl<'a> NodeContent<'a> for CustomNode<'a> {
    fn label(&self) -> &'a str {
        self.label
    }

    fn size(&self) -> (usize, usize) {
        (self.width, self.height)
    }

    fn painter(&self) -> Option<NodePaintFn> {
        self.painter
    }

    fn payload(&self) -> &'a str {
        self.payload
    }
}

/// `&str` is the everyday sugar: `g.add_node(1, "Client")` is a simple
/// node, byte-identical to every release before node objects existed.
impl<'a> NodeContent<'a> for &'a str {
    fn label(&self) -> &'a str {
        self
    }

    fn kind(&self) -> NodeKindTag {
        NodeKindTag::Simple
    }

    fn size_is_implicit(&self) -> bool {
        true
    }
}

/// Forwarding impl so borrowed content works: Rust does not infer
/// trait impls for references, and `g.add_node(AUTO, &my_card)` must
/// compile when `my_card` is reused across graphs.
impl<'a, T: NodeContent<'a>> NodeContent<'a> for &T {
    fn label(&self) -> &'a str {
        (**self).label()
    }

    fn size(&self) -> (usize, usize) {
        (**self).size()
    }

    fn painter(&self) -> Option<NodePaintFn> {
        (**self).painter()
    }

    fn payload(&self) -> &'a str {
        (**self).payload()
    }

    fn kind(&self) -> NodeKindTag {
        (**self).kind()
    }

    fn size_is_implicit(&self) -> bool {
        (**self).size_is_implicit()
    }
}

/// `&String` kept compiling into 0.10: the old `label: &'a str`
/// parameter deref-coerced it; a generic bound does not, so the impl
/// restores that path explicitly.
#[cfg(feature = "alloc")]
impl<'a> NodeContent<'a> for &'a alloc::string::String {
    fn label(&self) -> &'a str {
        self.as_str()
    }

    fn kind(&self) -> NodeKindTag {
        NodeKindTag::Simple
    }

    fn size_is_implicit(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::engine::region::{NodePaintCtx, NodeRegion};

    fn probe_painter(_region: &mut NodeRegion<'_, '_>, _ctx: NodePaintCtx<'_>) {}

    struct Card<'a> {
        title: &'a str,
        rows: &'a str,
    }

    impl<'a> NodeContent<'a> for Card<'a> {
        fn label(&self) -> &'a str {
            self.title
        }

        fn size(&self) -> (usize, usize) {
            (self.title.chars().count() + 4, 4)
        }

        fn painter(&self) -> Option<NodePaintFn> {
            Some(probe_painter)
        }

        fn payload(&self) -> &'a str {
            self.rows
        }
    }

    #[test]
    fn str_is_simple_with_classic_footprint() {
        let s: &str = "Client";
        assert_eq!(s.label(), "Client");
        assert_eq!(s.size(), (8, 1)); // [Client]
        assert_eq!(s.kind(), NodeKindTag::Simple);
        assert!(s.painter().is_none());
        assert_eq!(s.payload(), "");
    }

    #[test]
    fn built_ins_declare_their_kinds_and_sizes() {
        assert_eq!(SimpleNode("AB").size(), (4, 1));
        assert_eq!(SimpleNode("AB").kind(), NodeKindTag::Simple);
        assert_eq!(BoxedNode("AB").size(), (6, 3)); // D4: label+4 × 3
        assert_eq!(BoxedNode("AB").kind(), NodeKindTag::Boxed);
    }

    #[test]
    fn references_forward_everything() {
        let card = Card {
            title: "T",
            rows: "data",
        };
        let r = &card;
        assert_eq!(r.label(), "T");
        assert_eq!(r.size(), (5, 4));
        assert_eq!(r.kind(), NodeKindTag::Custom);
        assert!(r.painter().is_some());
        assert_eq!(r.payload(), "data");
        // Double reference rides the same impl.
        assert_eq!((&r).label(), "T");
    }

    #[test]
    fn custom_node_is_the_plain_five_field_form() {
        let n = CustomNode {
            label: "L",
            width: 7,
            height: 2,
            painter: None,
            payload: "p",
        };
        assert_eq!(n.size(), (7, 2));
        assert_eq!(n.kind(), NodeKindTag::Custom);
        assert!(n.painter().is_none()); // blank node
        assert_eq!(n.payload(), "p");
    }

    #[test]
    fn tag_round_trips_and_unknown_falls_back_to_simple() {
        for tag in [NodeKindTag::Simple, NodeKindTag::Boxed, NodeKindTag::Custom] {
            assert_eq!(NodeKindTag::from_u8(tag.to_u8()), tag);
        }
        assert_eq!(NodeKindTag::from_u8(250), NodeKindTag::Simple);
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn string_reference_still_compiles() {
        let owned = alloc::string::String::from("Owned");
        let r: &alloc::string::String = &owned;
        assert_eq!(r.label(), "Owned");
        assert_eq!(r.kind(), NodeKindTag::Simple);
    }
}
