# Roadmap: Lessons from zigraph

Inspired by comparing `ascii-dag` (Rust) with `zigraph` (Zig) — both authored
by the same developer — this document captures concrete improvements that
ascii-dag can adopt.

> **This is an internal working document.  Not for publishing.**

---

## P0 — `Result<T, LayoutError>` instead of `Option`

**Status: DONE** ✓

### Problem

The arena layout path returns `Option<LayoutIRArena>`.  When it fails the
caller sees `None` with zero diagnostics.  Root causes include:

| Cause | Current return |
|-------|---------------|
| Arena out of memory | `None` |
| Graph exceeds `MAX_NODES` (index-type limit) | `None` |
| Graph exceeds `MAX_LEVELS` (255) | `None` |
| Cycle detected | `None` (empty IR) |
| Builder allocation failed | `None` |

zigraph returns structured WDP error codes like `E.Layout.Algo.026` (OOM).

### Plan

1. Add `LayoutError` enum in `src/algorithms/sugiyama/error.rs`:
   ```rust
   pub enum LayoutError {
       ArenaOom,
       ExceedsMaxNodes { count: usize, max: usize },
       ExceedsMaxLevels { depth: usize, max: usize },
       CycleDetected,
       BuilderFailed,
   }
   ```
2. Change `compute_layout_arena` → `Result<LayoutIRArena, LayoutError>`.
3. Change `compute_layout_arena_csr` → `Result<LayoutIRArena, LayoutError>`.
4. Propagate through `CsrGraph::compute_layout_arena`.
5. Re-export `LayoutError` from `lib.rs`.
6. Update `examples/benchmark.rs` to use `Result`.

### Files to touch

- `src/algorithms/sugiyama/error.rs` (new)
- `src/algorithms/sugiyama/mod.rs` (add `pub mod error;`)
- `src/algorithms/sugiyama/arena.rs` (change return types, replace `?` → `.ok_or(…)?`)
- `src/algorithms/sugiyama/arena_csr.rs` (same)
- `src/graph/csr.rs` (propagate)
- `src/lib.rs` (re-export)
- `examples/benchmark.rs`

---

## P1a — Composable Crossing Reduction Pipeline

**Status: DONE** ✓

### Problem

ascii-dag has a fixed 4-pass median heuristic with no adjacent-exchange
refinement.  zigraph uses composable reducer pipelines:

```
median(4) → adjacent_exchange(2) → median(2)   // quality preset
```

The arena path has **no crossing reduction at all** (stub in `arena_phases.rs`).

### Plan

1. Add `CrossingReducer` enum in `src/algorithms/sugiyama/crossing.rs`:
   ```rust
   pub enum CrossingReducer {
       Median(usize),           // N passes of median heuristic
       AdjacentExchange(usize), // N passes of pairwise swap refinement
   }
   ```
2. Add `CrossingPipeline` type alias: `&[CrossingReducer]`.
3. Ship three presets:
   - `FAST`:     `[Median(2)]`
   - `STANDARD`: `[Median(4), AdjacentExchange(2)]`
   - `QUALITY`:  `[Median(8), AdjacentExchange(4), Median(2)]`
4. Implement `adjacent_exchange` for the heap path (swap adjacent
   nodes on a level and keep the swap if it reduces crossings).
5. Replace `crossing_reduction_passes: usize` on `DAG` with
   `crossing_pipeline: &[CrossingReducer]` (keep old setter as
   compat shim that sets `Median(n)`).
6. Wire into both heap and arena paths.

### Files to touch

- `src/algorithms/sugiyama/crossing.rs` (new)
- `src/algorithms/sugiyama/mod.rs` (add `pub mod crossing;`)
- `src/algorithms/sugiyama/heap.rs` (use pipeline)
- `src/algorithms/sugiyama.rs` (update `reduce_crossings`)
- `src/graph.rs` (`DAG` field + setter compat)
- `src/lib.rs` (re-export)

---

## P1b — Preset / Configuration System

**Status: DONE** ✓

### Problem

All layout parameters live as unstructured fields on `DAG`.  zigraph has
curated presets (`standard`, `fast`, `quality`) that bundle crossing
reduction, validation, and rendering settings.

### Plan

