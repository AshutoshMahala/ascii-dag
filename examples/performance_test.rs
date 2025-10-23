use ascii_dag::DAG;
use std::time::Instant;

fn main() {
    println!("=== Performance Test: Large DAG ===\n");
    
    // Create a large graph with 100 nodes and complex structure
    let mut dag = DAG::new();
    
    // Create labels that live long enough
    let labels: Vec<String> = (0..100).map(|i| format!("Node{}", i)).collect();
    let label_strs: Vec<&str> = labels.iter().map(|s| s.as_str()).collect();
    
    // Add 100 nodes
    let start = Instant::now();
    for i in 0..100 {
        dag.add_node(i, label_strs[i]);
    }
    println!("✓ Added 100 nodes in {:?}", start.elapsed());
    
    // Add edges to create a complex DAG (each node connects to 2-3 children)
    let start = Instant::now();
    for i in 0..80 {
        dag.add_edge(i, i + 10);
        dag.add_edge(i, i + 15);
        if i % 2 == 0 {
            dag.add_edge(i, i + 20);
        }
    }
    println!("✓ Added ~240 edges in {:?}", start.elapsed());
    
    // Render the graph (this is where optimizations matter most)
    let start = Instant::now();
    let output = dag.render();
    let render_time = start.elapsed();
    
    println!("✓ Rendered DAG in {:?}", render_time);
    println!("\nOutput summary:");
    println!("  - {} lines", output.lines().count());
    println!("  - {} characters", output.len());
    
    // Test auto-created node promotion performance
    let start = Instant::now();
    let mut dag2 = DAG::new();
    let promoted_labels: Vec<String> = (50..100).map(|i| format!("Promoted{}", i)).collect();
    let promoted_strs: Vec<&str> = promoted_labels.iter().map(|s| s.as_str()).collect();
    
    for i in 0..50 {
        dag2.add_edge(i, i + 50);  // Auto-creates node i+50
    }
    for i in 50..100 {
        dag2.add_node(i, promoted_strs[i - 50]);  // Promotes placeholder
    }
    println!("\n✓ Promoted 50 auto-created nodes in {:?}", start.elapsed());
    
    println!("\n=== Optimizations Applied ===");
    println!("• O(1) HashMap lookups for id→index (was O(n) scan)");
    println!("• O(1) HashSet for auto_created tracking (was O(n) Vec)");
    println!("• Cached node widths (avoids repeated chars().count())");
    println!("• Eliminated level cloning in Sugiyama passes");
}
