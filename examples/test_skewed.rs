use ascii_dag::DAG;

fn main() {
    let mut dag = DAG::new();

    dag.add_node(1, "L");
    dag.add_node(2, "R");
    dag.add_node(3, "X");
    dag.add_node(4, "Y");

    // Create skewed children: L connects to rightmost, R connects to leftmost
    dag.add_edge(1, 4); // L -> Y (right child)
    dag.add_edge(2, 3); // R -> X (left child)

    let output = dag.render();
    println!("{}", output);

    // Find line positions of each node
    let lines: Vec<&str> = output.lines().collect();
    for (idx, line) in lines.iter().enumerate() {
        println!("Line {}: '{}'", idx, line);
    }

    let find_node = |name: &str| -> (usize, usize) {
        for (line_idx, line) in lines.iter().enumerate() {
            if let Some(col) = line.find(name) {
                return (line_idx, col);
            }
        }
        panic!("Node {} not found", name);
    };

    let (l_line, l_col) = find_node("[L]");
    let (r_line, r_col) = find_node("[R]");
    let (x_line, x_col) = find_node("[X]");
    let (y_line, y_col) = find_node("[Y]");

    println!("\nPositions:");
    println!("L: line {}, col {}", l_line, l_col);
    println!("R: line {}, col {}", r_line, r_col);
    println!("X: line {}, col {}", x_line, x_col);
    println!("Y: line {}, col {}", y_line, y_col);
}
