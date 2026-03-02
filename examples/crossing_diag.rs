/// Diagnostic: verify crossing reduction actually reorders nodes.

use ascii_dag::graph::DAG;
use ascii_dag::QUALITY;

#[cfg(feature = "arena")]
use ascii_dag::graph::arena::Arena;

fn main() {
    println!("=== Crossing Reduction Diagnostic ===\n");

    test_forced_crossing();
    test_triple_crossing();
    test_interleaved_edges();

    #[cfg(feature = "arena")]
    {
        println!("\n=== Arena Path ===\n");
        test_forced_crossing_arena();
        test_interleaved_arena();
    }
}

fn test_forced_crossing() {
    println!("--- Test: Forced Crossing (heap) ---");
    println!("  Nodes: D(0), C(1), A(2), B(3). Edges: A->C, B->D");
    println!("  Natural level-1 order: [D, C] -> 1 crossing");
    println!("  Optimal level-1 order: [C, D] -> 0 crossings");

    // No crossing reduction
    let ir_none = build_forced_crossing_dag(true);
    print_node_positions("  No-CR", &ir_none);

    // Quality
    let ir_quality = build_forced_crossing_dag(false);
    print_node_positions("  Quality", &ir_quality);

    let nodes_none: Vec<_> = ir_none.nodes().iter().collect();
    let nodes_qual: Vec<_> = ir_quality.nodes().iter().collect();
    let c_none = nodes_none.iter().find(|n| n.label == "C").unwrap();
    let d_none = nodes_none.iter().find(|n| n.label == "D").unwrap();
    let c_qual = nodes_qual.iter().find(|n| n.label == "C").unwrap();
    let d_qual = nodes_qual.iter().find(|n| n.label == "D").unwrap();

    println!("  No-CR:   C.x={} D.x={} (D before C? {})",
             c_none.x, d_none.x, d_none.x < c_none.x);
    println!("  Quality: C.x={} D.x={} (C before D? {})",
             c_qual.x, d_qual.x, c_qual.x < d_qual.x);

    if d_none.x < c_none.x && c_qual.x < d_qual.x {
        println!("  PASS: no-CR has crossing, quality fixes it\n");
    } else if c_none.x < d_none.x {
        println!("  NOTE: no-CR already optimal order\n");
    } else {
        println!("  FAIL: quality didn't fix the crossing\n");
    }
}

fn build_forced_crossing_dag(no_cr: bool) -> ascii_dag::LayoutIR<'static> {
    let mut dag = DAG::new();
    dag.add_node(0, "D");
    dag.add_node(1, "C");
    dag.add_node(2, "A");
    dag.add_node(3, "B");
    dag.add_edge(2, 1, None); // A -> C
    dag.add_edge(3, 0, None); // B -> D

    if no_cr {
        dag.set_crossing_pipeline(&[]);
    } else {
        dag.set_crossing_pipeline(QUALITY);
    }
    dag.compute_layout()
}

fn test_triple_crossing() {
    println!("--- Test: Triple Crossing (heap) ---");
    println!("  Natural level-1: [Z, Y, X]. Optimal: [X, Y, Z]");

    let mut dag_none = DAG::new();
    dag_none.add_node(0, "Z");
    dag_none.add_node(1, "Y");
    dag_none.add_node(2, "X");
    dag_none.add_node(3, "A");
    dag_none.add_node(4, "B");
    dag_none.add_node(5, "C");
    dag_none.add_edge(3, 2, None);
    dag_none.add_edge(4, 1, None);
    dag_none.add_edge(5, 0, None);

    dag_none.set_crossing_pipeline(&[]);
    let ir_none = dag_none.compute_layout();
    print_node_positions("  No-CR", &ir_none);

    dag_none.set_crossing_pipeline(QUALITY);
    let ir_quality = dag_none.compute_layout();
    print_node_positions("  Quality", &ir_quality);

    let nodes_none: Vec<_> = ir_none.nodes().iter().collect();
    let z_none = nodes_none.iter().find(|n| n.label == "Z").unwrap();
    let x_none = nodes_none.iter().find(|n| n.label == "X").unwrap();
    println!("  No-CR:   Z.x={} X.x={} (reversed? {})",
             z_none.x, x_none.x, z_none.x < x_none.x);

    let nodes_qual: Vec<_> = ir_quality.nodes().iter().collect();
    let x_q = nodes_qual.iter().find(|n| n.label == "X").unwrap();
    let y_q = nodes_qual.iter().find(|n| n.label == "Y").unwrap();
    let z_q = nodes_qual.iter().find(|n| n.label == "Z").unwrap();

    if x_q.x < y_q.x && y_q.x < z_q.x {
        println!("  PASS: X < Y < Z\n");
    } else {
        println!("  FAIL: X(x={}) Y(x={}) Z(x={})\n", x_q.x, y_q.x, z_q.x);
    }
}