1. Add `LayoutConfig` struct:
   ```rust
   pub struct LayoutConfig<'a> {
       pub crossing_pipeline: &'a [CrossingReducer],
       pub render_mode: RenderMode,
   }
   ```
2. Ship presets as const fns:
   ```rust
   pub const fn fast() -> LayoutConfig<'static> { … }
   pub const fn standard() -> LayoutConfig<'static> { … }
   pub const fn quality() -> LayoutConfig<'static> { … }
   ```
3. Add `DAG::compute_layout_with(config)` method.  Keep existing
   `compute_layout()` as alias for `standard()`.
4. Re-export presets from top-level crate.

### Files to touch

- `src/algorithms/sugiyama/crossing.rs` (presets live here with the reducers)
- `src/graph.rs` (new `compute_layout_with`)
- `src/lib.rs` (re-export `LayoutConfig`)

---

## P1c — DAG → Graph Rename + Validation Layer

**Status: IN PROGRESS**

### Problem

Cycle breaking (back-edge reversal) made `DAG` a misnomer — the type
accepts cyclic graphs.  zigraph correctly calls its core type `Graph` and
treats acyclicity as a *validated property*, not a structural constraint.

Additionally, ascii-dag has no formal validation layer — `has_cycle()` is
called ad-hoc inside `render_to()` and the arena path.

### Plan — Phase A

**A1. WDP error infrastructure** (`src/errors.rs` — new):
```rust
// Building-block constants prevent typos, enforced at compile time
macro_rules! wdp {
    ($sev:expr, $comp:expr, $pri:expr, $seq:expr) => {
        concat!($sev, ".", $comp, ".", $pri, ".", $seq)
    };
}

// Severity
pub const E: &str = "E";
pub const W: &str = "W";

// Component
pub const GRAPH: &str = "Graph";
pub const LAYOUT: &str = "Layout";

// Primary
pub const NODE: &str = "Node";
pub const EDGE: &str = "Edge";
pub const DAG_P: &str = "Dag";
pub const ALGO: &str = "Algo";
pub const SUBGRAPH: &str = "Subgraph";

// Sequence (WDP Part 6)
pub const MISSING: &str = "001";
pub const MISMATCH: &str = "002";
pub const INVALID: &str = "003";
pub const DUPLICATE: &str = "007";
pub const UNSUPPORTED: &str = "009";
pub const NOT_FOUND: &str = "021";
pub const EXHAUSTED: &str = "026";

// Composed codes
pub const CYCLE_DETECTED: &str     = wdp!(E, GRAPH, DAG_P, INVALID);
pub const NODE_NOT_FOUND: &str     = wdp!(E, GRAPH, NODE, NOT_FOUND);
pub const SUBGRAPH_NOT_FOUND: &str = wdp!(E, GRAPH, SUBGRAPH, NOT_FOUND);
pub const SUBGRAPH_CYCLE: &str     = wdp!(E, GRAPH, SUBGRAPH, INVALID);
pub const EMPTY_GRAPH: &str        = wdp!(E, GRAPH, NODE, MISSING);
pub const ARENA_OOM: &str          = wdp!(E, LAYOUT, ALGO, EXHAUSTED);
pub const EXCEEDS_MAX: &str        = wdp!(E, LAYOUT, ALGO, INVALID);
pub const BUILDER_FAILED: &str     = wdp!(E, LAYOUT, ALGO, EXHAUSTED);
```

**A2. Unified `GraphError` enum** (replaces `LayoutError`):
```rust
pub enum GraphError {
    CycleDetected { path: Vec<usize> },
    NodeNotFound(usize),
    EmptyGraph,
    ArenaOom,
    ExceedsMaxNodes { count: usize, max: usize },
    ExceedsMaxLevels { depth: usize, max: usize },
    BuilderFailed,
    SubgraphNotFound(usize),
    SubgraphCycle,
    ValidationFailed(ValidationFailures),
}

impl GraphError {
    pub fn code(&self) -> &'static str { … }  // WDP code
    pub fn hint(&self) -> &'static str { … }  // Actionable advice
}
```

