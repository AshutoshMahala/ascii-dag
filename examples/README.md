# Examples

Runnable learning code, roughly in reading order. Many rendering
examples accept `--csr` with `--features arena` to show the arena
pipeline; check the individual example's flags. `ports` runs both
pipelines automatically when `arena` is enabled; `lean_render` is
direct no-alloc construction, and `svg_export` reads heap-backed scene
views. Matching IR-rendering options produce byte-identical output
across backends; `Graph::render` conveniences (horizontal chains,
cycle banners) are heap-only.

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
| `ports` | **Side ports**, every section through both pipelines: `from_port` / `to_port`, the three side vocabularies resolved per direction, port policies (`Single` / `Paired` / `Spread` / `Custom`) on one boxed hub, attachments on the IR and in JSON, the port warnings on both pipelines, and the no-alloc builder (`new_with_ports`, `set_edge_ports`, `set_node_port_policy`, exact sizing, the reporting layout entry); `--bt`/`--lr`/`--rl`, `--ascii`; `--features arena` for the no-alloc half |
| `hit_test` | Interactive mouse hit-testing (raw ANSI mouse reports, zero deps); `--probe X Y` for scripts |
| `svg_export` | **Semantic consumption**: a mini SVG exporter over scene views — no terminal text, no charset; `> graph.svg` |
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

[Nodes](../docs/nodes.md) covers node content;
[diagnostics](../docs/diagnostics.md) has a complete multi-stage report
recipe; [rendering](../docs/rendering.md#retain-cells-and-reuse-the-scene)
shows retained semantic cells and fixed-workspace rendering. These are
documentation recipes, not additional `cargo run --example` targets.

For upgrades, use [0.9 → 0.10](../docs/migrate-from-0.9.md) and
[0.10 → 0.11](../docs/migrate-from-0.10.md). Direct 0.9 → 0.11 upgrades
apply both guides without an intermediate installation.
