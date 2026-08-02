use ascii_dag::RenderOptions;
use ascii_dag::graph::Graph;

#[path = "support/csr.rs"]
mod csr;

fn show(dag: &ascii_dag::Graph<'_>) {
    if csr::requested() {
        println!("{}", csr::render(dag, &RenderOptions::plain()));
    } else {
        println!("{}", dag.render());
    }
}

fn main() {
    println!("\n=== Git History Visualization ===\n");

    let mut dag = Graph::new();

    // Key: 1=Initial, 2=Feat, 3=Fix, 4=Merge
    // Topology:
    // 1 -> 2 (Feature Branch)
    // 1 -> 3 (Main Branch Fix)
    // 2 -> 4 (Merge)
    // 3 -> 4 (Merge)

    dag.add_node(1, "Initial Commit (a1b2c)");
    dag.add_node(2, "Feat: Login (d4e5f)");
    dag.add_node(3, "Fix: Typos (g7h8i)");
    dag.add_node(4, "Merge branch 'feat/login' (j9k0l)");
    dag.add_node(5, "Release v1.0 (m1n2o)");

    dag.add_edge(1, 2, None);
    dag.add_edge(1, 3, None);
    dag.add_edge(2, 4, None);
    dag.add_edge(3, 4, None);
    dag.add_edge(4, 5, None);

    show(&dag);

    println!("\n=== Complex Branching ===\n");
    // Diverge -> Diverge -> Converge
    let mut complex = Graph::new();
    complex.add_node(10, "init");
    complex.add_node(11, "dev");
    complex.add_node(12, "feature-A");
    complex.add_node(13, "feature-B");
    complex.add_node(14, "dev-update");
    complex.add_node(15, "merge-A");
    complex.add_node(16, "merge-all");

    // init -> dev
    complex.add_edge(10, 11, None);

    // dev splits into A, B, and continues
    complex.add_edge(11, 12, None); // dev -> feature-A
    complex.add_edge(11, 13, None); // dev -> feature-B
    complex.add_edge(11, 14, None); // dev -> dev-update

    // A merges back to dev
    complex.add_edge(12, 15, None);
    complex.add_edge(14, 15, None);

    // B and A-Merge merge to final
    complex.add_edge(13, 16, None);
    complex.add_edge(15, 16, None);

    show(&complex);
}