**A3. Validation module** (`src/validation.rs` — new):
```rust
pub struct Requirements {
    pub acyclic: bool,
    pub non_empty: bool,
    pub all_directed: bool,
}

impl Requirements {
    pub const fn sugiyama() -> Self { … }
    pub const fn dag() -> Self { … }
    pub const fn permissive() -> Self { … }
}

impl Graph<'_> {
    pub fn validate(&self, req: &Requirements) -> Result<(), GraphError>;
}
```

**A4. Rename `DAG` → `Graph`** (all internal code).

**A5. `DAG<'a>` newtype wrapper** (validated, public):
```rust
pub struct DAG<'a>(Graph<'a>);

impl<'a> DAG<'a> {
    pub fn try_from(graph: Graph<'a>) -> Result<Self, GraphError> {
        graph.validate(&Requirements::dag())?;
        Ok(Self(graph))
    }
}
impl<'a> Deref for DAG<'a> {
    type Target = Graph<'a>;
    fn deref(&self) -> &Graph<'a> { &self.0 }
}
```

**A6. Backward compatibility**: Keep `DAG` as a re-export and the
primary documented type.  `Graph` is the lower-level escape hatch for
users who need cycles or mixed edge types.

### Files to touch

| File | Changes |
|------|---------|
| `src/errors.rs` | **NEW** — WDP macro, building blocks, composed codes |
| `src/graph.rs` | Rename struct `DAG` → `Graph`, add `DAG` newtype |
| `src/validation.rs` | **NEW** — `Requirements`, `validate()`, `ValidationFailures` |
| `src/algorithms/sugiyama/error.rs` | Replace `LayoutError` with re-export of `GraphError` |
| `src/algorithms/sugiyama/arena.rs` | Use `GraphError` |
| `src/algorithms/sugiyama/arena_csr.rs` | Use `GraphError` |
| `src/algorithms/sugiyama/heap.rs` | Use `GraphError` |
| `src/render/ascii.rs` | `DAG` → `Graph` in impl blocks |
| `src/render/classic.rs` | Same |
| `src/render/scanline.rs` | Same |
| `src/ir/*.rs` | Same |
| `src/lib.rs` | Re-export `Graph`, `DAG`, `GraphError`, `Requirements` |
| All examples | Update imports (most just need `use ascii_dag::DAG`) |
| All tests | Same |

---

## P2 — Renderer Trait (decouple render from IR)

**Status: Future**

Add a `Renderer` trait so users can plug custom backends without forking.

```rust
pub trait Renderer {
    fn render(&self, ir: &LayoutIR, buf: &mut dyn Write) -> io::Result<()>;
}
```

Ship `AsciiRenderer`, `ScanlineRenderer`.  Future: `SvgRenderer`, `JsonRenderer`.

---

## P2 — Cycle Breaking (virtual edge reversal)

**Status: DONE** ✓

Heap path detects back-edges via three-color DFS (`detect_back_edges()`)
and treats them as reversed for layering/routing.  Back edges are marked
`reversed: true` in the final IR.

---

## P3 — Subgraph / Cluster Support

**Status: DONE (Heap path) — Arena path deferred**

### Implementation Summary

Subgraph/cluster support is fully implemented for the heap-based Sugiyama pipeline
with zigraph-compatible fluent API, layout integration, bounding box computation,
and box-drawing border rendering.

#### What was implemented

| Component | Status | Details |
|-----------|--------|---------|
| `Graph` struct | ✅ | `Subgraph<'a>` struct, `subgraphs` vec, `node_subgraph` map, `next_subgraph_id` counter |
| `add_subgraph()` API | ✅ | Returns unique subgraph ID |
| `put_nodes().inside()` | ✅ | `NodePlacer` with validation (SubgraphNotFound, NodeNotFound) |
| `put_subgraphs().inside()` | ✅ | `SubgraphPlacer` with cycle detection via `is_ancestor()` |
| Query methods | ✅ | `subgraph_count()`, `node_subgraph()`, `has_subgraphs()`, `subgraph()`, `subgraphs()` |
| `LayoutIR` | ✅ | `SubgraphInfo<'a>` with `{ id, parent_id, label, x, y, width, height }` |
| Block-partitioned crossing | ✅ | `block_partition_level()` groups nodes by subgraph |
| Subgraph padding | ✅ | `subgraph_padding()` inserts H_PAD at boundary transitions |
| Bounding boxes | ✅ | `compute_bounding_boxes()` — node envelope → padding+label → bottom-up propagation |
| Scanline rendering | ✅ | `paint_subgraph_border()` + `paint_subgraph_label()` with z-layered rendering |
| Error variants | ✅ | `GraphError::SubgraphNotFound`, `SubgraphCycle`, `NodeNotFound` |
| Unit tests | ✅ | 14 tests covering API, validation, layout integration, rendering |

