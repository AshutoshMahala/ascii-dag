# Layout

How to change the *shape* of a graph: direction, spacing, clusters,
crossing reduction, and reading the layout IR directly.

For the *look* — colors, charsets, styling, streaming — see
[rendering.md](rendering.md).

## Directions

Levels are the ranks the layout assigns. Which physical axis they
occupy is the direction:

```rust
use ascii_dag::{Direction, Graph};

let mut g = Graph::new();
g.set_direction(Direction::LeftRight);
```

| Direction | Parses from | Levels are | Reads well for |
|---|---|---|---|
| `TopDown` (default) | `"TB"`, `"TD"` | rows, top → bottom | deep graphs |
| `BottomUp` | `"BT"` | rows, bottom → top | build-up / stack views |
| `LeftRight` | `"LR"` | columns, left → right | wide, shallow graphs |
| `RightLeft` | `"RL"` | columns, right → left | mirrored pipelines |

```text
TopDown                 LeftRight
                                          ┌→[Store]
  [Fetch]                     "raw"       │
     │ "raw"            [Fetch]──→[Parse]─┤
     ↓                                    │
  [Parse]                                 └→[Index]
   ┌─┴──┐
   ↓    ↓
[Store] [Index]
```

`BottomUp` and `RightLeft` are exact mirrors of their counterparts,
applied to the finished layout — so IR coordinates always match
rendered cells, whichever direction you pick.

Directions parse from the conventional short forms, which is handy
when the value comes from a config file or CLI flag:

```rust
let dir: ascii_dag::Direction = "LR".parse().unwrap();
```

### Choosing one

It depends on how many *levels* you have, not how many nodes.

A **deep** graph — a long pipeline, a dependency chain — has many
levels. `TopDown` turns each into a row, so it grows tall and
scrolls; `LeftRight` turns them into columns and spends the
terminal's width instead. A 4-node chain is 13 rows in `TopDown` and
6 in `LeftRight`.

A **broad, shallow** graph — one source fanning out to a dozen
siblings — is already short in `TopDown`, because siblings sit
side by side. `LeftRight` would stack those siblings vertically and
make it taller.

Rule of thumb: many levels → `LeftRight`; many nodes per level →
`TopDown`.

## Spacing

```rust
use ascii_dag::LayoutConfig;

let mut config = LayoutConfig::standard();
config.node_spacing = 4;   // gap between nodes WITHIN a level
config.level_spacing = 1;  // extra gap BETWEEN levels
let ir = g.compute_layout_with_config(&config);
```

Both settings are in role space, so the same config behaves sensibly
in every direction:

| Setting | `TopDown`/`BottomUp` | `LeftRight`/`RightLeft` |
|---|---|---|
| `node_spacing` | columns between siblings | rows between siblings |
| `level_spacing` | rows between levels | columns between levels |

`level_spacing` defaults to `0` because edge routing already reserves
the space it needs between levels; raise it when you want visual air.
In the horizontal directions rows are the cheap axis, so
`node_spacing = 1` often reads better than the default 3.

## Clusters (subgraphs)

Group nodes into labeled, nestable boxes:

```rust
let mut g = Graph::new();
g.add_node(1, "Users");
g.add_node(2, "Orders");
g.add_node(3, "Queue");

let services = g.add_subgraph("Services");
let async_box = g.add_subgraph("Async");
g.put_subgraphs(&[async_box]).inside(services)?;   // nest
g.put_nodes(&[1, 2]).inside(services)?;
g.put_nodes(&[3]).inside(async_box)?;
```

Placement *validates* — `put_nodes` fails with `NodeNotFound` rather
than silently creating a node, so a typo'd id is an error, not a
phantom box member.

Boxes affect layout, not just paint: members are kept together during
crossing reduction, borders reserve padding, and unaffiliated nodes
are pushed clear of a box's projected envelope. A box always grows to
fit its label (capped at 40 characters — longer labels are truncated
at render time rather than blowing up the canvas).