fn test_interleaved_edges() {
    println!("--- Test: Interleaved 4x4 (heap) ---");
    println!("  Natural level-1: [C4, C3, C2, C1] -> 6 crossings");
    println!("  Optimal: [C1, C2, C3, C4] -> 0 crossings");

    let mut dag = DAG::new();
    dag.add_node(0, "C4");
    dag.add_node(1, "C3");
    dag.add_node(2, "C2");
    dag.add_node(3, "C1");
    dag.add_node(4, "P1");
    dag.add_node(5, "P2");
    dag.add_node(6, "P3");
    dag.add_node(7, "P4");
    dag.add_edge(4, 3, None);
    dag.add_edge(5, 2, None);
    dag.add_edge(6, 1, None);
    dag.add_edge(7, 0, None);

    dag.set_crossing_pipeline(&[]);
    let ir_none = dag.compute_layout();
    print_node_positions("  No-CR", &ir_none);

    dag.set_crossing_pipeline(QUALITY);
    let ir_quality = dag.compute_layout();
    print_node_positions("  Quality", &ir_quality);

    let nodes_none: Vec<_> = ir_none.nodes().iter().collect();
    let c4_none = nodes_none.iter().find(|n| n.label == "C4").unwrap();
    let c1_none = nodes_none.iter().find(|n| n.label == "C1").unwrap();
    println!("  No-CR:   C4.x={} C1.x={} (C4 first? {})",
             c4_none.x, c1_none.x, c4_none.x < c1_none.x);

    let nodes_qual: Vec<_> = ir_quality.nodes().iter().collect();
    let positions: Vec<(&str, usize)> = ["C1", "C2", "C3", "C4"]
        .iter()
        .map(|name| {
            let n = nodes_qual.iter().find(|n| n.label == *name).unwrap();
            (*name, n.x)
        })
        .collect();
    let sorted = positions.windows(2).all(|w| w[0].1 < w[1].1);
    if sorted {
        println!("  PASS: {:?}\n", positions);
    } else {
        println!("  FAIL: {:?}\n", positions);
    }
}

#[cfg(feature = "arena")]
fn test_forced_crossing_arena() {
    println!("--- Test: Forced Crossing (arena) ---");

    // No crossing reduction
    let mut dag = DAG::new();
    dag.add_node(0, "D");
    dag.add_node(1, "C");
    dag.add_node(2, "A");
    dag.add_node(3, "B");
    dag.add_edge(2, 1, None);
    dag.add_edge(3, 0, None);

    dag.set_crossing_pipeline(&[]);
    let mut temp_buf = vec![0u8; 65536];
    let mut out_buf = vec![0u8; 65536];
    let mut temp = Arena::new(&mut temp_buf);
    let mut out = Arena::new(&mut out_buf);
    match dag.compute_layout_arena(&mut temp, &mut out) {
        Ok(ir) => {
            print!("  Arena no-CR: ");
            for n in ir.nodes() {
                print!("id{}(L{},x{}) ", n.id, n.level, n.x);
            }
            println!();
        }
        Err(e) => println!("  Arena no-CR error: {:?}", e),
    }

    dag.set_crossing_pipeline(QUALITY);
    let mut temp_buf = vec![0u8; 65536];
    let mut out_buf = vec![0u8; 65536];
    let mut temp = Arena::new(&mut temp_buf);
    let mut out = Arena::new(&mut out_buf);
    match dag.compute_layout_arena(&mut temp, &mut out) {
        Ok(ir) => {
            print!("  Arena quality: ");
            for n in ir.nodes() {
                print!("id{}(L{},x{}) ", n.id, n.level, n.x);
            }
            println!();
        }
        Err(e) => println!("  Arena quality error: {:?}", e),
    }
}