#### Files added/modified

| File | Changes |
|------|---------|
| `src/graph.rs` | `Subgraph`, `NodePlacer`, `SubgraphPlacer`, 3 new fields, fluent API, 14 unit tests |
| `src/algorithms/sugiyama/subgraph.rs` | **NEW** — `vnode_subgraph`, `block_partition_level`, `subgraph_padding`, `compute_bounding_boxes` |
| `src/algorithms/sugiyama.rs` | Added `pub(crate) mod subgraph;` |
| `src/algorithms/sugiyama/heap.rs` | Wired at 3 pipeline points: block partition → padding → bbox |
| `src/ir/mod.rs` | `SubgraphInfo<'a>`, `subgraphs` field on `LayoutIR` + builder |
| `src/render/scanline.rs` | Border frame (Z=0), edge routing (Z=1), label overlay (Z=2), nodes (Z=3) |
| `src/lib.rs` | Re-exports: `Subgraph`, `SubgraphInfo` |
| `examples/subgraphs.rs` | 5 showcase examples: simple, sibling, nested, CI/CD pipeline, disconnected |

#### Rendering z-order

1. **Subgraph border frame** (corners + `─` lines, `│` sides) — lowest priority
2. **Edge routing** — overwrites `─` where edges cross borders
3. **Subgraph labels** — painted ON TOP of edges so labels are always readable
4. **Node labels** — highest priority

#### Known limitations

- **Vertically-stacked siblings** sharing the same x-column: edges cross through
  borders. The edge characters overwrite `─` in the border, which is acceptable
  ASCII-art behavior. Labels remain readable due to the z-order layering.
- **Arena path** not yet implemented (deferred to a future phase).
- **Contiguous level enforcement** not implemented (would compact subgraph nodes
  to adjacent levels, improving visual clustering for sparse graphs).

> **Note:** The existing `find_subgraphs()` in `sugiyama.rs` and
> `render_subgraph()` in `classic.rs` detect **disconnected components**
> (nodes unreachable from each other).  This is unrelated to user-defined
> named clusters — it is purely automatic graph decomposition.

### What zigraph has (reference implementation)

zigraph implements full subgraph support across 5 submodules:

```zig
// Usage:
const backend = try g.addSubgraph("Backend");
const db      = try g.addSubgraph("Database");
try g.putSubgraphs(&.{db}).inside(backend);   // nesting
try g.putNodes(&.{ 2, 3, 4 }).inside(backend); // membership
```

**Storage:** `Subgraph { id, label, parent_id }` struct + 4 fields on Graph
(`subgraphs` list, `subgraph_id_to_index` map, `node_subgraph` map,
`next_subgraph_id` counter).  Zero cost when unused.

**IR output:** `SubgraphInfo { id, parent_id, label, x, y, width, height }`
bounding boxes stored alongside nodes/edges.

**Layout modules** (all in `algorithms/sugiyama/subgraph/`):

| Module | What it does | Complexity |
|--------|-------------|------------|
| `common` | `vnodeSubgraph` (resolves membership for real+dummy nodes — cross-subgraph dummies float freely), `ancestorChain`, `subgraphDepth`, `countBoundaryTransitions` (LCA-based) | O(depth) per query |
| `contiguous` | Compacts each subgraph's level span to be contiguous; processes bottom-up (deepest first); renormalizes max_level | O(V+S) |
| `crossing` | Block-partitioned median heuristic: levels split into per-subgraph blocks, nodes sorted within blocks, blocks ordered by average median; post-pass `enforceSubgraphAdjacency` via stable partition | O(V·passes) |
| `padding` | Horizontal: counts boundary transitions between adjacent vnodes, inserts `transitions × padding` extra x-space.  Vertical: builds level-presence matrix, computes stacking depth of opening/closing borders | O(V·S) |
| `bbox` | Bottom-up envelope computation: pass 1 = node min/max per subgraph, pass 2 = apply padding + label row + propagate to parent | O(V+S) |

