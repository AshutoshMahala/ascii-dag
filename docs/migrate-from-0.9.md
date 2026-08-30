# Migrating from 0.9 to 0.10

Most 0.9 code compiles and renders unchanged. Work through the
sections that apply to you; skip the rest.

## 1. Everyone: three breaks you might actually hit

**`add_node` returns a handle now.** Statement-position calls are
fine. Expression-position closures that must return `()` need a block:

```rust
// 0.9
nodes.iter().for_each(|n| g.add_node(n.id, n.label));
// 0.10 — add_node returns NodeId, for_each wants ()
nodes.iter().for_each(|n| { g.add_node(n.id, n.label); });
```

**String-ish arguments coerce less.** The label parameter became a
generic content slot, and Rust only auto-derefs to `&str` for concrete
`&str` parameters. Plain `&str` and `&String` still work; `&mut str`,
`&Box<str>` and similar wrappers need a nudge:

```rust
// 0.9
g.add_node(1, Box::leak(text.into_boxed_str()));   // &'static mut str
// 0.10
g.add_node(1, &*Box::leak(text.into_boxed_str())); // reborrow to &str
```

**Empty slice literals need a type.** `put_nodes` accepts handles or
raw ids now, so an *empty* literal has nothing to infer from:

```rust
g.put_nodes(&[] as &[usize]).inside(sg)?; // non-empty literals: unchanged
```

(Rare: storing `add_node`/`add_edge` as plain `fn` pointers no longer
coerces — wrap them in a closure.)

## 2. If you called `add_node_with_size`

It is deprecated (removed in 0.11). Size belongs to the node's
content object now — and unlike 0.9, the reserved area actually gets
painted:

```rust
// 0.9: reserve a 12×5 area (rendered as [label] + blank rows)
g.add_node_with_size(10, "Server", 12, 5);
// 0.10: declare content that knows its area and how to fill it
g.add_node(10, CustomNode {
    label: "Server", width: 12, height: 5,
    painter: Some(card),        // fn that draws the area — or None
    payload: "cpu: 4\nram: 16G",
});
```

See [nodes.md](nodes.md) for painters, `BoxedNode`, and blank nodes.

## 3. If you used the render entry points directly

The 0.9 renderers were replaced by one engine. The old names worked
through 0.10.x (deprecated) and are removed in 0.11 — chain with
migrate-from-0.10.md for the current equivalents:

| 0.9 call | 0.10 call |
|---|---|
| `ir.render_scanline()` | `ir.render_string(&RenderOptions::plain())` |
| `ir.render_scanline_colored(p)` | `ir.render_string(&RenderOptions::colored(p))` (set `legend = false` to match) |
| `ir.render_to_buffer(…)` (arena) | `ir.render_to_bytes(&options, &arena, &mut buf)` |
| `estimate_render_size` | `estimate_render_arena_size` + `estimate_render_output_size` |
| `ir.y_index()` / `items_at_line` | `ScenePlanner::new().plan(&ir, &options.plan)` + `scene.hit_test(x, y)` |

**Output note:** the engine canonicalizes a few cases the two 0.9
renderers disagreed on with each other — overlapping edge corners
merge into junctions (`├ ┼ ┬`), nested boxes sharing a border cell
merge, unfittable edge labels are skipped instead of truncated.
Typical graphs are byte-identical.

## 4. If you build IRs or CSR graphs by hand

- `LayoutNode { .. }` literals: add `content_tag: 0` (0 = simple).
- `LayoutIRArenaBuilder::add_node(…)`: new final `content_tag: u8`
  parameter — pass `0`.
- `LayoutIRArenaBuilder::new_with_subgraphs(…)` and
  `CsrGraphBuilder::new`/`new_with_subgraphs`: new trailing
  `max_custom` capacity — pass `0` for label-only graphs. (When you
  *do* declare painters/payloads, size label bytes to include payloads
  and use `required_arena_size_with_content`.)
- `Arena`: `alloc_slice_zeroed`, `alloc_slice_uninit`, `reset`, and
  `restore_position` are `unsafe fn` now, with their caller
  obligations documented. `alloc_slice_default` is the safe path.

## 5. If you `match` exhaustively

`GraphError` and the render vocabulary enums (`Charset`, `ColorMode`,
`LineWeight`, `MarkerShape`, `SubgraphBorder`, `LabelPosition`,
`LabelPlacement`, `HitResult`) are `#[non_exhaustive]`: add a `_` arm
once and future variants stop being breaking changes.

## 6. Worth adopting while you're here

- `add_node(AUTO, …)` + `NodeId` handles — no hand-tracked ids, and
  typo'd ids stop silently creating phantom nodes.
- Node objects — `BoxedNode`, `CustomNode`, your own `NodeContent`
  impls ([nodes.md](nodes.md)).
- `render_with(&options, &mut writer)` for streaming;
  `render_to_bytes` for no-alloc.
- JSON export now carries `"content_kind"` on every node (and
  `"payload"` on custom nodes).
