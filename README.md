# ascii-dag

[![Crates.io](https://img.shields.io/crates/v/ascii-dag.svg)](https://crates.io/crates/ascii-dag)
[![Documentation](https://docs.rs/ascii-dag/badge.svg)](https://docs.rs/ascii-dag)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)

This README describes the **unreleased 0.11** API on `main`.
Requires Rust 1.92 or newer.

A DAG layout engine that renders to text: Sugiyama-style layered
layout (cycle breaking, layering, crossing reduction, edge routing)
with a terminal renderer. The layout pipeline accepts cyclic input —
`compute_layout()` reverses back edges internally and renders them as
dashed return lines, so state machines and feedback graphs draw as
naturally as trees (the legacy `Graph::render()` keeps its
cycle-banner behavior instead). Zero dependencies. `no_std` + no-alloc
ready — the arena pipeline runs without a heap allocator.

<img src="assets/hero_colored_heap.png" alt="the hero example rendered with colors and a legend" height="620"/>

It is for showing a graph *where the user already is*: a build tool
explaining why a task ran, a package manager drawing a dependency
diamond, a CI log annotating a failed pipeline, an embedded console
with a 160×80 display and no framebuffer. Anywhere a picture would
help but opening one is not an option.

Because the layout is separable from the painting, it also works as a
plain layout engine — `compute_layout()` hands back positioned nodes
and routed edges you can draw with Canvas, SVG, or your own widget.

| | ascii-dag | petgraph | Graphviz |
|---|---|---|---|
| For | drawing, in a terminal | graph algorithms | drawing, as an image |
| Dependencies | none | a few | a C toolchain |
| Layout engine | built in | none | built in, far richer |
| WASM | 154–298 KB (0.11, default features; 129–261 KB without `ports`; [details](BENCHMARK.md#bundle-size-wasm)) | ~30 KB | 2 MB+ |

Reach for `petgraph` when you need shortest paths and flows, and
Graphviz when you need publication-quality images. Reach for this when
the output has to be text.

## Installation (0.11 development)

To try the API documented here before 0.11 is published:

```toml
[dependencies]
ascii-dag = { git = "https://github.com/AshutoshMahala/ascii-dag", branch = "main" }
```

This follows the development branch, which can change. For the stable
0.10 line, use `ascii-dag = "0.10"` and its
[release documentation](https://github.com/AshutoshMahala/ascii-dag/tree/release/v0.10).

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
(graph-assigned; returns a `NodeInsertion` receipt whose `.node` is
the `NodeId` handle). Receipts convert into handles for graph methods
that accept ids. Re-adding an existing id replaces that node.
Details: [docs/nodes.md](docs/nodes.md).

### Subgraphs (clusters)

```rust
let sg = g.add_subgraph("Services");
g.put_nodes(&[a, b]).inside(sg)?;      // handles or raw ids
g.put_subgraphs(&[inner]).inside(sg)?; // nesting (cycle-checked)
```

### Cycles

A cycle does not break rendering — the offending edges are reversed
internally and drawn dashed, so you still get a picture. To check
first:

```rust
if g.has_cycle() { /* Graph::render prints a cycle banner instead */ }
```

Cycle detection also works over **your own types**, with no `Graph`
involved — handy for validating a config or build plan before
anything is laid out:

```rust
use ascii_dag::algorithms::cycles::generic::detect_cycle_fn;

let cycle = detect_cycle_fn(&["app", "lib", "core"], |id| match *id {
    "app" => vec!["lib"], "lib" => vec!["core"], "core" => vec!["app"], _ => vec![],
});
assert!(cycle.is_some());
```

`has_cycle_fn` is the boolean form; `topological_sort_fn` (at
`algorithms::generic`) returns an order or the cycle that prevents one.
Both modules need the `generic` feature, on by default.

### Ports

An edge can say which side of a node it leaves from and which side it
arrives on:

```rust
use ascii_dag::PortSide;

g.add_edge(service, audit, Some("trail")).from_port(PortSide::Clockwise);
g.add_edge(gateway, cache, None).to_port(PortSide::West);
```

Compass sides (`North` / `East` / `South` / `West`) stay fixed on the
page; `Upstream`, `Downstream`, `Clockwise` and `Counterclockwise`
follow the layout direction. `Auto` (the default) leaves downstream
and arrives upstream. Each face shares one port by default; graph-wide
or per-node policies can choose paired, spread or custom placement.

The IR records requested and resolved sides; diagnostic-aware layout
reports sides it could not honor. See [the ports guide](docs/ports.md)
for direction mappings, policies and the no-alloc API, or run
[`examples/ports.rs`](examples/ports.rs).

### Render settings (`RenderOptions`)

Options live in three homes by what they affect: `plan` (resolved
semantics), `emit` (how they are written), `compose` (memory only).

| Field | Values | Default |
|---|---|---|
| `emit.charset` | `Unicode` / `Ascii` (equal projections of one canvas) | `Unicode` |
| `emit.color_mode` | `None` / `Ansi256` / `TrueColor` | `None` |
| `emit.render_legend` | print the legend block after the diagram | off |
| `plan.palette` | ANSI palette for edge coloring | `Ansi` |
| `plan.label_policy` | `placement`: `Geometric` / `AvoidNodeRows`; `overflow`: `Omit` / `Legend` | geometric, omit |
| `plan.show_dummy_nodes` | draw `◍` at routing waypoints | off |
| `plan.edge_style_fn` / `.subgraph_style_fn` / `.edge_label_style_fn` | per-element style callbacks (plain `fn`) | legacy look |
| `compose.band_rows_cap` | banded rendering: canvas memory = `width × cap` | 64 |

Presets: `RenderOptions::plain()`, `::colored(palette)`, `::ascii()`,
`::ascii_colored(palette)`. Render surfaces on both IR types:
`render_string`, `render_with(&options, &mut impl fmt::Write)`
(streaming), `render_to_bytes` (no-alloc). Introspection lives on
`Scene` (`ScenePlanner::plan` + `scene.hit_test(x, y)`).

### Layout settings

```rust
use ascii_dag::{Direction, LayoutConfig};

let mut config = LayoutConfig::standard();
config.direction = Direction::LeftRight; // TB (default), BT, LR, RL
config.node_spacing = 4;                 // gap between nodes within a level
config.level_spacing = 1;                // extra gap between levels
config.include_dummy_nodes = true;       // emit routing waypoints into the IR
let ir = g.compute_layout_with_config(&config);
```

All four directions lay out natively. `TB`/`BT` stack levels as rows;
`LR`/`RL` make them columns, which suits graphs with **many levels** —
a long pipeline is tall and scrolls in `TB`, but spends the terminal's
width in `LR`:

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

<img src="assets/hero_arena_rl_dummy.png" alt="the hero example in RightLeft with routing waypoints marked" width="620"/>

*The hero example under `--rl --dummy --color`: levels run right to
left and `◍` marks each routing waypoint.*

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
| `layout-vertical` | ✓ | `TopDown` / `BottomUp` layout |
| `layout-horizontal` | ✓ | `LeftRight` / `RightLeft` layout |
| `arena` | | CSR + arena layout pipeline (no-alloc capable) |
| `arena-idx-u8` / `-u16` / `-u32` | | index width: 255 / 65,535 / 4B nodes, with 16 / 16 / 32-bit cell coordinates (RAM tradeoff) |
| `ports` | ✓ | side ports: declared attachment sides on edges (`from_port` / `to_port`); off, nothing of it is linked |

Typical configurations: defaults for the heap API;
`default-features = false, features = ["arena", "layout-vertical"]`
for vertical no-alloc embedded use; or add `arena` to defaults for the
arena pipeline with the ergonomic `Graph` builder. A `no_std` build
with an allocator can select `alloc` plus its axis features instead.

At least one axis is required when defaults are off. The default
direction is `TopDown` if vertical is enabled, otherwise `LeftRight`.
Feature unification can enable another axis and change this default;
libraries should set direction explicitly. Disabled direction variants
are unavailable; exhaustive matches need a wildcard arm. See
[layout directions](docs/layout.md#directions).

Bundle and firmware measurements in [BENCHMARK.md](BENCHMARK.md) name
their measured release and feature set; they are not 0.11 size promises.

<img src="assets/longan_nano.jpg" alt="a Longan Nano board showing an ASCII graph on its LCD" width="620"/>

*The arena pipeline on a Longan Nano (RISC-V, 32 KB RAM, no
allocator) running 0.10.0: `LeftRight` and the ASCII charset, because
the LCD font has no box-drawing glyphs. It builds without the `ports`
feature; see `examples/longan_nano`.*

The recorded 0.11 demo build exceeded SRAM with separate layout,
render and text buffers. The earlier 0.10.x example's 2 KB layout
temp buffer was also too small after the skip-level router increased
the demo's requirement from 856 B to 3,340 B. These are limits of
those demo configurations, not a blanket lack of board support.
The current horizontal-only, no-ports example reuses its 4 KB layout
workspace for rendering, but still needs a fresh hardware check.
Layout scratch also consumes stack outside the arena buffers. See
[the recorded measurements and stack-accounting caveats](BENCHMARK.md#embedded-longan-nano-gd32vf103-risc-v-128-kb-flash--32-kb-ram);
the recorded firmware fits flash (116.8 KB of 128 KB).

## Errors and warnings

Fallible operations return `GraphError`; every variant carries a
diagnostic code via `.code()` and an actionable `.hint()`:

| Variant | Code | Meaning |
|---|---|---|
| `EmptyGraph` | `E.Graph.Node.001` | no nodes to lay out |
| `NodeNotFound` / `SubgraphNotFound` | `E.Graph.Node.021` / `E.Graph.Subgraph.021` | referenced id absent |
| `CycleDetected` / `SubgraphCycle` | `E.Graph.Dag.003` / `E.Graph.Subgraph.003` | DAG / nesting constraint violated |
| `ArenaOom` / `BuilderFailed` | `E.ArenaLayout.Alloc.026` / `…Builder.026` | arena too small — size with `estimate_layout_arena_size` |
| `ExceedsMaxNodes` / `ExceedsMaxLevels` / `ExceedsMaxExtent` | `E.ArenaLayout.Node.004` / `…Level.004` / `…Extent.004` | index-type or coordinate-type capacity exceeded |
| `RenderPlanOom` / `RenderCanvasTooSmall` / `RenderOutputTooSmall` | `E.Render.{Plan,Canvas,Sink}.026` | render buffer too small — size with `estimate_render_*` |

Non-fatal events are typed diagnostics. Opt in to a report to inspect
them; the library never writes to stderr:

```rust
let report = g.layout().reported();
for warning in report.warnings() {
    eprintln!("{}: {warning}", warning.code());
}
let ir = report.outcome().unwrap();
```

Layout warnings cover implicit nodes, crossing-reduction settings and
unhonored port sides. Render planning separately reports an unplaced
label as `W.Render.Label.031` when `plan.label_policy.overflow` is
`LabelOverflow::Omit`. To retain and print those labels, set overflow
to `LabelOverflow::Legend` **and** `emit.render_legend = true`;
enabling legend emission alone does not recover omitted labels.

Point events are receipts at the call site instead: `add_edge` returns
an `EdgeHandle` carrying an `EdgeInsertion` (did an endpoint get
auto-created?); `.receipt()` detaches it from the graph borrow.
`add_node` returns `NodeInsertion` (did this replace a node — with
`AUTO` involved?).

Layout and planning report at different stages. For one report covering
both, share a run and emit the diagnosed scene with `TerminalRenderer`;
the one-shot render conveniences discard planning warnings. See
[the diagnostics guide](docs/diagnostics.md) for the complete recipe,
including errors and bounded no-alloc collection.

## Documentation

- [docs/layout.md](docs/layout.md) — directions, clusters, spacing, crossing reduction, reading the IR
- [docs/rendering.md](docs/rendering.md) — options, styling, streaming, no-alloc output, hit-testing
- [docs/nodes.md](docs/nodes.md) — nodes as objects: painters, payloads, blank nodes
- [docs/ports.md](docs/ports.md) — side ports: the side vocabulary, how a side is drawn, attachments, warnings
- [docs/diagnostics.md](docs/diagnostics.md) — quiet/reported/context entry points, multi-stage reports, bounded sinks
- [docs/migrate-from-0.10.md](docs/migrate-from-0.10.md) — upgrading to 0.11
- [docs/migrate-from-0.9.md](docs/migrate-from-0.9.md) — upgrading to 0.10
- [examples/README.md](examples/README.md) — 20 runnable examples, including heap and arena rendering
- [BENCHMARK.md](BENCHMARK.md) — measured performance, desktop and embedded
- [ARCHITECTURE.md](ARCHITECTURE.md) — how the pipeline works

Upgrading directly from 0.9 to 0.11? Apply both migration guides; no
intermediate installation or release is needed.

## Limitations

Text-grid output: edges route orthogonally (no diagonals), wide
Unicode in labels counts as one cell per `char`, and layouts optimize
for readability rather than minimal area. Very dense graphs (hundreds
of edges crossing one level) reach a point where any text grid stops
being readable — that is a property of the medium, not the layout.

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.

## Versioning and releases

While the crate is pre-1.0, each **minor** version is a compatibility
line: `0.10 → 0.11` may break, `0.10.0 → 0.10.3` will not.

Each minor line lives on its own long-running branch — `release/v0.10`,
`release/v0.11` — and patch releases are tagged on it. There is no
branch per patch: `0.10.1`, `0.10.2` and so on are tags along
`release/v0.10`, which is what makes it possible to ship a fix for
0.10 after `main` has moved on to 0.11.

For example, this selects the stable 0.10 line and its patch fixes,
not the unreleased 0.11 API documented above:

```toml
ascii-dag = "0.10"     # 0.10.x, including later patch fixes
```

## Contribution

Contributions welcome! This project aims to stay small and focused.

Fixes land on `main` first, then get backported to the release
branches that need them — that way a bug fixed in a patch cannot
reappear in the next minor.

---

Created by [Ash](https://github.com/AshutoshMahala)
