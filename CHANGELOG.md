# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.2.0] - 2025-10-23

### Added
- **Adjacency list caching**: Massive performance improvement for dense graphs
  - Added `children` and `parents` adjacency lists to DAG struct
  - `get_children()` and `get_parents()` now O(1) instead of O(E)
  - New `get_children_indices()` and `get_parents_indices()` for zero-copy access
- **Feature flags** for bundle size optimization:
  - `generic` feature (default) - Generic algorithms, cycle detection, impact analysis, metrics
  - Core renderer only: `--no-default-features --features std` (~41KB WASM)
  - Full features: default (~77KB WASM)
- **Comprehensive performance documentation**:
  - Resource limits and tested configurations
  - Security considerations for untrusted input
  - Memory usage per node/edge
  - Big-O complexity for all operations

### Performance Improvements
- Eliminated allocation hotspots in rendering pipeline:
  - `assign_x_coordinates`: Now uses cached widths instead of `format_node()`
  - `calculate_canvas_dimensions`: O(1) width lookup
  - `compact_level`: Direct buffer writes with `write_node()`
  - `draw_vertical_connections`: Pre-computed node widths
  - `render_subgraph`: Zero allocations during traversal
- Child/parent lookups: **100x+ faster** for dense graphs (O(1) vs O(E))
- Rendering: **Eliminated thousands of temporary Vec allocations**
- Layout: Reuses cached data structures across passes

### Changed
- **BREAKING**: DAG struct now includes `children` and `parents` fields
  - Affects manual struct construction (use `DAG::new()` or `DAG::from_edges()`)
- **BREAKING**: Generic modules require `generic` feature flag
  - `cycles::generic`, `layout::generic`, impact analysis, metrics
  - Add `features = ["generic"]` if using these modules with `default-features = false`
- Updated examples to use feature flags where needed

### Removed
- Dead code cleanup:
  - Removed unused `format_node()` method (replaced by `write_node()`)
  - Removed unused `build_adjacency_lists()` method (replaced by cached lists)
  - 78 lines of dead code removed

### Documentation
- Added "Performance & Configuration" section to README
- Documented feature flags and bundle size impact
- Added resource limits and security considerations
- Updated crate-level documentation with performance info
- Added comprehensive module-level docs in `graph.rs`

### Migration Guide (0.1 → 0.2)
- If using generic features with `default-features = false`, add `features = ["generic"]`
- If manually constructing DAG structs, use `DAG::new()` or `DAG::default()` instead
- No API changes for normal usage - just performance improvements!

## [0.1.0] - 2025-10-22

### Added
- Initial release of ascii-dag
- Core DAG rendering with Sugiyama-style hierarchical layout
- Two construction modes:
  - Builder API: `DAG::new()` + `add_node()` + `add_edge()`
  - Batch construction: `DAG::from_edges()`
- Auto-created placeholder nodes (`⟨ID⟩` format)
- Node promotion: placeholders can be upgraded to labeled nodes
- Cycle detection with detailed error reporting
- Horizontal and vertical rendering modes
- Multiple disconnected subgraph support
- Unicode box-drawing characters for clean output
- no_std compatibility (requires `alloc`)
- Zero dependencies

### Performance Optimizations
- O(1) HashMap lookups for node ID → index mapping
- O(1) HashSet for auto-created node tracking
- Cached node widths (avoids repeated formatting)
- Zero-allocation rendering (direct buffer writes)
- Split borrows to eliminate level cloning in layout algorithm
- Custom integer formatting (avoids `format!` macro overhead)

### Known Limitations (v0.1.x)
- No cross-level edge routing (edges simplified for clarity)
- Requires Unicode terminal support
- Optimized for small-to-medium graphs (<1000 nodes)
- 0.x API may have breaking changes between minor versions

### Documentation
- Comprehensive README with examples
- API documentation with examples
- Optimization guide (`docs/OPTIMIZATIONS.md`)
- Example programs:
  - `basic.rs` - Simple chain
  - `error_chain.rs` - Error diagnostics
  - `circular_dependency.rs` - Cycle detection
  - `complex_error.rs` - Diamond DAG
  - `minimal.rs` - Smallest example
  - `performance_test.rs` - Benchmarking

### Testing
- 13 unit tests covering core functionality
- 13 documentation tests
- All tests passing on stable Rust

[0.1.0]: https://github.com/AshutoshMahala/ascii-dag/releases/tag/v0.1.0
