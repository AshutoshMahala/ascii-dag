//! Example of "Lean" rendering with buffer reuse.
//!
//! This demonstrates how to minimize Peak RAM usage by reusing the
//! temporary layout buffer for the final text rendering.
//!
//! Lifecycle:
//! 1. Buffer A -> Graph Storage (Permanent)
//! 2. Buffer B -> Output Storage (Layout Result, semi-permanent)
//! 3. Buffer C -> Temp Storage (Layout Calculations, transient)
//! 4. ... Layout finishes ...
//! 5. Buffer C -> Render Storage (Text output, reuses Temp memory)

use ascii_dag::algorithms::sugiyama::config::LayoutConfig;
use ascii_dag::graph::arena::Arena;
use ascii_dag::graph::csr::CsrGraphBuilder;

fn main() {
    // 1. Single Block Allocation (e.g. 16KB on stack)
    // In embedded, this might be your entire available RAM.
    let mut heap = [0u8; 16 * 1024];
    println!("Total RAM: {} bytes", heap.len());

    // 2. Partition: Graph Storage
    // We reserve the first 4KB for the graph structure itself.
    let (graph_mem, remaining) = heap.split_at_mut(4096);
    let mut graph_arena = Arena::new(graph_mem);

    // Build Graph (Same as simulate_pure)
    println!("> Building Graph...");
    let mut builder = CsrGraphBuilder::new(&mut graph_arena, 10, 10, 64).expect("arena too small");
    let n0 = builder.add_node(0, "Source").expect("add Source");
    let n1 = builder.add_node(1, "Middle").expect("add Middle");
    let n2 = builder.add_node(2, "Sink").expect("add Sink");
    builder.add_edge(n0, n1).expect("edge Source→Middle");
    builder.add_edge(n1, n2).expect("edge Middle→Sink");
    let graph = builder.build().expect("build graph");

    // 3. Partition: Processing Memory
    // We have 12KB left.
    // Layout needs two parts:
    //  - Output: Where the coordinates/result lives.
    //  - Temp: Scratch space for median calculation, etc.
    let (output_mem, temp_mem) = remaining.split_at_mut(remaining.len() / 2);

    // Note: In a real app, you might tune this split.
    // Complex layouts need more Temp. Complex outputs need more Output.
    println!(
        "> Layout Config: Output={}B, Temp={}B",
        output_mem.len(),
        temp_mem.len()
    );

    let mut output_arena = Arena::new(output_mem);
    // 4. Compute Layout
    // Scope block to ensure temp_arena is dropped before we reuse temp_mem
    let layout = {
        let mut temp_arena = Arena::new(temp_mem);
        graph.compute_layout_arena(
            &LayoutConfig::standard(),
            &mut temp_arena,
            &mut output_arena,
        )
    };

    if let Ok(layout) = layout {
        println!(
            "> Layout Success! Size: {}x{}",
            layout.width(),
            layout.height()
        );

        // 5. BUFFER REUSE MAGIC
        // 'temp_arena' is now dropped.
        // We reuse 'temp_mem' for the render buffer.

        let render_buffer = temp_mem;
        // We also need a small line buffer for the renderer logic

        println!("> Reusing Temp Buffer for Rendering...");

        let options = ascii_dag::render::engine::RenderOptions::plain();
        {
            let mut scratch_buffer = vec![0u8; layout.estimate_render_arena_size(&options)];
            let render_arena = ascii_dag::graph::arena::Arena::new(&mut scratch_buffer);
            if let Ok(bytes) =
                layout.render_to_bytes(&options, &render_arena, render_buffer)
            {
                if let Ok(s) = core::str::from_utf8(&render_buffer[..bytes]) {
                    println!("\n{}", s);
                }
                println!("> Rendered {} bytes into reused memory.", bytes);
            } else {
                println!("> Render failed: Output buffer too small!");
            }
        }
    } else if let Err(e) = layout {
        println!("> Layout failed: {}", e);
    }
}
