//! Subgraph (cluster) examples for ascii-dag.
//!
//! Demonstrates the fluent `add_subgraph` / `put_nodes` / `put_subgraphs`
//! API for creating named clusters with double-line box-drawing borders,
//! matching zigraph's subgraph model.
//!
//! Run with `--csr` to use the arena/CSR pipeline instead of heap.

use ascii_dag::graph::Graph;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let use_csr = args.iter().any(|a| a == "--csr");

    if use_csr {
        println!("=== Subgraph Examples (CSR Mode) ===\n");
        run_csr();
    } else {
        println!("=== Subgraph Examples ===\n");
        run_heap();
    }
}

fn run_heap() {
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
    g.put_nodes(&[1, 2]).inside(backend).expect("place nodes in Backend");

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
    g.put_nodes(&[1]).inside(fe).expect("place React in Frontend");
    g.put_nodes(&[2, 3]).inside(be).expect("place nodes in Backend");

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
    g.put_nodes(&[1, 2, 3]).inside(be).expect("place nodes in Backend");
    g.put_nodes(&[3]).inside(db).expect("place Postgres in Database");
    g.put_subgraphs(&[db]).inside(be).expect("nest Database in Backend");

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
    g.put_nodes(&[2, 3]).inside(fe).expect("place FE nodes");
    g.put_nodes(&[4, 5]).inside(be).expect("place BE nodes");

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
    g.put_subgraphs(&[order_api, order_data]).inside(order_svc).expect("nest in OrderService");
    g.put_nodes(&[10, 11]).inside(order_api).expect("place order API nodes");
    g.put_nodes(&[12]).inside(order_data).expect("place OrderDB");

    let pay_svc = g.add_subgraph("PaymentService");
    let pay_proc = g.add_subgraph("Processing");
    let pay_store = g.add_subgraph("Storage");
    g.put_subgraphs(&[pay_proc, pay_store]).inside(pay_svc).expect("nest in PaymentService");
    g.put_nodes(&[20, 21]).inside(pay_proc).expect("place payment proc nodes");
    g.put_nodes(&[22]).inside(pay_store).expect("place PaymentDB");

    let monitoring = g.add_subgraph("Monitoring");
    g.put_nodes(&[30, 31]).inside(monitoring).expect("place monitoring nodes");

    let ir = g.compute_layout();
    for sg in ir.subgraphs() {
        println!("   {:?} parent={:?} at ({},{}) {}×{}", sg.label, sg.parent_id, sg.x, sg.y, sg.width, sg.height);
    }
    println!();
    println!("{}", ir.render_scanline());
    println!();
}

// ── CSR Mode ────────────────────────────────────────────────────────────

