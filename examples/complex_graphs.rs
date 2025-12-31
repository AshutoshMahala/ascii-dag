//! Test complex graph structures to identify rendering limitations and improvements.

use ascii_dag::graph::DAG;

fn main() {
    println!("=== Complex Graph Rendering Tests ===\n");

    // Test 1: Wide divergence (1 node to many)
    test_wide_divergence();

    // Test 2: Wide convergence (many nodes to 1)
    test_wide_convergence();

    // Test 3: Binary tree (balanced)
    test_binary_tree();

    // Test 4: Deep chain with side branches
    test_deep_with_branches();

    // Test 5: Grid-like structure
    test_grid_structure();

    // Test 6: Multiple roots merging
    test_multiple_roots();

    // Test 7: Cascade pattern
    test_cascade();

    // Test 8: Long labels
    test_long_labels();

    // Test 9: Mixed complexity
    test_mixed_complexity();

    // Test 10: Inverted tree (convergence at each level)
    test_inverted_tree();
}

fn test_wide_divergence() {
    println!("1. Wide Divergence (1 → 5 children):");
    let dag = DAG::from_edges(
        &[
            (1, "Root"),
            (2, "A"),
            (3, "B"),
            (4, "C"),
            (5, "D"),
            (6, "E"),
        ],
        &[(1, 2), (1, 3), (1, 4), (1, 5), (1, 6)],
    );
    println!("{}", dag.render());
    println!();
}

fn test_wide_convergence() {
    println!("2. Wide Convergence (5 → 1):");
    let dag = DAG::from_edges(
        &[
            (1, "A"),
            (2, "B"),
            (3, "C"),
            (4, "D"),
            (5, "E"),
            (6, "Sink"),
        ],
        &[(1, 6), (2, 6), (3, 6), (4, 6), (5, 6)],
    );
    println!("{}", dag.render());
    println!();
}

fn test_binary_tree() {
    println!("3. Binary Tree (3 levels):");
    let dag = DAG::from_edges(
        &[
            (1, "Root"),
            (2, "L1"),
            (3, "R1"),
            (4, "LL"),
            (5, "LR"),
            (6, "RL"),
            (7, "RR"),
        ],
        &[(1, 2), (1, 3), (2, 4), (2, 5), (3, 6), (3, 7)],
    );
    println!("{}", dag.render());
    println!();
}

fn test_deep_with_branches() {
    println!("4. Deep Chain with Side Branches:");
    let dag = DAG::from_edges(
        &[
            (1, "Main1"),
            (2, "Main2"),
            (3, "Main3"),
            (4, "Main4"),
            (10, "Branch1"),
            (20, "Branch2"),
        ],
        &[
            (1, 2),
            (2, 3),
            (3, 4),
            (1, 10), // branch from Main1
            (2, 20), // branch from Main2
        ],
    );
    println!("{}", dag.render());
    println!();
}

fn test_grid_structure() {
    println!("5. Grid-like Structure (2x3):");
    let dag = DAG::from_edges(
        &[
            (1, "A1"),
            (2, "A2"),
            (3, "A3"),
            (4, "B1"),
            (5, "B2"),
            (6, "B3"),
        ],
        &[
            (1, 4),
            (1, 5), // A1 → B1, B2
            (2, 4),
            (2, 5),
            (2, 6), // A2 → B1, B2, B3
            (3, 5),
            (3, 6), // A3 → B2, B3
        ],
    );
    println!("{}", dag.render());
    println!();
}

fn test_multiple_roots() {
    println!("6. Multiple Roots → Intermediate → Single Sink:");
    let dag = DAG::from_edges(
        &[
            (1, "R1"),
            (2, "R2"),
            (3, "R3"),
            (4, "Mid1"),
            (5, "Mid2"),
            (6, "Sink"),
        ],
        &[(1, 4), (2, 4), (2, 5), (3, 5), (4, 6), (5, 6)],
    );
    println!("{}", dag.render());
    println!();
}

fn test_cascade() {
    println!("7. Cascade Pattern (staircase):");
    let dag = DAG::from_edges(
        &[
            (1, "Step1"),
            (2, "Step2"),
            (3, "Step3"),
            (4, "Out1"),
            (5, "Out2"),
            (6, "Out3"),
        ],
        &[(1, 2), (1, 4), (2, 3), (2, 5), (3, 6)],
    );
    println!("{}", dag.render());
    println!();
}

fn test_long_labels() {
    println!("8. Long Labels:");
    let dag = DAG::from_edges(
        &[
            (1, "InitializeConfiguration"),
            (2, "ValidateInput"),
            (3, "ProcessData"),
            (4, "GenerateOutput"),
        ],
        &[(1, 2), (1, 3), (2, 4), (3, 4)],
    );
    println!("{}", dag.render());
    println!();
}

fn test_mixed_complexity() {
    println!("9. Mixed Complexity (diverge, chain, converge):");
    let dag = DAG::from_edges(
        &[
            (1, "Start"),
            (2, "Fork1"),
            (3, "Fork2"),
            (4, "Fork3"),
            (5, "Process1"),
            (6, "Process2"),
            (7, "Merge"),
            (8, "End"),
        ],
        &[
            (1, 2),
            (1, 3),
            (1, 4), // diverge
            (2, 5),
            (3, 5),
            (3, 6),
            (4, 6), // intermediate
            (5, 7),
            (6, 7), // converge
            (7, 8), // chain to end
        ],
    );
    println!("{}", dag.render());
    println!();
}

fn test_inverted_tree() {
    println!("10. Inverted Tree (convergence at each level):");
    let dag = DAG::from_edges(
        &[
            (1, "L1"),
            (2, "L2"),
            (3, "L3"),
            (4, "L4"),
            (5, "M1"),
            (6, "M2"),
            (7, "Top"),
        ],
        &[
            (1, 5),
            (2, 5), // L1,L2 → M1
            (3, 6),
            (4, 6), // L3,L4 → M2
            (5, 7),
            (6, 7), // M1,M2 → Top
        ],
    );
    println!("{}", dag.render());
    println!();
}
