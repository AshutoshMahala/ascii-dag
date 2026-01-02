//! Edge case tests to identify rendering issues and limitations.

use ascii_dag::graph::DAG;

fn main() {
    println!("=== Edge Case Rendering Tests ===\n");

    // Edge case 1: Asymmetric diamond
    test_asymmetric_diamond();

    // Edge case 2: Skip-level connections
    test_skip_level();

    // Edge case 3: Very wide (7+ nodes at same level)
    test_very_wide();

    // Edge case 4: Cross connections
    test_cross_connections();

    // Edge case 5: Multiple independent subgraphs
    test_disconnected_subgraphs();

    // Edge case 6: Single path with many skip connections
    test_highway_with_exits();

    // Edge case 7: Nested diamonds
    test_nested_diamonds();

    // Edge case 8: Partial grid (some connections missing)
    test_partial_grid();

    // Edge case 9: Fan-in then fan-out
    test_hourglass();

    // Edge case 10: Complex real-world scenario - build pipeline
    test_build_pipeline();
}

fn test_asymmetric_diamond() {
    println!("1. Asymmetric Diamond (different path lengths):");
    let dag = DAG::from_edges(
        &[
            (1, "Root"),
            (2, "Short"),
            (3, "Long1"),
            (4, "Long2"),
            (5, "Merge"),
        ],
        &[
            (1, 2),
            (1, 3),
            (2, 5), // short path
            (3, 4),
            (4, 5), // long path
        ],
    );
    println!("{}", dag.render());
    println!();
}

fn test_skip_level() {
    println!("2. Skip-Level Connections (A→D skipping B,C):");
    let dag = DAG::from_edges(
        &[(1, "A"), (2, "B"), (3, "C"), (4, "D")],
        &[
            (1, 2),
            (2, 3),
            (3, 4),
            (1, 4), // skip connection
        ],
    );
    println!("{}", dag.render());
    println!();
}

fn test_very_wide() {
    println!("3. Very Wide (8 children):");
    let dag = DAG::from_edges(
        &[
            (0, "Root"),
            (1, "N1"),
            (2, "N2"),
            (3, "N3"),
            (4, "N4"),
            (5, "N5"),
            (6, "N6"),
            (7, "N7"),
            (8, "N8"),
        ],
        &[
            (0, 1),
            (0, 2),
            (0, 3),
            (0, 4),
            (0, 5),
            (0, 6),
            (0, 7),
            (0, 8),
        ],
    );
    println!("{}", dag.render());
    println!();
}

fn test_cross_connections() {
    println!("4. Cross Connections (A1→B2, A2→B1):");
    let dag = DAG::from_edges(
        &[(1, "A1"), (2, "A2"), (3, "B1"), (4, "B2")],
        &[
            (1, 3), // A1 → B1
            (1, 4), // A1 → B2
            (2, 3), // A2 → B1
            (2, 4), // A2 → B2
        ],
    );
    println!("{}", dag.render());
    println!();
}

fn test_disconnected_subgraphs() {
    println!("5. Disconnected Subgraphs (2 separate DAGs):");
    let dag = DAG::from_edges(
        &[(1, "X1"), (2, "X2"), (3, "X3"), (10, "Y1"), (11, "Y2")],
        &[
            (1, 2),
            (2, 3),   // X chain
            (10, 11), // Y chain
        ],
    );
    println!("{}", dag.render());
    println!();
}

fn test_highway_with_exits() {
    println!("6. Highway with Exits (main path + skip connections):");
    let dag = DAG::from_edges(
        &[
            (1, "Start"),
            (2, "Stop1"),
            (3, "Stop2"),
            (4, "Stop3"),
            (5, "End"),
        ],
        &[
            (1, 2),
            (2, 3),
            (3, 4),
            (4, 5), // main path
            (1, 3), // skip 1
            (2, 4), // skip 1
            (1, 5), // skip all
        ],
    );
    println!("{}", dag.render());
    println!();
}

fn test_nested_diamonds() {
    println!("7. Nested Diamonds (diamond within diamond):");
    let dag = DAG::from_edges(
        &[
            (1, "Top"),
            (2, "L"),
            (3, "R"),
            (4, "LeftLeft"),
            (5, "LeftRright"),
            (6, "RightLeft"),
            (7, "RightRight"),
            (8, "Bot"),
        ],
        &[
            (1, 2),
            (1, 3), // outer top
            (2, 4),
            (2, 5), // left branch splits
            (3, 6),
            (3, 7), // right branch splits
            (4, 8),
            (5, 8), // left merges
            (6, 8),
            (7, 8), // right merges
        ],
    );
    println!("{}", dag.render());
    println!();
}

fn test_partial_grid() {
    println!("8. Partial Grid (missing some connections):");
    let dag = DAG::from_edges(
        &[
            (1, "A1"),
            (2, "A2"),
            (3, "A3"),
            (4, "B1"),
            (5, "B2"),
            (6, "B3"),
            (7, "C1"),
            (8, "C2"),
            (9, "C3"),
        ],
        &[
            (1, 4), // A1 → B1 only
            (2, 4),
            (2, 5),
            (2, 6), // A2 → all B
            (3, 6), // A3 → B3 only
            (4, 7),
            (4, 8), // B1 → C1, C2
            (5, 8), // B2 → C2 only
            (6, 8),
            (6, 9), // B3 → C2, C3
        ],
    );
    println!("{}", dag.render());
    println!();
}

fn test_hourglass() {
    println!("9. Hourglass (fan-in then fan-out):");
    let dag = DAG::from_edges(
        &[
            (1, "In1"),
            (2, "In2"),
            (3, "In3"),
            (4, "Center"),
            (5, "Out1"),
            (6, "Out2"),
            (7, "Out3"),
        ],
        &[
            (1, 4),
            (2, 4),
            (3, 4), // fan-in
            (4, 5),
            (4, 6),
            (4, 7), // fan-out
        ],
    );
    println!("{}", dag.render());
    println!();
}

fn test_build_pipeline() {
    println!("10. Build Pipeline (realistic example):");
    let dag = DAG::from_edges(
        &[
            (1, "Checkout"),
            (2, "Install"),
            (3, "Lint"),
            (4, "Test"),
            (5, "Build"),
            (6, "UnitTests"),
            (7, "IntegTests"),
            (8, "Coverage"),
            (9, "Package"),
            (10, "Deploy"),
        ],
        &[
            (1, 2), // Checkout → Install
            (2, 3),
            (2, 4),
            (2, 5), // Install → Lint, Test, Build
            (3, 9), // Lint → Package (if lint passes)
            (4, 6),
            (4, 7), // Test → UnitTests, IntegTests
            (5, 9), // Build → Package
            (6, 8),
            (7, 8),  // Tests → Coverage
            (8, 9),  // Coverage → Package
            (9, 10), // Package → Deploy
        ],
    );
    println!("{}", dag.render());
    println!();
}
