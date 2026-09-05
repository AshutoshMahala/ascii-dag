# Ports

Which side of a node an edge leaves from, and which side it arrives
on. Undeclared, every edge attaches head-on: it leaves the face the
flow leaves by and arrives on the face the flow arrives on. A port
declaration moves one end of one edge to another side; the layout
routes it there when it can, and says what it did either way.

For the shape of the graph as a whole — direction, spacing, clusters —
see [layout.md](layout.md).

## Declaring a side

`add_edge` returns a handle; declare on it:

```rust
use ascii_dag::{Graph, PortSide};

let mut g = Graph::new();
g.add_node(1usize, "Client");
g.add_node(2usize, "Service");
g.add_node(3usize, "Audit");
g.add_node(4usize, "Store");
g.add_edge(1usize, 2usize, None);
g.add_edge(2usize, 3usize, Some("trail")).from_port(PortSide::Clockwise);
g.add_edge(2usize, 4usize, None).to_port(PortSide::Downstream);
// By index, for a declaration made after insertion:
g.set_edge_ports(0, PortSide::Auto, PortSide::West);
```

`from_port` names the side the edge LEAVES its `from` node from;
`to_port` the side it ARRIVES on at its `to` node. Both refer to the
endpoints as declared: when a cycle forces the layout to draw an edge
reversed, its sides stay with the nodes they were declared on.

On the no-alloc builder the same calls exist on the handle
`CsrGraphBuilder::add_edge` returns, provided the builder was made
with `new_with_ports` (size its arena with
`CsrGraph::required_arena_size_with_ports`). They return `None` from a
builder without a port table, never from memory pressure.
`examples/ports.rs`, its last section, is the working program.

## The sides

Three vocabularies name a side. Pick by what should stay fixed when
the graph is rendered in another direction.

| Side | Meaning | `TopDown` | `BottomUp` | `LeftRight` | `RightLeft` |
|---|---|---|---|---|---|
| `North` `East` `South` `West` | a compass side, fixed on the page | itself | itself | itself | itself |
| `Upstream` | the face the flow arrives on | North | South | West | East |
| `Downstream` | the face the flow leaves by | South | North | East | West |
| `Clockwise` | the traveler's right hand, facing downstream | West | East | South | North |
| `Counterclockwise` | the traveler's left hand, facing downstream | East | West | North | South |
| `Auto` | head-on: leave `Downstream`, arrive `Upstream` | | | | |

The picture behind the flow words: stand in the river facing the way
it runs. `Upstream` is behind you, `Downstream` ahead, `Clockwise` on
your right, `Counterclockwise` on your left. A graph declared in those
words reads the same in `LeftRight` as in `TopDown`; a graph declared
in compass words keeps its page geometry instead. Each word means the
same at either end of an edge: `Upstream` at a source leaves through
the node's arrival face, `Upstream` at a target arrives through it.

`examples/ports.rs` prints this table from the router itself, and
draws the same graph declared each way.

## How a side is drawn

- **Head-on** (`Auto`, or a declared side that is the head-on face
  anyway): the ordinary trunk. A declaration that agrees with the flow
  costs nothing and changes nothing.
- **Against the flow** (`Upstream` at a source, `Downstream` at a
  target): the edge leaves or enters through the face the flow would
  use the other way. It steps out of that face into a lane beside the
  node, runs around it in a band of its own, and rejoins the flow. The
  layout adds rows for the band above the first level or between
  levels as needed.
- **Beside the node** (a compass side across the flow, or a
  rotation): a short leg out of the node's side onto a lane beside
  it, then the ordinary trunk.

A lane is a free cell beside the node in the gap between neighbors.
When there is none — a neighbor at `node_spacing = 0`, an edge span
passing by, a self-loop marker on that row — the end falls back to
its head-on face and the run reports it (below).

## Where on a face: port policies

A node places the ends declared on each of its faces by a policy, a
graph-wide default with a per-node override:

```rust
use ascii_dag::{PortBound, PortPolicy, PortSlot};

g.set_port_policy(PortPolicy::Paired);                              // every node
g.set_node_port_policy(hub, PortPolicy::Spread(PortBound::Ports(3))); // this node
g.set_node_port_policy(card, PortPolicy::Custom(my_placer));        // fn(PortSlot) -> usize
```

