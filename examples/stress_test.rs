use ascii_dag::DAG;

fn main() {
    println!("=== Stress Test: Breaking the Renderer ===\n");

    // Test 1: Very long vs very short labels
    println!("1. Mixed Label Lengths:");
    let dag = DAG::from_edges(
        &[
            (1, "A"),
            (2, "VeryVeryVeryLongErrorNameThatShouldBreakAlignment"),
            (3, "B"),
            (4, "AnotherExtremelyLongDiagnosticMessageHere"),
            (5, "C"),
        ],
        &[
            (1, 2),  // Short -> Long
            (2, 3),  // Long -> Short
            (3, 4),  // Short -> Long
            (4, 5),  // Long -> Short
        ],
    );
    println!("{}\n", dag.render());

    // Test 2: Extreme convergence (many -> one)
    println!("2. Extreme Convergence (10 errors -> 1):");
    let mut dag = DAG::new();
    dag.add_node(1, "E1");
    dag.add_node(2, "E2");
    dag.add_node(3, "E3");
    dag.add_node(4, "E4");
    dag.add_node(5, "E5");
    dag.add_node(6, "E6");
    dag.add_node(7, "E7");
    dag.add_node(8, "E8");
    dag.add_node(9, "E9");
    dag.add_node(10, "E10");
    dag.add_node(11, "Final");
    
    for i in 1..=10 {
        dag.add_edge(i, 11);
    }
    println!("{}\n", dag.render());

    // Test 3: Extreme divergence (one -> many)
    println!("3. Extreme Divergence (1 -> 8):");
    let dag = DAG::from_edges(
        &[
            (1, "Root"),
            (2, "Child1"), (3, "Child2"), (4, "Child3"), (5, "Child4"),
            (6, "Child5"), (7, "Child6"), (8, "Child7"), (9, "Child8"),
        ],
        &[
            (1, 2), (1, 3), (1, 4), (1, 5),
            (1, 6), (1, 7), (1, 8), (1, 9),
        ],
    );
    println!("{}\n", dag.render());

    // Test 4: Complex multi-layer DAG
    println!("4. Complex Multi-Layer DAG:");
    let dag = DAG::from_edges(
        &[
            (1, "L1A"), (2, "L1B"), (3, "L1C"),      // Layer 1: 3 nodes
            (4, "L2A"), (5, "L2B"),                  // Layer 2: 2 nodes
            (6, "L3A"), (7, "L3B"), (8, "L3C"),      // Layer 3: 3 nodes
            (9, "Final"),                             // Layer 4: 1 node
        ],
        &[
            // Layer 1 -> Layer 2
            (1, 4), (2, 4), (3, 5),
            // Layer 2 -> Layer 3
            (4, 6), (4, 7), (5, 7), (5, 8),
            // Layer 3 -> Layer 4
            (6, 9), (7, 9), (8, 9),
        ],
    );
    println!("{}\n", dag.render());

    // Test 5: Single character labels
    println!("5. Single Character Labels:");
    let dag = DAG::from_edges(
        &[(1, "A"), (2, "B"), (3, "C"), (4, "D"), (5, "E")],
        &[(1, 3), (2, 3), (3, 4), (3, 5)],
    );
    println!("{}\n", dag.render());

    // Test 6: Empty label (edge case)
    println!("6. Empty/Minimal Labels:");
    let dag = DAG::from_edges(
        &[(1, ""), (2, "X"), (3, "")],
        &[(1, 2), (2, 3)],
    );
    println!("{}\n", dag.render());

    // Test 7: Unicode in labels
    println!("7. Unicode Characters in Labels:");
    let dag = DAG::from_edges(
        &[
            (1, "🔴 Error"),
            (2, "⚠️ Warning"),
            (3, "✓ Fixed"),
        ],
        &[(1, 2), (2, 3)],
    );
    println!("{}\n", dag.render());

    // Test 8: Mixed long and short with convergence
    println!("8. Mixed Lengths + Convergence:");
    let dag = DAG::from_edges(
        &[
            (1, "ShortError1"),
            (2, "ThisIsAnExtremelyLongErrorMessageThatGoesOnAndOn"),
            (3, "Err3"),
            (4, "AnotherVeryLongDiagnosticNameHereForTesting"),
            (5, "Result"),
        ],
        &[
            (1, 5),
            (2, 5),
            (3, 5),
            (4, 5),
        ],
    );
    println!("{}\n", dag.render());

    // Test 9: Deep nesting
    println!("9. Deep Nesting (10 levels):");
    let dag = DAG::from_edges(
        &[
            (1, "Level1"), (2, "Level2"), (3, "Level3"), (4, "Level4"), (5, "Level5"),
            (6, "Level6"), (7, "Level7"), (8, "Level8"), (9, "Level9"), (10, "Level10"),
        ],
        &[
            (1, 2), (2, 3), (3, 4), (4, 5),
            (5, 6), (6, 7), (7, 8), (8, 9), (9, 10),
        ],
    );
    println!("{}\n", dag.render());

    // Test 10: Wide graph (5 parallel chains)
    println!("10. Wide Graph (5 parallel chains):");
    let dag = DAG::from_edges(
        &[
            (1, "C0L0"), (2, "C0L1"), (3, "C0L2"),
            (4, "C1L0"), (5, "C1L1"), (6, "C1L2"),
            (7, "C2L0"), (8, "C2L1"), (9, "C2L2"),
            (10, "C3L0"), (11, "C3L1"), (12, "C3L2"),
            (13, "C4L0"), (14, "C4L1"), (15, "C4L2"),
        ],
        &[
            (1, 2), (2, 3),
            (4, 5), (5, 6),
            (7, 8), (8, 9),
            (10, 11), (11, 12),
            (13, 14), (14, 15),
        ],
    );
    println!("{}\n", dag.render());

    // Test 11: Diamond within diamond
    println!("11. Diamond within Diamond:");
    let dag = DAG::from_edges(
        &[
            (1, "Root"),
            (2, "L1"), (3, "R1"),
            (4, "L2"), (5, "R2"),
            (6, "L3"), (7, "R3"),
            (8, "Merge1"),
            (9, "Merge2"),
        ],
        &[
            (1, 2), (1, 3),      // Root splits
            (2, 4), (2, 5),      // Left splits
            (3, 6), (3, 7),      // Right splits
            (4, 8), (5, 8),      // Left converges
            (6, 9), (7, 9),      // Right converges
        ],
    );
    println!("{}\n", dag.render());

    println!("=== Stress Test Complete ===");
}
