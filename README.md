# ascii-dag

[![Crates.io](https://img.shields.io/crates/v/ascii-dag.svg)](https://crates.io/crates/ascii-dag)
[![Documentation](https://docs.rs/ascii-dag/badge.svg)](https://docs.rs/ascii-dag)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](LICENSE)

Lightweight ASCII DAG (Directed Acyclic Graph) renderer for error chains, build systems, and dependency visualization.

Perfect for:
- 📋 Error diagnostic visualization (Rust errors, etc.)
- 🔧 Build dependency graphs
- 📊 Task scheduling visualization
- 🌐 IoT/WASM error analysis (no_std compatible)

## Features

- ✅ **Tiny**: ~77KB compiled, zero dependencies
- ✅ **Fast**: Zero-copy rendering, batch construction
- ✅ **no_std**: Works in embedded/WASM environments
- ✅ **Flexible**: Builder API or batch construction
- ✅ **Safe**: Cycle detection built-in
- ✅ **Beautiful**: Clean ASCII art with Unicode box drawing

## Quick Start

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

```rust
pub struct DAG<'a> { /* ... */ }

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

## Size Comparison

| Library | Compiled Size | Dependencies |
|---------|---------------|--------------|
| ascii-dag | ~2-3KB | 0 |
| petgraph | ~200KB+ | Many |

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
```

## Use Cases

- **Error Diagnostics**: Visualize error dependency chains
- **Build Systems**: Show compilation dependencies
- **Task Scheduling**: Display task ordering
- **Data Pipelines**: Illustrate data flow
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
