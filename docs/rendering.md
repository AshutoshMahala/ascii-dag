# Rendering

How to change the *look* of a graph, get the output into wherever it
needs to go, and answer "what did the user just click on".

For the *shape* — direction, spacing, clusters — see
[layout.md](layout.md).

## Output surfaces

These exist on both IR types (`LayoutIR` from the heap pipeline,
`LayoutIRArena` from the arena one), so the choice of *pipeline* never
changes how you render. What does change things is the `alloc`
feature: the convenience surfaces need somewhere to put their result.

| You want | Use | Needs `alloc` |
|---|---|:---:|
| A `String` | `render_string(&options)` | yes |
| To write into something | `render_with(&options, &mut writer)` | yes |
| Bytes, into your buffer | `render_to_bytes(&options, &arena, &mut buf)` | no |
| To inspect, not paint | `ScenePlanner::new().plan(&ir, &options.plan)` | yes |
| …the same, no-alloc | `ScenePlanner::new_in(&mut workspace)` | no |
| What is at this cell | `scene.hit_test(x, y)` | no |
| Repeated renders of one scene | `TerminalRenderer::new(&emit, req)` + `render(&scene, &mut out)` | no (`new_in`) |

On a build with `default-features = false, features = ["arena"]`,
`render_to_bytes` and the `new_in` planner are what you have — which
is the whole no-allocation story, just spelled explicitly.

```rust
use ascii_dag::{Graph, RenderOptions};

let g = Graph::from_edges(&[(1, "A"), (2, "B")], &[(1, 2)]);
let ir = g.compute_layout();
print!("{}", ir.render_string(&RenderOptions::plain()));
```

## Options

Options live in three homes by what they affect: `plan` holds
everything that changes resolved semantics, `emit` holds how those
semantics are written, and `compose` holds memory knobs that never
affect output.

```rust
let mut options = RenderOptions::plain();
options.emit.charset = ascii_dag::Charset::Ascii;
options.plan.label_policy.overflow = ascii_dag::LabelOverflow::Legend;
options.emit.render_legend = true;
options.compose.band_rows_cap = 32;
```

