# ascii-dag

[![Crates.io](https://img.shields.io/crates/v/ascii-dag.svg)](https://crates.io/crates/ascii-dag)
[![Documentation](https://docs.rs/ascii-dag/badge.svg)](https://docs.rs/ascii-dag)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](LICENSE)

DAG layout engine. Zero dependencies. `no_std` ready.

![Example Image](assets/hero_colored_heap.png)

**ascii-dag** is a high-performance **layout engine** for placing nodes and routing edges in a fixed-width grid.

## Why?
- **Zero Dependencies**: Drop it into any `no_std`, WASM, or embedded project.
- **Visual Error Chains**: Show users *why* their build failed (Cycle detected? Dependency missing?).
- **Fast**: ~5ms for 1000 nodes with Arena layout.

## Features
- **Tiny**: ~55KB WASM
- **Fast**: Sugiyama layout with median crossing reduction
- **Headless**: Layout IR for custom renderers (Canvas, SVG, TUI)
- **Robust**: Handles diamonds, cycles, skip-level edges
- **Embedded**: `no_std`, no-alloc (Arena mode)
- **Colored Edges**: Up to 8 colors with automatic greedy coloring
- **Edge Labels**: Inline labels displayed along edge paths


## Use Cases

**Tool Authors**
- Error diagnostics ("Circular dependency in module X")
- Build systems (task execution graphs)
- Package managers (dependency trees with diamonds)

**Systems Engineers**
- Data pipelines (ETL, Airflow DAGs)
- Distributed systems (request tracing)

**Embedded & Web**
- IoT consoles (state machines on devices with no display)
- Headless rendering (Canvas/SVG from Layout IR)

## Alternatives Comparison

Here's a quick comparison with other popular graph tools to help you choose the right tool for your needs:

| Feature | ascii-dag | petgraph | Graphviz (dot) |
|---|---:|:---:|:---:|
| Primary Goal | Visualization (Terminal) | Algorithms (Shortest Path, etc.) | Visualization (Image / SVG / PDF) |
| Dependencies | 0 (Zero) | Minimal | Heavy (binary / C libs) |
| WASM Size | **~46 - 55 KB** | ~30 KB | 2 MB+ (via Viz.js) |
| Layout Engine | Built-in (Sugiyama) | None (manual positioning) | Built-in (advanced, many options) |
| Environment | Terminal / Web / Headless | Code / Logic | Desktop / Web

Use ascii-dag when you want compact, zero-dependency terminal visualization with a built-in layout engine. If you need heavy graph algorithms use `petgraph`; for high-fidelity image output and advanced layout options, use Graphviz.



## Quick Start

### DAG Rendering

```rust
use ascii_dag::DAG;

fn main() {
    // Batch construction (fast!)
    let dag = DAG::from_edges(
        &[(1, "Error1"), (2, "Error2"), (3, "Error3")],
        &[(1, 2), (2, 3)]
    );
    
    println!("{}", dag.render());
}
```

Output:
```
  [Error1]
   │
   ↓
  [Error2]
   │
   ↓
  [Error3]

```

### Generic Cycle Detection

Detect cycles in **any data structure** using higher-order functions:

```rust
use ascii_dag::algorithms::cycles::generic::detect_cycle_fn;

// Example: Check for circular dependencies in a package manager
let get_deps = |package: &str| match package {
    "app" => vec!["lib-a", "lib-b"],
    "lib-a" => vec!["lib-c"],
    "lib-b" => vec!["lib-c"],
    "lib-c" => vec![],  // No cycle
    _ => vec![],
};

let packages = ["app", "lib-a", "lib-b", "lib-c"];
if let Some(cycle) = detect_cycle_fn(&packages, get_deps) {
    panic!("Circular dependency: {:?}", cycle);
} else {
    println!("✓ No cycles detected");
}
```

## Usage

### Builder API (Dynamic Construction)

```rust
use ascii_dag::DAG;

let mut dag = DAG::new();

// Add nodes
dag.add_node(1, "Parse");
dag.add_node(2, "Compile");
dag.add_node(3, "Link");

// Add edges (dependencies) - third parameter is optional label
dag.add_edge(1, 2, None);  // Parse -> Compile
dag.add_edge(2, 3, None);  // Compile -> Link

println!("{}", dag.render());
```

### Batch Construction (Static, Fast)

```rust
let dag = DAG::from_edges(
    &[
        (1, "A"),
        (2, "B"),
        (3, "C"),
        (4, "D"),
    ],
    &[
        (1, 2),  // A -> B
        (1, 3),  // A -> C
        (2, 4),  // B -> D
        (3, 4),  // C -> D (diamond!)
    ]
);

println!("{}", dag.render());
```

Output:
```text
   [A]
    │
 ┌─────┐
 ↓     ↓
[B]   [C]
 │     │
 └─────┘
    ↓
   [D]
```

### Zero-Copy Rendering

```rust
let dag = DAG::from_edges(...);
let mut buffer = String::with_capacity(dag.estimate_size());
dag.render_to(&mut buffer);  // No allocation!
```

### Colored Edges

Render edges with distinct colors to visualize different dependency types or highlight paths:

```rust
use ascii_dag::DAG;
use ascii_dag::render::colors::Palette;

let dag = DAG::from_edges(
    &[(1, "Root"), (2, "A"), (3, "B"), (4, "C"), (5, "End")],
    &[(1, 2), (1, 3), (1, 4), (2, 5), (3, 5), (4, 5)]
);

let ir = dag.compute_layout();

// Render with colors (greedy coloring minimizes same-color crossings)
let output = ir.render_scanline_colored(Palette::Ansi);
println!("{}", output);
```

### Edge Labels

Add labels to edges to describe relationships:

```rust
use ascii_dag::DAG;
use ascii_dag::render::colors::Palette;

let mut dag = DAG::new();
dag.add_node(1, "Parser");
dag.add_node(2, "Lexer");
dag.add_node(3, "AST");

dag.add_edge(1, 2, Some("uses"));     // Labeled edge
dag.add_edge(1, 3, Some("produces")); // Labeled edge  
dag.add_edge(2, 3, None);              // No label

let ir = dag.compute_layout();

// Render with colors and legend for any skipped labels
let output = ir.render_scanline_colored_with_legend(Palette::Ansi);
println!("{}", output);
```

### Cycle Detection

```rust
use ascii_dag::DAG;

let mut dag = DAG::new();
dag.add_node(1, "A");
dag.add_node(2, "B");
dag.add_node(3, "C");

dag.add_edge(1, 2, None);
dag.add_edge(2, 3, None);
dag.add_edge(3, 1, None);  // Cycle!

if dag.has_cycle() {
    eprintln!("Error: Circular dependency detected!");
}
```

### Generic Cycle Detection for Custom Types

Use the trait-based API for cleaner code:

```rust
use ascii_dag::algorithms::cycles::generic::CycleDetectable;

struct ErrorRegistry {
    errors: HashMap<usize, Error>,
}

impl CycleDetectable for ErrorRegistry {
    type Id = usize;
    
    fn get_children(&self, id: &usize) -> Vec<usize> {
        self.errors.get(id)
            .map(|e| e.caused_by.clone())
            .unwrap_or_default()
    }
}

// Now just call:
if registry.has_cycle() {
    panic!("Circular error chain detected!");
}
```

### Root Finding & Impact Analysis

```rust
use ascii_dag::algorithms::cycles::generic::roots::find_roots_fn;
use ascii_dag::algorithms::generic::impact::compute_descendants_fn;

let get_deps = |pkg: &&str| match *pkg {
    "app" => vec!["lib-a", "lib-b"],
    "lib-a" => vec!["core"],
    "lib-b" => vec!["core"],
    "core" => vec![],
    _ => vec![],
};

let packages = ["app", "lib-a", "lib-b", "core"];

// Find packages with no dependencies (can build first)
let roots = find_roots_fn(&packages, get_deps);
// roots = ["core"]

// What breaks if "core" changes?
let impacted = compute_descendants_fn(&packages, &"core", get_deps);
// impacted = ["lib-a", "lib-b", "app"]
```

### Node Collection and Traversal

```rust
use ascii_dag::algorithms::generic::traversal::collect_all_nodes_fn;

// Collect all reachable nodes (handles cycles automatically)
let all_nodes = collect_all_nodes_fn(&["start"], |node| {
    // Return children for each node
    get_children(node)
});

// Use case: PII redaction in error diagnostics
let error_ids = collect_all_nodes_fn(&[root_error], |&id| {
    get_related_errors(id)  // Includes caused_by and related
});

// Now redact PII from all errors in the chain
for error_id in error_ids {
    redact_pii(&mut diagnostics[error_id]);
}
```

### Graph Metrics

```rust
use ascii_dag::algorithms::generic::metrics::GraphMetrics;

let metrics = GraphMetrics::compute(&packages, get_deps);
println!("Total packages: {}", metrics.node_count());
println!("Dependencies: {}", metrics.edge_count());
println!("Max depth: {}", metrics.max_depth());
println!("Avg dependencies: {:.2}", metrics.avg_dependencies());
println!("Is tree: {}", metrics.is_tree());
```

## Supported Patterns

### Simple Chain
```text
[A] → [B] → [C]
```

### Diamond (Convergence)
```text
   [A]
    │
 ┌─────┐
 ↓     ↓
[B]   [C]
 │     │
 └─────┘
    ↓
   [D]
```

### Variable-Length Paths
```text
     [Root]
        │
   ┌─────────┐
   ↓         ↓
[Short]   [Long1]
   │         │
   ↓         ↓
   │       [Long2]
   │         │
   └─────────┘
        ↓
      [End]
```

### Multi-Convergence
```text
[E1]   [E2]   [E3]
  │      │      │
  └──────┴──────┘
        ↓
     [Final]
```

## no_std Support

```rust
#![no_std]
extern crate alloc;

use ascii_dag::DAG;

// Works in embedded environments!
```

## WASM Integration

```rust
use wasm_bindgen::prelude::*;
use ascii_dag::DAG;

#[wasm_bindgen]
pub fn render_errors() -> String {
    let dag = DAG::from_edges(
        &[(1, "Error1"), (2, "Error2")],
        &[(1, 2)]
    );
    dag.render()
}
```

## API Reference

### Core Modules

The library is organized into focused, independently-usable modules:

#### `ascii_dag::graph` - DAG Structure
```rust
use ascii_dag::graph::DAG;  // or just `use ascii_dag::DAG;`

impl<'a> DAG<'a> {
    // Construction
    pub fn new() -> Self;
    pub fn from_edges(nodes: &[(usize, &'a str)], edges: &[(usize, usize)]) -> Self;
    
    // Building
    pub fn add_node(&mut self, id: usize, label: &'a str);
    pub fn add_edge(&mut self, from: usize, to: usize);
    
    // Rendering
    pub fn render(&self) -> String;
    pub fn render_to(&self, buf: &mut String);
    pub fn estimate_size(&self) -> usize;
    
    // Validation
    pub fn has_cycle(&self) -> bool;
}
```

#### `ascii_dag::algorithms::cycles::generic` - Generic Cycle Detection
```rust
use ascii_dag::algorithms::cycles::generic::{detect_cycle_fn, CycleDetectable};

// Function-based API
pub fn detect_cycle_fn<Id, F>(
    all_ids: &[Id],
    get_children: F
) -> Option<Vec<Id>>
where
    Id: Clone + Eq + Hash,
    F: Fn(&Id) -> Vec<Id>;

// Trait-based API
pub trait CycleDetectable {
    type Id: Clone + Eq + Hash;
    fn get_children(&self, id: &Self::Id) -> Vec<Self::Id>;
    fn has_cycle(&self) -> bool { /* ... */ }
    fn find_cycle(&self) -> Option<Vec<Self::Id>> { /* ... */ }
}
```

#### `ascii_dag::algorithms::sugiyama` - Graph Layout
Sugiyama hierarchical layout algorithm for positioning nodes.

#### `ascii_dag::algorithms::generic` - Generic Graph Algorithms
Impact analysis, traversal, and metrics for any graph-like data structure.

#### `ascii_dag::render` - ASCII Rendering
Vertical, horizontal, and cycle visualization modes.

## Migrating to 0.9.0

The module hierarchy was reorganized for clarity. Old paths still compile but emit deprecation warnings.

| Old Path (deprecated) | New Path |
|---|---|
| `ascii_dag::arena` | `ascii_dag::graph::arena` |
| `ascii_dag::csr` | `ascii_dag::graph::csr` |
| `ascii_dag::cycles` | `ascii_dag::algorithms::cycles` |
| `ascii_dag::layout` | `ascii_dag::algorithms::sugiyama` |
| `ascii_dag::layout::generic` | `ascii_dag::algorithms::generic` |

Top-level convenience re-exports (`ascii_dag::DAG`, `ascii_dag::LayoutIR`, etc.) remain unchanged.

## How it Works (Algorithms & Tradeoffs)

This library implements a **pragmatic variation** of the Sugiyama Layered Graph Layout algorithm, optimized for speed and readability in fixed-width terminals.

| Phase | Standard Sugiyama | ascii-dag Implementation | Why? |
| :--- | :--- | :--- | :--- |
| **Cycle Breaking** | Edge Reversal | **Explicit Visualization** | We usually *want* to see cycles in errors/deps, not hide them. |
| **Layering** | Simplex / Longest Path | **Iterative Longest Path** | Fast, deterministic `O(N)` layering. |
| **Crossing Reduction** | Barycenter Method | **Median Heuristic** | Efficiently untangles most common spaghetti patterns. |
| **Routing** | Spline Routing | **Grid-Router & "Side-Channel"** | Long skip-edges are routed via "dummy nodes" to the side, preventing visual clutter in the main flow. |

## Limitations & Design Choices (v0.8.x)

This is a production-ready, zero-dependency rendering engine.

### Rendering
- **Grid-based Layout**: Positions are snapped to character cells. Perfect for terminals, less flexible than pixel-based layouts.
- **Unicode box characters**: Uses `│`, `└`, `─` etc. requires a Unicode-capable font (Cascadia Code, Fira Code, etc).
- **"Side-Channel" Routing**: Skip-level edges (A -> D, skipping B/C) are routed along the outer edges of the graph to avoid cutting through the center. (This is a feature, not a bug!)

### Performance
- **Optimized hot paths**: O(1) HashMap lookups, cached widths, zero allocations in rendering loop.
- **Scale**:
    - **Tiny**: ~46-55KB WASM binary.
    - **Fast**: Renders 1000+ nodes in milliseconds.
    - **Scalable**: Regularly tested with 50,000+ nodes and deep hierarchies.

### What This Crate Does Well
**Error Chain Visualization**: The primary use-case.
**CLI Build Tools**: Visualizing task dependencies in terminal output.
**Embedded/WASM**: Works where heavy layout engines can't run.

### What To Use Instead
- **Graphviz/Dot**: If you need SVG export or non-hierarchical layouts.
- **Petgraph**: If you need complex graph theory algorithms (shortest path, max flow).

## Examples

Run examples:
```bash
cargo run --example basic
cargo run --example error_chain
cargo run --example generic_cycles      # Generic cycle detection
cargo run --example error_registry      # Error chain with cycle detection
cargo run --example topological_sort    # Dependency ordering
cargo run --example dependency_analysis # Full dependency analysis suite
cargo run --example layout_ir_demo      # Build → Compute IR → Process → Render workflow
```

## Performance & Configuration

### Optimizations

The library is optimized for both performance and bundle size:

- **Cached Adjacency Lists**: O(1) child/parent lookups instead of O(E) iteration
- **Zero-Copy Rendering**: Direct buffer writes without intermediate allocations
- **Cached Node Widths**: Pre-computed to avoid repeated string formatting
- **HashMap Indexing**: O(1) ID→index lookups instead of O(N) scans

### Feature Flags

Control bundle size and memory usage:

```toml
[dependencies]
ascii-dag = { version = "0.8", default-features = false }
```

**Available features:**

| Feature | Default | Description |
|---------|---------|-------------|
| `std`   | ✓ | Standard library support |
| `alloc` | (via std) | Heap allocation (`Vec`, `String`) |
| `generic` | ✓ | Cycle detection, topological sort, impact analysis, metrics |
| `warnings` | | Debug warnings for auto-created nodes |

**Arena mode (no-alloc, embedded):**

| Feature | Description |
|---------|-------------|
| `arena` | Enable arena allocator (no heap, fixed memory) |
| `arena-idx-u8` | Max 255 nodes/edges (tiny MCUs, 2-8KB RAM) |
| `arena-idx-u16` | Max 65,535 nodes/edges (small MCUs, 16-256KB RAM) |
| `arena-idx-u32` | Max 4B nodes/edges (default for desktop) |

**Bundle Size (Optimized WASM):**
- **Heap Mode** (with `std`): ~69 KB
- **Arena Mode** (minimal `no_std`): ~14 KB

### Resource Limits

**Tested configurations**:
- ✅ Up to 1,000 nodes with acceptable performance
- ✅ Dense graphs (high edge count) handled efficiently via cached adjacency lists
- ⚠️ Very large graphs (>10,000 nodes) will take seconds to layout but will complete successfully.

**Benchmark Results** (Apple M2 Ultra, ARM64, Release Build):

| Topology | Nodes | Mode | Build | Compute | Render | **Total** | Speedup |
| :--- | ---: | :--- | ---: | ---: | ---: | ---: | ---: |
| **Chain** | 100 | Heap | 27µs | 101µs | 27µs | **157µs** | |
| | | Arena | 3µs | 19µs | 54µs | **77µs** | **2.0x** |
| **Chain** | 500 | Heap | 97µs | 646µs | 131µs | **875µs** | |
| | | Arena | 8µs | 32µs | 505µs | **546µs** | **1.6x** |
| **Diamond** | 100 | Heap | 28µs | 219µs | 79µs | **327µs** | |
| | | Arena | 2µs | 13µs | 93µs | **109µs** | **3.0x** |
| **Diamond** | 500 | Heap | 117µs | 1198µs | 391µs | **1708µs** | |
| | | Arena | 12µs | 48µs | 1024µs | **1085µs** | **1.6x** |
| **WideFan** | 100 | Heap | 25µs | 107µs | 114µs | **247µs** | |
| | | Arena | 2µs | 10µs | 231µs | **244µs** | **1.0x** |
| **WideFan** | 500 | Heap | 121µs | 794µs | 2781µs | **3697µs** | |
| | | Arena | 17µs | 46µs | 2µs | **65µs** | **56.9x** |

*Chain = linear (best case), Diamond = skip-level edges (stress), WideFan = fan-out/fan-in (crossing worst case)*
*Measured via `cargo run --example benchmark --release`*

**Embedded Performance** (RP2040 / Cortex-M0+ @ 125MHz):

| Graph | Nodes | Mode | Build | Compute | Render | **Total** | RAM | Speedup |
| :--- | :--- | :--- | :--- | :--- | :--- | :--- | :--- | :--- |
| **Chain 10** | 10 | **Heap** | 0.4 ms | 1.8 ms | 0.6 ms | **2.8 ms** | 4.2 KB | |
| | | **Arena** | 0.3 ms | 0.5 ms | 0.4 ms | **1.2 ms** | **0.7 KB** | **2.3x** |
| **Chain 50** | 50 | **Heap** | 1.4 ms | 7.4 ms | 2.2 ms | **11.0 ms** | 20.4 KB | |
| | | **Arena** | 0.6 ms | 0.9 ms | 3.6 ms | **5.1 ms** | **3.7 KB** | **2.2x** |
| **Chain 100** | 100 | **Heap** | 2.9 ms | 15.3 ms | 4.3 ms | **22.6 ms** | 40.7 KB | |
| | | **Arena** | 1.3 ms | 1.6 ms | 12.1 ms | **15.0 ms** | **7.5 KB** | **1.5x** |

**Key Embedded Win**: Arena layout computation is **~10x faster** than Heap (indices vs pointers) and uses **5.5x less RAM**.
**Note**: Arena rendering is slightly slower due to fixed-buffer safety checks, but the massive layout speedup yields a faster overall result.

*Measured on physical hardware (Raspberry Pi Pico) using `examples/rp2040_pico`.*

**Embedded Performance** (ESP32-S3 / Xtensa LX7 @ 240MHz):

| Graph | Nodes | Edges | Build | Render | RAM |
| :--- | ---: | ---: | ---: | ---: | ---: |
| **Diamond** | 4 | 4 | 0.5ms | 2.5ms | 1.5 KB |
| **Build Pipeline** | 10 | 12 | 0.4ms | 4.2ms | 3.4 KB |
| **Fan-Out/Fan-In** | 12 | 16 | 0.5ms | 3.8ms | 4.6 KB |
| **Binary Tree** | 31 | 30 | 0.9ms | 7ms | 11.5 KB |
| **Deep Chain** | 50 | 49 | 1.8ms | 3.5ms | 19.5 KB |
| **Diamond Lattice** | 64 | 112 | 2.7ms | 18.8ms | 25.5 KB |

*Measured on physical hardware (Seeed XIAO ESP32-S3) using `examples/esp32s3`.*

### Scalability (Stress Tests)

Verified iteratively safe on scaling topologies (no stack overflow):

| Topology | Nodes | Mode | Time | Output Size | Speedup |
| :--- | ---: | :--- | ---: | ---: | ---: |
| **Diamond** | 20,164 | Heap | 1.77s | 0.61 MB | |
| | | Arena | 0.50s | | **3.5x** |
| **Diamond** | 50,176 | Heap | 10.8s | 1.52 MB | |
| | | Arena | 3.07s | | **3.5x** |
| **Wide Fan** | 50,000 | Heap | 26.6s | 2.81 MB | |
| | | Arena | 3.08s | | **8.6x** |

*Tested on Apple M2 Ultra (ARM64), release build. Wide Fan is worst-case for crossing reduction.*

**Performance characteristics**:
- Node/edge insertion: O(1) amortized
- Cycle detection: O(V + E) with early termination
- Rendering: O(V + E) layout (thanks to "Side-Channel" routing optimization)

**Security considerations**:
- Minimal unsafe in arena allocator (Miri-tested)
- Deterministic execution
- For untrusted input, consider limiting graph size to prevent resource exhaustion
- Maximum node ID is `usize::MAX` (formatted as up to 20 digits)


## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.

## Advanced Usage

### Custom Renderers (Layout Engine)
Want to draw to an HTML Canvas, SVG, or ANSI terminal?
Use `compute_layout()` to get the intermediate representation (IR) with calculated positions:

```rust
use ascii_dag::DAG;

let dag = DAG::from_edges(
    &[(1, "A"), (2, "B"), (3, "C")],
    &[(1, 2), (1, 3), (2, 3)]
);

let ir = dag.compute_layout();

// Layout dimensions
println!("Canvas: {}x{} chars", ir.width(), ir.height());
println!("Levels: {}", ir.level_count());

// Iterate nodes with full position info
for node in ir.nodes() {
    println!(
        "Node '{}' (id={}) at ({}, {}), width={}, center_x={}",
        node.label, node.id, node.x, node.y, node.width, node.center_x
    );
}

// Iterate edges with routing info
for edge in ir.edges() {
    println!(
        "Edge {} → {}: from ({},{}) to ({},{})",
        edge.from_id, edge.to_id,
        edge.from_x, edge.from_y,
        edge.to_x, edge.to_y
    );
    
    // Check routing type
    match &edge.path {
        ascii_dag::ir::EdgePath::Direct => println!("  Route: direct"),
        ascii_dag::ir::EdgePath::Corner { horizontal_y } => {
            println!("  Route: L-shaped at y={}", horizontal_y);
        }
        ascii_dag::ir::EdgePath::SideChannel { channel_x, .. } => {
            println!("  Route: side channel at x={}", channel_x);
        }
        ascii_dag::ir::EdgePath::MultiSegment { waypoints } => {
            println!("  Route: {} waypoints", waypoints.len());
        }
    }
}

// Useful helpers
if let Some(node) = ir.node_by_id(2) {       // O(1) lookup
    println!("Found node B at level {}", node.level);
}

for node in ir.nodes_at_level(1) {           // Get nodes at depth 1
    println!("Level 1: {}", node.label);
}

if let Some(node) = ir.node_at(5, 2) {       // Hit testing for mouse interaction
    println!("Clicked on: {}", node.label);
}
```

**IR Structures (Heap mode):**

| Struct | Fields | Description |
|--------|--------|-------------|
| `LayoutIR` | `width()`, `height()`, `level_count()` | Overall layout dimensions |
| `LayoutNode` | `id`, `label`, `x`, `y`, `width`, `center_x`, `level` | Node position & metadata |
| `LayoutEdge` | `from_id`, `to_id`, `from_x`, `from_y`, `to_x`, `to_y`, `path` | Edge routing info |
| `EdgePath` | `Direct`, `Corner`, `SideChannel`, `MultiSegment` | How the edge is routed |

**IR Structures (Arena mode - no-alloc):**

For embedded/no_std environments, use `compute_layout_arena()` which returns `LayoutIRArena`:

```rust
use ascii_dag::graph::arena::Arena;

let mut temp_buffer = [0u8; 16384];
let mut output_buffer = [0u8; 16384];
let mut temp_arena = Arena::new(&mut temp_buffer);
let mut output_arena = Arena::new(&mut output_buffer);

if let Some(ir) = dag.compute_layout_arena(&mut temp_arena, &mut output_arena) {
    // ir is LayoutIRArena - same API as LayoutIR but no heap allocation
    let mut render_buffer = [0u8; 4096];
    let mut line_buffer = [' '; 256];
    ir.render_to_buffer(&mut render_buffer, &mut line_buffer);
}
```

| Arena Struct | Description |
|--------------|-------------|
| `LayoutIRArena` | Arena-allocated layout (slices instead of Vec) |
| `LayoutNodeArena` | Node with fixed-size label reference |
| `LayoutEdgeArena` | Edge with arena-allocated waypoints |

### Render Modes
Control the layout direction.

**⚠️ Warning:** `Horizontal` mode is strictly for linear chains. If used on a branching graph, it will only render the first child and **discard** other branches.

```rust
use ascii_dag::graph::RenderMode;

// Force horizontal (Compact, but lossy for branches)
dag.set_render_mode(RenderMode::Horizontal); 

// Default (Vertical, handles all topology)
dag.set_render_mode(RenderMode::Auto);
```

## Contribution

Contributions welcome! This project aims to stay small and focused.


---

Created by [Ash](https://github.com/AshutoshMahala)