Border style, color, and label position are *rendering* settings —
see [rendering.md](rendering.md#subgraph-styling).

## Crossing reduction

Fewer edge crossings, more time. Presets, or build your own pipeline:

```rust
use ascii_dag::{CrossingReducer, FAST, QUALITY, STANDARD};

g.set_crossing_pipeline(QUALITY);
// or a custom sequence:
g.set_crossing_pipeline(&[
    CrossingReducer::Median(4),
    CrossingReducer::AdjacentExchange(2),
]);
```

`FAST` skips most work, `STANDARD` is the default, `QUALITY` runs
more passes. On dense fan-in/fan-out shapes the difference is
visible; on chains and trees it usually is not.

## Cycles

The layout is a DAG layout, but it does not fall over on a cycle —
though the two entry points respond differently:

- `compute_layout()` breaks the cycle internally and lays the graph
  out anyway. The reversed edges render dashed with a back-pointing
  arrowhead, so you still get a picture.
- `Graph::render()` checks first and prints a **cycle banner** naming
  the cyclic chain instead of a layout.

To decide for yourself:

```rust
if g.has_cycle() {
    // e.g. warn, then still draw it with compute_layout()
}
```

There is also cycle detection **over your own types**, with no
`Graph` involved — useful for validating a config or a build plan
before you ever build a graph:

```rust
use ascii_dag::algorithms::cycles::generic::detect_cycle_fn;

let ids = ["app", "lib", "core"];
let cycle = detect_cycle_fn(&ids, |id| match *id {
    "app" => vec!["lib"],
    "lib" => vec!["core"],
    "core" => vec!["app"],   // ← cycle
    _ => vec![],
});
assert!(cycle.is_some());
```

`has_cycle_fn` is the boolean form; `topological_sort_fn` (at
`algorithms::generic`) returns either an order or the offending cycle.
All three need the `generic` feature, which is on by default.

## Routing waypoints

Edges that skip levels route through invisible waypoints. Two
switches make them visible — one per side of the pipeline:

```rust
let mut config = LayoutConfig::standard();
config.include_dummy_nodes = true;   // layout: emit them into the IR
let ir = g.compute_layout_with_config(&config);

let mut options = RenderOptions::plain();
options.plan.show_dummy_nodes = true; // render: draw the marker
```

Both are required — the render switch has nothing to draw unless the
layout emitted them. With `include_dummy_nodes` on they appear in the
IR as nodes with `kind == ascii_dag::ir::NodeKind::Dummy` (the IR
enum — crate-root `NodeKind` is the scene vocabulary) and an `edge_index`
back-link to the edge they belong to, which is how you attribute a
routing cell to its edge.

Note that they are *real IR nodes*: `nodes()` gets longer and level
lists include them. Leave the flag off (the default) unless you are
debugging routing or drawing your own edges.

## Reading the IR directly (headless)

`compute_layout()` returns a `LayoutIR` whose coordinates are
physical — they are the cells the text renderer would paint. That
makes it a layout engine for *any* renderer: Canvas, SVG, a TUI
widget, an IDE overlay.

```rust
let ir = g.compute_layout();
for node in ir.nodes() {
    println!("{} at ({}, {}) {}×{}", node.label, node.x, node.y, node.width, node.height);
}
for edge in ir.edges() {
    // `flow_axis` says which physical axis this edge's trunk runs
    // along — read it rather than inferring from the endpoints,
    // which is ambiguous for corner paths.
    println!("{} → {} ({:?})", edge.from_id, edge.to_id, edge.flow_axis);
}
```

`EdgePath` describes the route: `Direct`, `Corner { bend_at }`,
`MultiSegment { waypoints, .. }`, `Orthogonal { bends }` (an explicit
polyline — every turn stated, physical `(x, y)` cells on either axis;
the shape of a route that leaves a node against the flow or beside
it), plus `SideChannel` and `Spline` variants for hand-built IRs. The
level-axis scalars (`bend_at`, `channel_at`, `span_start`/`span_end`)
live on the axis the edge's `flow_axis` names — a row for `Y` trunks,
a column for `X`. Every edge also reports its attachments:
`from_port` / `to_port` say which side each end asked for and which
physical side it took — see [ports.md](ports.md).

Export it as JSON (schema v1.5) for a renderer in another language:

```rust
let json = ir.to_json();
```

`LayoutIRBuilder` lets you construct an IR by hand and render it, if
you want the painter without the layout.

## No-allocation layout

The arena pipeline computes the same layout with no allocator at all
— see [rendering.md](rendering.md#no-allocation-rendering) for the
full flow, and `examples/lean_render.rs` for a working program.