#[cfg(feature = "arena")]
fn test_interleaved_arena() {
    println!("--- Test: Interleaved 4x4 (arena) ---");

    let mut dag = DAG::new();
    dag.add_node(0, "C4");
    dag.add_node(1, "C3");
    dag.add_node(2, "C2");
    dag.add_node(3, "C1");
    dag.add_node(4, "P1");
    dag.add_node(5, "P2");
    dag.add_node(6, "P3");
    dag.add_node(7, "P4");
    dag.add_edge(4, 3, None);
    dag.add_edge(5, 2, None);
    dag.add_edge(6, 1, None);
    dag.add_edge(7, 0, None);

    dag.set_crossing_pipeline(&[]);
    let mut temp_buf = vec![0u8; 65536];
    let mut out_buf = vec![0u8; 65536];
    let mut temp = Arena::new(&mut temp_buf);
    let mut out = Arena::new(&mut out_buf);
    match dag.compute_layout_arena(&mut temp, &mut out) {
        Ok(ir) => {
            print!("  Arena no-CR: ");
            for n in ir.nodes() {
                print!("id{}(L{},x{}) ", n.id, n.level, n.x);
            }
            println!();
        }
        Err(e) => println!("  Arena no-CR error: {:?}", e),
    }

    dag.set_crossing_pipeline(QUALITY);
    let mut temp_buf = vec![0u8; 65536];
    let mut out_buf = vec![0u8; 65536];
    let mut temp = Arena::new(&mut temp_buf);
    let mut out = Arena::new(&mut out_buf);
    match dag.compute_layout_arena(&mut temp, &mut out) {
        Ok(ir) => {
            print!("  Arena quality: ");
            for n in ir.nodes() {
                print!("id{}(L{},x{}) ", n.id, n.level, n.x);
            }
            println!();
        }
        Err(e) => println!("  Arena quality error: {:?}", e),
    }
}

/// Skip-level edges: tests that dummy-to-dummy crossing reduction works.
///
/// Level 0: [A(5), B(6)]
/// Level 1: [dummy-for-A->X, dummy-for-B->Y] (these are skip-level intermediates)
/// Level 2: [Y(0), X(1)]  <-- natural order creates crossing
///
/// A(5) -> X(1) skips level 1 (creates dummy)
/// B(6) -> Y(0) skips level 1 (creates dummy)
///
/// With quality CR: both the dummy layer AND real level 2 should be reordered
/// to eliminate crossings.
fn test_skip_level_crossing() {
    println!("--- Test: Skip-level crossing (heap) ---");

    let mut dag = DAG::new();
    dag.add_node(0, "Y");  // child of B, level 2
    dag.add_node(1, "X");  // child of A, level 2
    dag.add_node(2, "M");  // mid-level node, level 1
    dag.add_node(3, "N");  // mid-level node, level 1
    dag.add_node(4, "P");  // another level-0 parent
    dag.add_node(5, "A");  // parent, level 0
    dag.add_node(6, "B");  // parent, level 0

    dag.add_edge(5, 1, None); // A -> X (skip: 0 -> 2)
    dag.add_edge(6, 0, None); // B -> Y (skip: 0 -> 2)
    dag.add_edge(5, 2, None); // A -> M (0 -> 1)
    dag.add_edge(6, 3, None); // B -> N (0 -> 1)
    dag.add_edge(2, 1, None); // M -> X (1 -> 2)
    dag.add_edge(3, 0, None); // N -> Y (1 -> 2)

    dag.set_crossing_pipeline(&[]);
    let ir_none = dag.compute_layout();
    print_node_positions("  No-CR", &ir_none);

    dag.set_crossing_pipeline(QUALITY);
    let ir_quality = dag.compute_layout();
    print_node_positions("  Quality", &ir_quality);

    // Check if X and Y are properly ordered
    let nodes_qual: Vec<_> = ir_quality.nodes().iter().collect();
    let x_q = nodes_qual.iter().find(|n| n.label == "X").unwrap();
    let y_q = nodes_qual.iter().find(|n| n.label == "Y").unwrap();
    if x_q.x < y_q.x {
        println!("  PASS: X before Y -- skip-level crossing eliminated\n");
    } else {
        println!("  FAIL: Y(x={}) X(x={}) -- skip-level crossing remains\n", y_q.x, x_q.x);
    }
}