**Rendering:** SVG renders subgraphs as `<rect>` with dashed borders and
bold label text.  Reverse iteration (parents first) for correct z-order.

### Plan

#### Phase 1: Graph Storage + API

1. Add `Subgraph` struct:
   ```rust
   pub struct Subgraph {
       pub id: usize,
       pub label: String,          // or &'a str
       pub parent_id: Option<usize>,
   }
   ```

2. Add fields to `DAG`:
   ```rust
   pub(crate) subgraphs: Vec<Subgraph>,
   pub(crate) subgraph_id_to_index: HashMap<usize, usize>,
   pub(crate) node_subgraph: HashMap<usize, usize>,  // node_id → subgraph_id
   pub(crate) next_subgraph_id: usize,
   ```

3. Add public API methods:
   ```rust
   dag.add_subgraph("Backend")         -> usize  // returns subgraph ID
   dag.put_nodes(&[2, 3]).inside(sg)   -> Result<(), SubgraphError>
   dag.put_subgraphs(&[db]).inside(be) -> Result<(), SubgraphError>
   dag.subgraph_count()                -> usize
   dag.node_subgraph(node_id)          -> Option<usize>
   dag.has_subgraphs()                 -> bool
   ```
   Fluent builders: `NodePlacer`, `SubgraphPlacer` with `.inside(id)`.
   `SubgraphPlacer::inside()` walks ancestor chain to detect cycles.

4. Add `SubgraphError` enum:
   ```rust
   pub enum SubgraphError {
       SubgraphNotFound(usize),
       NodeNotFound(usize),
       CycleDetected,
   }
   ```

#### Phase 2: IR Extensions

5. Add `SubgraphInfo` to both IR types:
   ```rust
   // Heap IR
   pub struct SubgraphInfo {
       pub id: usize,
       pub parent_id: Option<usize>,
       pub label: String,
       pub x: usize, pub y: usize,
       pub width: usize, pub height: usize,
   }

   // Arena IR — uses Coord for compactness
   pub struct SubgraphInfoArena {
       pub id: Idx,
       pub parent_id: Idx,  // Idx::MAX = none
       pub label_offset: usize, pub label_len: usize,
       pub x: Coord, pub y: Coord,
       pub width: Coord, pub height: Coord,
   }
   ```

6. Add `subgraphs: Vec<SubgraphInfo>` to `LayoutIR`.
7. Add `subgraphs: &'a [SubgraphInfoArena]` to `LayoutIRArena`.

#### Phase 3: Layout Algorithm Changes

All subgraph-aware logic is **orchestration wrappers** around existing
phases — the hot loops (median sort, adjacent exchange) are not modified.

8. **`vnode_subgraph()` helper** (common):
   Resolve subgraph for a VNode.  Real nodes: look up `node_subgraph`.
   Dummy nodes: return subgraph only if both edge endpoints are in the
   *same* subgraph (cross-subgraph dummies float freely).

9. **Contiguous level enforcement** (new pass, after level assignment):
   Process subgraphs bottom-up (deepest nesting first).  For each,
   compact its member nodes to occupy contiguous levels.  Renormalize
   `max_level` afterwards.

10. **Block-based crossing reduction** (wraps existing median/exchange):
    When `dag.has_subgraphs()`:
    - Partition each level into blocks (one per subgraph + unaffiliated)
    - Run existing median/exchange *within* each block
    - Order blocks by average median of their members
    - Post-pass: stable-partition to enforce adjacency

11. **Subgraph padding** (new pass, after x-coordinate assignment):
    - Horizontal: for adjacent vnodes on a level, count boundary
      transitions via `countBoundaryTransitions` (LCA), insert
      `transitions × SUBGRAPH_PADDING` extra x-space
    - Vertical: build level-presence matrix, compute stacking depth
      of subgraph open/close borders for per-level y-offsets

12. **Bounding box computation** (new pass, after coordinate assignment):
    - Pass 1: compute min/max (x, y) envelope per subgraph from members
    - Pass 2 (bottom-up): apply padding + label row, propagate to parent
    - Emit `SubgraphInfo` entries to IR builder

