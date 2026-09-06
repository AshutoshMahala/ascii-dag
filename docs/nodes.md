# Nodes as objects

A node's declaration is the only source of what it is — its label,
its size, and what fills its area at render time. Nothing else in the
system (styles, render options) can change a node's content.

## The three kinds

```rust
use ascii_dag::{Graph, AUTO, BoxedNode, CustomNode};

let mut g = Graph::new();
let a = g.add_node(AUTO, "Client");              // simple: [Client]
let b = g.add_node(AUTO, BoxedNode("Database")); // boxed: ┌─ box ─┐
let c = g.add_node(AUTO, CustomNode {            // custom: you paint it
    label: "Server",
    width: 12,
    height: 5,
    painter: Some(card),
    payload: "cpu: 4\nram: 16G",
});
g.add_edge(a, c, None);
g.add_edge(c, b, None);
```

- **`&str`** — the classic `[label]`, one row. Identical to every
  release before node objects existed.
- **`BoxedNode(label)`** — a light-stroke box, `label+4 × 3` cells.
- **`CustomNode { .. }`** — you declare the reserved area and a
  painter; layout routes edges around the area, render hands your
  painter a clipped region to fill.

## Custom painters: template + data

The painter is a plain `fn` — the **template**, shared by every node
that uses it. The per-node **data** is the `payload` string, declared
on the node and delivered back at paint time:

```rust
use ascii_dag::render::engine::{NodePaintCtx, NodeRegion};

fn card(region: &mut NodeRegion<'_, '_>, ctx: NodePaintCtx<'_>) {
    region.write_str(1, 0, ctx.label);
    region.hrule(0, region.width() - 1, 1); // semantic rule: `─`, or `-` under --ascii
    for (i, line) in ctx.payload.lines().enumerate() {
        region.write_str(1, 2 + i, line);
    }
}
```

Rules of the region: coordinates are node-local, writes outside the
declared area are silently dropped (painters cannot damage the rest of
the diagram), and painters are replayed per band — draw the same
content every call, and use `ctx.visible_rows` to skip clipped rows in
tall nodes. Draw structure through the semantic primitives — `hrule`,
`vrule`, `frame`, `arrow` — and it decodes per charset at emission
like all engine ink (and merges into proper tees and junctions where
strokes meet); text via `set`/`write_str` passes through untranslated.

For full control without a struct of your own, implement the
`NodeContent` trait: `label()`, `size()`, `painter()`, `payload()`.
A `CustomNode` with `painter: None` is a **blank** node — the area is
reserved (edges route around it) but nothing paints: a spacer, or a
rectangle something else will fill.

## Ids and handles

`add_node(AUTO, …)` numbers nodes for you and returns a `NodeInsertion`
receipt. Its `.node` is the `NodeId` handle; the receipt also converts
into a handle for graph methods accepting ids. This is the recommended
style when the graph is built from scratch. Explicit ids (`add_node(7, …)`)
remain first-class for graphs built from external identities (package
ids, task numbers): edges can then be added straight from external
pairs, and `add_edge` auto-creates missing endpoints. Mixing is safe
in the natural order (explicit first, `AUTO` after — the counter stays
above every id it has seen); re-adding an existing id replaces that
node, and the `NodeInsertion` receipt every `add_node` returns
records the replacement — flagging AUTO involvement on either side,
the variant worth attention.

## Both backends, JSON

Declarations travel the whole system: the heap pipeline, the
CSR/arena pipeline (including graphs built directly on
`CsrGraphBuilder` — it accepts the same content vocabulary), and JSON
export, where every node carries `"content_kind"` and custom nodes
carry their `"payload"`. Painters are code; they never serialize.
