use ascii_dag::graph::DAG;

fn main() {
    let mut dag = DAG::new();
    let width = 224;
    let height = 224;
    for y in 0..height {
        for x in 0..width {
            let id = y * width + x;
            dag.add_node(id, ".");
            if y < height - 1 {
                let next_y_base = (y + 1) * width;
                dag.add_edge(id, next_y_base + x, None);
                if x < width - 1 {
                    dag.add_edge(id, next_y_base + (x + 1), None);
                }
            }
        }
    }
    let est = dag.estimate_layout_arena_size();
    println!("50k Diamond: 50176 nodes, ~99681 edges");
    println!("estimate = {est} bytes = {:.1} KB = {:.1} MB",
        est as f64 / 1024.0, est as f64 / 1024.0 / 1024.0);
    println!("per-arena (x1.2) = {:.1} KB", (est * 6 / 5) as f64 / 1024.0);
    println!("total 2 arenas = {:.1} KB", 2.0 * (est * 6 / 5) as f64 / 1024.0);
}
