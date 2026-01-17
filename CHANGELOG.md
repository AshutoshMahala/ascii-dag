# Changelog

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
