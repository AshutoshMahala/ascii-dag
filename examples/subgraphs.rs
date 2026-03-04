//! Subgraph (cluster) examples for ascii-dag.
//!
//! Demonstrates the fluent `add_subgraph` / `put_nodes` / `put_subgraphs`
//! API for creating named clusters with double-line box-drawing borders,
//! matching zigraph's subgraph model.

use ascii_dag::graph::Graph;

fn main() {
    println!("=== Subgraph Examples ===\n");

    example_simple_cluster();
    example_two_clusters();
    example_nested_clusters();
    example_cluster_with_edges();
    example_horizontal_siblings();
}

/// A single named cluster around two nodes.
fn example_simple_cluster() {
    println!("1. Simple Cluster:");
    println!("   put_nodes(&[1, 2]).inside(backend)\n");

    let mut g = Graph::new();
    g.add_node(1, "Server");
    g.add_node(2, "Database");
    g.add_edge(1, 2, None);

    let backend = g.add_subgraph("Backend");
    g.put_nodes(&[1, 2]).inside(backend).unwrap();

    let ir = g.compute_layout();
    println!("   {} subgraph(s) in IR", ir.subgraphs().len());
    for sg in ir.subgraphs() {
        println!("   {:?} at ({},{}) {}×{}", sg.label, sg.x, sg.y, sg.width, sg.height);
    }
    println!();
    println!("{}", ir.render_scanline());
    println!();
}

/// Two sibling clusters with an edge between them.
fn example_two_clusters() {
    println!("2. Two Sibling Clusters:");
    println!("   Frontend and Backend with a cross-cluster edge\n");

    let mut g = Graph::new();
    g.add_node(1, "React");
    g.add_node(2, "API");
    g.add_node(3, "Postgres");
    g.add_edge(1, 2, None); // cross-cluster
    g.add_edge(2, 3, None); // within Backend

    let fe = g.add_subgraph("Frontend");
    let be = g.add_subgraph("Backend");
    g.put_nodes(&[1]).inside(fe).unwrap();
    g.put_nodes(&[2, 3]).inside(be).unwrap();

    let ir = g.compute_layout();
    println!("   {} subgraph(s) in IR", ir.subgraphs().len());
    println!();
    println!("{}", ir.render_scanline());
    println!();
}

/// Nested clusters: Database inside Backend.
fn example_nested_clusters() {
    println!("3. Nested Clusters:");
    println!("   put_subgraphs(&[db]).inside(backend)\n");

    let mut g = Graph::new();
    g.add_node(1, "API");
    g.add_node(2, "Cache");
    g.add_node(3, "Postgres");
    g.add_edge(1, 2, None);
    g.add_edge(1, 3, None);

    let be = g.add_subgraph("Backend");
    let db = g.add_subgraph("Database");
    g.put_nodes(&[1, 2, 3]).inside(be).unwrap();
    g.put_nodes(&[3]).inside(db).unwrap();
    g.put_subgraphs(&[db]).inside(be).unwrap();

    let ir = g.compute_layout();
    for sg in ir.subgraphs() {
        println!("   {:?} parent={:?} at ({},{}) {}×{}", sg.label, sg.parent_id, sg.x, sg.y, sg.width, sg.height);
    }
    println!();
    println!("{}", ir.render_scanline());
    println!();
}

/// A larger graph with clusters and cross-cluster edges.
fn example_cluster_with_edges() {
    println!("4. CI/CD Pipeline with Clusters:\n");

    let mut g = Graph::new();
    g.add_node(1, "Checkout");
    g.add_node(2, "Build-FE");
    g.add_node(3, "Test-FE");
    g.add_node(4, "Build-BE");
    g.add_node(5, "Test-BE");
    g.add_node(6, "Deploy");

    g.add_edge(1, 2, None);
    g.add_edge(1, 4, None);
    g.add_edge(2, 3, None);
    g.add_edge(4, 5, None);
    g.add_edge(3, 6, None);
    g.add_edge(5, 6, None);

    let fe = g.add_subgraph("Frontend");
    let be = g.add_subgraph("Backend");
    g.put_nodes(&[2, 3]).inside(fe).unwrap();
    g.put_nodes(&[4, 5]).inside(be).unwrap();

    println!("{}", g.render());
    println!();
}

/// Horizontal sibling subgraphs with nested children (zigraph parity).
///
/// Frontend fans out to OrderService and PaymentService side-by-side,
/// each with internal sub-clusters.  Monitoring below.
fn example_horizontal_siblings() {
    println!("5. Horizontal Sibling Subgraphs:\n");

    let mut g = Graph::new();

    g.add_node(1, "Frontend");

    // Order Service
    g.add_node(10, "OrderAPI");
    g.add_node(11, "OrderWorker");
    g.add_node(12, "OrderDB");

    // Payment Service
    g.add_node(20, "PaymentAPI");
    g.add_node(21, "PaymentProc");
    g.add_node(22, "PaymentDB");

    // Monitoring
    g.add_node(30, "Metrics");
    g.add_node(31, "Alerts");

    // Edges
    g.add_edge(1, 10, None);
    g.add_edge(1, 20, None);
    g.add_edge(10, 11, None);
    g.add_edge(11, 12, None);
    g.add_edge(20, 21, None);
    g.add_edge(21, 22, None);
    g.add_edge(12, 30, None);
    g.add_edge(22, 30, None);
    g.add_edge(30, 31, None);

    // Subgraph hierarchy
    let order_svc = g.add_subgraph("OrderService");
    let order_api = g.add_subgraph("API");
    let order_data = g.add_subgraph("Data");
    g.put_subgraphs(&[order_api, order_data]).inside(order_svc).unwrap();
    g.put_nodes(&[10, 11]).inside(order_api).unwrap();
    g.put_nodes(&[12]).inside(order_data).unwrap();

    let pay_svc = g.add_subgraph("PaymentService");
    let pay_proc = g.add_subgraph("Processing");
    let pay_store = g.add_subgraph("Storage");
    g.put_subgraphs(&[pay_proc, pay_store]).inside(pay_svc).unwrap();
    g.put_nodes(&[20, 21]).inside(pay_proc).unwrap();
    g.put_nodes(&[22]).inside(pay_store).unwrap();

    let monitoring = g.add_subgraph("Monitoring");
    g.put_nodes(&[30, 31]).inside(monitoring).unwrap();

    let ir = g.compute_layout();
    for sg in ir.subgraphs() {
        println!("   {:?} parent={:?} at ({},{}) {}×{}", sg.label, sg.parent_id, sg.x, sg.y, sg.width, sg.height);
    }
    println!();
    println!("{}", ir.render_scanline());
    println!();
}