#### Phase 4: Rendering

13. **Scanline renderer** (heap): Draw subgraph border characters
    (`┌─┐│└─┘`) and label text at computed bounding box coordinates.
    Render parents first (correct z-order for nesting).

14. **Arena buffer renderer**: Same border drawing using pre-allocated
    buffer.  Border characters go on the canvas before node/edge drawing.

#### Phase 5: Arena Allocation

15. Add subgraph temp buffers to `LayoutTemps`:
    - `node_subgraph_map: &'a mut [Idx]` (node_idx → subgraph_id, Idx::MAX = none)
    - `subgraph_parents: &'a mut [Idx]` (sg_idx → parent sg_idx)
    - `subgraph_bbox: &'a mut [(Coord, Coord, Coord, Coord)]` (x, y, w, h)

16. Update `estimate_layout_arena_size()` for sub graph buffer overhead.

### Files to touch

| File | Changes |
|------|---------|
| `src/graph.rs` | `Subgraph` struct, 4 new fields on `DAG`, `add_subgraph()`, `put_nodes()`, `put_subgraphs()`, `NodePlacer`, `SubgraphPlacer`, query methods |
| `src/algorithms/sugiyama/subgraph.rs` | **NEW** — `vnode_subgraph()`, `contiguous_levels()`, `subgraph_padding()`, `compute_bounding_boxes()` |
| `src/algorithms/sugiyama/mod.rs` | `pub mod subgraph;` |
| `src/algorithms/sugiyama/heap.rs` | Wire subgraph passes into `compute_layout()`: contiguous → crossing (block-aware) → x-assign → padding → bbox |
| `src/algorithms/sugiyama/arena.rs` | Wire subgraph passes into `compute_layout_arena()`, add temp buffers to `LayoutTemps` |
| `src/algorithms/sugiyama/arena_phases.rs` | Block-aware crossing reduction variant, bbox computation |
| `src/algorithms/sugiyama/crossing.rs` | `BlockPartitionedMedian` variant or adapt existing `Median`/`AdjacentExchange` |
| `src/ir/mod.rs` | `SubgraphInfo` struct, add `subgraphs` field to `LayoutIR` and builder |
| `src/ir/arena.rs` | `SubgraphInfoArena` struct, add `subgraphs` field to `LayoutIRArena` |
| `src/render/scanline.rs` | Subgraph border rendering pass |
| `src/ir/arena_render.rs` | Subgraph border rendering in buffer renderer |
| `src/lib.rs` | Re-export `Subgraph`, `SubgraphInfo`, `SubgraphError` |

### Design constraints

- **Zero cost when unused:** All subgraph storage is empty by default.
  `has_subgraphs()` short-circuits all layout extensions.  No per-node
  overhead when subgraphs are not used.
- **`no_std` compatible:** Arena path must work without `HashMap` — use
  flat arrays indexed by node/subgraph index instead.
- **Hot loops untouched:** Block-partitioning happens *around* the
  existing median/exchange functions, not inside them.  They receive
  smaller slices, not different logic.

---

## P3 — Network Simplex Layering

**Status: Future**

Optimal layering (Gansner et al. 1993) minimizes total edge span.
Biggest algorithmic quality upgrade for deep graphs.

---

## P4 — Generic Coordinate Type on IR

**Status: Future**

Allow IR to carry `u16` or `f32` coordinates.  Enables SVG output and
halves arena IR memory with `u16`.

---

## P4 — Spline Edge Paths in IR

**Status: Future**

Add `EdgePath::Spline { cp1, cp2 }` variant for Bézier curves.  Even
without SVG, this future-proofs the IR for graphical backends.

---

## What to Keep (ascii-dag advantages over zigraph)

| Strength | Why |
|----------|-----|
| `no_std` + arena IR (zero-alloc) | zigraph can't do true zero-alloc |
| CSR graph (packed adjacency) | Cache-friendly, zigraph uses ArrayLists |
| Scanline Y-index (lazy `OnceCell`) | O(items_on_line) vs full-grid scan |
| `BitSet` packed bitmaps | 64× memory reduction |
| Edge `min_y`/`max_y` early-exit | Arena renderer skips off-screen edges |
| Configurable `Idx` (u8/u16/u32) | Fine-grained memory control |
