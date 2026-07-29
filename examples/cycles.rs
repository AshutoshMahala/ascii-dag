use ascii_dag::graph::Graph;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let use_csr = args.iter().any(|a| a == "--csr");

    if use_csr {
        println!("=== Cycle Detection Examples (CSR Mode) ===\n");
        run_csr();
    } else {
        println!("=== Cycle Detection Examples ===\n");
        run_heap();
    }
}

fn run_heap() {
    // Example 1: Simple cycle A → B → C → A
    println!("1. Simple Cycle (A → B → C → A):");
    let mut dag = Graph::new();
    dag.add_node(1, "A");
    dag.add_node(2, "B");
    dag.add_node(3, "C");
    dag.add_edge(1, 2, None); // A → B
    dag.add_edge(2, 3, None); // B → C
    dag.add_edge(3, 1, None); // C → A (creates cycle!)

    println!("{}\n", dag.render());

    // Example 2: Self-referencing cycle
    println!("2. Self-Reference (A → A):");
    let mut dag = Graph::new();
    dag.add_node(1, "A");
    dag.add_edge(1, 1, None); // Points to itself

    println!("{}\n", dag.render());

    // Example 3: Longer cycle chain
    println!("3. Longer Cycle (E1 → E2 → E3 → E4 → E2):");
    let mut dag = Graph::new();
    dag.add_node(1, "Error1");
    dag.add_node(2, "Error2");
    dag.add_node(3, "Error3");
    dag.add_node(4, "Error4");
    dag.add_edge(1, 2, None); // E1 → E2
    dag.add_edge(2, 3, None); // E2 → E3
    dag.add_edge(3, 4, None); // E3 → E4
    dag.add_edge(4, 2, None); // E4 → E2 (cycle!)

    println!("{}\n", dag.render());

    // Example 4: Valid DAG (no cycle)
    println!("4. Valid DAG - No Cycle:");
    let dag = Graph::from_edges(
        &[(1, "Valid1"), (2, "Valid2"), (3, "Valid3")],
        &[(1, 2), (2, 3)],
    );

    println!("{}", dag.render());
}

fn run_csr() {
    use ascii_dag::LayoutConfig;
    use ascii_dag::graph::arena::Arena;

    // Helper: build CsrGraph from Graph, layout + render via arena
    fn render_csr(dag: &Graph) {
        let csr_size = dag.estimate_csr_arena_size() * 2;
        let mut csr_buf = vec![0u8; csr_size];
        let mut csr_arena = Arena::new(&mut csr_buf);
        let csr = dag.to_csr(&mut csr_arena).expect("CSR conversion failed");

        let layout_size = dag.estimate_layout_arena_size();
        let size = ((layout_size * 6) / 5).max(128 * 1024);
        let mut temp_buf = vec![0u8; size];
        let mut out_buf = vec![0u8; size];
        let mut temp_arena = Arena::new(&mut temp_buf);
        let mut out_arena = Arena::new(&mut out_buf);

        let ir = csr
            .compute_layout_arena(&LayoutConfig::standard(), &mut temp_arena, &mut out_arena)
            .expect("Layout failed");

        let options = ascii_dag::render::engine::RenderOptions::plain();
        let mut arena_buf = vec![0u8; ir.estimate_render_arena_size(&options)];
        let render_arena = ascii_dag::graph::arena::Arena::new(&mut arena_buf);
        let mut render_buffer = vec![0u8; ir.estimate_render_output_size(&options)];
        if let Ok(len) = ir.render_to_bytes(&options, &render_arena, &mut render_buffer)
            && let Ok(s) = std::str::from_utf8(&render_buffer[..len])
        {
            println!("{}\n", s);
        }
    }

    // Example 1: Simple cycle A → B → C → A
    println!("1. Simple Cycle (A → B → C → A):");
    let mut dag = Graph::new();
    dag.add_node(1, "A");
    dag.add_node(2, "B");
    dag.add_node(3, "C");
    dag.add_edge(1, 2, None);
    dag.add_edge(2, 3, None);
    dag.add_edge(3, 1, None);
    render_csr(&dag);

    // Example 2: Self-referencing cycle
    println!("2. Self-Reference (A → A):");
    let mut dag = Graph::new();
    dag.add_node(1, "A");
    dag.add_edge(1, 1, None);
    render_csr(&dag);

    // Example 2b: Two-node cycle (A ↔ B)
    println!("2b. Two-Node Cycle (A → B → A):");
    let mut dag = Graph::new();
    dag.add_node(1, "Ping");
    dag.add_node(2, "Pong");
    dag.add_edge(1, 2, None);
    dag.add_edge(2, 1, None);
    render_csr(&dag);

    // Example 3: Longer cycle chain
    println!("3. Longer Cycle (E1 → E2 → E3 → E4 → E2):");
    let mut dag = Graph::new();
    dag.add_node(1, "Error1");
    dag.add_node(2, "Error2");
    dag.add_node(3, "Error3");
    dag.add_node(4, "Error4");
    dag.add_edge(1, 2, None);
    dag.add_edge(2, 3, None);
    dag.add_edge(3, 4, None);
    dag.add_edge(4, 2, None);
    render_csr(&dag);

    // Example 4: Valid DAG (no cycle)
    println!("4. Valid DAG - No Cycle:");
    let dag = Graph::from_edges(
        &[(1, "Valid1"), (2, "Valid2"), (3, "Valid3")],
        &[(1, 2), (2, 3)],
    );
    render_csr(&dag);
}