| Policy | Ports per face | Where |
|---|---|---|
| `Single` (default) | one | the center; every arrival and departure declared on the face shares it |
| `Paired` | two | the face's primary direction at the center — arrivals on the face the flow arrives on, departures on the face it leaves by, arrivals on a side face — and the other direction on the next cell |
| `Spread(bound)` | up to the bound | spread evenly and centered in tangent order, the peer's position along the face; ends beyond the bound share round-robin. `PortBound::Face` allows as many ports as the face has cells, `Ports(n)` at most `n` |
| `Custom(placer)` | the placer's | a plain `fn(PortSlot) -> usize` returning the cell offset; the slot names the node, the physical face, its cells, its arrivals and departures, the end's index and direction; the answer is clamped to the face |

`Single` is the default because it is the drawing every undeclared
fan-in and fan-out already has: one cell per face, every edge through
it. A declared side on the flow's own face is then exactly `Auto`,
however many edges declare it. Sharing a cell is the ordinary drawing,
never a condition, and a node is never widened for its ports.

A face with one cell holds one port whatever the policy: `Paired`
shares it, `Spread` has one port, `Custom` is clamped to it. So the
side faces of a one-row `[Label]` node always share, while its top and
bottom faces, as wide as the label, take every policy; boxed and
custom nodes have the rows a `Paired` or `Spread` side face needs.
Arrival and departure are the declared ends of an edge, its `to` and
its `from`, which a cycle reversal does not change.

A graph carries one custom placer: a `Custom` policy with a second,
different function is refused (`false`), so one function places every
node, told the node id. `clear_node_port_policy` removes a node's
override so it follows the graph-wide policy again, now and after that
policy changes.

The no-alloc builder has the same setters, `set_port_policy`,
`set_node_port_policy` and `clear_node_port_policy`, all returning
`Option`: a policy byte per node rides the port table, and the builder
carries the one placer. `CsrGraph::compute_layout_arena_reporting`
is the layout entry that replays the port conditions (below) into a
diagnostic context, the twin of the heap graph's reporting run; the
plain `compute_layout_arena` stays quiet.

## Reading attachments back

The layout reports what it did on every edge, declared or not:

```rust
let ir = g.compute_layout();
for e in ir.edges() {
    // `requested` is `Auto` for an undeclared end; `side` is the
    // physical side the end took.
    println!("{} → {}: {} / {}", e.from_id, e.to_id,
             e.from_port.side.name(), e.to_port.side.name());
}
```

`from_port` / `to_port` are `PortAttachment { requested: PortSide,
side: PhysicalSide }` on `LayoutEdge`, `LayoutEdgeArena`, and the
scene's `EdgeView`. JSON (schema 1.5) writes `from_side` / `to_side`
on every edge and `from_port` / `to_port` on the edges that declared
one. A route that leaves a node against the flow or beside it has the
path shape `EdgePath::Orthogonal { bends }`: an explicit polyline,
every turn stated. Hand-built IRs set the attachments themselves;
`PortAttachment::auto(side)` is the value for an undeclared end.

## When a side cannot be honored

Both conditions are warnings on the layout run; the picture still
renders.

| Code | Condition | What was drawn |
|---|---|---|
| `W.Graph.Port.034` | a side declared on a self-loop | the loop marker, as without the declaration |
| `W.Graph.Port.035` | no lane beside the node for the declared side | head-on on that end; the warning names the end, the side asked for and the side used |

```rust
for d in g.layout().reported().warnings() {
    println!("{d}");
}
```

Neither is an error, and sharing a port cell is never a condition.
On the no-alloc pipeline the same two conditions reach a
`DiagnosticContext` through `compute_layout_arena_reporting`.

## Feature and cost

Ports are behind the `ports` feature, on by default. Off, declaring is
unavailable and nothing of the routing is linked; a port-free graph
under the default build pays nothing either — its cells are the 0.10
cells byte for byte. The scratch a layout with ports needs grows with
the ends that route sideways, not with the graph; the arena
estimators account for it. The bare-metal `longan_nano` example
builds without the feature.
