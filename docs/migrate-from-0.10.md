# Migrating from 0.10 to 0.11

Everyday graph construction and the current one-shot render methods
remain available. The main breaks are the removed legacy methods,
scene-based introspection, split render options, insertion return types,
diagnostic delivery, and mandatory axis selection when defaults are off.
Work through the sections that apply to you; skip the rest.

Coming directly from 0.9? Apply [the 0.9 → 0.10 guide](migrate-from-0.9.md)
first, then this one. You can make both sets of changes together; there
is no need to install or release against 0.10 in between.

[The migration corpus](../tests/migration_corpus.rs) checks selected
rendering, sized-node, layout-preset and hit-testing replacements against
reviewed fixtures. It is not a claim that every graph, removed query, or
snippet is byte-identical; documented bug fixes can change output.

## 1. Everyone: what you might actually hit

**`add_node` returns a receipt, `add_edge` a handle.** `add_node`
returns `NodeInsertion` (its `node` is the `NodeId`; it converts into
`NodeId` and `usize` and is `Copy`), so every place that accepted the
handle still does — `add_edge(a, b, None)`, `put_nodes(&[a, b])`. A
binding typed as `NodeId` reads `.node`:

```rust
// 0.10
let a: NodeId = g.add_node(AUTO, "A");
// 0.11
let a = g.add_node(AUTO, "A").node;   // or keep the receipt: it converts
```

`add_edge` returns an `EdgeHandle` that borrows the graph: chain a
port declaration on it, or take `.receipt()` (an `EdgeInsertion`:
the edge's input index and whether either endpoint was auto-created)
and let it go. Holding the handle across the next `add_*` call is a
borrow error, which the compiler points out.

**`ascii_dag::NodeKind` is the scene's enum now** (`Real { origin } |
Dummy`). The IR's `Explicit | Implicit | Dummy` enum is unchanged at
`ascii_dag::ir::NodeKind`; only its crate-root re-export is gone.

**`errors::Diagnostic` is `ErrorChain`.** Same type, same
`caused_by_chain`; the `Diagnostic` name now belongs to the
diagnostics channel (§7).

**Nothing prints to stderr any more.** The `warnings` feature is gone;
its conditions are typed data on a diagnostic run (§7). A build that
enabled `warnings` should drop the feature.

**Exhaustive matches need a wildcard arm.** `Direction`, `EdgePath` and
`EdgePathArena` are `#[non_exhaustive]` (a new `Orthogonal` shape
arrives with the `ports` feature), and `HitResult` gained
`Dummy { edge, level }`. `PlanOptions`, `EmitOptions` and
`ComposeBudget` are `#[non_exhaustive]` structs: build them from a
preset or `Default` and set fields, not as literals.

**Defaults-off builds must select at least one layout axis.** Both axes
remain enabled by default. Update old `features = ["arena"]` or
`features = ["alloc"]` configurations to include `layout-vertical`,
`layout-horizontal`, or both. For example, a vertical no-alloc build:

```toml
ascii-dag = { version = "0.11", default-features = false, features = ["arena", "layout-vertical"] }
```

