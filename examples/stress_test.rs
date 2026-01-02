use ascii_dag::graph::DAG;

// Simple Linear Congruential Generator to avoid adding 'rand' dependency
struct SimpleRng {
    state: u64,
}

impl SimpleRng {
    fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_mul(6364136223846793005).wrapping_add(1);
        self.state
    }

    fn gen_range(&mut self, min: usize, max: usize) -> usize {
        let range = (max - min) as u64;
        let random = self.next_u64();
        min + (random % range) as usize
    }
}

fn main() {
    println!("=== ASCII DAG Stress Test Suite ===\n");

    let tests = [
        (
            "The Double Helix",
            test_double_helix as fn() -> DAG<'static>,
        ),
        ("The Skyscraper", test_skyscraper),
        ("The Wide Fan", test_wide_fan),
        ("The Diamond Lattice", test_diamond_lattice),
        ("The Disconnected Islands", test_disconnected_islands),
        ("The Random Hairball", test_random_hairball),
        ("The Skip-Level Nightmare", test_skip_level_nightmare),
        ("The Verbose Logger", test_verbose_logger),
        ("The Ouroboros", test_ouroboros),
    ];

    for (name, test_fn) in tests {
        println!("\n>>> RUNNING: {} <<<\n", name);
        let dag = test_fn();
        let start = std::time::Instant::now();
        let output = dag.render();
        let duration = start.elapsed();

        println!("{}", output);
        println!(">>> Rendered in {:?} <<<\n", duration);
        println!("------------------------------------------------------------");
    }
}

fn test_double_helix() -> DAG<'static> {
    let mut dag = DAG::new();
    // Two intertwined chains
    for i in 0..10 {
        dag.add_node(i * 2, "A");
        dag.add_node(i * 2 + 1, "B");

        if i > 0 {
            dag.add_edge((i - 1) * 2, i * 2); // A -> A
            dag.add_edge((i - 1) * 2 + 1, i * 2 + 1); // B -> B

            // Cross connections
            if i % 2 == 0 {
                dag.add_edge((i - 1) * 2, i * 2 + 1); // A -> B
                dag.add_edge((i - 1) * 2 + 1, i * 2); // B -> A
            }
        }
    }
    dag
}

fn test_skyscraper() -> DAG<'static> {
    let mut dag = DAG::new();
    // Very deep, narrow graph to test vertical spacing
    for i in 0..50 {
        dag.add_node(i, Box::leak(format!("Floor {}", i).into_boxed_str()));
        if i > 0 {
            dag.add_edge(i - 1, i);
        }
    }
    dag
}

fn test_wide_fan() -> DAG<'static> {
    let mut dag = DAG::new();
    dag.add_node(0, "Source");
    dag.add_node(1000, "Sink");

    // 50 parallel nodes
    for i in 1..51 {
        dag.add_node(i, Box::leak(format!("Worker {}", i).into_boxed_str()));
        dag.add_edge(0, i);
        dag.add_edge(i, 1000);
    }
    dag
}

fn test_diamond_lattice() -> DAG<'static> {
    let mut dag = DAG::new();
    let width = 5;
    let height = 10;

    for y in 0..height {
        for x in 0..width {
            let id = y * width + x;
            dag.add_node(id, "♦");

            if y > 0 {
                // Connect to parents
                let p_id = (y - 1) * width + x;
                dag.add_edge(p_id, id);

                // Cross connections
                if x > 0 {
                    dag.add_edge((y - 1) * width + (x - 1), id);
                }
                if x < width - 1 {
                    dag.add_edge((y - 1) * width + (x + 1), id);
                }
            }
        }
    }
    dag
}

fn test_disconnected_islands() -> DAG<'static> {
    let mut dag = DAG::new();

    // Create 5 separate small graphs
    for island in 0..5 {
        let base = island * 10;
        dag.add_node(base, "Island");
        dag.add_node(base + 1, "Palm");
        dag.add_node(base + 2, "Coconuts");

        dag.add_edge(base, base + 1);
        dag.add_edge(base + 1, base + 2);
    }
    dag
}

fn test_random_hairball() -> DAG<'static> {
    let mut dag = DAG::new();
    let mut rng = SimpleRng::new(42);
    let nodes = 30;

    for i in 0..nodes {
        dag.add_node(i, Box::leak(format!("N{}", i).into_boxed_str()));
    }

    // Add random edges, ensuring no cycles (i < j rule)
    for i in 0..nodes {
        let num_edges = rng.gen_range(1, 4);
        for _ in 0..num_edges {
            let target = rng.gen_range(i + 1, nodes + 5); // +5 allows some out of bounds (ignored) or valid
            if target < nodes {
                dag.add_edge(i, target);
            }
        }
    }
    dag
}

fn test_skip_level_nightmare() -> DAG<'static> {
    let mut dag = DAG::new();
    // Root
    dag.add_node(0, "Root");

    // Levels 1, 2, 3, 4, 5
    for i in 1..6 {
        dag.add_node(i, Box::leak(format!("L{}", i).into_boxed_str()));
        dag.add_edge(i - 1, i); // Normal connection
    }

    // Skip edges: 0->2, 0->3, 0->4, 0->5
    for i in 2..6 {
        dag.add_edge(0, i);
    }

    // Nested skips: 1->3, 2->5
    dag.add_edge(1, 3);
    dag.add_edge(2, 5);

    dag
}

fn test_verbose_logger() -> DAG<'static> {
    let mut dag = DAG::new();
    dag.add_node(1, "A");
    dag.add_node(2, "B");
    // Long text to test centering
    dag.add_node(
        3,
        Box::leak(
            "Error: NullPointerException at line 55 (Critical Failure in Module X)"
                .to_string()
                .into_boxed_str(),
        ),
    );
    dag.add_node(4, "C");

    dag.add_edge(1, 2);
    dag.add_edge(1, 3);
    dag.add_edge(3, 4);
    dag
}

fn test_ouroboros() -> DAG<'static> {
    let mut dag = DAG::new();
    dag.add_node(1, "Head");
    dag.add_node(2, "Body");
    dag.add_node(3, "Tail");

    dag.add_edge(1, 2);
    dag.add_edge(2, 3);
    dag.add_edge(3, 1); // Cycle!
    dag
}
