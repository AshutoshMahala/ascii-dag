use ascii_dag::arena::Arena;
use ascii_dag::csr::CsrGraphBuilder;

fn main() {
    let args: Vec<String> = std::env::args().collect();

    // Allocate a buffer on the stack (no heap!)
    // 4KB is plenty for this small graph
    let mut buffer = [0u8; 4096];
    let mut arena = Arena::new(&mut buffer);

    // Create builder
    // Capacity: 4 nodes, 4 edges, 16 bytes for labels
    let mut builder = CsrGraphBuilder::new(&mut arena, 4, 4, 16).unwrap();

    // Add nodes
    // Returns indices 0, 1, 2
    let n0 = builder.add_node(1, "Root").unwrap();
    let n1 = builder.add_node(3, "Leaf").unwrap();
    let n2 = builder.add_node(4, "Child").unwrap();

    // Add edges
    // Root(0) -> Child(2)
    // Child(2) -> Leaf(1)
    builder.add_edge(n0, n2).unwrap();
    builder.add_edge(n2, n1).unwrap();

    let graph = builder.build().unwrap();

    let mut render_buf = [0u8; 1024];
    if let Some(bytes) = graph.render_to_buffer(&mut render_buf)
        && args.len() > 100
    {
        // Prevent optimization
        let s = unsafe { core::str::from_utf8_unchecked(&render_buf[..bytes]) };
        println!("{}", s);
    }
}