`layout-vertical` provides `TopDown`/`BottomUp`; `layout-horizontal`
provides `LeftRight`/`RightLeft`. Disabled variants do not exist, and
parsing a disabled direction returns an error. The default direction is
`TopDown` when vertical is enabled, otherwise `LeftRight`. Cargo feature
unification can enable vertical through another dependency and change
that default; reusable libraries should select a direction explicitly.
See [layout.md](layout.md#directions).

**Slightly different output on a few graphs.** Skip-level edges
route in their own lanes, edge labels slide to the nearest free spot
instead of vanishing, self-loops with a label reach the legend, and a
cluster layout that used to compress nodes below their gap rule is
repaired (a little taller or wider). The migration corpus pins the
common shapes byte-identical.

## 2. If you used the 0.9 render entry points

The names 0.10 deprecated are removed. These options reproduce the
legacy looks on the migration fixtures, subject to the output changes
above:

| 0.10 call (deprecated) | 0.11 call |
|---|---|
| `ir.render_scanline()` / `render_scanline_to(w)` | `ir.render_string(&RenderOptions::plain())` / `ir.render_with(&opts, &mut w)` |
| `ir.render_scanline_with_buffer(..)` / `render_scanline_to_bytes(..)` | `ir.render_with(&opts, &mut w)` (any `fmt::Write`) |
| `ir.render_scanline_colored(p)` | `render_string(&colored_no_legend(p))` — see below |
| `ir.render_scanline_colored_to(&mut w, p)` | `ir.render_with(&colored_no_legend(p), &mut w)` |
| `ir.render_scanline_colored_with_legend(p)` | `ir.render_string(&RenderOptions::colored(p))` |
| `arena_ir.render_to_buffer(&mut out, &mut line, &mut scratch)` | `arena_ir.render_to_bytes(&opts, &arena, &mut out)` — buffer migration below |
| `render_to_buffer_colored` / `_with_legend` | `render_to_bytes` with the colored options above |
| `arena_ir.estimate_render_size(..)` | `estimate_render_arena_size(&opts)` + `estimate_render_output_size(&opts)` |
| `arena_ir.compute_edge_colors(..)` | the plan resolves colors; read them from `scene.edges()` |

The 0.10 "colored, no legend" look is an explicit policy now:

```rust
use ascii_dag::{LabelPolicy, RenderOptions};
use ascii_dag::render::colors::Palette;

fn colored_no_legend(palette: Palette) -> RenderOptions {
    let mut o = RenderOptions::colored(palette);
    o.plan.label_policy = LabelPolicy::default(); // geometric placement, overflow omitted
    o.emit.render_legend = false;
    o
}
```

The old arena call returned `Option<usize>` and accepted an output byte
buffer, a `[char]` line buffer, and `[usize]` scratch. Its
`estimate_render_size()` returned `(output_bytes, scratch_len_in_usize)`.
The replacement returns `Result<usize, GraphError>` and takes one render
arena backed by `[u8]`, plus the output buffer. Both new estimates are
in **bytes**; the old line buffer and separate color/skip buffers are
not inputs to the new API.

For a `LayoutIRArena` named `ir`:

```rust,ignore
use ascii_dag::{RenderOptions, graph::arena::Arena};

let options = RenderOptions::plain();
let mut workspace = vec![0u8; ir.estimate_render_arena_size(&options)];
let arena = Arena::new(&mut workspace);
let mut output = vec![0u8; ir.estimate_render_output_size(&options)];
let written = ir.render_to_bytes(&options, &arena, &mut output)?;
```

The `Vec`s illustrate provisioning on a host. A no-allocator caller
provides arrays/slices of sufficient capacity instead; rendering itself
does not allocate. Do not reuse old element counts as byte capacities.
Use the colored options above for the colored wrappers. The unrelated
`CsrGraph::render_to_buffer` and `CsrGraph::estimate_render_size` debug
helpers remain available.

## 3. If you inspected a plan or hit-tested

`RenderPlan` is no longer public. `ir.render_plan(&options)` and
`render_plan_in(..)` move to `ScenePlanner`, and
`ir.hit_test(&plan, x, y)` moves to `Scene::hit_test`. The scene carries
its layout, so a hit-test can no longer be paired with the wrong plan:

```text
// 0.10
let plan = ir.render_plan(&options);
match ir.hit_test(&plan, x, y) { .. }
// 0.11
let mut planner = ScenePlanner::new();
let scene = planner.plan(&ir, &options.plan).quiet()?;
match scene.hit_test(x, y) { .. }
```

Differences worth knowing: a shown routing waypoint reports
`HitResult::Dummy { edge, level }` (its owning edge's input index and
the level) instead of a synthetic node id; clicking a `↺` reports the
self-loop as `Edge(scene_index)` instead of its node. Resolve that
scene index with `scene.edge(index)`; its `input_index` identifies the
original edge insertion. Style callbacks also run for self-loops now.

`ir.y_index()`, `items_at_line` and `LineOccupancy` are removed without
a like-for-like row-candidate query. `hit_test` returns one winning
owner at a cell, not every element spanning a row. For picking, use
`scene.hit_test`; for final cells, use `SceneComposer::visit_cells`;
for all candidates, build your own index over IR geometry or scene
element views. The hit-test golden does not prove row-query equivalence.

`RenderPlan::band_count()` is gone with no public count replacement.
Band partitioning depends on scene geometry and `ComposeBudget`; use
`scene.composition_requirements(&budget)` to size the workspace, not a
band count.

The scene is also the read surface the old plan never had:
`scene.nodes()`, `edges()`, `subgraphs()`, `legend()` yield views
with resolved geometry, colors, weights and label slots, identical
across backends. [The SVG example](../examples/svg_export.rs) reads
these element views; it does not compose cells. For a cell-based
consumer and retained no-alloc sizing, see
[rendering.md](rendering.md#beyond-the-terminal-scenecomposer).

## 4. If you set `RenderOptions` fields

`RenderOptions` keeps its name and its four presets (`plain()`,
`colored(p)`, `ascii()`, `ascii_colored(p)`), but is a composite now:

| 0.10 field | 0.11 field |
|---|---|
| `opts.charset` | `opts.emit.charset` |
| `opts.color_mode` | `opts.emit.color_mode` |
| `opts.legend` | `opts.emit.render_legend` (the block) + `opts.plan.label_policy` (placement/overflow) |
| `opts.palette` | `opts.plan.palette` |
| `opts.edge_style_fn` / `subgraph_style_fn` / `edge_label_style_fn` | `opts.plan.*` |
| `opts.show_dummy_nodes` | `opts.plan.show_dummy_nodes` |
| `opts.band_rows_cap` | `opts.compose.band_rows_cap` |

In 0.10, color plus legend silently switched label placement to avoid
node rows. That pair is the explicit `LabelPolicy { placement:
LabelPlacementPolicy::{Geometric, AvoidNodeRows}, overflow:
LabelOverflow::{Omit, Legend} }`; the presets map the 0.10 looks
exactly, and every combination is now expressible.

## 5. If you configured layout with `SugiyamaConfig`

The crate-root re-export, `Graph::compute_layout_with` (deprecated
since 0.9), `set_sugiyama_config` and `with_sugiyama_config` are gone:

```rust
// 0.10
let mut cfg = SugiyamaConfig::standard();
cfg.crossing_pipeline = QUALITY.to_vec();
g.set_sugiyama_config(cfg);
// 0.11 — the presets map one for one …
let ir = g.compute_layout_with_config(&LayoutConfig::quality()); // or ::fast() / ::standard()
// … or keep the graph's config and set the pipeline on it
g.set_crossing_pipeline(QUALITY);
let ir = g.compute_layout();
```

`set_crossing_reduction_passes` and `set_crossing_pipeline` keep
working. The corpus test `layout_config_presets_replace_sugiyama_config`
pins the preset mapping.

## 6. If you sized nodes with `add_node_with_size` / `add_node_with_width`

Declare content that knows its area:

```rust
g.add_node(10, CustomNode {
    label: "Server", width: 12, height: 5,
    painter: Some(card),   // draws the area — None reserves it blank
    payload: "cpu: 4\nram: 16G",
});
```

For byte-identical 0.10 output (`[label` padded to the width, then
`]`), use the `legacy_sized_look` painter in
`tests/migration_corpus.rs`. See [nodes.md](nodes.md).

## 7. If you relied on warnings, or want diagnostics

Non-fatal conditions are typed data now — a `Diagnostic` with a stable
`code()`, `severity()`, `DiagnosticKind` and `hint()`. Fatal failures
stay in `Result`. Each stage reports only its own conditions: a layout
report does **not** include warnings discovered during later planning.

```rust
use ascii_dag::{DiagnosticRef, DiagnosticRun, Graph, PlanOptions,
                ScenePlanner, VecDiagnostics};

let mut g = Graph::new();
g.add_node(1, "A");
g.add_edge(1, 2, None); // layout reports the implicit placeholder
g.add_edge(1, 1, Some("retry")); // planning reports the omitted loop label
let mut run = DiagnosticRun::new(VecDiagnostics::default());
let ir = g.layout().compute(&mut run.context());
let options = PlanOptions::new();
let mut planner = ScenePlanner::new();
let outcome = planner.plan(&ir, &options).compute(&mut run.context());
let report = run.finish(outcome); // earlier warnings survive a planning failure

for item in report.diagnostics() {
    match item {
        DiagnosticRef::Retained(d) => eprintln!("{d}"),
        DiagnosticRef::Failure(e) => eprintln!("{e}"),
        DiagnosticRef::Cause(e) => eprintln!("caused by: {e}"),
    }
}
let (outcome, _sink) = report.into_parts();
// `outcome` is Result<Scene, GraphError>; handle it or pass the scene
// to a retained renderer. It still borrows `planner` and `ir`.
assert!(outcome.is_ok());
```

Only this caller chooses stderr. `.reported()` on either the layout
builder or the plan builder is a convenience for collecting **that
operation** into an owned report. Use one `DiagnosticRun` and the
context-taking forms to collect across stages.

`compute_layout()`, `compute_layout_with_config()`, `.quiet()`, and the
one-shot render methods (`render_string`, `render_with`,
`render_to_bytes`) discard their diagnostics. In particular, rendering
an IR from a reported layout with `render_string` still discards
planning's omitted-label warnings. Keep the diagnosed scene and use
`TerminalRenderer` to emit it without replanning.

See [diagnostics.md](diagnostics.md) for a complete layout → plan →
render report, failure presentation, bounded no-alloc sinks, and the
difference between `warnings()` and `diagnostics()`. Point events — a
placeholder's creation, an `AUTO` replacement — remain on receipts (§1).

## 8. If you used the raw color helpers

`colors::get`, `escape::fg256` and `escape::write_fg256` are gone:
index `Palette::colors()` directly, or read resolved colors from the
scene (`CellColor::as_rgb()` / `as_ansi256()` are public). The engine
emits escapes itself under `RenderOptions::colored`. `colors::get_custom`
and `escape::RESET` remain.

In the current tree, heap `LayoutIR::compute_edge_colors` and
`edge_color_index` on both IR types still exist; do not confuse them
with the removed arena `compute_edge_colors`. New scene consumers
should use resolved edge colors instead of recomputing them.

## 9. If you build IRs or CSR graphs by hand

- `LayoutEdge` / `LayoutEdgeArena` literals: add `from_port` and
  `to_port` — `PortAttachment::auto(PhysicalSide::South)` /
  `auto(PhysicalSide::North)` for an undeclared, non-reversed TopDown
  edge. These describe the declared ends: swap them for a reversed
  TopDown edge, and use the corresponding physical faces for other
  directions (see [ports.md](ports.md)).
- `LayoutIRArenaBuilder::new_with_subgraphs(..)`: new trailing
  `max_self_loops` capacity — pass `0` when there are none.
  `add_self_loop` takes the node's table index, and the estimators
  grew by one record per edge bound.
- `CsrGraphBuilder::new_with_ports(..)` + `required_arena_size_with_ports`
  when edges declare sides; the plain builder's handles return `None`
  from `from_port` / `to_port`.
- `NodePaintCtx` lost its `charset`: painters draw structure through
  `NodeRegion::{hrule, vrule, frame, arrow}` and the emission decodes
  it for either charset.
- JSON schema is `"1.5"`: a `self_loops[]` array (omitted when
  empty), the `orthogonal` path type, and `from_side` / `to_side` on
  every edge (`from_port` / `to_port` when declared).
- Workspace requirements changed; size from the current estimators,
  never from a 0.10 constant. For retained scenes, use the
  [three sizing contracts](rendering.md#no-alloc-sizing-three-contracts)
  instead of the one-shot render arena estimate.

## 10. Worth adopting while you're here

- Plan once, render many: `ScenePlanner` → `Scene` →
  `TerminalRenderer::render` under any emission options, with zero
  replanning and, in steady state, no allocation.
- Side ports: `add_edge(a, b, None).from_port(PortSide::East)`, port
  policies per node, and the attachments reported on every IR edge
  ([ports.md](ports.md), `examples/ports.rs`).
- Self-loops are records with identity, label and style — they reach
  the legend and hit-test as edges.
- Receipts instead of guesswork: `EdgeInsertion::created_source` /
  `created_target` say when an edge invented an endpoint.
