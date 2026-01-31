//! Edge case stress test - tries to break the library
//! Run with: cargo run --example edge_case_stress --release

use ascii_dag::graph::DAG;

fn main() {
    println!("=== ASCII-DAG Edge Case Stress Test ===\n");
    println!("Testing with panic=\"abort\" - any panic = immediate crash!\n");

    let mut passed = 0;
    let mut failed = 0;

    // Helper macro to run tests
    macro_rules! test {
        ($name:expr, $code:expr) => {
            print!("Testing: {} ... ", $name);
            match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| $code)) {
                Ok(_) => {
                    println!("✓ PASSED");
                    passed += 1;
                }
                Err(_) => {
                    println!("✗ PANICKED!");
                    failed += 1;
                }
            }
        };
    }

    // ============================================
    // EMPTY / MINIMAL CASES
    // ============================================
    println!("\n--- Empty/Minimal Cases ---");

    test!("Empty DAG", {
        let dag: DAG = DAG::new();
        let _ = dag.render();
    });

    test!("Single node", {
        let dag = DAG::from_edges(&[(1, "alone")], &[]);
        let _ = dag.render();
    });

    test!("Single node with empty label", {
        let dag = DAG::from_edges(&[(1, "")], &[]);
        let _ = dag.render();
    });

    // ============================================
    // STRING EDGE CASES
    // ============================================
    println!("\n--- String Edge Cases ---");

    test!("Very long label (1000 chars)", {
        let long_label: String = "x".repeat(1000);
        let dag = DAG::from_edges(&[(1, &long_label)], &[]);
        let _ = dag.render();
    });

    test!("Very long label (10000 chars)", {
        let long_label: String = "x".repeat(10000);
        let dag = DAG::from_edges(&[(1, &long_label)], &[]);
        let _ = dag.render();
    });

    test!("Unicode: emoji", {
        let dag = DAG::from_edges(&[(1, "🔥 Fire"), (2, "💧 Water")], &[(1, 2)]);
        let _ = dag.render();
    });

    test!("Unicode: CJK characters", {
        let dag = DAG::from_edges(&[(1, "日本語"), (2, "中文")], &[(1, 2)]);
        let _ = dag.render();
    });

    test!("Unicode: RTL (Arabic)", {
        let dag = DAG::from_edges(&[(1, "مرحبا"), (2, "عالم")], &[(1, 2)]);
        let _ = dag.render();
    });

    test!("Unicode: combining characters", {
        let dag = DAG::from_edges(
            &[(1, "e\u{0301}"), (2, "n\u{0303}")], // é and ñ as combining
            &[(1, 2)],
        );
        let _ = dag.render();
    });

    test!("Whitespace only labels", {
        let dag = DAG::from_edges(&[(1, "   "), (2, "\t\t"), (3, "  \n  ")], &[(1, 2), (2, 3)]);
        let _ = dag.render();
    });

    test!("Null byte in label", {
        let dag = DAG::from_edges(&[(1, "before\0after")], &[]);
        let _ = dag.render();
    });

    test!("Control characters", {
        let dag = DAG::from_edges(&[(1, "\x01\x02\x03")], &[]);
        let _ = dag.render();
    });

    test!("Box drawing characters (conflict with rendering)", {
        let dag = DAG::from_edges(&[(1, "┌─┐│└┘├┤┬┴┼"), (2, "───────")], &[(1, 2)]);
        let _ = dag.render();
    });

    // ============================================
    // GRAPH STRUCTURE EDGE CASES
    // ============================================
    println!("\n--- Graph Structure Edge Cases ---");

    test!("Self-reference (node points to itself)", {
        let dag = DAG::from_edges(&[(1, "self")], &[(1, 1)]);
        let _ = dag.render();
    });

    test!("Circular dependency (A->B->A)", {
        let dag = DAG::from_edges(&[(1, "A"), (2, "B")], &[(1, 2), (2, 1)]);
        let _ = dag.render();
    });

    test!("Edge to non-existent node", {
        let dag = DAG::from_edges(&[(1, "exists")], &[(1, 999)]);
        let _ = dag.render();
    });

    test!("Edge from non-existent node", {
        let dag = DAG::from_edges(&[(1, "exists")], &[(999, 1)]);
        let _ = dag.render();
    });

    test!("Duplicate edges", {
        let dag = DAG::from_edges(&[(1, "A"), (2, "B")], &[(1, 2), (1, 2), (1, 2)]);
        let _ = dag.render();
    });

    test!("Duplicate node IDs", {
        let dag = DAG::from_edges(&[(1, "first"), (1, "second"), (1, "third")], &[]);
        let _ = dag.render();
    });

    test!("Deep linear chain (1000 nodes)", {
        let nodes: Vec<(usize, &str)> = (0..1000).map(|i| (i, "node")).collect();
        let edges: Vec<(usize, usize)> = (0..999).map(|i| (i, i + 1)).collect();
        let dag = DAG::from_edges(&nodes, &edges);
        let _ = dag.render();
    });

    test!("Wide graph (1000 siblings)", {
        let mut nodes: Vec<(usize, &str)> = vec![(0, "root")];
        nodes.extend((1..=1000).map(|i| (i, "child")));
        let edges: Vec<(usize, usize)> = (1..=1000).map(|i| (0, i)).collect();
        let dag = DAG::from_edges(&nodes, &edges);
        let _ = dag.render();
    });

    test!("Diamond pattern (4 nodes)", {
        let dag = DAG::from_edges(
            &[(1, "top"), (2, "left"), (3, "right"), (4, "bottom")],
            &[(1, 2), (1, 3), (2, 4), (3, 4)],
        );
        let _ = dag.render();
    });

    test!("Complex diamond (100 converging paths)", {
        let mut nodes: Vec<(usize, &str)> = vec![(0, "root")];
        nodes.extend((1..=100).map(|i| (i, "mid")));
        nodes.push((101, "sink"));

        let mut edges: Vec<(usize, usize)> = (1..=100).map(|i| (0, i)).collect();
        edges.extend((1..=100).map(|i| (i, 101)));

        let dag = DAG::from_edges(&nodes, &edges);
        let _ = dag.render();
    });

    test!("Many roots (disconnected nodes)", {
        let nodes: Vec<(usize, &str)> = (0..100).map(|i| (i, "root")).collect();
        let dag = DAG::from_edges(&nodes, &[]);
        let _ = dag.render();
    });

    test!("Cross-level edges (skip levels)", {
        let dag = DAG::from_edges(
            &[(0, "L0"), (1, "L1"), (2, "L2"), (3, "L3"), (4, "L4")],
            &[(0, 1), (1, 2), (2, 3), (3, 4), (0, 4)], // Skip from L0 to L4
        );
        let _ = dag.render();
    });

    // ============================================
    // NUMERIC/SIZE EDGE CASES
    // ============================================
    println!("\n--- Numeric/Size Edge Cases ---");

    test!("Node with many parents (100)", {
        let mut nodes: Vec<(usize, &str)> = (0..100).map(|i| (i, "parent")).collect();
        nodes.push((100, "child"));
        let edges: Vec<(usize, usize)> = (0..100).map(|i| (i, 100)).collect();
        let dag = DAG::from_edges(&nodes, &edges);
        let _ = dag.render();
    });

    test!("Large node ID (usize::MAX / 2)", {
        let big_id = usize::MAX / 2;
        let dag = DAG::from_edges(&[(big_id, "big")], &[]);
        let _ = dag.render();
    });

    test!("Zero as node ID", {
        let dag = DAG::from_edges(&[(0, "zero")], &[]);
        let _ = dag.render();
    });

    // ============================================
    // API EDGE CASES
    // ============================================
    println!("\n--- API Edge Cases ---");

    test!("Builder: add_node without edges", {
        let mut dag = DAG::new();
        dag.add_node(1, "lonely");
        let _ = dag.render();
    });

    test!("Builder: add_edge before nodes", {
        let mut dag = DAG::new();
        dag.add_edge(1, 2, None); // Nodes don't exist yet
        dag.add_node(1, "A");
        dag.add_node(2, "B");
        let _ = dag.render();
    });

    test!("Builder: add_edge only (no explicit nodes)", {
        let mut dag = DAG::new();
        dag.add_edge(1, 2, None);
        dag.add_edge(2, 3, None);
        let _ = dag.render();
    });

    test!("Render multiple times", {
        let dag = DAG::from_edges(&[(1, "A"), (2, "B")], &[(1, 2)]);
        for _ in 0..100 {
            let _ = dag.render();
        }
    });

    test!("Clone and render", {
        let dag = DAG::from_edges(&[(1, "A"), (2, "B")], &[(1, 2)]);
        let dag2 = dag.clone();
        let _ = dag.render();
        let _ = dag2.render();
    });

    // ============================================
    // RENDER MODE EDGE CASES
    // ============================================
    println!("\n--- Render Mode Edge Cases ---");

    test!("Horizontal mode", {
        let mut dag = DAG::from_edges(&[(1, "A"), (2, "B")], &[(1, 2)]);
        dag.set_render_mode(ascii_dag::graph::RenderMode::Horizontal);
        let _ = dag.render();
    });

    test!("Vertical mode", {
        let mut dag = DAG::from_edges(&[(1, "A"), (2, "B")], &[(1, 2)]);
        dag.set_render_mode(ascii_dag::graph::RenderMode::Vertical);
        let _ = dag.render();
    });

    test!("Mode on complex graph", {
        let mut dag = DAG::from_edges(
            &[(1, "A"), (2, "B"), (3, "C"), (4, "D")],
            &[(1, 2), (1, 3), (2, 4), (3, 4)],
        );
        dag.set_render_mode(ascii_dag::graph::RenderMode::Horizontal);
        let _ = dag.render();
    });

    // ============================================
    // STRESS COMBINATIONS
    // ============================================
    println!("\n--- Stress Combinations ---");

    test!("Long labels + deep chain", {
        let long = "abcdefghij".repeat(10);
        let nodes: Vec<(usize, String)> = (0..50).map(|i| (i, long.clone())).collect();
        let nodes_ref: Vec<(usize, &str)> = nodes.iter().map(|(i, s)| (*i, s.as_str())).collect();
        let edges: Vec<(usize, usize)> = (0..49).map(|i| (i, i + 1)).collect();
        let dag = DAG::from_edges(&nodes_ref, &edges);
        let _ = dag.render();
    });

    test!("Wide + deep (grid-like)", {
        // 10 levels, 10 nodes each
        let mut nodes: Vec<(usize, &str)> = Vec::new();
        let mut edges: Vec<(usize, usize)> = Vec::new();

        for level in 0..10 {
            for pos in 0..10 {
                let id = level * 10 + pos;
                nodes.push((id, "n"));
                if level > 0 {
                    // Connect to node above
                    edges.push((id - 10, id));
                }
            }
        }
        let dag = DAG::from_edges(&nodes, &edges);
        let _ = dag.render();
    });

    // ============================================
    // SUMMARY
    // ============================================
    println!("\n===========================================");
    println!("RESULTS: {} passed, {} failed", passed, failed);
    println!("===========================================");

    if failed > 0 {
        println!("\n⚠️  WARNING: Some tests panicked!");
        println!("With panic=\"abort\", these would crash the user's program!");
        std::process::exit(1);
    } else {
        println!("\n✓ All edge cases handled gracefully!");
        println!("The library is safe to use with panic=\"abort\"");
    }
}
