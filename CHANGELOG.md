# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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
