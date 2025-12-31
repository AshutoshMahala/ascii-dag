//! Example: Collecting all nodes in a graph with cycles
//!
//! This demonstrates the `collect_all_nodes_fn` utility for traversing
//! graphs and collecting all reachable nodes while handling cycles.

use ascii_dag::layout::generic::traversal::{collect_all_nodes_dfs_fn, collect_all_nodes_fn};

fn main() {
    println!("=== Node Collection Examples ===\n");

    // Example 1: Simple tree
    println!("1. Simple Tree:");
    println!("   Build dependency tree:");
    println!("   app.exe -> main.o, utils.o");
    println!("   main.o -> main.c");
    println!("   utils.o -> utils.c");

    let get_deps = |file: &&str| match *file {
        "app.exe" => vec!["main.o", "utils.o"],
        "main.o" => vec!["main.c"],
        "utils.o" => vec!["utils.c"],
        _ => vec![],
    };

    let all_files = collect_all_nodes_fn(&["app.exe"], get_deps);
    println!("   All files needed: {:?}", all_files);
    println!("   Count: {}\n", all_files.len());

    // Example 2: Diamond dependency (DAG)
    println!("2. Diamond Dependency:");
    println!("       1");
    println!("      / \\");
    println!("     2   3");
    println!("      \\ /");
    println!("       4");

    let get_children = |&node: &usize| match node {
        1 => vec![2, 3],
        2 => vec![4],
        3 => vec![4],
        _ => vec![],
    };

    let nodes = collect_all_nodes_fn(&[1], get_children);
    println!("   Collected nodes (BFS): {:?}", nodes);
    println!("   Node 4 appears once despite two paths\n");

    // Example 3: Graph with cycle
    println!("3. Cyclic Graph:");
    println!("   Error chain with cycle:");
    println!("   1 (Network Error)");
    println!("   ↓");
    println!("   2 (Connection Timeout)");
    println!("   ↓");
    println!("   3 (DNS Resolution)");
    println!("   ↓");
    println!("   1 (cycles back)");

    let get_related = |&err: &usize| match err {
        1 => vec![2],
        2 => vec![3],
        3 => vec![1], // Creates cycle
        _ => vec![],
    };

    let error_chain = collect_all_nodes_fn(&[1], get_related);
    println!("   Collected error IDs: {:?}", error_chain);
    println!("   Each error visited once despite cycle\n");

    // Example 4: Multiple starting points
    println!("4. Multiple Starting Points:");
    println!("   Independent error sources:");
    println!("   1 → 3 → 4");
    println!("   2 → 3 → 4");
    println!("   (3 and 4 are shared)");

    let get_causes = |&err: &usize| match err {
        1 => vec![3],
        2 => vec![3],
        3 => vec![4],
        _ => vec![],
    };

    let all_errors = collect_all_nodes_fn(&[1, 2], get_causes);
    println!("   Collected from both sources: {:?}", all_errors);
    println!("   Shared nodes (3, 4) visited once\n");

    // Example 5: BFS vs DFS order
    println!("5. BFS vs DFS Traversal Order:");
    println!("       1");
    println!("      /|\\");
    println!("     2 3 4");
    println!("     |");
    println!("     5");

    let get_deps_tree = |&node: &usize| match node {
        1 => vec![2, 3, 4],
        2 => vec![5],
        _ => vec![],
    };

    let bfs_order = collect_all_nodes_fn(&[1], get_deps_tree);
    let dfs_order = collect_all_nodes_dfs_fn(&[1], get_deps_tree);

    println!("   BFS order: {:?}", bfs_order);
    println!("   DFS order: {:?}", dfs_order);
    println!("   (Different traversal strategies)\n");

    // Example 6: Use case - PII redaction in error diagnostics
    println!("6. Real-World Use Case: PII Redaction");
    println!("   Error diagnostic with nested causes:");

    #[derive(Clone, Debug)]
    struct ErrorDiagnostic {
        id: usize,
        message: String,
        caused_by: Vec<usize>,
    }

    let diagnostics = [
        ErrorDiagnostic {
            id: 1,
            message: "Failed to authenticate user@example.com".to_string(),
            caused_by: vec![2],
        },
        ErrorDiagnostic {
            id: 2,
            message: "Database connection failed to 192.168.1.10".to_string(),
            caused_by: vec![3],
        },
        ErrorDiagnostic {
            id: 3,
            message: "Network timeout connecting to internal-db.corp".to_string(),
            caused_by: vec![],
        },
    ];

    let get_causes_by_id = |&id: &usize| {
        diagnostics
            .iter()
            .find(|d| d.id == id)
            .map(|d| d.caused_by.clone())
            .unwrap_or_default()
    };

    let all_diagnostic_ids = collect_all_nodes_fn(&[1], get_causes_by_id);

    println!("   Original error chain:");
    for id in &all_diagnostic_ids {
        let diag = diagnostics.iter().find(|d| d.id == *id).unwrap();
        println!("     {}: {}", id, diag.message);
    }

    println!("\n   After PII redaction (would redact emails, IPs, hostnames):");
    for id in &all_diagnostic_ids {
        let diag = diagnostics.iter().find(|d| d.id == *id).unwrap();
        let redacted = diag
            .message
            .replace("user@example.com", "[EMAIL]")
            .replace("192.168.1.10", "[IP]")
            .replace("internal-db.corp", "[HOSTNAME]");
        println!("     {}: {}", id, redacted);
    }

    println!(
        "\n   All {} diagnostics processed (including nested causes)",
        all_diagnostic_ids.len()
    );
}
