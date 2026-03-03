// Quick verification of crossing reduction fix on key stress test graphs
// (skips massive diamond tests that take too long)
use ascii_dag::graph::Graph;
use ascii_dag::LayoutConfig;
use std::time::Instant;

fn main() {
    println!("=== Crossing Reduction Fix Verification ===\n");

    let tests: Vec<(&str, Box<dyn Fn() -> Graph<'static>>)> = vec![
        ("Double Helix", Box::new(test_double_helix)),
        ("Wide Fan", Box::new(test_wide_fan)),
        ("Diamond Lattice", Box::new(test_diamond_lattice)),
        ("Random Hairball", Box::new(test_random_hairball)),
        ("Skip-Level Nightmare", Box::new(test_skip_level_nightmare)),
        ("Ouroboros", Box::new(test_ouroboros)),
        ("Cycle Breaking Demo", Box::new(test_cycle_breaking_demo)),
    ];

    for (name, test_fn) in &tests {
        let dag = test_fn();
        let start = Instant::now();
        let ir = dag.compute_layout_with(&LayoutConfig::quality());
        let elapsed = start.elapsed();
        let rendered = ir.render_scanline();
        let lines: Vec<&str> = rendered.lines().collect();
        println!(">>> {} — {}x{} chars, {:?}", name, 
                 lines.iter().map(|l| l.len()).max().unwrap_or(0),
                 lines.len(), elapsed);
        
        // Print first 20 lines
        for line in lines.iter().take(20) {
            println!("  {}", line);
        }
        if lines.len() > 20 {
            println!("  ... ({} more lines)", lines.len() - 20);
        }
        println!();
    }
    println!("=== All tests completed successfully ===");
}

fn test_double_helix() -> Graph<'static> {
    let mut dag = Graph::new();
    for i in 0..10 {
        dag.add_node(i * 2, "A");
        dag.add_node(i * 2 + 1, "B");
    }
    for i in 0..9 {
        dag.add_edge(i * 2, (i + 1) * 2, None);
        dag.add_edge(i * 2 + 1, (i + 1) * 2 + 1, None);
        if i % 2 == 0 {
            dag.add_edge(i * 2, (i + 1) * 2 + 1, None);
            dag.add_edge(i * 2 + 1, (i + 1) * 2, None);
        }
    }
    dag
}

fn test_wide_fan() -> Graph<'static> {
    let mut dag = Graph::new();
    dag.add_node(0, "Root");
    for i in 1..=20 {
        let label: &'static str = Box::leak(format!("C{}", i).into_boxed_str());
        dag.add_node(i, label);
        dag.add_edge(0, i, None);
    }
    dag
}

fn test_diamond_lattice() -> Graph<'static> {
    let mut dag = Graph::new();
    let layers = [1, 4, 6, 8, 6, 4, 1]; // diamond shape
    let mut id = 0;
    let mut layer_ids: Vec<Vec<usize>> = Vec::new();

    for &count in &layers {
        let mut ids = Vec::new();
        for _ in 0..count {
            let label: &'static str = Box::leak(format!("N{}", id).into_boxed_str());
            dag.add_node(id, label);
            ids.push(id);
            id += 1;
        }
        layer_ids.push(ids);
    }

    for i in 0..layer_ids.len() - 1 {
        let current = &layer_ids[i];
        let next = &layer_ids[i + 1];
        for &src in current {
            for &dst in next {
                dag.add_edge(src, dst, None);
            }
        }
    }
    dag
}

fn test_random_hairball() -> Graph<'static> {
    let mut dag = Graph::new();
    let mut state: u64 = 42;
    let nodes = 30;

    for i in 0..nodes {
        let label: &'static str = Box::leak(format!("N{}", i).into_boxed_str());
        dag.add_node(i, label);
    }

    for i in 0..nodes {
        let num_edges = {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
            1 + (state % 3) as usize
        };
        for _ in 0..num_edges {
            let target = {
                state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
                let range = (nodes + 5 - i - 1) as u64;
                i + 1 + (state % range) as usize
            };
            if target < nodes {
                dag.add_edge(i, target, None);
            }
        }
    }
    dag
}

fn test_skip_level_nightmare() -> Graph<'static> {
    let mut dag = Graph::new();
    for i in 0..15 {
        let label: &'static str = Box::leak(format!("L{}", i).into_boxed_str());
        dag.add_node(i, label);
    }
    // Mix of direct and skip-level edges
    dag.add_edge(0, 1, None);
    dag.add_edge(0, 5, None);
    dag.add_edge(0, 10, None);
    dag.add_edge(1, 2, None);
    dag.add_edge(1, 7, None);
    dag.add_edge(2, 3, None);
    dag.add_edge(3, 4, None);
    dag.add_edge(3, 8, None);
    dag.add_edge(4, 9, None);
    dag.add_edge(5, 6, None);
    dag.add_edge(5, 11, None);
    dag.add_edge(6, 7, None);
    dag.add_edge(7, 8, None);
    dag.add_edge(8, 9, None);
    dag.add_edge(9, 10, None);
    dag.add_edge(10, 11, None);
    dag.add_edge(11, 12, None);
    dag.add_edge(12, 13, None);
    dag.add_edge(13, 14, None);
    dag.add_edge(0, 14, None);
    dag
}

fn test_ouroboros() -> Graph<'static> {
    let mut dag = Graph::new();
    for i in 0..8 {
        let label: &'static str = Box::leak(format!("O{}", i).into_boxed_str());
        dag.add_node(i, label);
    }
    for i in 0..8 {
        dag.add_edge(i, (i + 1) % 8, None);
    }
    dag
}

fn test_cycle_breaking_demo() -> Graph<'static> {
    let mut dag = Graph::new();
    dag.add_node(0, "A");
    dag.add_node(1, "B");
    dag.add_node(2, "C");
    dag.add_node(3, "D");
    dag.add_edge(0, 1, None);
    dag.add_edge(1, 2, None);
    dag.add_edge(2, 3, None);
    dag.add_edge(3, 0, None); // back edge
    dag.add_edge(0, 2, None);
    dag
}
