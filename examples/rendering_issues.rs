//! Focused tests on specific rendering issues.
//! These tests highlight areas where the ASCII rendering could be improved.

use ascii_dag::graph::Graph;
use std::fs::File;
use std::io::Write;

fn main() {
    let mut output = String::new();
    output.push_str("=== Rendering Issues Analysis ===\n\n");

    // Issue 1: Skip-level edges not visible
    issue_skip_level_edges(&mut output);

    // Issue 2: Cross-level edges in grid patterns
    issue_cross_edges(&mut output);

    // Issue 3: Asymmetric paths
    issue_asymmetric_paths(&mut output);

    // Write to file
    let mut file = File::create("rendering_issues_output.txt").expect("Failed to create file");
    file.write_all(output.as_bytes()).expect("Failed to write");
    println!("Output written to rendering_issues_output.txt");
}

fn issue_skip_level_edges(output: &mut String) {
    output.push_str("ISSUE 1: Skip-Level Edges\n");
    output.push_str("=".repeat(60).as_str());
    output.push('\n');

    output.push_str("\nGraph: A->B->C->D + A->D (skip edge)\n\n");

    let dag = Graph::from_edges(
        &[(1, "A"), (2, "B"), (3, "C"), (4, "D")],
        &[(1, 2), (2, 3), (3, 4), (1, 4)],
    );
    output.push_str(&dag.render());
    output.push_str("\n\n");
}

fn issue_cross_edges(output: &mut String) {
    output.push_str("ISSUE 2: Cross Connections\n");
    output.push_str("=".repeat(60).as_str());
    output.push('\n');

    output.push_str("\nGraph: A1->B1, A1->B2, A2->B1, A2->B2 (full bipartite)\n\n");

    let dag = Graph::from_edges(
        &[(1, "A1"), (2, "A2"), (3, "B1"), (4, "B2")],
        &[(1, 3), (1, 4), (2, 3), (2, 4)],
    );
    output.push_str(&dag.render());
    output.push_str("\n\n");
}

fn issue_asymmetric_paths(output: &mut String) {
    output.push_str("ISSUE 3: Asymmetric Paths\n");
    output.push_str("=".repeat(60).as_str());
    output.push('\n');

    output.push_str("\nGraph: Root->Short->End + Root->Long1->Long2->End\n");
    output.push_str("Short->End is a skip edge (level 1 to 3)\n\n");

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
            (2, 5), // Short path (skip!)
            (3, 4),
            (4, 5), // Long path
        ],
    );
    output.push_str(&dag.render());
    output.push('\n');
}
