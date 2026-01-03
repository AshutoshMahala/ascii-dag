# Changelog

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