| Field | Values | Default | What it does |
|---|---|---|---|
| `emit.charset` | `Unicode` / `Ascii` | `Unicode` | Equal projections of one canvas — `Ascii` swaps `┌─│→` for `+-\|>` |
| `emit.color_mode` | `None` / `Ansi256` / `TrueColor` | `None` | ANSI escapes for edge coloring |
| `emit.render_legend` | `bool` | `false` | Print the legend block after the diagram |
| `plan.palette` | `Palette::*` | `Ansi` | Which colors the edge coloring draws from |
| `plan.label_policy.placement` | `Geometric` / `AvoidNodeRows` | `Geometric` | Whether inline labels may sit on rows hosting nodes |
| `plan.label_policy.overflow` | `Omit` / `Legend` | `Omit` | Where labels that found no inline position go |
| `plan.show_dummy_nodes` | `bool` | `false` | Draw `◍` at routing waypoints (see [layout.md](layout.md#routing-waypoints)) |
| `plan.edge_style_fn` etc. | `fn` pointers | legacy look | Per-element styling, below |
| `compose.band_rows_cap` | `usize` | `64` | Canvas memory ceiling — see [banding](#banding-and-memory) |

Four `const` presets cover the common combinations:
`RenderOptions::plain()`, `::colored(palette)`, `::ascii()`,
`::ascii_colored()`.

Pick `Ascii` when the destination cannot render box-drawing glyphs —
a log aggregator, a CI annotation, a small embedded font. Both
charsets decode the same semantic canvas, so topology, junctions and
arrow directions all survive. ASCII has fewer glyphs to spend, so some
distinctions collapse: light, dashed and double verticals all become
`|`, and mixed-weight junctions become `+`. The graph is still
correct; it is just less expressive.

## Labels and the legend

Edge labels are placed geometrically: the renderer puts each one where
it fits without overwriting anything that carries meaning. When a
label cannot be placed, it goes to the legend instead of being
dropped:

```rust
let mut options = RenderOptions::plain();
options.plan.label_policy.overflow = ascii_dag::LabelOverflow::Legend;
options.emit.render_legend = true;
```

```text
Edge labels:
Client → Gateway: "http"
Users → DB: "read"
```

The legend works in colored *and* plain output. In color, entries are
tinted to match their edge; in plain they are self-keying, since each
line names both endpoints. It is off by default, so enabling it never
changes existing output.

## Styling

Three plain `fn` pointers, resolved once per element at plan time —
never per cell. Being `fn` rather than a closure keeps them
`no_std`-safe and `Copy`.

### Edges

```rust
use ascii_dag::render::engine::{EdgeStyle, EdgeStyleCtx, LineWeight, MarkerShape};

fn dashed_back_edges(ctx: EdgeStyleCtx<'_>) -> EdgeStyle {
    EdgeStyle {
        weight: Some(if ctx.reversed { LineWeight::Dashed } else { LineWeight::Light }),
        marker_end: MarkerShape::Arrow,
        ..EdgeStyle::default()
    }
}

let mut options = RenderOptions::plain();
options.plan.edge_style_fn = dashed_back_edges;
```

`EdgeStyleCtx` carries the edge index, both endpoint ids, the label,
and whether the edge is directed or was reversed during cycle
breaking — enough to color by endpoint, weight by semantic, or
suppress arrowheads on undirected graphs.

`MarkerShape::None` on `marker_end` removes the arrowhead;
`marker_start: MarkerShape::Arrow` gives you a double-headed edge.

### Subgraph styling

```rust
use ascii_dag::render::engine::{LabelPosition, SubgraphBorder, SubgraphStyle, SubgraphStyleCtx};

fn light_boxes(_ctx: SubgraphStyleCtx<'_>) -> SubgraphStyle {
    SubgraphStyle {
        border: SubgraphBorder::Light,
        label_pos: LabelPosition::InsideBottom,
        ..SubgraphStyle::default()
    }
}
```

`SubgraphBorder::None` groups nodes without drawing any box — the
label still paints, and the cluster still affects layout. Useful when
you want the grouping to influence placement without the visual
weight.

### Edge labels

`edge_label_style_fn` takes the same `EdgeStyleCtx` and returns an
`EdgeLabelStyle`, so a label can be colored independently of its edge.

## Streaming

`render_with` writes into any `core::fmt::Write` and never builds the
whole string:

```rust
let mut out = String::new();
ir.render_with(&options, &mut out)?;
```

Rendering is banded, so this genuinely streams: a band is composited,
emitted, and its buffer reused. **Peak canvas memory is one band**,
not one canvas, however tall the graph is. (The render plan is
separate and stays proportional to nodes and edges.)

`render_with` takes a `core::fmt::Write`, which is *not* the same
trait as `std::io::Write` — there is no standard adapter between them.
For a socket or file, either write a four-line shim:

```rust
struct IoSink<W>(W);

impl<W: std::io::Write> core::fmt::Write for IoSink<W> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        self.0.write_all(s.as_bytes()).map_err(|_| core::fmt::Error)
    }
}
```

…or render to bytes and hand them over with `io::Write::write_all`.

## No-allocation rendering

The arena path never allocates. Size the two buffers from the IR, hand
them over, and you get bytes back:

```rust
use ascii_dag::graph::arena::Arena;

let mut arena_buf = vec![0u8; ir.estimate_render_arena_size(&options)];
let arena = Arena::new(&mut arena_buf);
let mut out = vec![0u8; ir.estimate_render_output_size(&options)];

let written = ir.render_to_bytes(&options, &arena, &mut out)?;
let text = core::str::from_utf8(&out[..written]).unwrap();
```

Both estimates are upper bounds — an exactly-sized buffer always
suffices, which is what lets you provision statically on a
microcontroller. Undersized buffers do not panic; they return
`RenderPlanOom`, `RenderCanvasTooSmall`, or `RenderOutputTooSmall`,
each with a hint naming what to grow.

On a target with no allocator at all, the buffers are plain arrays:

```rust
let mut arena_buf = [0u8; 4096];
let mut out = [0u8; 2048];
```

`examples/lean_render.rs` carves the graph, the layout, and the render
out of one fixed 16 KB block — nothing allocates, and the layout's
scratch memory is reused for rendering. `examples/longan_nano` does
the same on a 32 KB RISC-V board.

## Banding and memory

The renderer composites in horizontal bands. `band_rows_cap` caps the
band height, so canvas memory is `width × min(cap, height)` cells
regardless of how tall the graph is — a 50,000-node fan renders in
bounded memory.

Output is byte-identical at every cap; the setting trades a little
per-band overhead against peak memory, and 64 (the default) is a good
balance. Lower it on memory-constrained targets.

## Hit-testing (interactive and IDE consumers)

A `Scene` gives you the renderer's own geometry without painting
anything: a `ScenePlanner` resolves the layout once (styles run
exactly once), and `hit_test` answers what occupies a cell. The scene
borrows both the planner and the layout, so a stale plan/layout
pairing is a compile error:

```rust
use ascii_dag::render::engine::HitResult;
use ascii_dag::ScenePlanner;

let mut planner = ScenePlanner::new();
let scene = planner.plan(&ir, &options.plan)?;
match scene.hit_test(x, y) {
    HitResult::Node(id) => println!("node {id}"),
    HitResult::Edge(index) => println!("edge #{index}"),
    HitResult::Subgraph(id) => println!("cluster {id}"),
    HitResult::None => {}
    _ => {}
}
```

This is what an editor plugin, TUI, or web terminal needs to make a
rendered graph clickable: translate a click to a cell (subtract your
own origin, account for scroll), then ask the scene.

The semantics are deliberately hybrid, because that is what feels
right under a cursor:

- **Nodes** hit as their full reserved rectangle, so a custom painter's
  whole area belongs to the node.
- **Self-loops** own their `↺` marker cell — clicking it selects the
  loop edge (`HitResult::Edge` of its scene index), not the node.
- **Edges** hit as *painted ink* — the exact cells the compositor drew,
  not a bounding box, so two crossing edges resolve correctly. A
  painted edge label belongs to its edge.
- **Bordered clusters** own their complete rectangle — border, label,
  and blank interior alike (clicking inside a box selects it); a
  borderless cluster (`SubgraphBorder::None`) has only its label cells
  to hit.

Nodes win over edges, edges over clusters — the visual z-order.

A scene is bound to the layout it was planned from — it borrows both
its planner and the layout, so a stale pairing is a compile error
rather than a runtime surprise. A query outside its canvas returns
`HitResult::None` rather than panicking. Re-plan when the layout
changes; a scene is a snapshot, not a live view.

`Scene` also exposes `width`, `height`, and `legend_entries` for
laying out a viewport around the graph.

## Beyond the terminal (`SceneComposer`)

Terminal strings are one projection of the composed canvas. A
`SceneComposer` hands you the canvas itself — one `CellView` per cell,
row-major — for SVG writers, TUI buffers, or interactive pickers:

```rust
use ascii_dag::SceneComposer;

let req = scene.composition_requirements(&options.compose);
let mut composer = SceneComposer::new(req);
composer.visit_cells(&scene, |x, y, cell| {
    // cell.kind:  Empty / Text / Stroke { per-arm weights } / Marker
    // cell.color: resolved color, whatever the emission mode
    // cell.owner: what hit_test reports for this cell
})?;
```

Stroke cells carry per-arm weights instead of pre-picked glyphs —
Unicode and ASCII terminal output are two projections of this same
vocabulary, and an SVG consumer can draw real vector strokes from it.
`try_visit_cells` is the early-exit form for fallible sinks. The
composer retains its workspace: steady-state repaint allocates
nothing, and `SceneComposer::new_in` composes out of a caller-provided
byte slice for no-alloc targets.

### No-alloc sizing (three contracts)

Every retained buffer in the scene pipeline has its own estimator, and
each is pinned by exact-size tests — a buffer of precisely the
estimated size always suffices:

| Buffer | Sized by | Handed to |
|---|---|---|
| Scene storage | `ir.estimate_scene_size(&plan_options)` | `ScenePlanner::new_in` |
| Composition workspace | `req.workspace_bytes()` (semantic) / `req.terminal_workspace_bytes(&emit)` (terminal) | `SceneComposer::new_in` / `TerminalRenderer::new_in` |
| Output bytes | `scene.estimate_output_size(&emit)` | your buffer |

(`req` is `scene.composition_requirements(&budget)`. The one-shot
`render_to_bytes` keeps its combined
`estimate_render_arena_size`/`estimate_render_output_size` pair — it
plans and composes in a single arena.)

See `examples/hit_test.rs` for a working terminal program — it enables
xterm mouse reporting with raw escape sequences and no dependencies,
and `--probe X Y` makes it scriptable.

## Rendering someone else's IR

Nothing above requires the layout to have come from this crate's
pipeline. `LayoutIRBuilder` builds an IR by hand, and every render
surface accepts it — useful if you have your own positions and just
want the painter.
