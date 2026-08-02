# Examples

Runnable learning code, roughly in reading order. Every rendering
example accepts `--csr` to render through the arena/no-alloc pipeline
(`--features arena`) — output is byte-identical to the heap path,
except `Graph::render` conveniences (horizontal chains, cycle banners),
which are heap-only.

```bash
cargo run --example basic
cargo run --example node_painting --features arena -- --csr
```

| Example | Teaches |
|---|---|
| `basic` | Build a graph, render it; `from_edges` batch construction |
| `node_painting` | **Nodes as objects**: `&str`, `BoxedNode`, `CustomNode` with painter + payload |
| `subgraphs` | Clusters: `add_subgraph`, `put_nodes(..).inside(..)`, nesting |
| `edge_label_demo` | Edge labels and colored rendering |
| `color_demo` | ANSI palettes and colored output |
| `cycles` | Cycle detection and the cycle banner (heap path) |
| `git_log` | A commit-history shaped graph |
| `dependency_analysis` | Roots/leaves/impact queries over a package graph |
| `topological_sort` | Topological ordering (no rendering) |
| `dummy_nodes` | Routing waypoints: `include_dummy_nodes` (IR) + `show_dummy_nodes` (◍ markers) |
| `hit_test` | Interactive mouse hit-testing (raw ANSI mouse reports, zero deps); `--probe X Y` for scripts |
| `hero` | The showcase graph; `--bt`/`--lr`/`--rl` (direction), `--ascii`, `--color`, `--dummy` (mark routing waypoints), `--csr` |
| `hero_colored` | Colored hero with the legend |
| `layout_ir_demo` | The layout IR: positions, JSON export |
| `lean_render` | Direct no-alloc build: `CsrGraphBuilder` + `render_to_bytes` |
| `benchmark` | Timing both pipelines on generated graphs |
| `content_overhead` | Measured node-content storage cost, simple vs boxed vs custom |
| `stress_test` | Large-graph shapes (deep, wide, dense) |
| `subgraph_stress` | Cluster-heavy layouts |

Bare-metal demos live in their own crates (`longan_nano`, `rp2040_pico`,
`esp32s3`) — build them from their own directory. `longan_nano` renders
to a 160×80 LCD with `RenderOptions::ascii()` (its `FONT_4X6` has no
box-drawing glyphs) and `Direction::LeftRight` (40 columns × 13 lines
is wide and short, so levels-as-columns fits where top-down does not);
its whole working set is ~10 KB of stack, no allocator.

`docs/nodes.md` covers the node-content API; `docs/migrate-from-0.9.md`
covers upgrading.
