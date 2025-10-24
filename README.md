# ascii-dag

[![Crates.io](https://img.shields.io/crates/v/ascii-dag.svg)](https://crates.io/crates/ascii-dag)
[![Documentation](https://docs.rs/ascii-dag/badge.svg)](https://docs.rs/ascii-dag)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](LICENSE)

Modular ASCII DAG (Directed Acyclic Graph) renderer and generic cycle detection library for error chains, build systems, and dependency visualization.

Perfect for:
- 📋 Error diagnostic visualization (Rust errors, etc.)
- 🔧 Build dependency graphs
- 📊 Task scheduling visualization
- 🔄 Generic cycle detection in any data structure
- 🌐 IoT/WASM error analysis (no_std compatible)

## Features

- ✅ **Tiny**: ~77KB WASM (minimal example with release optimizations), zero dependencies
- ✅ **Fast**: O(log n) grouping with binary search, zero-copy rendering
- ✅ **no_std**: Works in embedded/WASM environments
- ✅ **Modular**: Use DAG rendering, cycle detection, or both independently
- ✅ **Generic**: Cycle detection, topological sorting, and dependency analysis work on any data structure
- ✅ **Rich Analysis**: Root finding, impact analysis, graph metrics
- ✅ **Safe**: Cycle detection built-in
- ✅ **Beautiful**: Clean ASCII art with Unicode box drawing

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
use ascii_dag::cycles::generic::detect_cycle_fn;

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

// Add edges (dependencies)
dag.add_edge(1, 2);  // Parse -> Compile
dag.add_edge(2, 3);  // Compile -> Link

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
```

  [A]
   │ ─────────│
   ↓          ↓
  [B]        [C]
   └─────────┘
        ↓
       [D]

```

### Zero-Copy Rendering

```rust
let dag = DAG::from_edges(...);
let mut buffer = String::with_capacity(dag.estimate_size());
dag.render_to(&mut buffer);  // No allocation!
```

### Cycle Detection

```rust
use ascii_dag::DAG;

let mut dag = DAG::new();
dag.add_node(1, "A");
dag.add_node(2, "B");
dag.add_node(3, "C");

dag.add_edge(1, 2);
dag.add_edge(2, 3);
dag.add_edge(3, 1);  // Cycle!

if dag.has_cycle() {
    eprintln!("Error: Circular dependency detected!");
}
```

### Generic Cycle Detection for Custom Types

Use the trait-based API for cleaner code:

```rust
use ascii_dag::cycles::generic::CycleDetectable;

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
use ascii_dag::cycles::generic::roots::find_roots_fn;
use ascii_dag::layout::generic::impact::compute_descendants_fn;

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

### Graph Metrics

```rust
use ascii_dag::layout::generic::metrics::GraphMetrics;

let metrics = GraphMetrics::compute(&packages, get_deps);
println!("Total packages: {}", metrics.node_count());
println!("Dependencies: {}", metrics.edge_count());
println!("Max depth: {}", metrics.max_depth());
println!("Avg dependencies: {:.2}", metrics.avg_dependencies());
println!("Is tree: {}", metrics.is_tree());
```

## Supported Patterns

### Simple Chain
```
[A] -> [B] -> [C]
```

### Diamond (Convergence)
```
    [A]
   /   \
  [B] [C]
   \   /
    [D]
```

### Variable-Length Paths
```
[Root]
  ├─→ [Short] ──────┐
  │                 │
  └─→ [Long1]       │
       │            │
       ↓            │
      [Long2]       │
       └────────────┘
            ↓
          [End]
```

### Multi-Convergence
```
[E1]   [E2]   [E3]
  └─────┴──────┘
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
use ascii_dag::graph::DAG;  // or just `use ascii_dag::DAG;` for backward compat

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

#### `ascii_dag::cycles::generic` - Generic Cycle Detection
```rust
use ascii_dag::cycles::generic::{detect_cycle_fn, CycleDetectable};

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

#### `ascii_dag::layout` - Graph Layout
Sugiyama hierarchical layout algorithm for positioning nodes.

#### `ascii_dag::render` - ASCII Rendering
Vertical, horizontal, and cycle visualization modes.

## Limitations & Design Choices (v0.1.x)

This is an **initial 0.x release** focused on simplicity and zero dependencies. Current limitations:

### Rendering
- **No cross-level edge routing**: Long-distance edges are simplified (suitable for error chains, not general graphs)
- **Unicode box characters required**: Best viewed in terminals with Unicode support
- **Small-to-medium graphs**: Optimized for <1000 nodes (typical error chains, build graphs)

### Auto-created Nodes
- Nodes referenced in edges are **auto-created as placeholders** (`⟨ID⟩` format)
- Calling `add_node()` on a placeholder **promotes it** to a labeled node (`[Label]` format)
- This enables flexible graph construction (add edges first, labels later)

### Performance
- **Optimized hot paths**: O(1) HashMap lookups, cached widths, zero allocations in rendering
- **Intended scale**: Hundreds of nodes render in microseconds
- Not optimized for: Massive graphs (>10k nodes), real-time updates, interactive editing

### API Stability
- **0.x series**: Breaking changes possible between minor versions
- **Focused scope**: No plans to add general graph algorithms (use petgraph for that)
- **Simple is better**: Will resist feature creep to maintain zero dependencies

### What This Crate Does Well
✅ Error chain visualization (primary use case)  
✅ Build dependency graphs  
✅ Small task DAGs  
✅ no_std/WASM compatibility  
✅ Fast compilation, tiny binaries  

### What To Use Instead
- **Large graphs with layout algorithms** → graphviz, petgraph
- **Interactive graph editing** → egui-graphs, graph-viz
- **Advanced graph algorithms** → petgraph, pathfinding

## Examples

Run examples:
```bash
cargo run --example basic
cargo run --example error_chain
cargo run --example generic_cycles      # Generic cycle detection
cargo run --example error_registry      # Error chain with cycle detection
cargo run --example topological_sort    # Dependency ordering
cargo run --example dependency_analysis # Full dependency analysis suite
```

## Performance & Configuration

### Optimizations

The library is optimized for both performance and bundle size:

- **Cached Adjacency Lists**: O(1) child/parent lookups instead of O(E) iteration
- **Zero-Copy Rendering**: Direct buffer writes without intermediate allocations
- **Cached Node Widths**: Pre-computed to avoid repeated string formatting
- **HashMap Indexing**: O(1) ID→index lookups instead of O(N) scans

### Feature Flags

Control bundle size by enabling only what you need:

```toml
[dependencies]
ascii-dag = { version = "0.1", default-features = false, features = ["std"] }
```

Available features:
- `std` (default): Standard library support
- `generic` (default): Generic cycle detection, topological sort, impact analysis, and metrics
- `warnings`: Enable debug warnings for auto-created nodes

**Bundle Size Impact**:
- Core renderer only (`--no-default-features --features std`): ~41KB WASM
- With generic features (default): ~77KB WASM

### Resource Limits

**Tested configurations**:
- ✅ Up to 1,000 nodes with acceptable performance
- ✅ Dense graphs (high edge count) handled efficiently via cached adjacency lists
- ⚠️ Very large graphs (>10,000 nodes) may experience slower layout computation

**Memory usage**:
- Base overhead: ~100 bytes per node (cached data structures)
- Adjacency lists: ~16 bytes per edge (index storage)
- Rendering buffers: Pre-allocated based on graph size estimate

**Performance characteristics**:
- Node/edge insertion: O(1) amortized
- Cycle detection: O(V + E) with early termination
- Rendering: O(V log V + E) for layout, O(V) for output generation

**Security considerations**:
- No unsafe code
- Deterministic execution
- For untrusted input, consider limiting graph size to prevent resource exhaustion
- Maximum node ID is `usize::MAX` (formatted as up to 20 digits)

## Use Cases

- **Error Diagnostics**: Visualize error dependency chains with cycle prevention
- **Build Systems**: Show compilation dependencies and detect circular imports
- **Task Scheduling**: Display task ordering and validate DAG structure
- **Data Pipelines**: Illustrate data flow and check for feedback loops
- **Package Managers**: Detect circular dependencies in packages
- **Generic Cycle Detection**: Apply to any tree/graph structure via closures
- **IoT**: Lightweight error reporting
- **WASM**: Client-side error visualization

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.

## Contribution

Contributions welcome! This project aims to stay small and focused.

## Why ascii-dag?

- **Simple and focused**: Does one thing well - ASCII DAG rendering
- **Not petgraph**: We don't need general graph algorithms, just visualization
- **Not graphviz**: No external dependencies, works everywhere
- **Zero dependencies**: Works in no_std, WASM, and embedded environments

---

Created by [Ash](https://github.com/AshutoshMahala)
