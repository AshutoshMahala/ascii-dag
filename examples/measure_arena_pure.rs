use ascii_dag::csr::CsrGraph;

// Copy of CsrGraph struct layout to simulate it
struct SimulatedCsrGraph<'a> {
    nodes: &'a mut [usize],
    node_count: usize,
    edges: &'a mut [usize],
    edge_count: usize,
    children_offsets: &'a [usize],
    children_data: &'a [usize],
    parents_offsets: &'a [usize],
    parents_data: &'a [usize],
    labels: &'a [u8],
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    
    // Stack data strictly necessary for the graph
    let mut nodes = [1, 0, 4, 3, 4, 4, 2, 8, 5]; // id, label_ptr, label_len
    let mut edges = [0, 2, 2, 1]; // from, to
    let children_off = [0, 1, 1, 2];
    let children_data = [2, 1];
    let parents_off = [0, 0, 1, 2];
    let parents_data = [0, 2];
    let labels = *b"RootLeafChild";
    
    unsafe {
        let sim = SimulatedCsrGraph {
            nodes: &mut nodes,
            node_count: 3,
            edges: &mut edges,
            edge_count: 2,
            children_offsets: &children_off,
            children_data: &children_data,
            parents_offsets: &parents_off,
            parents_data: &parents_data,
            labels: &labels,
        };
        
        // Transmute to the real CsrGraph
        // This effectively "links" only the CsrGraph code
        let graph: &CsrGraph = std::mem::transmute(&sim);
        
        let mut buffer = [0u8; 1024];
        if let Some(bytes) = graph.render_to_buffer(&mut buffer) {
             if args.len() > 100 {
                // Use it to prevent optimization
                println!("{}", std::str::from_utf8_unchecked(&buffer[..bytes]));
             }
        }
    }
}
