# Changelog

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
