//! `Graph::render()` facade snapshots.
//!
//! `Graph::render()` is a FACADE, not a pure wrapper over the scene
//! pipeline: it keeps two deliberate non-scene paths — the
//! cyclic-dependency banner and the simple-chain shortcut — and
//! delegates only the normal layout path to plan→compose→emit.
//! Removing either special path would be an intentional output break;
//! these snapshots pin them byte-for-byte so any change to their
//! output is a conscious decision, never drift.

#![cfg(feature = "layout-vertical")]

use ascii_dag::Graph;

/// A graph with a cycle never reaches layout: the facade prints the
/// banner with the offending chain (auto-created nodes in `⟨⟩`,
/// declared ones in `[]`).
#[test]
fn cycle_banner_is_pinned() {
    let mut g = Graph::new();
    g.add_node(1, "A");
    g.add_node(2, "B");
    g.add_edge(1, 2, None);
    g.add_edge(2, 1, None);
    assert_eq!(
        g.render(),
        "⚠\u{fe0f}  CYCLE DETECTED - Not a valid DAG\n\n\
         Cyclic dependency chain:\n\
         [A] → [B] ⇄ [A]\n\n\
         This creates an infinite loop in error dependencies.\n"
    );
}

/// A simple chain under `RenderMode::Auto` takes the one-line
/// horizontal shortcut instead of the layout pipeline (TopDown only —
/// the shortcut cannot honor a rank direction).
#[test]
fn simple_chain_shortcut_is_pinned() {
    let mut g = Graph::new();
    g.add_node(1, "fetch");
    g.add_node(2, "build");
    g.add_node(3, "deploy");
    g.add_edge(1, 2, None);
    g.add_edge(2, 3, None);
    assert_eq!(g.render(), "[fetch] → [build] → [deploy]\n");
}

/// The normal layout path — everything that is neither a cycle nor a
/// simple chain — goes through the scene pipeline: `render()` matches
/// the plain one-step wrapper byte-for-byte.
#[test]
fn normal_path_is_the_scene_pipeline() {
    let mut g = Graph::new();
    g.add_node(1, "a");
    g.add_node(2, "b");
    g.add_node(3, "c");
    g.add_edge(1, 2, None);
    g.add_edge(1, 3, None); // fan-out: not a simple chain
    let rendered = g.render();
    let ir = g.compute_layout();
    assert_eq!(
        rendered,
        ir.render_string(&ascii_dag::RenderOptions::plain())
    );
}
