use ascii_dag::graph::DAG;

fn main() {
    println!("=== Error Chain Visualization ===\n");

    // Simulate an error dependency chain
    // FileNotFound -> ParseError -> CompilationFailed -> BuildFailed

    let mut dag = DAG::new();

    dag.add_node(1, "FileNotFound");
    dag.add_node(2, "ParseError");
    dag.add_node(3, "CompilationFailed");
    dag.add_node(4, "BuildFailed");

    dag.add_edge(1, 2); // FileNotFound caused ParseError
    dag.add_edge(2, 3); // ParseError caused CompilationFailed
    dag.add_edge(3, 4); // CompilationFailed caused BuildFailed

    println!("Error Dependency Chain:");
    println!("{}", dag.render());

    // More complex error scenario
    println!("\n=== Complex Error Scenario ===\n");

    let dag = DAG::from_edges(
        &[
            (1, "ConfigMissing"),
            (2, "DBConnFail"),
            (3, "AuthFail"),
            (4, "InitError"),
            (5, "StartupFail"),
        ],
        &[
            (1, 2), // ConfigMissing -> DBConnFail
            (1, 3), // ConfigMissing -> AuthFail
            (2, 4), // DBConnFail -> InitError
            (3, 4), // AuthFail -> InitError
            (4, 5), // InitError -> StartupFail
        ],
    );

    println!("Multiple Error Paths:");
    println!("{}", dag.render());

    // Check for cycles (should not have any)
    if dag.has_cycle() {
        println!("\n⚠️  WARNING: Circular error dependency detected!");
    } else {
        println!("\n✓ No circular dependencies");
    }
}