fn run_csr() {
    use ascii_dag::graph::arena::Arena;
    use ascii_dag::LayoutConfig;

    /// Helper: convert Graph to CSR, lay out, and render via arena pipeline.
    fn render_csr(dag: &Graph) {
        let csr_size = dag.estimate_csr_arena_size() * 2;
        let mut csr_buf = vec![0u8; csr_size];
        let mut csr_arena = Arena::new(&mut csr_buf);
        let csr = dag.to_csr(&mut csr_arena).expect("CSR conversion");

        let layout_size = dag.estimate_layout_arena_size();
        let size = ((layout_size * 6) / 5).max(128 * 1024);
        let mut temp_buf = vec![0u8; size];
        let mut out_buf = vec![0u8; size];
        let mut temp_arena = Arena::new(&mut temp_buf);
        let mut out_arena = Arena::new(&mut out_buf);

        let ir = csr
            .compute_layout_arena(&LayoutConfig::standard(), &mut temp_arena, &mut out_arena)
            .expect("Layout failed");

        println!("   {} subgraph(s) in IR", ir.subgraph_count());
        for sg in ir.subgraphs() {
            println!(
                "   id={} parent={} at ({},{}) {}×{}",
                sg.id,
                if sg.parent_idx == usize::MAX { "none".to_string() } else { sg.parent_idx.to_string() },
                sg.x, sg.y, sg.width, sg.height,
            );
        }
        println!();

        let (render_bytes, _) = ir.estimate_render_size();
        let render_size = render_bytes * 4 + 4096;
        let mut render_buffer = vec![0u8; render_size];
        let mut line_buffer = vec![' '; ir.width().max(1) + 16];
        let mut scratch_buffer = vec![0usize; (ir.height() + ir.edge_count() * 2).max(1)];

        if let Some(len) =
            ir.render_to_buffer(&mut render_buffer, &mut line_buffer, &mut scratch_buffer)
        {
            if let Ok(s) = std::str::from_utf8(&render_buffer[..len]) {
                println!("{}\n", s);
            }
        }
    }

    // 1. Simple Cluster
    println!("1. Simple Cluster (CSR):\n");
    {
        let mut g = Graph::new();
        g.add_node(1, "Server");
        g.add_node(2, "Database");
        g.add_edge(1, 2, None);
        let backend = g.add_subgraph("Backend");
        g.put_nodes(&[1, 2]).inside(backend).expect("place nodes");
        render_csr(&g);
    }

    // 2. Two Sibling Clusters
    println!("2. Two Sibling Clusters (CSR):\n");
    {
        let mut g = Graph::new();
        g.add_node(1, "React");
        g.add_node(2, "API");
        g.add_node(3, "Postgres");
        g.add_edge(1, 2, None);
        g.add_edge(2, 3, None);
        let fe = g.add_subgraph("Frontend");
        let be = g.add_subgraph("Backend");
        g.put_nodes(&[1]).inside(fe).expect("place React");
        g.put_nodes(&[2, 3]).inside(be).expect("place BE nodes");
        render_csr(&g);
    }

    // 3. Nested Clusters
    println!("3. Nested Clusters (CSR):\n");
    {
        let mut g = Graph::new();
        g.add_node(1, "API");
        g.add_node(2, "Cache");
        g.add_node(3, "Postgres");
        g.add_edge(1, 2, None);
        g.add_edge(1, 3, None);
        let be = g.add_subgraph("Backend");
        let db = g.add_subgraph("Database");
        g.put_nodes(&[1, 2, 3]).inside(be).expect("place nodes");
        g.put_nodes(&[3]).inside(db).expect("place Postgres");
        g.put_subgraphs(&[db]).inside(be).expect("nest Database");
        render_csr(&g);
    }

    // 4. CI/CD Pipeline with Clusters
    println!("4. CI/CD Pipeline with Clusters (CSR):\n");
    {
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
        g.put_nodes(&[2, 3]).inside(fe).expect("place FE nodes");
        g.put_nodes(&[4, 5]).inside(be).expect("place BE nodes");
        render_csr(&g);
    }

    // 5. Horizontal Sibling Subgraphs
    println!("5. Horizontal Sibling Subgraphs (CSR):\n");
    {
        let mut g = Graph::new();
        g.add_node(1, "Frontend");
        g.add_node(10, "OrderAPI");
        g.add_node(11, "OrderWorker");
        g.add_node(12, "OrderDB");
        g.add_node(20, "PaymentAPI");
        g.add_node(21, "PaymentProc");
        g.add_node(22, "PaymentDB");
        g.add_node(30, "Metrics");
        g.add_node(31, "Alerts");

        g.add_edge(1, 10, None);
        g.add_edge(1, 20, None);
        g.add_edge(10, 11, None);
        g.add_edge(11, 12, None);
        g.add_edge(20, 21, None);
        g.add_edge(21, 22, None);
        g.add_edge(12, 30, None);
        g.add_edge(22, 30, None);
        g.add_edge(30, 31, None);

        let order_svc = g.add_subgraph("OrderService");
        let order_api = g.add_subgraph("API");
        let order_data = g.add_subgraph("Data");
        g.put_subgraphs(&[order_api, order_data]).inside(order_svc).expect("nest in OrderService");
        g.put_nodes(&[10, 11]).inside(order_api).expect("place order API nodes");
        g.put_nodes(&[12]).inside(order_data).expect("place OrderDB");

        let pay_svc = g.add_subgraph("PaymentService");
        let pay_proc = g.add_subgraph("Processing");
        let pay_store = g.add_subgraph("Storage");
        g.put_subgraphs(&[pay_proc, pay_store]).inside(pay_svc).expect("nest in PaymentService");
        g.put_nodes(&[20, 21]).inside(pay_proc).expect("place payment proc");
        g.put_nodes(&[22]).inside(pay_store).expect("place PaymentDB");

        let monitoring = g.add_subgraph("Monitoring");
        g.put_nodes(&[30, 31]).inside(monitoring).expect("place monitoring nodes");

        render_csr(&g);
    }
}
