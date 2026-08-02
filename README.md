# ascii-dag

[![Crates.io](https://img.shields.io/crates/v/ascii-dag.svg)](https://crates.io/crates/ascii-dag)
[![Documentation](https://docs.rs/ascii-dag/badge.svg)](https://docs.rs/ascii-dag)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](LICENSE)

A DAG layout engine that renders to text: Sugiyama-style layered
layout (cycle breaking, layering, crossing reduction, edge routing)
with a terminal renderer. Zero dependencies. `no_std` ready — the
arena pipeline runs without a heap allocator.

<img src="assets/hero_colored_heap.png" alt="hero example — colored output" width="300"/>

## Example

```rust
use ascii_dag::{Graph, AUTO, RenderOptions};

let mut g = Graph::new();
let fetch  = g.add_node(AUTO, "Fetch");
let build  = g.add_node(AUTO, "Build");
let test   = g.add_node(AUTO, "Test");
let deploy = g.add_node(AUTO, "Deploy");
g.add_edge(fetch, build, None);
g.add_edge(fetch, test, None);
g.add_edge(build, deploy, None);
g.add_edge(test, deploy, Some("gate"));

// Heap path:
println!("{}", g.compute_layout().render_string(&RenderOptions::plain()));
```

```text
          [Fetch]
         ┌───┴─────┐
         ↓         ↓
      [Build]   [Test]
         └────┬────┘
           "gate"
              ↓
          [Deploy]
```

The same graph through the arena/no-alloc pipeline (`--features
arena`) — byte-identical output:

```rust
use ascii_dag::LayoutConfig;
use ascii_dag::graph::arena::Arena;

let mut csr_buf = vec![0u8; g.estimate_csr_arena_size()];
let mut csr_arena = Arena::new(&mut csr_buf);
let csr = g.to_csr(&mut csr_arena).unwrap();

let mut temp = vec![0u8; g.estimate_layout_arena_size()];
let mut out = vec![0u8; g.estimate_layout_arena_size()];
let (mut ta, mut oa) = (Arena::new(&mut temp), Arena::new(&mut out));
let ir = csr.compute_layout_arena(&LayoutConfig::standard(), &mut ta, &mut oa).unwrap();
println!("{}", ir.render_string(&RenderOptions::plain()));
```

On embedded targets the buffers are static arrays and
`render_to_bytes` writes into a caller buffer — no allocation
anywhere. See `examples/lean_render.rs`.

## Usage

### Nodes

The content slot of `add_node` decides what a node *is*:

```rust
g.add_node(1, "Client");                          // [Client]
g.add_node(2, BoxedNode("Database"));             // boxed label
g.add_node(3, CustomNode {                        // your painter + data
    label: "Server", width: 12, height: 5,
    painter: Some(card), payload: "cpu: 4\nram: 16G",
});
```

Ids: explicit (`add_node(7, …)`, for graphs built from external
identities — edges auto-create missing endpoints) or `AUTO`
(graph-assigned; returns a `NodeId` handle usable anywhere an id is
accepted). Re-adding an existing id replaces that node.
Details: [docs/nodes.md](docs/nodes.md).

### Subgraphs (clusters)

```rust
let sg = g.add_subgraph("Services");
g.put_nodes(&[a, b]).inside(sg)?;      // handles or raw ids
g.put_subgraphs(&[inner]).inside(sg)?; // nesting (cycle-checked)
```

### Render settings (`RenderOptions`)

| Field | Values | Default |
|---|---|---|
| `charset` | `Unicode` / `Ascii` (equal projections of one canvas) | `Unicode` |
| `color_mode` | `None` / `Ansi256` / `TrueColor` | `None` |
| `palette` | ANSI palette for edge coloring | `Ansi` |
| `legend` | list labels that could not be placed inline | off |
| `band_rows_cap` | banded rendering: canvas memory = `width × cap` | 64 |
| `show_dummy_nodes` | draw `◍` at routing waypoints | off |
| `edge_style_fn` / `subgraph_style_fn` / `edge_label_style_fn` | per-element style callbacks (plain `fn`) | legacy look |

Presets: `RenderOptions::plain()`, `::colored(palette)`, `::ascii()`,
`::ascii_colored()`. Render surfaces on both IR types:
`render_string`, `render_with(&options, &mut impl fmt::Write)`
(streaming), `render_to_bytes` (no-alloc), `render_plan` + `hit_test`
(introspection).

### Layout settings

```rust
g.set_direction(Direction::LeftRight);   // TB (default), BT, LR, RL
let mut config = LayoutConfig::standard();
config.node_spacing = 4;                 // gap between nodes within a level
config.level_spacing = 1;                // extra gap between levels
config.include_dummy_nodes = true;       // emit routing waypoints into the IR
```

All four directions lay out natively. `TB`/`BT` stack levels as rows;
`LR`/`RL` make them columns, which suits wide, shallow graphs:

```text
TB (default)          LR
                                       ┌→[Store]
  [Fetch]                   "raw"      │
     │ "raw"          [Fetch]──→[Parse]┤
     ↓                                 │
  [Parse]                              └→[Index]
   ┌─┴──┐
   ↓    ↓
[Store] [Index]
```

`BT` and `RL` are exact mirrors of `TB` and `LR`. The spacing settings
follow the direction — `node_spacing` separates nodes within a level,
`level_spacing` separates levels — so the same config reads sensibly
whichever way the graph flows.

Crossing-reduction presets `FAST` / `STANDARD` / `QUALITY` (or a
custom `CrossingReducer` pipeline) via `set_crossing_pipeline`. Full
reference: [docs.rs/ascii-dag](https://docs.rs/ascii-dag).

### Feature flags

| Feature | Default | What it enables |
|---------|:---:|---|
| `std` | ✓ | standard library (implies `alloc`) |
| `alloc` | via `std` | heap `Graph` API on `no_std` (needs a global allocator) |
| `generic` | ✓ | cycle detection / toposort / impact analysis over your own types |
| `arena` | | CSR + arena layout pipeline (no-alloc capable) |
| `arena-idx-u8` / `-u16` / `-u32` | | index width: 255 / 65,535 / 4B nodes (RAM tradeoff) |
| `warnings` | | stderr diagnostics (see below) |

Typical configurations: default (`ascii-dag = "0.10"`) for the heap
API; `default-features = false, features = ["arena"]` for no-alloc
embedded; add `arena` to defaults for the fast arena pipeline with
the ergonomic `Graph` builder.

## Errors and warnings

Fallible operations return `GraphError`; every variant carries a
diagnostic code via `.code()` and an actionable `.hint()`:

| Variant | Code | Meaning |
|---|---|---|
| `EmptyGraph` | `E.Graph.Node.001` | no nodes to lay out |
| `NodeNotFound` / `SubgraphNotFound` | `E.Graph.Node.021` / `E.Graph.Subgraph.021` | referenced id absent |
| `CycleDetected` / `SubgraphCycle` | `E.Graph.Dag.003` / `E.Graph.Subgraph.003` | DAG / nesting constraint violated |
| `ArenaOom` / `BuilderFailed` | `E.ArenaLayout.Alloc.026` / `…Builder.026` | arena too small — size with `estimate_layout_arena_size` |
| `ExceedsMaxNodes` / `ExceedsMaxLevels` | `E.ArenaLayout.Node.004` / `…Level.004` | index-type capacity exceeded |
| `RenderPlanOom` / `RenderCanvasTooSmall` / `RenderOutputTooSmall` | `E.Render.{Plan,Canvas,Sink}.026` | render buffer too small — size with `estimate_render_*` |

Warnings are best-effort stderr diagnostics (they never panic, even
with stderr closed):

| Code | Fires when | Gate |
|---|---|---|
| `W.Graph.Node.021` | an edge referenced a node that was never added (placeholder auto-created) | `warnings` feature |
| `W.Graph.Node.007` | a duplicate `add_node` replaced a node with `AUTO` numbering involved | `warnings` feature |
| `W.Graph.Dag.003` | a layout-config value was clamped (e.g. absurd `crossing_reduction_passes`) | any `std` build |

## Documentation

- [docs/nodes.md](docs/nodes.md) — nodes as objects: painters, payloads, blank nodes
- [docs/migrate-from-0.9.md](docs/migrate-from-0.9.md) — upgrading to 0.10
- [examples/README.md](examples/README.md) — 16 runnable examples; each renders via `--csr` too
- [BENCHMARK.md](BENCHMARK.md) — measured performance, desktop and embedded
- [ARCHITECTURE.md](ARCHITECTURE.md) — how the pipeline works

## Limitations

Text-grid output: edges route orthogonally (no diagonals), wide
Unicode in labels counts as one cell per `char`, and layouts optimize
for readability rather than minimal area. For heavy graph *algorithms*
use `petgraph`; for image-quality output use Graphviz.

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.

## Contribution

Contributions welcome! This project aims to stay small and focused.

---

Created by [Ash](https://github.com/AshutoshMahala)
