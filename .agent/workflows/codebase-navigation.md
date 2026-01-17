---
description: How to navigate and understand the ascii-dag codebase
---

# Codebase Navigation Guide

## Quick Start

1. Read `ARCHITECTURE.md` for a high-level overview of all modules
2. Read `README.md` for feature overview and usage examples

## Key Entry Points

### Understanding the Core
- **Main API**: Start with `src/lib.rs` for re-exports and module structure
- **Graph Structure**: `src/graph.rs` - The `DAG<'a>` struct is the main entry point
- **Layout Algorithm**: `src/layout.rs` - Sugiyama algorithm with crossing reduction

### Arena/Embedded Path
- **Arena Allocator**: `src/arena.rs` - Bump allocator for no_std
- **CSR Format**: `src/csr.rs` - Compressed Sparse Row graph format
- **Arena Layout**: `src/layout/arena.rs` - Zero-allocation layout computation
- **Index Types**: `src/layout/idx.rs` - Configurable u8/u16/u32 indices

### Rendering
- **ASCII Renderer**: `src/render/ascii.rs` (largest file, ~1800 lines)
- **Scanline Renderer**: `src/render/scanline.rs` - Arena-friendly rendering
- **Intermediate Rep**: `src/ir/mod.rs` - `LayoutIR` and `EdgePath` types

## Common Tasks

### Adding a new graph algorithm
1. Add generic version in `src/layout/generic/` or `src/cycles/generic/`
2. Expose via `src/layout.rs` or `src/cycles.rs`
3. Add tests in the same file

### Modifying the layout algorithm
1. `src/layout.rs` - Level calculation, crossing reduction
2. `src/layout/arena.rs` - Arena-based equivalent
3. Update both to stay in sync

### Adding a new rendering mode
1. Add to `src/render/`
2. Implement on `LayoutIR` in `src/ir/mod.rs`
3. Optionally add arena version in `src/ir/arena.rs`

### Fixing arena buffer issues
1. Check `alloc_layout_temps()` and `alloc_layout_temps_csr()` in `src/layout/arena.rs`
2. The `max_vnodes` calculation is critical for buffer sizing
3. Always add bounds checks before array access

## Running Tests

// turbo
```bash
cargo test --lib
```

// turbo
```bash
cargo run --example arena_benchmark --release
```

// turbo
```bash
./scripts/pre-push.sh
```

## Security Testing

```bash
./scripts/security-check.sh quick
./scripts/security-check.sh fuzz
```

## Feature Flags

```toml
# Default (std + generic)
ascii-dag = "0.6"

# Minimal (no_std compatible)
ascii-dag = { version = "0.6", default-features = false }

# Arena support
ascii-dag = { version = "0.6", features = ["arena"] }

# Small index types for embedded
ascii-dag = { version = "0.6", features = ["arena", "arena-idx-u8"] }
```

## Key Concepts

- **VirtualNode**: Tagged pointer that's either a real node or dummy node for edge routing
- **EdgePath**: How an edge is routed (Direct, Corner, SideChannel, MultiSegment)
- **LayoutIR**: Intermediate representation between layout and rendering
- **CsrGraph**: Contiguous memory layout for arena-based processing
