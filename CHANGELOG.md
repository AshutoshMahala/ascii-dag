# Changelog

## [0.10.0] - Unreleased

### Fixed
- **CSR layout depth is no longer capped at 256 levels**: per-level layout buffers were fixed-size (256), so deeper graphs crashed with an index-out-of-bounds (later a clean `ExceedsMaxLevels`). Buffers are now sized from each graph's real depth — computed before allocation — so depth is bounded only by the index type's node capacity (`arena-idx-u8`: 255; u16: 65,535; u32: ~4B) and the arena the caller provides. Wide, shallow graphs get *cheaper*: the old design charged every graph ~35 KB for 256 levels; a 3-level graph now pays ~0.4 KB, and `estimate_layout_arena_size` computes the exact depth (falling back to node count only for unbroken cycles), so provisioned arenas track the real graph shape. Deep chains are pinned byte-identical across backends at 20,000 levels with exactly estimate-sized arenas. Unbroken cycles (`CycleBreaking::None`) that pump level relaxation past any DAG-possible depth still fail cleanly with `ExceedsMaxLevels`.
- **CSR pipeline lost implicit-node identity**: auto-created nodes (referenced by an edge but never added) rendered as `NodeKind::Explicit` in the arena IR while the heap IR marked them `Implicit` — invisible until a node style consulted `is_implicit`. The CSR conversion now carries the flag (`CsrGraph::node_is_implicit`), and both backends agree.
- **CSR slot allocator now matches the heap allocator's decisions**: the CSR path tracked one bounding box per routing slot where the heap path keeps per-slot interval lists, so the two backends routed fan-ins differently (CSR could not share a routing row between disjoint spans). The CSR allocator now uses pooled interval lists with the identical accept/reject logic — the backends produce byte-identical routing (Massive Diamond CSR output shrank ~20% to exactly the heap output). Costs a temp-arena interval pool (two entries per edge).
- **Nested-cluster boxes: one shared parent-gap rule**: the CSR path expanded a parent cluster's box by the full node padding around a child box (one column too wide per side); both backends now use the shared `PARENT_CHILD_H_GAP` rule in `geometry.rs`.
- **CSR canvas width double-counted its margins**: the CSR path added the routing buffer and label margin into `max_width` and then again into the canvas width, advertising an IR up to 12 columns wider than the heap backend's for the same graph (invisible in rendered text — lines are right-trimmed — but wrong in `width()` and oversized render buffers). Both backends now agree; caught by the render engine's `LayoutView` equivalence tests.
- **O(E²) two-node-cycle detection in the CSR layout path**: membership is now precomputed in one O(E log E) pass (sort by normalized endpoint pair, scan runs for anti-parallel twins with opposite back flags). Massive Diamond 50k CSR layout+render: 6.2 s → 88 ms (70×). **The heap path also gains the feature it silently lacked**: a 2-node cycle sharing a column now renders its anti-parallel pair side by side (`↓` next to `⇡`) in both backends, byte-identically — previously the heap drew both edges overlapping in one column.
- **O(N·E) self-loop scan in the heap layout path**: `has_self_loop` was computed with a full edge scan per node; now a single O(E) pre-pass. Massive Diamond (50k nodes): 2.57 s → 0.38 s (6.7×); 20k: 3.5×. (The CSR path was already O(E).)
- **Heap/CSR layout parity + readable back-edge arrows**: the two backends disagreed on edge-routing geometry, the CSR path silently dropped edge labels (it placed them on its own corner-routing row, where the renderer's collision check discards them), and a reversed edge's `⇡` arrowhead could be embedded in a horizontal routing run (`──⇡──`). Both backends now share the routing rules in `geometry.rs`: corners start one row below the source, edge labels sit on the first row beneath the level's routing block, and every reversed edge's arrowhead cell is pre-reserved in the slot allocator (*arrow-cell reservation*) so a horizontal span that would cross an arrowhead is pushed to a deeper slot — only levels with a genuine conflict pay an extra row. The heap slot allocator now works in layout direction (back edges participate), matching CSR. Pinned by cross-backend parity tests: IR geometry and byte-identical rendered text.
- **`LayoutConfig` spacing is now honored**: `node_spacing` and `level_spacing` were silently ignored (gaps hardcoded); both now apply in the heap and CSR layout paths. `level_spacing` default changed 2 → 0 to keep existing output unchanged.
- **Cluster-width feedback**: unaffiliated nodes are pushed clear of subgraph borders (label-widened or inherited from wider levels), and the canvas now covers label-widened borders instead of clipping them. A subgraph label's contribution to box width is capped at 40 chars (longer labels are truncated by the renderer). Applies to both layout paths.

### Improved
- **Vertical compaction**: level bands no longer pay for rows they don't use. The edge-label row is budgeted per level (only where a labeled edge is sourced — previously one labeled edge anywhere cost every level in the graph two extra rows, one of them permanently blank), and skip-level edges claim routing rows only where they actually change column (straight pass-throughs render as plain verticals — previously every intermediate level charged one row per passing edge, jogging or not). Hero example: 50 → 45 rows. Both layout paths, pinned by cross-backend parity tests.
- **Tighter subgraph layouts**: a per-level tightening pass reclaims the horizontal slack left behind when sibling clusters are shifted apart, so children align under their parents again. Both layout paths.
- **Inter-cluster compaction**: root clusters and loose nodes are pulled back together after overlap separation, removing the empty gulfs between boxes (up to −24% canvas width on the stress tiers). Both layout paths.
- **Edges never cross node text**: dummy waypoints are realigned when compaction moves their endpoints and nudged out of node spans; an edge that must cross something crosses a subgraph border (rendered as a junction), never a node. Both layout paths.

### Added
- **Dummy-node visualization (`LayoutConfig.include_dummy_nodes`)**: the flag existed but was never honored; it now works. When enabled, skip-level routing dummies appear in both IRs as nodes with `kind == NodeKind::Dummy`, a new `edge_index` back-link to their owning edge (zigraph parity; `None`/`usize::MAX` for real nodes), synthetic ids excluded from `node_by_id`, width 1 at the drawn waypoint column. Zero cost when disabled (default). The JSON `edge_index` field — previously always `null` — now carries the value.
- **Rank direction (`Direction`)**: `graph.set_direction(...)` / `LayoutConfig.direction` record the rank direction on the layout IR; parses from `"TB"`/`"TD"`/`"BT"`/`"LR"`/`"RL"`; re-exported from the crate root. For `BottomUp`, both layout paths emit physical (flipped) coordinates — the IR always matches rendered cells. The render engine (below) paints `TopDown` and `BottomUp`; the legacy scanline renderers remain `TopDown`-only until they are replaced. `LeftRight`/`RightLeft` are accepted by the parser but not yet laid out.
- **Unified render engine (`render::engine`)**: one paint path for both IRs — the render layer has no "backends". Public surface on both `LayoutIR` and `LayoutIRArena`: `render_with(&RenderOptions, &mut impl fmt::Write)` (streaming-primary), `render_string`, and a zero-allocation `render_to_bytes(&RenderOptions, &Arena, &mut [u8])` with `estimate_render_arena_size`/`estimate_render_output_size`; `render_plan`/`render_plan_in` expose the introspectable `RenderPlan` (dimensions, band count, legend entries, `hit_test`); the `Renderer` trait + `EngineRenderer` host external backends. `RenderOptions` selects color mode (`None`/`Ansi256`/`TrueColor`), charset (`Unicode`/`Ascii` — equal projections of one semantic canvas), legend, dummy-marker visibility, and `band_rows_cap` (default 64): rendering is banded, so canvas memory is `width × min(band_rows_cap, height)` cells regardless of graph height, and overlapping horizontal spans are painted as merged runs (Massive Fan 50k render: 17 s → 0.08 s). Undersized caller buffers report WDP `Render` codes (`E.Render.Plan.026`, `E.Render.Canvas.026`, `E.Render.Sink.026`) with actionable hints. The legacy renderers (`render_scanline*`, arena `render_to_buffer`) are unchanged and remain the output of record until the engine replaces them.
- **Per-element styling (render engine)**: `RenderOptions` carries four plain-`fn` style callbacks resolved once per element at plan time — never per cell. `EdgeStyle` (color, `LineWeight` light/dashed/double, `MarkerShape` at either end — suppress arrowheads or make edges double-headed), `NodeStyle` (`NodeBorder` bracket/angle — e.g. mark implicit nodes `<so>`, text color), `SubgraphStyle` (`SubgraphBorder` double/light/dashed/none, border color, `LabelPosition` inside-top/bottom), `EdgeLabelStyle` (color). Defaults reproduce the legacy output byte-for-byte. `const` presets in one place: `plain`, `colored`, `ascii`, `ascii_colored` (the colored legend's "from → to" arrow follows the charset: `->` in ASCII).

### Deprecated
- **The 0.9.x render entry points** are now thin wrappers over the unified engine, marked `#[deprecated]` and removed in 0.11 — each note names its replacement. Heap: `render_scanline`, `render_scanline_to`, `render_scanline_with_buffer`, `render_scanline_to_bytes`, `render_scanline_colored`, `render_scanline_colored_to`, `render_scanline_colored_with_legend` → `render_string`/`render_with`. Arena: `render_to_buffer`, `render_to_buffer_colored`, `render_to_buffer_colored_with_legend`, `estimate_render_size` → `render_to_bytes` + `estimate_render_arena_size`/`estimate_render_output_size` (the legacy sizing contract still works: `estimate_render_size` now returns engine-adequate sizes, and the engine's working memory is carved from the caller's scratch buffer — still zero allocation). The colored arena wrappers require the `alloc` feature (their legacy signatures provide no safely reusable memory for a no-alloc engine render; no-alloc callers use `render_to_bytes`). Also deprecated: `LayoutIR::y_index`/`edges_at`/`items_at_line` and `LineOccupancy` (use `render_plan` + `hit_test`), and the raw escape helpers in `render::colors` (`get`, `escape::fg256`, `escape::write_fg256`).
- **Legacy renderer implementations deleted** (`render/scanline.rs`, `ir/arena_render.rs`, `ir/arena_colored.rs`, ~1,900 lines): every paint rule now exists exactly once, in the engine. Rendered output for graphs with overlapping edge routes changes per the 0.10.0 rulings — junction cells merge (`├ ┼ ┬`) where the old plain renderer lossily kept the last corner drawn; nested boxes sharing a border cell merge into proper junctions; implicit nodes paint at their IR width; unfittable edge labels are skipped (never truncated mid-word). The hero golden snapshot is regenerated accordingly (6 lines changed, all junction glyphs).
- **`Graph::render()` / `render_to()`** (unchanged API) now render through the engine; the cycle banner and horizontal chain shortcut are untouched.
- **Everyday names re-exported from the crate root**: `ascii_dag::{RenderOptions, Palette, Charset, ColorMode}`.

### Breaking Changes
- **`LayoutEdge.label_position: Option<(usize, usize)>` → `label_x: usize, label_y: usize`**: the heap IR now uses the same scalar shape as `LayoutEdgeArena`, zigraph, and the JSON wire format (values are meaningful iff `label` is present; the JSON output is unchanged). Saves 8 bytes per edge.
- **`LayoutNode` / `LayoutNodeArena` gained `edge_index`** (`Option<usize>` / `usize` with `usize::MAX` sentinel): code constructing these structs by literal must add the field. `LayoutIRArenaBuilder::add_node` takes it as a new final parameter.

### Internal
- Rendered-output tests (`tests/layout_output.rs`): spacing config is now verified against the text a user sees, in both backends, plus a golden snapshot of the hero example (`cargo run --example hero` to regenerate).
- Cross-backend parity suite: the same graph must produce identical node/box/label geometry in both IRs, byte-identical rendered text, and matching dummy sets; BottomUp IRs must be exact vertical mirrors of TopDown. These tests found (and now pin) several silent backend divergences.
- Shared routing rules centralized in `geometry.rs`: `EDGE_START_ROW`, `ARROW_CELL_PAD`, `edge_label_row_offset`, `routing_overhead`, `passthrough_rows` — one definition, both backends.
- Shared cluster-geometry constants moved to `algorithms/sugiyama/geometry.rs` — previously duplicated across the heap and CSR backends, where they could silently drift.
- Packed vnode encoding in `arena_csr.rs` is now behind accessors (`vnode_kind`/`vnode_payload`/`vnode_set`).
- Removed orphaned `src/layout/arena.rs` (631 lines, never included in the module tree).

## [0.9.1] - 2026-03-25

### Improved
- **Subgraph layout quality**: Added iterative x-coordinate refinement and cascading subgraph compaction passes. Nodes now align more tightly with their connected neighbors across levels, reducing zigzag edges inside clusters.

## [0.9.0] - 2026-03-07

### Added
- **`LayoutConfig<'a>` — primary no-alloc config type**: New enum-of-structs design with `AlgorithmConfig<'a>` enum, lifetime parameter, `const fn` presets (`fast`, `standard`, `quality`), and zero heap allocation. Works in `no_std` without alloc.
- **Crossing reduction in CSR/Arena path**: The arena layout pipeline now accepts `&LayoutConfig` and applies crossing reduction + configurable spacing. Previously hardcoded.
- **`Graph::compute_layout_with_config(&LayoutConfig)`**: New primary method for heap path, accepting the no-alloc config type directly.
- **JSON serialization (zigraph v1.2)**: Both IR surfaces can serialize to zigraph-compatible JSON.
  - Heap: `LayoutIR::to_json() -> String` (requires `alloc`)
  - Arena: `LayoutIRArena::serialize_json(&mut [u8]) -> Option<usize>` (no_std, no alloc)
- **`center_y`** on `LayoutNode` and `LayoutNodeArena` — computed during layout.
- **`directed`** field on `LayoutEdgeArena` — matches heap `LayoutEdge`.
- **`Spline` path variant** on both `EdgePath` and `EdgePathArena` — cubic control points (`cp1_x/y`, `cp2_x/y`), forward-compatible with zigraph's spline hints.
- **`SideChannel` path variant** on both `EdgePath` and `EdgePathArena` — lateral routing slot, forward-compatible with zigraph.
- **True `no_alloc` support**: The arena/CSR layout path now compiles without the `alloc` feature. Use `--no-default-features --features arena` for a fully heap-free build suitable for `no_std` embedded targets.
- **Feature flag cleanup**: `generic` feature now implies `alloc` (all generic algorithms require heap allocation). `render::chars` and `render::colors` are available without alloc.
- **Subgraph / cluster support**: Group nodes into labeled, nestable clusters with `add_subgraph()`, `put_nodes().inside()`, and `put_subgraphs().inside()`. Rendered as double-line boxes (`╔═╗║╚═╝`) with labels inside, matching zigraph's visual style.
- **Edge–border junction characters**: Edges crossing subgraph borders produce proper junction glyphs (`╤ ╧ ╪ ╫ ╞ ╡`).
- **New example**: `examples/subgraphs.rs` — 5 demos covering simple, sibling, nested, and pipeline clusters.

### Breaking Changes
- **`DAG` → `Graph`**: Core type renamed throughout the public API.
- **`find_subgraphs()` → `find_connected_components()`**: Renamed to avoid confusion with the new cluster feature.
- **`LayoutError` enum**: Errors now use structured WDP codes (`E.Graph.…`, `W.Graph.…`) instead of plain strings.

### Internal
- **Module restructure**: Split 4 monolithic files into 15 focused modules under `graph/`, `algorithms/`, `ir/`, `render/`.

## [0.8.3] - 2026-02-07

### Fixes
- **Horizontal Edge Overlap**: Replaced lane-based slot system (`rank % MAX_LANES`) with unified source-node-based slot assignment. Edges from 5+ sources converging on one target no longer collide. Heap layout now matches Arena behavior.
- **Memory Explosion on Extreme Fan-in**: Added `MAX_SLOTS_PER_LEVEL = 8` cap to prevent unbounded horizontal growth. "Massive Fan (50k→1)" dropped from 14.9 GB / 67.7s to 5.8 MB / 29.1s.
- **Performance Regression**: Replaced O(N×E) inner loop scanning all edges per target with O(N+E) adjacency-list lookup via `get_parents_indices()`.
- **Clippy Dead-if**: Removed `if has_labeled_edges { 1 } else { 1 }` in both `graph.rs` and `layout/arena.rs`.
- **Arena Labels Example**: Fixed missing `scratch_buffer` argument in `arena_labels_test.rs` after API change.

### Code Quality
- **Comment Cleanup**: Removed ~100 lines of thinking-out-loud / stale comments across `graph.rs`, `layout/arena.rs`, `render/scanline.rs`, and `ir/arena.rs`. Replaced with concise, descriptive comments.

## [0.8.2] - 2026-02-02

### Fixes
- **Arena Layout Crash**: Fixed index out of bounds panic in `build_dummy_positions_arena` by clamping prefix sum calculations. This resolves crashes on specific graph topologies found via fuzzing.

### Performance
- **Spatial Indexing**: Implemented spatial indexing for `render_to_buffer`, significantly improving rendering performance for large graphs.
- **Scalability**: Removed `255` level limit (u8) in fallback mode, allowing for arbitrarily deep graphs (up to `usize::MAX`).
- **Benchmark Update**: Verified linear scaling up to 200k nodes.

### Breaking Changes
- **`render_to_buffer` API**: `LayoutIRArena::render_to_buffer` now requires a third argument: `scratch_buffer: &mut [usize]`. This is used for the spatial index optimization.
- **`estimate_render_size` API**: Now returns a tuple `(usize, usize)` representing `(render_bytes, scratch_slots)`.

## [0.8.1] - 2026-02-01

### Performance
- **Updated benchmarks**: Fresh measurements on M2 Ultra showing 2.1x-56.9x Arena speedups

### Arena Layout Engine Refactor
- **Deep Graph Fix**: Replaced fixed-size stack arrays in `compute_layout_arena_csr` with dynamic arena allocations, resolving panics on deep graphs.
- **Performance Optimization**: Replaced linear source scanning with O(1) lookups, yielding **56.9x speedup** on WideFan topologies.
- **Vertical Spacing**: Ported dynamic row height calculation to Arena mode, ensuring parity with Heap mode layouts.
- **Visual Fix**: Resolved edge label/breakout overlap by reserving dedicated rows for routing.

## [0.8.0] - 2026-01-31

### Breaking Changes
- **`add_edge()` API change**: Now takes an optional label parameter
  - Before: `dag.add_edge(1, 2)`
  - After: `dag.add_edge(1, 2, None)` or `dag.add_edge(1, 2, Some("label"))`
- **`LayoutEdge` struct**: Added `label` and `label_position` fields

### Edge Labels
- **Labeled edges**: `add_edge(from, to, Some("label"))` displays inline labels on edges
- **`from_edges_labeled()`**: New batch constructor for graphs with labeled edges
- **Label positioning**: Automatic label placement at edge midpoints
- **Collision detection**: Labels that would overlap are skipped
- **Legend fallback**: Skipped labels appear in a legend below the graph

### Colored Rendering
- **New color module**: `ascii_dag::render::colors` with `Palette` enum
- **Three palettes**: `Palette::Ansi` (default), `Palette::AnsiDark`, `Palette::AnsiLight`
- **Greedy graph coloring**: `compute_edge_colors()` assigns colors to minimize same-color adjacent edges
- **Colored scanline render**: `render_scanline_colored(Palette::Ansi)` for ANSI terminal colors
- **Colored legend**: `render_scanline_colored_with_legend()` includes legend for skipped labels
- **Arena colored rendering**: Full color support in `LayoutIRArena` with `render_to_buffer_colored()`

### Arena Stability & Robustness
- **Fuzz-tested**: 17.4 million iterations (1 hour) with zero crashes
- **Improved memory estimation**: `estimate_layout_arena_size()` now accurately predicts arena requirements
- **Bounds checking hardening**: Comprehensive bounds checks in layout and rendering paths
- **Fixed subtraction overflow**: `paint_node` handles zero-width nodes safely
- **Fixed dummy node width**: Arena mode now uses width 3 (matching heap mode) for better edge separation
- **Stress test improvements**: 20k/50k node tests work reliably (31MB/78MB memory)

### New Examples
- `edge_label_demo.rs`: Demonstrates edge labels and legend feature
- `color_demo.rs`: Shows all color palettes
- `arena_labels_test.rs`: Tests arena mode with labels and colors
- `hero_colored.rs`: Complex graph with labels and performance test

### Performance
- **Updated benchmarks**: Fresh measurements on M2 Ultra showing 2.1x-37.3x Arena speedups

## [0.7.1] - 2026-01-17

### Benchmarks
- **Topology-based benchmarks**: Chain, Diamond, WideFan tests showing Arena vs Heap speedups
- **20K node stress test**: Added `test_massive_diamond_20k()` for mid-scale testing
- **Microsecond precision**: Desktop benchmarks now report in µs for clarity
- **ARM platform note**: Documented M2 Ultra (ARM64) as benchmark platform

### Documentation
- **README accuracy**: Updated performance claim to reflect actual Arena benchmarks (~5ms for 1000 nodes)
- **Scalability table**: Added Heap vs Arena comparison (3.5x-8.6x speedup at scale)
- **Security section**: Corrected "no unsafe" claim → "minimal unsafe in arena allocator (Miri-tested)"

## [0.7.0] - 2026-01-17

### No-Alloc / Arena Mode
- **Full no-alloc support**: `LayoutIRArena` for embedded/`no_std` environments
- **Arena index types**: `arena-idx-u8`, `arena-idx-u16`, `arena-idx-u32` for memory optimization
- **Dual IR types**: `LayoutIR` (heap) and `LayoutIRArena` (arena) with same API

### Layout Engine Improvements
- **Skip-level edge separation**: Bounded offset (edge_idx % 4) prevents convergent edges from merging
- **Crossing reduction refinements**: Improved visual quality for complex graphs
- **Custom width/height for WASM**: Layout IR now supports custom dimensions

### Documentation
- **Feature flags table**: Documented all arena feature flags
- **Dual IR documentation**: Explained both `LayoutIR` and `LayoutIRArena` with code examples
- **New example**: `layout_ir_demo` demonstrating Build → Compute IR → Process → Render workflow

### Fixes
- Fixed clippy warnings (repeat_n migration)
- Fixed broken doc link to `idx` module

## [0.6.1] - 2026-01-04

### 🎯 Embedded Examples
- **ESP32-S3 Example**: Added comprehensive performance benchmark for ESP32-S3 (Xtensa LX7 @ 240MHz)
  - 6 benchmark scenarios: Diamond, Pipeline, Fan-Out/In, Binary Tree, Deep Chain (50 nodes), Diamond Lattice (64 nodes)
  - Real hardware measurements: 0.4-2.7ms build time, 2.5-18.8ms render time
  - Heap tracking: 1.5-25.5 KB for various graph sizes
  - Tested on Seeed XIAO ESP32-S3 with 128KB heap allocation
- **Enhanced Embedded Documentation**: Added detailed ESP32-S3 performance metrics to README
  - Side-by-side comparison with RP2040 benchmarks
  - Clear toolchain setup instructions (ESP toolchain vs standard Rust)
- **Longan Nano & RP2040 Examples**: Improved documentation and setup guides

### 📚 Documentation
- Added ESP toolchain clarification (Rust 1.85/1.86 fork for Xtensa)
- Updated README with aggressive embedded benchmarks showing real-world performance
- Improved troubleshooting sections for embedded examples

## [0.6.0] - 2026-01-03

### 🚀 Performance & Optimizations
- **Massive Speedup**: Layout engine is now **~9x faster** for standard workloads (1000 nodes: 173ms -> 18ms).
- **Reduced Bundle Size**: WASM binary size reduced (~55KB raw), with strategic inlining for smaller footprint.
- **Stack Safety**: Replaced recursive DFS with an **Iterative Stack** implementation. The library is now stack-overflow proof and handles deep graphs (tested up to 50,000 nodes).
- **Zero-Allocation Hot Paths**: optimized internal helpers (`count_digits`, `write_node`) to avoid temporary string allocations.

### 🛡️ Robustness
- **Stress Tested**: Verified stability on 50,000+ node graphs (Diamond Lattice, Wide Fan) without crashing.
- **WASM Support**: Verified support for `wasm-opt` bulk-memory optimizations.

### 🛠️ Fixes
- Fixed stack overflow crash on extremely deep graphs (e.g., 10k node Diamond Lattice).
