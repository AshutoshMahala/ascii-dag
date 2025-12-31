use ascii_dag::graph::DAG;
use std::fs::File;
use std::io::Write;

fn main() {
    let mut f = File::create("debug_layout.txt").unwrap();

    // Test: Simple skip edge
    writeln!(f, "=== Debug: Simple Skip Edge ===").unwrap();
    let dag = DAG::from_edges(
        &[(1, "A"), (2, "B"), (3, "C"), (4, "D")],
        &[(1, 2), (2, 3), (3, 4), (1, 4)], // A->D is the skip edge
    );

    writeln!(f, "\nExpected levels:").unwrap();
    writeln!(f, "  A (id=1) should be level 0 (root)").unwrap();
    writeln!(f, "  B (id=2) should be level 1 (child of A)").unwrap();
    writeln!(f, "  C (id=3) should be level 2 (child of B)").unwrap();
    writeln!(
        f,
        "  D (id=4) should be level 3 (child of C and A, max+1=3)"
    )
    .unwrap();

    writeln!(f, "\nSkip edges:").unwrap();
    writeln!(
        f,
        "  A->D spans levels 0 to 3, so needs dummies at levels 1 and 2"
    )
    .unwrap();

    writeln!(f, "\nRendered output:").unwrap();
    writeln!(f, "{}", dag.render()).unwrap();

    writeln!(f, "\n=== Debug: Test 5 Wide Graph ===").unwrap();
    let dag = DAG::from_edges(
        &[(1, "A"), (2, "B"), (3, "C"), (4, "X"), (5, "Y"), (6, "Z")],
        &[
            (1, 4),
            (2, 5),
            (3, 6), // A->X, B->Y, C->Z
            (4, 5),
            (5, 6), // X->Y->Z chain
            (1, 6), // A->Z skip edge
        ],
    );

    writeln!(f, "Expected levels (longest path):").unwrap();
    writeln!(f, "  A, B, C should be level 0 (roots, no parents)").unwrap();
    writeln!(f, "  X should be level 1 (child of A)").unwrap();
    writeln!(
        f,
        "  Y should be level 2 (child of B via 2->5, but also child of X via 4->5)"
    )
    .unwrap();
    writeln!(
        f,
        "  Z should be level 3 (child of C via 3->6, but also child of Y via 5->6)"
    )
    .unwrap();

    writeln!(f, "\nRendered output:").unwrap();
    writeln!(f, "{}", dag.render()).unwrap();

    println!("Debug output written to debug_layout.txt");
}
