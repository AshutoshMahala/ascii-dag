//! Per-axis smoke tests for the layout-axis features: a
//! single-axis build must lay out and
//! render through its own axis end to end. The full golden/parity
//! suites pin exact output under the default (both-axes) build; these
//! only prove each gated configuration is alive at runtime.

use ascii_dag::graph::Graph;
use ascii_dag::render::engine::RenderOptions;
use ascii_dag::{Direction, LayoutConfig};

fn sibling_graph() -> Graph<'static> {
    let mut g = Graph::new();
    g.add_node(1usize, "P");
    g.add_node(2usize, "A");
    g.add_node(3usize, "B");
    g.add_edge(1usize, 2usize, None);
    g.add_edge(1usize, 3usize, None);
    g
}

fn smoke(direction: Direction) {
    let mut config = LayoutConfig::standard();
    config.direction = direction;
    let ir = sibling_graph().compute_layout_with_config(&config);
    assert!(ir.width() > 0 && ir.height() > 0);
    let out = ir.render_string(&RenderOptions::plain());
    for label in ["[P]", "[A]", "[B]"] {
        assert!(out.contains(label), "{direction:?}: missing {label}\n{out}");
    }
}

#[cfg(feature = "layout-vertical")]
#[test]
fn vertical_axis_is_alive() {
    smoke(Direction::TopDown);
    smoke(Direction::BottomUp);
    assert_eq!(Direction::default(), Direction::TopDown);
}

#[cfg(feature = "layout-horizontal")]
#[test]
fn horizontal_axis_is_alive() {
    smoke(Direction::LeftRight);
    smoke(Direction::RightLeft);
}

#[cfg(all(feature = "layout-horizontal", not(feature = "layout-vertical")))]
#[test]
fn horizontal_only_default_is_left_right() {
    assert_eq!(Direction::default(), Direction::LeftRight);
    // A disabled axis's string forms parse to an error whose message
    // names the feature to enable.
    let err = "TB".parse::<Direction>().unwrap_err();
    assert!(format!("{err}").contains("layout-vertical"), "{err}");
}