/// Arena buffer overflow test: graph where a level has more vnodes than node_count.
///
/// 3 real nodes + many skip-level edges -> many dummies per level  
/// node_count=3, but a single level could have 3+ dummies.
#[cfg(feature = "arena")]
fn test_arena_buffer_overflow() {
    println!("--- Test: Arena buffer sizing (arena) ---");

    // Create graph with many skip-level edges
    // 4 sources at level 0, 4 sinks at level 3
    // Each source connects to each sink -> skip 2 levels -> 2 dummies per edge
    // Level 1 and 2 will each have 16 dummy nodes with only 8 real nodes
    let mut dag = DAG::new();
    // Sinks first (lower IDs for adversarial ordering)
    dag.add_node(0, "S4");
    dag.add_node(1, "S3");
    dag.add_node(2, "S2");
    dag.add_node(3, "S1");
    // Sources
    dag.add_node(4, "P1");
    dag.add_node(5, "P2");
    dag.add_node(6, "P3");
    dag.add_node(7, "P4");
    // Mid-level node to force level 3 assignment
    dag.add_node(8, "M1");
    dag.add_node(9, "M2");
    dag.add_edge(4, 8, None); // P1 -> M1
    dag.add_edge(5, 9, None); // P2 -> M2
    dag.add_edge(8, 3, None); // M1 -> S1
    dag.add_edge(9, 2, None); // M2 -> S2
    dag.add_edge(8, 0, None); // M1 -> S4
    dag.add_edge(9, 1, None); // M2 -> S3
    // Skip-level edges
    dag.add_edge(6, 0, None); // P3 -> S4 (skip 2)
    dag.add_edge(7, 1, None); // P4 -> S3 (skip 2)

    dag.set_crossing_pipeline(QUALITY);
    let mut temp_buf = vec![0u8; 131072];
    let mut out_buf = vec![0u8; 131072];
    let mut temp = Arena::new(&mut temp_buf);
    let mut out = Arena::new(&mut out_buf);
    match dag.compute_layout_arena(&mut temp, &mut out) {
        Ok(ir) => {
            print!("  Arena quality (no crash): ");
            for n in ir.nodes() {
                print!("id{}(L{},x{}) ", n.id, n.level, n.x);
            }
            println!();
            println!("  PASS: No buffer overflow\n");
        }
        Err(e) => println!("  Result: {:?}\n", e),
    }
}

fn print_node_positions(label: &str, ir: &ascii_dag::LayoutIR) {
    print!("{}: ", label);
    let mut nodes: Vec<_> = ir.nodes().iter().collect();
    nodes.sort_by_key(|n| (n.level, n.x));
    for n in &nodes {
        print!("{}(L{},x{}) ", n.label, n.level, n.x);
    }
    println!();
}
