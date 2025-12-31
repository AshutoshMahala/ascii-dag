# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.4.1] - 2024-12-31

### Added
- docs.rs metadata configuration for proper documentation builds
- `rust-version` field specifying MSRV of 1.85
- `#![cfg_attr(docsrs, feature(doc_cfg))]` for docs.rs feature annotations

### Fixed
- Documentation build configuration for docs.rs compatibility

## [0.4.0] - 2024-12-31

### Added
- **Dummy node insertion for skip-level edges**: Classic Sugiyama algorithm improvement
  - `VirtualNode` enum: `Real(usize)` for actual nodes, `Dummy(usize)` for edge routing
  - `VirtualLayout` struct for managing virtual nodes during rendering
  - Skip-level edges (spanning 2+ levels) now route through dummy nodes for visibility
  - Memory cost: ~24 bytes per dummy node, O(N + E*D) total
- **Mixed convergence/divergence handling**: K2,2 bipartite patterns now render correctly
  - `draw_mixed_connections()` with multi-line routing for crossing patterns
  - Proper separation of divergence and convergence lines
- **New example files**:
  - `test_dummy_nodes.rs` - Tests for skip-level edge rendering
  - `k22_analysis.rs` - K2,2 bipartite graph analysis
  - `analyze_algo.rs` - Algorithm behavior analysis
  - `debug_layout.rs`, `debug_skip.rs` - Layout debugging utilities
  - `rendering_issues.rs`, `edge_cases.rs`, `complex_graphs.rs` - Comprehensive tests
- **Algorithm analysis documentation**: `docs/ALGORITHM_ANALYSIS.md`
  - Documents the 4-pass Sugiyama layout approach
  - Explains identified issues and solutions
  - ASCII art limitations and acceptable simplifications

### Changed
- **Rendering algorithm refactored** to use virtual layout system:
  - `render_vertical()` now delegates to `build_virtual_layout()` + `render_virtual_layout()`
  - `build_virtual_edges()` routes edges through dummies using edge index lookup
  - `assign_virtual_x_coordinates()` positions dummies at end of level for visibility
- Connection drawing methods updated to work with virtual layout:
  - `draw_virtual_connections()` replaces `draw_connections_sugiyama()`
  - `draw_convergence_connections()`, `draw_divergence_connections()`, `draw_simple_connections()`

### Removed
- **Dead code cleanup** from previous rendering implementation:
  - Removed `draw_connections_sugiyama()` and related Manhattan routing methods
  - Removed unused `assign_x_coordinates()`, `compact_level()`, `calculate_canvas_dimensions()` from layout.rs

### Fixed
- Skip-level edges now visible (previously rendered in same column as source node)
- K2,2 and similar cross-connection patterns no longer fall through to simple vertical lines
- Dummy node positions properly tracked after crossing reduction reordering

## [0.3.1] - 2025-10-30

### Added
- **Graph traversal utilities**: New `layout::generic::traversal` module
  - `collect_all_nodes_fn()` - BFS traversal collecting all reachable nodes
  - `collect_all_nodes_dfs_fn()` - DFS variant for memory-efficient deep traversal
  - `NodeCollectable` trait for convenient traversal methods
  - Automatic cycle handling with visited tracking
  - Useful for PII redaction, node processing, and graph analysis

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
