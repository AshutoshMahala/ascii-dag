//! Debug test for skip-level edges

use ascii_dag::graph::Graph;
use std::fs::File;
use std::io::Write;

fn main() {
    let mut output = String::new();
    output.push_str("=== Debug Skip-Level Rendering ===\n\n");

    // Test 1: Simple skip (level span of 2)
    output.push_str("Test 1: Skip span of 2 (A->B->C + A->C)\n");
    let dag = Graph::from_edges(
        &[(1, "A"), (2, "B"), (3, "C")],
        &[(1, 2), (2, 3), (1, 3)], // A->B->C + A->C
    );
    output.push_str(&dag.render());
    output.push_str("\n\n");

    // Test 2: Skip span of 3
    output.push_str("Test 2: Skip span of 3 (A->B->C->D + A->D)\n");
    let dag = Graph::from_edges(
        &[(1, "A"), (2, "B"), (3, "C"), (4, "D")],
        &[(1, 2), (2, 3), (3, 4), (1, 4)], // A->B->C->D + A->D
    );
    output.push_str(&dag.render());
    output.push_str("\n\n");

    // Test 3: Parallel paths of different lengths
    output.push_str("Test 3: Parallel paths (Root->Short->End + Root->Long1->Long2->End)\n");
    output.push_str("Edges: (1,2), (1,3), (2,5), (3,4), (4,5)\n");
    output.push_str("Expected levels: Root=0, Short=1, Long1=1, Long2=2, End=3\n");
    output.push_str("Skip edge: Short(level 1) -> End(level 3) spans 2 levels\n\n");
    let dag = Graph::from_edges(
        &[
            (1, "Root"),
            (2, "Short"),
            (3, "Long1"),
            (4, "Long2"),
            (5, "End"),
        ],
        &[
            (1, 2),
            (1, 3), // Root splits
            (2, 5), // Short path: Root->Short->End (skip!)
            (3, 4),
            (4, 5), // Long path: Root->Long1->Long2->End
        ],
    );
    output.push_str(&dag.render());
    output.push_str("\n\n");

    // Test 4: Wide graph with skip
    output.push_str("Test 4: Wide graph with skip\n");
    let dag = Graph::from_edges(
        &[(1, "Top"), (2, "L"), (3, "M"), (4, "R"), (5, "Bottom")],
        &[
            (1, 2),
            (1, 3),
            (1, 4), // Top diverges
            (2, 5),
            (3, 5),
            (4, 5), // All converge (L and R skip level!)
        ],
    );
    output.push_str(&dag.render());

    // Write to file
    let mut file = File::create("debug_output.txt").expect("Failed to create file");
    file.write_all(output.as_bytes()).expect("Failed to write");
    println!("Output written to debug_output.txt");
}
