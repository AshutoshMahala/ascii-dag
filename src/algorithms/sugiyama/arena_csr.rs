//! CSR-based arena layout computation.
//!
//! Pure-CSR layout pipeline: avoids all heap allocations and HashMap lookups
//! by operating directly on CSR graph indices.

use super::config::{CycleBreaking, LayoutConfig};
use super::crossing::CrossingReducer;
use crate::errors::GraphError;
use crate::graph::arena::Arena;
use crate::graph::csr::CsrGraph;
use crate::ir::arena::{EdgePathArena, LayoutEdgeArena, LayoutIRArena, LayoutIRArenaBuilder};

use super::geometry::{ARROW_CELL_PAD, Axis, EDGE_START_OFFSET, edge_label_offset};

// ── Packed vnode encoding accessors ──────────────────────────────────────
// `vnode_data` stores two `Idx` per virtual node: `[kind, payload]`.
// kind 0 = Real (payload = node index), kind 1 = Dummy (payload = edge
// index). Always go through these accessors — the packed encoding is an
// implementation detail and must stay changeable in one place.

#[inline(always)]
fn vnode_in_bounds(vnode_data: &[Idx], pos: usize) -> bool {
    pos * 2 + 1 < vnode_data.len()
}

#[inline(always)]
fn vnode_kind(vnode_data: &[Idx], pos: usize) -> Idx {
    vnode_data[pos * 2]
}

#[inline(always)]
fn vnode_is_dummy(vnode_data: &[Idx], pos: usize) -> bool {
    vnode_data[pos * 2] == 1
}

#[inline(always)]
fn vnode_is_real(vnode_data: &[Idx], pos: usize) -> bool {
    vnode_data[pos * 2] == 0
}

#[inline(always)]
fn vnode_payload(vnode_data: &[Idx], pos: usize) -> Idx {
    vnode_data[pos * 2 + 1]
}

#[inline(always)]
fn vnode_set(vnode_data: &mut [Idx], pos: usize, kind: Idx, payload: Idx) {
    vnode_data[pos * 2] = kind;
    vnode_data[pos * 2 + 1] = payload;
}

// Import configurable index types
#[cfg(feature = "arena")]
use super::idx::{Coord, Idx, MAX_NODES};

// Fallback types when arena feature not enabled (for compilation)
#[cfg(not(feature = "arena"))]
type Idx = u32;
#[cfg(not(feature = "arena"))]
type Coord = u16;
/// One §4.7 DP cell: lexicographic cost + predecessor candidate index.
type LaneDpEntry = ((usize, usize, usize, usize), usize);
#[cfg(not(feature = "arena"))]
const MAX_NODES: usize = u32::MAX as usize;

/// Maximum horizontal routing slots per level (caps height on extreme fan-out).
const MAX_SLOTS_PER_LEVEL: usize = 8;

/// Temporary buffers for arena-based layout computation.
///
/// All slices are allocated from a single arena. This struct is used by both
/// the CsrGraph layout path and the Graph→CsrGraph path.
#[allow(dead_code)] // Some fields only used by Graph→CsrGraph path in layout/arena.rs
pub(crate) struct LayoutTemps<'a> {
    pub(crate) node_levels: &'a mut [Idx],
    pub(crate) edge_indices: &'a mut [(Idx, Idx)],
    pub(crate) vlevel_offsets: &'a mut [Idx],
    pub(crate) level_counts: &'a mut [Idx],
    pub(crate) vnode_data: &'a mut [Idx],
    pub(crate) x_coords: &'a mut [Coord],
    pub(crate) widths: &'a mut [Coord],
    pub(crate) real_coords: &'a mut [(usize, usize, usize, usize)],
    pub(crate) dummy_offsets: &'a mut [Idx],
    pub(crate) dummy_data: &'a mut [(Idx, Coord)],
    pub(crate) medians: &'a mut [(Idx, u32)],
    pub(crate) positions: &'a mut [Idx],
    pub(crate) node_is_source: &'a mut [bool],
    pub(crate) source_counts: &'a mut [Idx],
    pub(crate) dummy_counts: &'a mut [Idx],
    pub(crate) level_offsets: &'a mut [usize],
    pub(crate) node_slots: &'a mut [usize],
    pub(crate) level_slot_next: &'a mut [Idx],
    /// Interval pool for slot allocation: (min_x, max_x, next) linked
    /// lists — the same per-slot interval-list structure the heap
    /// allocator uses, so both backends make identical slot decisions.
    pub(crate) slot_pool: &'a mut [(usize, usize, usize)],
    /// Head pool-index per (level, slot); `usize::MAX` = empty.
    pub(crate) slot_heads: &'a mut [usize],
    /// Tail pool-index per (level, slot); `usize::MAX` = empty.
    pub(crate) slot_tails: &'a mut [usize],
    pub(crate) level_dummy_next: &'a mut [Idx],
    /// Per-level flag: 1 if a labeled edge is sourced at this level
    /// (its label row is budgeted only in that level's band).
    pub(crate) level_labeled_src: &'a mut [Idx],
    /// Scratch for 2-node-cycle detection: edge indices sorted by
    /// normalized endpoint pair.
    pub(crate) two_cycle_order: &'a mut [Idx],
    /// Per-edge flag: this edge has an anti-parallel twin with the
    /// opposite back flag (a 2-node cycle).
    pub(crate) edge_in_two_cycle: &'a mut [bool],
    /// Explicit port requests on Auto faces — two per edge is the
    /// bound; EMPTY when the graph declares no port (nothing carved).
    pub(crate) port_requests: &'a mut [super::ports::FaceRequest],
    /// Per-edge positioned `(from, to)` cross lines, `usize::MAX` =
    /// no explicit position; EMPTY when the graph declares no port.
    pub(crate) port_cross: &'a mut [(usize, usize)],
    /// Detour plans `(edge index, plan)` for the detouring edges only,
    /// sorted by edge index; EMPTY when no end detours.
    #[cfg(feature = "ports")]
    pub(crate) detour_plans: &'a mut [(usize, super::ports::Detour)],
    /// Jog bend blocks `(level, bend counter, min, max)` — one per kept
    /// waypoint at most; EMPTY when no end detours. The slot index is
    /// `1 + label row + counter`, applied at allocation.
    #[cfg(feature = "ports")]
    pub(crate) jog_blocks: &'a mut [(usize, usize, usize, usize)],
    /// Lane blockers `(level, lo, hi, kind)`, inclusive, on the
    /// detouring nodes' levels — node spans and dummy columns (kind 0,
    /// block every row) and self-loop marker cells (kind 1, block the
    /// top row only) — sorted and merged per level; EMPTY when no end
    /// detours.
    #[cfg(feature = "ports")]
    pub(crate) lane_blockers: &'a mut [(usize, usize, usize, usize)],
    pub(crate) waypoint_scratch: &'a mut [(usize, usize)],
    // ── Lane pass (temp/09 P4). All empty when the shared budget
    //    (`geometry::lane_pass_enabled`) disables the pass — the heap
    //    backend evaluates the same predicate, so the two cannot
    //    disagree about whether lanes run. ──
    /// Fixed-claim CSR offsets per gap (`n_gaps + 1`).
    pub(crate) lane_fixed_offsets: &'a mut [usize],
    /// Fixed claims: adjacent-level real-to-real edge sweeps, by gap.
    pub(crate) lane_fixed: &'a mut [crate::algorithms::sugiyama::geometry::GapClaim],
    /// Committed-claim CSR offsets per gap (`n_gaps + 1`).
    pub(crate) lane_committed_offsets: &'a mut [usize],
    /// Per-gap write cursors for committed claims (`n_gaps`).
    pub(crate) lane_cursors: &'a mut [usize],
    /// Committed chain-segment claims, grouped by gap (`D + C` total).
    pub(crate) lane_committed: &'a mut [crate::algorithms::sugiyama::geometry::GapClaim],
    /// Chain sort keys `(t_level, span, edge)`.
    pub(crate) lane_chains: &'a mut [(usize, usize, usize)],
    /// Span scratch: union region + per-gap region (2 × span budget).
    pub(crate) lane_spans: &'a mut [crate::algorithms::sugiyama::geometry::CrossSpan],
    /// DP candidate coordinates, all interior levels of one chain.
    pub(crate) lane_cands: &'a mut [usize],
    /// Candidate CSR offsets per interior level (`max_levels + 1`).
    pub(crate) lane_cand_offsets: &'a mut [usize],
    /// DP rows parallel to `lane_cands`: (cost, predecessor index).
    pub(crate) lane_dp: &'a mut [LaneDpEntry],
    pub(crate) level_vdummy_counts: &'a mut [Idx],

    // ── Edge routing Y tracking ──────────────────────────────────────
    /// Per-level routing floor: max Y used by edge routing at each level.
    /// Bottom borders of subgraphs closing at level L must be placed BELOW this floor.
    pub(crate) level_routing_floor: &'a mut [usize],
    /// Per-level max node LEVEL extent. Geometry stays `usize` — the
    /// configurable index type must never hold extents (a 256-wide LR
    /// node would wrap to 0 under `arena-idx-u8`).
    pub(crate) level_max_extents: &'a mut [usize],

    // ── Subgraph temporaries ─────────────────────────────────────────
    /// Per-subgraph (first_level, last_level) range; usize::MAX = unset
    pub(crate) sg_ranges: &'a mut [(usize, usize)],
    /// Per-subgraph nesting depth
    pub(crate) sg_depths: &'a mut [usize],
    /// Per-subgraph bounding box: (min_x, min_y, max_x, max_y)
    pub(crate) sg_envelopes: &'a mut [(usize, usize, usize, usize)],
    /// Per-level boundary extras for subgraph borders
    pub(crate) sg_y_extras: &'a mut [usize],
    /// Per-level frontier scratch for cluster passes (depth-sized;
    /// empty when the graph has no subgraphs)
    pub(crate) sg_frontier_a: &'a mut [usize],
    /// Second per-level frontier scratch for cluster passes
    pub(crate) sg_frontier_b: &'a mut [usize],
}

// ── Subgraph layout constants ────────────────────────────────────────────
/// Append an interval to a (level, slot) linked list in the slot pool.
/// Full pool degrades by dropping the interval (allocation still works;
/// only sharing precision is lost — same spirit as the slot cap).
#[inline]
fn slot_push(
    pool: &mut [(usize, usize, usize)],
    pool_len: &mut usize,
    heads: &mut [usize],
    tails: &mut [usize],
    base: usize,
    min_x: usize,
    max_x: usize,
) {
    if *pool_len >= pool.len() || base >= heads.len() {
        return;
    }
    let idx = *pool_len;
    pool[idx] = (min_x, max_x, usize::MAX);
    if tails[base] == usize::MAX {
        heads[base] = idx;
    } else {
        pool[tails[base]].2 = idx;
    }
    tails[base] = idx;
    *pool_len += 1;
}

/// Allocate a routing-row slot at `lvl` for the exact interval
/// `[min_x, max_x]`: the first slot whose list does not collide, a new
/// slot under the per-level cap, else slot 0 — the heap backend's
/// `alloc_slot`, over the pool.
#[cfg_attr(not(feature = "ports"), allow(dead_code))]
#[allow(clippy::too_many_arguments)]
fn alloc_slot_csr(
    pool: &mut [(usize, usize, usize)],
    pool_len: &mut usize,
    heads: &mut [usize],
    tails: &mut [usize],
    next: &mut [Idx],
    jogs: &[(usize, usize, usize, usize)],
    labeled: &[Idx],
    lvl: usize,
    min_x: usize,
    max_x: usize,
) -> usize {
    let used = next.get(lvl).map_or(0, |&n| n as usize);
    let label_row = usize::from(labeled.get(lvl).is_some_and(|&l| l != 0));
    let in_jog = |s: usize| {
        jogs.iter().any(|&(l, k, a, b)| {
            l == lvl && 1 + label_row + k == s && slot_hits(a, b, min_x, max_x)
        })
    };
    for s in 0..MAX_SLOTS_PER_LEVEL {
        let base = lvl * MAX_SLOTS_PER_LEVEL + s;
        let in_slot = s < used && slot_collides(pool, heads, base, min_x, max_x);
        if in_slot || in_jog(s) {
            continue;
        }
        slot_push(pool, pool_len, heads, tails, base, min_x, max_x);
        if let Some(n) = next.get_mut(lvl) {
            *n = (*n).max(s as Idx + 1);
        }
        return s;
    }
    {
        slot_push(
            pool,
            pool_len,
            heads,
            tails,
            lvl * MAX_SLOTS_PER_LEVEL,
            min_x,
            max_x,
        );
        0
    }
}

/// Does a registered `[s, e]` collide with a request `[min_x, max_x]`?
/// A run (with length) collides on a shared column; a point only when
/// strictly inside — the heap `alloc_slot`'s test.
#[inline]
fn slot_hits(s: usize, e: usize, min_x: usize, max_x: usize) -> bool {
    if s == e {
        s < max_x && e > min_x
    } else {
        s <= max_x && e >= min_x
    }
}

/// Does `[min_x, max_x]` collide with any interval in the (level, slot)
/// list, by [`slot_hits`]?
#[inline]
fn slot_collides(
    pool: &[(usize, usize, usize)],
    heads: &[usize],
    base: usize,
    min_x: usize,
    max_x: usize,
) -> bool {
    if base >= heads.len() {
        return false;
    }
    let mut cur = heads[base];
    while cur != usize::MAX {
        let (s0, e0, next) = pool[cur];
        if slot_hits(s0, e0, min_x, max_x) {
            return true;
        }
        cur = next;
    }
    false
}

/// Compute layout using arena allocation for temporaries, specialized for CsrGraph.
///
/// This avoids all heap allocations and HashMap lookups by using the CSR indices directly.
/// The `config` parameter controls the layout pipeline (crossing reduction, spacing, etc.).
///
/// Dispatches on the direction exactly as the heap backend does:
/// `LeftRight`/`RightLeft` lay out through `Horizontal`, everything
/// else through `Vertical`. The two backends must agree — the parity
/// rule is not optional.
pub fn compute_layout_arena_csr<'b>(
    graph: &CsrGraph<'_>,
    config: &LayoutConfig<'_>,
    temp_arena: &mut Arena<'_>,
    output_arena: &'b mut Arena<'b>,
) -> Result<LayoutIRArena<'b>, GraphError> {
    match config.direction {
        #[cfg(feature = "layout-horizontal")]
        crate::graph::Direction::LeftRight | crate::graph::Direction::RightLeft => {
            compute_layout_arena_csr_axis::<super::geometry::Horizontal>(
                graph,
                config,
                temp_arena,
                output_arena,
            )
        }
        #[cfg(feature = "layout-vertical")]
        _ => compute_layout_arena_csr_axis::<super::geometry::Vertical>(
            graph,
            config,
            temp_arena,
            output_arena,
        ),
    }
}

/// Axis-parameterized CSR layout (temp/08 D1): one pipeline computing
/// in role space, with `A` naming which physical axis each role maps
/// to. The public wrapper picks the profile from the direction.
pub(crate) fn compute_layout_arena_csr_axis<'b, A: Axis>(
    graph: &CsrGraph<'_>,
    config: &LayoutConfig<'_>,
    temp_arena: &mut Arena<'_>,
    output_arena: &'b mut Arena<'b>,
) -> Result<LayoutIRArena<'b>, GraphError> {
    let node_count = graph.node_count();
    let edge_count = graph.edge_count();

    // Validate against index type limits
    let max_count = node_count.max(edge_count);
    if max_count > MAX_NODES {
        return Err(GraphError::ExceedsMaxNodes {
            count: max_count,
            max: MAX_NODES,
        });
    }

    // Calculate total label bytes (node + edge labels, iterating CSR is cheap)
    let mut total_label_bytes = 0;
    for i in 0..node_count {
        total_label_bytes += graph.node_label(i).len();
    }
    let has_labeled_edges = graph.has_edge_labels();
    if has_labeled_edges {
        for i in 0..edge_count {
            total_label_bytes += graph.edge_label(i).len();
        }
    }
    // Self-loops leave the routed list but survive as records; count
    // them once so the builder carves exactly that many.
    let self_loop_count = graph.edges_iter().filter(|&(from, to)| from == to).count();

    // Step 1: Cycle breaking — allocate back_edges and run DFS before other temps
    let back_edges = {
        let be_size = edge_count.max(1);
        let (be_ptr, _) = temp_arena
            .alloc_raw::<bool>(be_size)
            .ok_or(GraphError::ArenaOom)?;
        // SAFETY: alloc_raw zeroes memory, so all false
        unsafe { core::slice::from_raw_parts_mut(be_ptr, be_size) }
    };
    match config.cycle_breaking() {
        CycleBreaking::DepthFirst => {
            detect_back_edges_csr(graph, back_edges, temp_arena);
        }
        CycleBreaking::None => {} // already all-false from alloc_raw
    }

    // Step 2: Calculate levels (back edges have direction flipped).
    // This needs only the per-node level array, so it runs BEFORE the
    // main temps allocation — the per-level buffers are then sized from
    // the graph's true depth instead of a fixed cap (heap parity: that
    // backend has never had a depth cap).
    let node_levels = {
        let (ptr, _) = temp_arena
            .alloc_raw_uninit::<Idx>(node_count)
            .ok_or(GraphError::ArenaOom)?;
        // SAFETY: freshly allocated for node_count entries; initialized
        // by calculate_levels_csr before any read.
        unsafe { core::slice::from_raw_parts_mut(ptr, node_count) }
    };
    let max_level = calculate_levels_csr(graph, node_levels, back_edges);
    let depth = (max_level as usize).saturating_add(1);

    // A DAG's depth cannot exceed its node count; a deeper result means
    // an unbroken cycle pumped the relaxation (CycleBreaking::None).
    // Reject cleanly — sizing buffers from a saturated depth would ask
    // the arena for nonsense.
    if depth > node_count.max(1) {
        return Err(GraphError::ExceedsMaxLevels {
            depth,
            max: node_count,
        });
    }

    // Exact routing capacities (no silent caps): every skip-level edge
    // spanning k levels contributes k-1 dummies, one per intermediate
    // level. Buffers size from these counts; graphs whose virtual-node
    // total exceeds the index type's capacity fail explicitly.
    let (level_real, level_dummy) = {
        let real = temp_arena
            .alloc_slice_default::<usize>(depth.max(1))
            .ok_or(GraphError::ArenaOom)?;
        let dummy = temp_arena
            .alloc_slice_default::<usize>(depth.max(1))
            .ok_or(GraphError::ArenaOom)?;
        (real, dummy)
    };
    for &lvl in node_levels.iter() {
        if (lvl as usize) < level_real.len() {
            level_real[lvl as usize] += 1;
        }
    }
    let mut total_dummies: usize = 0;
    for (ei, (f, t)) in graph.edges_iter().enumerate() {
        if f == t {
            continue;
        }
        let _ = ei;
        let lf = node_levels[f] as usize;
        let lt = node_levels[t] as usize;
        let (lo, hi) = (lf.min(lt), lf.max(lt));
        if hi > lo + 1 {
            total_dummies += hi - lo - 1;
            for slot in level_dummy.iter_mut().take(hi).skip(lo + 1) {
                *slot += 1;
            }
        }
    }
    let vnode_total = node_count
        .checked_add(total_dummies)
        .ok_or(GraphError::ExceedsMaxNodes {
            count: usize::MAX,
            max: MAX_NODES,
        })?;
    if vnode_total > MAX_NODES {
        return Err(GraphError::ExceedsMaxNodes {
            count: vnode_total,
            max: MAX_NODES,
        });
    }
    let max_level_width = level_real
        .iter()
        .zip(level_dummy.iter())
        .map(|(r, d)| r + d)
        .max()
        .unwrap_or(0);

    // Step 3: Allocate the remaining layout temporaries, per-level
    // buffers sized to the real depth and exact vnode counts.
    let sg_count = graph.subgraph_count();
    // Port scratch only for a graph that declared ports: two request
    // slots per edge (the bound) and one position pair per edge.
    #[cfg(feature = "ports")]
    let (port_request_cap, port_cross_len) = if graph.has_ports() {
        (edge_count.saturating_mul(2), edge_count)
    } else {
        (0, 0)
    };
    #[cfg(not(feature = "ports"))]
    let (port_request_cap, port_cross_len) = (0usize, 0usize);
    // The detour budget — which edges, nodes, ends and levels the
    // detour pass touches — decided here from the same face rule the
    // pass applies, so every detour table is sized by what detours, not
    // by the graph. A declared port on its role's own face costs
    // nothing. The marks feed the sparse tables below.
    #[cfg(feature = "ports")]
    let (budget, _node_marks, level_marks) = if graph.has_ports() {
        let node_marks = temp_arena
            .alloc_slice_default::<bool>(node_count.max(1))
            .ok_or(GraphError::ArenaOom)?;
        let level_marks = temp_arena
            .alloc_slice_default::<bool>(depth.max(1))
            .ok_or(GraphError::ArenaOom)?;
        let flipped = super::ports::level_flipped::<A>(config.direction);
        let budget = super::ports::detour_budget(
            edge_count,
            &|ei| graph.edge(ei),
            &|ei| graph.edge_ports(ei),
            &|ei| back_edges.get(ei).copied().unwrap_or(false),
            &|n| node_levels[n] as usize,
            A::FLOW_AXIS,
            flipped,
            level_real,
            level_dummy,
            node_marks,
            level_marks,
        );
        (budget, node_marks, &*level_marks)
    } else {
        (super::ports::DetourBudget::NONE, &mut [][..], &[][..])
    };
    #[cfg(not(feature = "ports"))]
    let budget = super::ports::DetourBudget::NONE;
    let mut temps = alloc_layout_temps_csr(
        temp_arena,
        node_count,
        edge_count,
        sg_count,
        node_levels,
        depth,
        total_dummies,
        max_level_width,
        port_request_cap,
        port_cross_len,
        budget,
    )
    .ok_or(GraphError::ArenaOom)?;

    // 2-node-cycle detection in O(E log E): sort edge indices by their
    // normalized endpoint pair, then scan each run for an anti-parallel
    // twin with the opposite back flag. Replaces an O(E) scan per
    // straight edge in emission (O(E²) worst case — ~6 s of the 50k
    // diamond's layout time). Mirrors the heap backend exactly.
    {
        temps.edge_in_two_cycle.fill(false);
        let pair_key = |ei: usize| {
            let (f, t) = graph.edge(ei);
            if f <= t { (f, t) } else { (t, f) }
        };
        for (i, slot) in temps.two_cycle_order.iter_mut().enumerate() {
            *slot = i as Idx;
        }
        temps
            .two_cycle_order
            .sort_unstable_by_key(|&ei| pair_key(ei as usize));

        let order = &*temps.two_cycle_order;
        let mut run_start = 0;
        while run_start < order.len() {
            let mut run_end = run_start + 1;
            while run_end < order.len()
                && pair_key(order[run_end] as usize) == pair_key(order[run_start] as usize)
            {
                run_end += 1;
            }
            let mut counts = [[0usize; 2]; 2];
            for &ei in &order[run_start..run_end] {
                let (f, t) = graph.edge(ei as usize);
                if f == t {
                    continue; // self-loop
                }
                let dir = usize::from(f > t);
                let back = usize::from(back_edges.get(ei as usize).copied().unwrap_or(false));
                counts[dir][back] += 1;
            }
            for &ei in &order[run_start..run_end] {
                let (f, t) = graph.edge(ei as usize);
                if f == t {
                    continue;
                }
                let dir = usize::from(f > t);
                let back = usize::from(back_edges.get(ei as usize).copied().unwrap_or(false));
                if counts[1 - dir][1 - back] > 0 {
                    temps.edge_in_two_cycle[ei as usize] = true;
                }
            }
            run_start = run_end;
        }
    }

    // Step 4: Build virtual levels (back edges have direction flipped)
    let (_vnode_count, _max_level_size) = build_virtual_levels_csr(
        graph,
        temps.node_levels,
        temps.vlevel_offsets,
        temps.level_counts,
        temps.vnode_data,
        max_level,
        back_edges,
    );

    // Populate edge_indices for crossing reduction
    for (i, (from, to)) in graph.edges_iter().enumerate() {
        if i < temps.edge_indices.len() {
            temps.edge_indices[i] = (from as Idx, to as Idx);
        }
    }

    // Populate level_vdummy_counts for crossing reduction
    temps.level_vdummy_counts.fill(0);
    for level in 0..=(max_level as usize) {
        if level + 1 >= temps.vlevel_offsets.len() {
            break;
        }
        let start = temps.vlevel_offsets[level] as usize;
        let end = temps.vlevel_offsets[level + 1] as usize;
        for pos in start..end {
            if vnode_in_bounds(temps.vnode_data, pos) && vnode_is_dummy(temps.vnode_data, pos) {
                if level < temps.level_vdummy_counts.len() {
                    temps.level_vdummy_counts[level] += 1;
                }
            }
        }
    }

    // Step 4: Crossing reduction
    reduce_crossings_csr(
        graph,
        config.crossing_pipeline(),
        temps.vlevel_offsets,
        temps.vnode_data,
        max_level as usize,
        temps.medians,
        temps.positions,
        temps.edge_indices,
        temps.level_vdummy_counts,
    );

    // Step 4b: Block-partitioned level ordering for subgraph adjacency
    if graph.has_subgraphs() {
        block_partition_levels_csr(
            graph,
            temps.vlevel_offsets,
            temps.vnode_data,
            max_level as usize,
        );
    }

    // Step 5: Assign x-coordinates
    let node_spacing: Coord = config.node_spacing.min(Coord::MAX as usize) as Coord;
    let node_spacing_usize: usize = config.node_spacing;
    assign_x_coords_csr::<A>(
        graph,
        temps.vlevel_offsets,
        temps.vnode_data,
        temps.x_coords,
        temps.widths,
        max_level,
        node_spacing,
    );

    // Step 5b: Subgraph horizontal padding
    if graph.has_subgraphs() {
        subgraph_padding_csr::<A>(
            graph,
            temps.vlevel_offsets,
            temps.vnode_data,
            temps.x_coords,
            temps.widths,
            max_level,
            node_spacing_usize,
        );
    }

    // Step 5c: Refine x-coordinates and compact subgraphs (iterative).
    // x-refinement is only beneficial for subgraph layouts; skipping it for
    // plain graphs avoids an O(N²/L) cost on large inputs.
    if graph.has_subgraphs() {
        let compact_rounds = 3;
        for _ in 0..compact_rounds {
            refine_x_positions_csr::<A>(
                graph,
                temps.vlevel_offsets,
                temps.vnode_data,
                temps.x_coords,
                temps.widths,
                max_level as usize,
                node_spacing,
            );
            compact_subgraphs_csr::<A>(
                graph,
                temps.vlevel_offsets,
                temps.vnode_data,
                temps.x_coords,
                temps.widths,
                max_level as usize,
                node_spacing,
            );
        }
    }

    // Compute max_width after refinement + compaction
    let mut max_width: Coord = {
        let mut new_max: Coord = 0;
        for level in 0..=max_level as usize {
            if level + 1 >= temps.vlevel_offsets.len() {
                break;
            }
            let start = temps.vlevel_offsets[level] as usize;
            let end = temps.vlevel_offsets[level + 1] as usize;
            for pos in start..end {
                let right = temps.x_coords[pos].saturating_add(temps.widths[pos]);
                if right > new_max {
                    new_max = right;
                }
            }
        }
        // Cross-axis safety margin — profile-decided (see the heap twin).
        new_max.saturating_add(
            A::cross_margin(graph.has_edge_labels(), graph.has_subgraphs()) as Coord,
        )
    };

    // Step 6: Build real node coordinates
    build_real_coords_csr(
        temps.vlevel_offsets,
        temps.vnode_data,
        temps.x_coords,
        temps.widths,
        temps.real_coords,
        max_level,
        max_width,
        !graph.has_subgraphs(), // skip per-level centering for subgraph layouts
    );

    // Step 6b: Fix sibling subgraph overlaps introduced by centering
    if graph.has_subgraphs() {
        let extra = fix_subgraph_overlaps_csr::<A>(
            graph,
            temps.real_coords,
            temps.sg_envelopes,
            temps.sg_depths,
            temps.node_slots,
        );
        max_width = max_width.saturating_add(extra as Coord);

        // Reclaim slack the sibling shifts left behind: pull nodes toward
        // their connected neighbors within current level bounds.

        tighten_levels_csr::<A>(
            graph,
            temps.real_coords,
            max_level as usize,
            node_spacing_usize,
            temps.positions,
        );

        // Step 6c: Cluster-width feedback — push unaffiliated nodes clear
        // of each cluster's projected border envelope (cross-level extent
        // + label minimum). Runs after overlap repair so it sees the
        // coordinates the bounding boxes will actually be computed from.
        let pushed = clear_external_overlaps_csr::<A>(
            graph,
            temps.real_coords,
            max_level as usize,
            node_spacing_usize,
            temps.sg_envelopes,
            temps.sg_depths,
            temps.positions,
            temps.sg_frontier_a,
            temps.sg_frontier_b,
        );

        max_width = max_width.saturating_add(pushed as Coord);

        // Pull whole root clusters (and loose nodes) back together after
        // the overlap shifts — reclaims the empty gulfs between boxes.
        let reclaimed = compact_clusters_csr::<A>(
            graph,
            temps.real_coords,
            max_level as usize,
            node_spacing_usize,
            temps.sg_envelopes,
            temps.sg_depths,
            temps.vlevel_offsets,
            temps.vnode_data,
            temps.x_coords,
            temps.sg_frontier_a,
            temps.sg_frontier_b,
        );

        max_width = max_width.saturating_sub(reclaimed as Coord);

        // Last-resort overlap repair (mirrors the heap backend): none of
        // the passes above moves a node with no edges, so compaction
        // clamps can survive to here as overlapping cluster members.
        // Layouts with neither a node overlap nor a leading-pad
        // violation pass through unchanged. Runs BEFORE dummy clearance
        // so waypoints are nudged off the final node positions.
        let widened = repair_level_overlaps_csr::<A>(
            graph,
            temps.real_coords,
            node_spacing_usize,
            temps.positions,
        );
        max_width = max_width.saturating_add(widened as Coord);
        // Waypoints must never cross node text (crossing a border renders
        // as a junction and is acceptable; crossing a node is not).
        nudge_dummies_off_nodes_csr::<A>(
            graph,
            temps.real_coords,
            temps.vlevel_offsets,
            temps.vnode_data,
            temps.x_coords,
            max_level as usize,
        );
    }

    // Step 7: Build dummy positions using actual virtual level positions
    build_dummy_positions_csr::<A>(
        graph,
        temps.vlevel_offsets,
        temps.vnode_data,
        temps.x_coords,
        temps.widths,
        temps.dummy_offsets,
        temps.dummy_data,
        max_level,
        max_width,
        !graph.has_subgraphs(), // skip centering for subgraph layouts (match build_real_coords)
    );

    // A leading-side lateral lane needs a cell BEFORE the node on the
    // cross axis; a node packed at cross 0 has none. One leading cross
    // cell is opened for the whole layout when any end declares such a
    // face (matches heap; zero for every other layout).
    #[allow(unused_mut)] // set only by the ports pass
    let mut cross_extra: Coord = 0;
    #[cfg(feature = "ports")]
    if graph.has_ports() {
        use super::ports::{EndRole, Face};
        let flipped = super::ports::level_flipped::<A>(config.direction);
        for (ei, (from_idx, to_idx)) in graph.edges_iter().enumerate() {
            if from_idx == to_idx {
                continue;
            }
            let is_back = back_edges.get(ei).copied().unwrap_or(false);
            let (src_side, dst_side) = graph.edge_ports(ei);
            let (src_side, dst_side) = if is_back {
                (dst_side, src_side)
            } else {
                (src_side, dst_side)
            };
            if matches!(
                Face::of(src_side, A::FLOW_AXIS, flipped, EndRole::Source),
                Face::CrossLeading
            ) || matches!(
                Face::of(dst_side, A::FLOW_AXIS, flipped, EndRole::Target),
                Face::CrossLeading
            ) {
                cross_extra = 1;
                break;
            }
        }
    }
    if cross_extra > 0 {
        // The opened cell must stay representable everywhere it shifts
        // — every node, every dummy, the canvas — or the layout fails
        // cleanly instead of wrapping.
        let dummy_total =
            temps.dummy_offsets[edge_count.min(temps.dummy_offsets.len() - 1)] as usize;
        let widest = temps
            .real_coords
            .iter()
            .map(|c| c.2)
            .chain(temps.dummy_data[..dummy_total].iter().map(|d| d.1 as usize))
            .chain(core::iter::once(max_width as usize))
            .max()
            .unwrap_or(0);
        let extent = widest.saturating_add(cross_extra as usize);
        if extent > Coord::MAX as usize {
            return Err(GraphError::ExceedsMaxExtent {
                extent,
                max: Coord::MAX as usize,
            });
        }
        for coords in temps.real_coords.iter_mut() {
            coords.2 += cross_extra as usize;
        }
        for entry in temps.dummy_data[..dummy_total].iter_mut() {
            entry.1 += cross_extra;
        }
    }

    // temp/09 P4: chain-lane allocation, mirror of the heap pass (§4).
    // Envelopes are re-projected first so obstacles reflect the final
    // coordinates; the canvas is widened by the reach so the flip sees
    // the same width (§4.8).
    {
        let sg_count = graph.subgraph_count();
        if sg_count > 0 {
            let max_depth = temps.sg_depths[..sg_count]
                .iter()
                .copied()
                .max()
                .unwrap_or(0);
            project_sg_envelopes_csr::<A>(
                graph,
                temps.real_coords,
                graph.node_count(),
                sg_count,
                max_depth,
                temps.sg_envelopes,
                temps.sg_depths,
            );
        }
        let lane_reach =
            allocate_chain_lanes_csr::<A>(graph, back_edges, max_level as usize + 1, &mut temps);
        max_width = max_width
            .saturating_add(cross_extra)
            .max(lane_reach.min(Coord::MAX as usize) as Coord);
    }

    // Explicit ports on a LEVEL face get POSITIONS along it — the
    // layout role's own Auto face and its opposite alike (opposite
    // ends detour around their node below) — the same slice-based
    // pass as the heap backend, run on carved scratch; skipped
    // outright (and nothing carved) when the graph declares no port.
    #[cfg(feature = "ports")]
    let level_flipped = super::ports::level_flipped::<A>(config.direction);
    #[cfg(feature = "ports")]
    if graph.has_ports() {
        use super::ports::{EndRole, Face, FaceRequest, PortSide, assign_level_face_positions};
        let real_coords: &[(usize, usize, usize, usize)] = &*temps.real_coords;
        let port_cross: &mut [(usize, usize)] = &mut *temps.port_cross;
        let port_requests: &mut [FaceRequest] = &mut *temps.port_requests;
        port_cross.fill((usize::MAX, usize::MAX));
        let cross_span = |idx: usize| -> (usize, usize) {
            let (_, _, base, _) = real_coords[idx];
            (
                base,
                A::cross_extent(graph.node_width(idx), graph.node_height(idx)),
            )
        };
        let mut count = 0usize;
        for (edge_idx, (from_idx, to_idx)) in graph.edges_iter().enumerate() {
            if from_idx == to_idx {
                continue;
            }
            let is_back = back_edges.get(edge_idx).copied().unwrap_or(false);
            let (src, dst) = if is_back {
                (to_idx, from_idx)
            } else {
                (from_idx, to_idx)
            };
            let (src_side, dst_side) = graph.edge_ports(edge_idx);
            let (src_side, dst_side) = if is_back {
                (dst_side, src_side)
            } else {
                (src_side, dst_side)
            };
            for (node, peer, side, end, arrival) in [
                (src, dst, src_side, EndRole::Source, is_back),
                (dst, src, dst_side, EndRole::Target, !is_back),
            ] {
                if matches!(side, PortSide::Auto) {
                    continue;
                }
                let face = Face::of(side, A::FLOW_AXIS, level_flipped, end);
                if !face.is_level() {
                    continue;
                }
                let (peer_base, peer_extent) = cross_span(peer);
                port_requests[count] = FaceRequest {
                    node,
                    face,
                    key: A::cross_center(peer_base, peer_extent),
                    edge: edge_idx,
                    end,
                    arrival,
                };
                count += 1;
            }
        }
        // The node's policy: its override, else the graph's — and the
        // node's id, which a custom placer is told.
        let policy = |idx: usize| -> (super::ports::PortPolicy, usize) {
            (graph.node_port_policy(idx), graph.node_id(idx))
        };
        // `port_cross` holds each end's position ALONG its face: the
        // cross line on a level face, the row offset on a lateral one.
        assign_level_face_positions::<A>(
            &mut port_requests[..count],
            cross_span,
            policy,
            level_flipped,
            |edge, end, cross| match end {
                EndRole::Source => port_cross[edge].0 = cross,
                EndRole::Target => port_cross[edge].1 = cross,
            },
        );
        // Lateral requests, on the same scratch: spread along the LEVEL
        // axis, keyed by the peer's level.
        let mut count = 0usize;
        for (edge_idx, (from_idx, to_idx)) in graph.edges_iter().enumerate() {
            if from_idx == to_idx {
                continue;
            }
            let is_back = back_edges.get(edge_idx).copied().unwrap_or(false);
            let (src, dst) = if is_back {
                (to_idx, from_idx)
            } else {
                (from_idx, to_idx)
            };
            let (src_side, dst_side) = graph.edge_ports(edge_idx);
            let (src_side, dst_side) = if is_back {
                (dst_side, src_side)
            } else {
                (src_side, dst_side)
            };
            for (node, peer, side, end, arrival) in [
                (src, dst, src_side, EndRole::Source, is_back),
                (dst, src, dst_side, EndRole::Target, !is_back),
            ] {
                if matches!(side, PortSide::Auto) {
                    continue;
                }
                let face = Face::of(side, A::FLOW_AXIS, level_flipped, end);
                if face.is_level() || count >= port_requests.len() {
                    continue;
                }
                port_requests[count] = FaceRequest {
                    node,
                    face,
                    key: real_coords[peer].0,
                    edge: edge_idx,
                    end,
                    arrival,
                };
                count += 1;
            }
        }
        super::ports::assign_cross_face_positions::<A>(
            &mut port_requests[..count],
            |idx| {
                (
                    0,
                    A::level_extent(graph.node_width(idx), graph.node_height(idx)),
                )
            },
            policy,
            level_flipped,
            |edge, end, along| match end {
                EndRole::Source => port_cross[edge].0 = along,
                EndRole::Target => port_cross[edge].1 = along,
            },
        );
    }

    // Step 8: Geometry-aware horizontal slot allocation for edge separation
    // Assigns horizontal routing slots to non-vertical source nodes so that
    // edges whose horizontal spans don't overlap can share the same slot row.
    // This matches the heap path's interval-based slot allocator.
    let alloc_size = max_level as usize + 1;

    // 1. Initialize geometry-aware slot tracking. Each (level, slot)
    // holds a linked interval list in the pool — the heap allocator's
    // structure, so both backends make identical placement decisions.
    temps.node_slots.fill(usize::MAX); // usize::MAX = unassigned sentinel
    temps.level_slot_next.fill(0);
    temps.slot_heads.fill(usize::MAX);
    temps.slot_tails.fill(usize::MAX);
    let mut slot_pool_len = 0usize;

    // Arrow-cell reservation: a reversed edge paints ⇡ on the first
    // routing row, directly below its layout-source. Pre-occupy that
    // cell (± ARROW_CELL_PAD) on slot 0 so any horizontal span that
    // would run through the arrowhead is pushed to a deeper slot by
    // the collision scan below. Mirrored in the heap allocator — the
    // two must not drift.
    for (ei, (from_idx, to_idx)) in graph.edges_iter().enumerate() {
        if from_idx == to_idx || !back_edges.get(ei).copied().unwrap_or(false) {
            continue;
        }
        let src_idx = to_idx; // back edge: layout flow is to → from
        if src_idx >= temps.real_coords.len() {
            continue;
        }
        let (src_level, _, x, w) = temps.real_coords[src_idx];
        let lvl = src_level;
        if lvl >= alloc_size {
            continue;
        }
        let ax = x + w / 2;
        if temps.level_slot_next[lvl] == 0 {
            temps.level_slot_next[lvl] = 1;
        }
        slot_push(
            temps.slot_pool,
            &mut slot_pool_len,
            temps.slot_heads,
            temps.slot_tails,
            lvl * MAX_SLOTS_PER_LEVEL,
            ax.saturating_sub(ARROW_CELL_PAD),
            ax + ARROW_CELL_PAD,
        );
    }

    // Detour ends: an explicit side on the level face OPPOSITE the
    // layout role's own, or on a lateral face. Sparse throughout — plans
    // for the detouring edges only (sorted by edge index, looked up by
    // binary search), a flag table for the detouring nodes only, lane
    // blockers on their levels only, head-on records for the ends at
    // those nodes only — all sized by the budget computed before the
    // carve. Decided in the order the facts allow: faces, lanes (an end
    // without one attaches head-on after all), the EFFECTIVE occupancy,
    // the lateral and level-face conflicts, the arrow-cell
    // reservations. Mirrors the heap backend decision for decision.
    #[cfg(feature = "ports")]
    if budget.any() {
        use super::ports::{Detour, EndRole, Face, choose_lane, detours, lateral_lane};
        let real_coords: &[(usize, usize, usize, usize)] = &*temps.real_coords;
        let dummy_data: &[(Idx, Coord)] = &*temps.dummy_data;
        let dummy_total =
            temps.dummy_offsets[edge_count.min(temps.dummy_offsets.len() - 1)] as usize;
        let port_cross: &mut [(usize, usize)] = &mut *temps.port_cross;
        let extent = |idx: usize| A::cross_extent(graph.node_width(idx), graph.node_height(idx));
        let level_extent =
            |idx: usize| A::level_extent(graph.node_width(idx), graph.node_height(idx));
        let center = |idx: usize| {
            let (_, _, base, _) = real_coords[idx];
            A::cross_center(base, extent(idx))
        };
        let on_marked_level = |l: usize| level_marks.get(l).copied().unwrap_or(false);
        let resolved = |port_cross: &[(usize, usize)], ei: usize, idx: usize, target: bool| {
            let positioned = port_cross
                .get(ei)
                .map_or(usize::MAX, |p| if target { p.1 } else { p.0 });
            if positioned != usize::MAX {
                positioned
            } else {
                center(idx)
            }
        };
        let side_row = |port_cross: &[(usize, usize)], ei: usize, idx: usize, target: bool| {
            let along = port_cross
                .get(ei)
                .map_or(usize::MAX, |p| if target { p.1 } else { p.0 });
            if along == usize::MAX {
                A::level_center(0, level_extent(idx))
            } else {
                along
            }
        };
        let layout_ends = |ei: usize| -> Option<(usize, usize)> {
            let (from_idx, to_idx) = graph.edge(ei);
            if from_idx == to_idx {
                return None;
            }
            Some(if back_edges.get(ei).copied().unwrap_or(false) {
                (to_idx, from_idx)
            } else {
                (from_idx, to_idx)
            })
        };
        // Lane blockers `(level, lo, hi, marker)` on the marked levels:
        // node spans (a zero-extent node occupies no cell) and dummy
        // columns block every row; a self-loop marker cell only its own
        // top row. Sorted, spans merged per level, so a query is one
        // predecessor lookup — the heap backend's per-level lists in
        // carved scratch.
        let blockers: &mut [(usize, usize, usize, usize)] = &mut *temps.lane_blockers;
        let mut bn = 0usize;
        for idx in 0..node_count {
            let (l, _, base, _) = real_coords[idx];
            let ext = extent(idx);
            if ext > 0 && on_marked_level(l) && bn < blockers.len() {
                blockers[bn] = (l, base, base + ext - 1, 0);
                bn += 1;
            }
        }
        for (f, t) in graph.edges_iter() {
            if f != t {
                continue;
            }
            let (l, _, base, _) = real_coords[f];
            let marker = base + extent(f);
            if on_marked_level(l) && bn < blockers.len() {
                blockers[bn] = (l, marker, marker, 1);
                bn += 1;
            }
        }
        for &(l, x) in &dummy_data[..dummy_total] {
            let l = l as usize;
            if on_marked_level(l) && bn < blockers.len() {
                blockers[bn] = (l, x as usize, x as usize, 0);
                bn += 1;
            }
        }
        // Sort; merge hard spans; drop duplicates and every single
        // cell a hard span already covers — so on each level the entries
        // are disjoint hard spans and single cells between them, and a
        // query needs two predecessors at most.
        blockers[..bn].sort_unstable();
        let mut w = 0usize;
        for r in 0..bn {
            let cur = blockers[r];
            if w > 0 && blockers[w - 1].0 == cur.0 {
                let prev = blockers[w - 1];
                if cur.3 == 0 && prev.3 == 0 && cur.1 <= prev.2 {
                    blockers[w - 1].2 = prev.2.max(cur.2);
                    continue;
                }
                if cur == prev || (prev.3 == 0 && cur.1 >= prev.1 && cur.2 <= prev.2) {
                    continue;
                }
            }
            blockers[w] = cur;
            w += 1;
        }
        let blockers: &[(usize, usize, usize, usize)] = &blockers[..w];
        // `Some(0)`: a span or dummy covers `(level, col)`; `Some(1)`:
        // only a marker cell.
        let blocked_kind = |level: usize, col: usize| -> Option<usize> {
            let start = blockers.partition_point(|b| b.0 < level);
            let end = blockers.partition_point(|b| b.0 <= level);
            let lv = &blockers[start..end];
            let i = lv.partition_point(|b| b.1 <= col);
            let mut hit = None;
            for k in i.saturating_sub(2)..i {
                let (_, lo, hi, kind) = lv[k];
                if lo <= col && col <= hi {
                    if kind == 0 {
                        return Some(0);
                    }
                    hit = Some(1);
                }
            }
            hit
        };
        // Another edge's trunk at `col` in gap `gap` (matches the heap's
        // per-gap table, computed on demand — a lane query asks about
        // two columns, so no table is carved): a head-on source end at
        // level L runs through gap L + 1, a head-on target end at level
        // L through gap L, a dummy at level L through both.
        let other_trunk = |pc: &[(usize, usize)], gap: usize, col: usize, ei: usize| -> bool {
            graph.edges_iter().enumerate().any(|(qe, (f, t))| {
                if qe == ei || f == t {
                    return false;
                }
                let is_back = back_edges.get(qe).copied().unwrap_or(false);
                let (qs, qt) = if is_back { (t, f) } else { (f, t) };
                let (ss, ds) = graph.edge_ports(qe);
                let (ss, ds) = if is_back { (ds, ss) } else { (ss, ds) };
                let sf = Face::of(ss, A::FLOW_AXIS, level_flipped, EndRole::Source);
                let df = Face::of(ds, A::FLOW_AXIS, level_flipped, EndRole::Target);
                if !detours(ss, A::FLOW_AXIS, level_flipped, EndRole::Source)
                    && sf.is_level()
                    && real_coords[qs].0 + 1 == gap
                    && resolved(pc, qe, qs, false) == col
                {
                    return true;
                }
                if !detours(ds, A::FLOW_AXIS, level_flipped, EndRole::Target)
                    && df.is_level()
                    && real_coords[qt].0 == gap
                    && resolved(pc, qe, qt, true) == col
                {
                    return true;
                }
                let ds_ = temps.dummy_offsets[qe] as usize;
                let de_ = (temps.dummy_offsets[qe + 1] as usize).min(dummy_total);
                dummy_data[ds_..de_].iter().any(|&(l, x)| {
                    let l = l as usize;
                    x as usize == col && (l == gap || l + 1 == gap)
                })
            })
        };
        // The gaps an end's lane run crosses (matches heap).
        let run_gaps = |level: usize, face: Face, is_target: bool| -> (usize, usize) {
            if face.is_level() {
                (level, level + 1)
            } else if is_target {
                (level, level)
            } else {
                (level + 1, level + 1)
            }
        };
        // Plans for the detouring edges, in edge order — faces first.
        let plans: &mut [(usize, Detour)] = &mut *temps.detour_plans;
        plans.fill((usize::MAX, Detour::NONE));
        let mut pn = 0usize;
        for (ei, (from_idx, to_idx)) in graph.edges_iter().enumerate() {
            if from_idx == to_idx {
                continue;
            }
            let is_back = back_edges.get(ei).copied().unwrap_or(false);
            let (src_side, dst_side) = graph.edge_ports(ei);
            let (src_side, dst_side) = if is_back {
                (dst_side, src_side)
            } else {
                (src_side, dst_side)
            };
            let mut det = Detour::NONE;
            det.src_wants = detours(src_side, A::FLOW_AXIS, level_flipped, EndRole::Source);
            det.dst_wants = detours(dst_side, A::FLOW_AXIS, level_flipped, EndRole::Target);
            det.src_face = Face::of(src_side, A::FLOW_AXIS, level_flipped, EndRole::Source);
            det.dst_face = Face::of(dst_side, A::FLOW_AXIS, level_flipped, EndRole::Target);
            if (det.src_wants || det.dst_wants) && pn < plans.len() {
                plans[pn] = (ei, det);
                pn += 1;
            }
        }
        let plans: &mut [(usize, Detour)] = &mut plans[..pn];
        let has_loop = |idx: usize| graph.edges_iter().any(|(f, t)| f == t && f == idx);
        // 1. Lanes. An end without one attaches head-on after all — on
        // its role's own face, at the center.
        for pi in 0..plans.len() {
            let (ei, mut det) = plans[pi];
            let Some((src, dst)) = layout_ends(ei) else {
                continue;
            };
            for (is_target, node, peer) in [(false, src, dst), (true, dst, src)] {
                let (wants, face) = if is_target {
                    (det.dst_wants, det.dst_face)
                } else {
                    (det.src_wants, det.src_face)
                };
                if !wants {
                    continue;
                }
                let (level, _, base, _) = real_coords[node];
                // A lane another edge already chose on this level or an
                // adjacent one is taken (matches the heap's `taken`
                // list); an edge's own ends may share, their runs are
                // one line.
                // Another edge's lane in a gap this run crosses (matches
                // the heap's `taken` list); a side-face stub at this
                // very node shares the column with this end's by design.
                let gaps = run_gaps(level, face, is_target);
                let overlaps = |n: usize, f: Face, target: bool| {
                    let (a, b) = run_gaps(real_coords[n].0, f, target);
                    a <= gaps.1 && b >= gaps.0 && !(!f.is_level() && n == node && !face.is_level())
                };
                let taken_here = |c: usize| {
                    plans[..pi].iter().any(|&(qe, qd)| {
                        layout_ends(qe).is_some_and(|(qs, qt)| {
                            (qd.src_lane == c && overlaps(qs, qd.src_face, false))
                                || (qd.dst_lane == c && overlaps(qt, qd.dst_face, true))
                        })
                    })
                };
                let trunk_here =
                    |c: usize| (gaps.0..=gaps.1).any(|g| other_trunk(port_cross, g, c, ei));
                let lane = if face.is_level() {
                    choose_lane(
                        base,
                        extent(node),
                        has_loop(node),
                        center(peer) > center(node),
                        max_width as usize,
                        &|c| blocked_kind(level, c).is_some() || trunk_here(c) || taken_here(c),
                    )
                } else {
                    let top_row = side_row(port_cross, ei, node, is_target) == 0;
                    lateral_lane(base, extent(node), face, max_width as usize, &|c| {
                        match blocked_kind(level, c) {
                            Some(0) => true,
                            Some(1) => top_row || trunk_here(c) || taken_here(c),
                            _ => trunk_here(c) || taken_here(c),
                        }
                    })
                };
                if lane == usize::MAX {
                    if is_target {
                        det.dst_wants = false;
                        port_cross[ei].1 = usize::MAX;
                    } else {
                        det.src_wants = false;
                        port_cross[ei].0 = usize::MAX;
                    }
                } else if is_target {
                    det.dst_lane = lane;
                } else {
                    det.src_lane = lane;
                }
            }
            plans[pi].1 = det;
        }
        // A policy places both directions of a face deterministically,
        // so colliding ends share their cell — the ordinary drawing —
        // and nothing is shifted or cancelled here.
        // 4. Bottom-face arrivals reserve their arrowhead cell on the
        // target level's slot 0 exactly as reversed edges do.
        for pi in 0..plans.len() {
            let (ei, det) = plans[pi];
            if det.dst_lane == usize::MAX || !det.dst_face.is_level() {
                continue;
            }
            let Some((_, dst)) = layout_ends(ei) else {
                continue;
            };
            let ax = resolved(port_cross, ei, dst, true);
            let lvl = real_coords[dst].0;
            if lvl < alloc_size {
                if temps.level_slot_next[lvl] == 0 {
                    temps.level_slot_next[lvl] = 1;
                }
                slot_push(
                    temps.slot_pool,
                    &mut slot_pool_len,
                    temps.slot_heads,
                    temps.slot_tails,
                    lvl * MAX_SLOTS_PER_LEVEL,
                    ax.saturating_sub(ARROW_CELL_PAD),
                    ax + ARROW_CELL_PAD,
                );
            }
        }
    }

    // 2. Assign slots by scanning edges (same iteration order as Step 9)
    for (ei, (from_idx, to_idx)) in graph.edges_iter().enumerate() {
        if from_idx == to_idx {
            continue;
        }
        let is_back = back_edges.get(ei).copied().unwrap_or(false);
        let (src_idx, dst_idx) = if is_back {
            (to_idx, from_idx)
        } else {
            (from_idx, to_idx)
        };
        if src_idx >= temps.real_coords.len() || dst_idx >= temps.real_coords.len() {
            continue;
        }
        let (src_level, _, src_x_base, src_width) = temps.real_coords[src_idx];
        let (dst_level, _, dst_x_base, dst_width) = temps.real_coords[dst_idx];

        if dst_level <= src_level {
            continue;
        }

        let src_x_center = src_x_base + src_width / 2;
        let dst_x_center = dst_x_base + dst_width / 2;
        let is_vertical = src_x_center == dst_x_center && dst_level == src_level + 1;
        if is_vertical {
            continue;
        }

        // Inclusive center-to-center interval — the heap convention.
        let (min_x, max_x) = if src_x_center < dst_x_center {
            (src_x_center, dst_x_center)
        } else {
            (dst_x_center, src_x_center)
        };

        let lvl = src_level;
        if lvl >= alloc_size {
            continue;
        }

        if temps.node_slots[src_idx] != usize::MAX {
            // Reuse the source's slot: merge into the tail interval when
            // overlapping, else append (heap rule).
            let slot = temps.node_slots[src_idx];
            let base = lvl * MAX_SLOTS_PER_LEVEL + slot;
            if slot < temps.level_slot_next[lvl] as usize && base < temps.slot_tails.len() {
                let tail = temps.slot_tails[base];
                let mut merged = false;
                if tail != usize::MAX {
                    let (ts, te, _) = temps.slot_pool[tail];
                    if min_x <= te && max_x >= ts {
                        temps.slot_pool[tail].0 = ts.min(min_x);
                        temps.slot_pool[tail].1 = te.max(max_x);
                        merged = true;
                    }
                }
                if !merged {
                    slot_push(
                        temps.slot_pool,
                        &mut slot_pool_len,
                        temps.slot_heads,
                        temps.slot_tails,
                        base,
                        min_x,
                        max_x,
                    );
                }
            }
        } else {
            // New source — greedy first-fit over the interval lists
            // (tail fast-path, then full collide scan; heap rule).
            let slots_used = temps.level_slot_next[lvl] as usize;
            let mut chosen = None;

            for s in 0..slots_used {
                let base = lvl * MAX_SLOTS_PER_LEVEL + s;
                let tail = if base < temps.slot_tails.len() {
                    temps.slot_tails[base]
                } else {
                    usize::MAX
                };
                if tail != usize::MAX && min_x >= temps.slot_pool[tail].1 {
                    slot_push(
                        temps.slot_pool,
                        &mut slot_pool_len,
                        temps.slot_heads,
                        temps.slot_tails,
                        base,
                        min_x,
                        max_x,
                    );
                    chosen = Some(s);
                    break;
                }
                if !slot_collides(temps.slot_pool, temps.slot_heads, base, min_x, max_x) {
                    slot_push(
                        temps.slot_pool,
                        &mut slot_pool_len,
                        temps.slot_heads,
                        temps.slot_tails,
                        base,
                        min_x,
                        max_x,
                    );
                    chosen = Some(s);
                    break;
                }
            }

            let slot = if let Some(s) = chosen {
                s
            } else if slots_used < MAX_SLOTS_PER_LEVEL {
                let s = slots_used;
                slot_push(
                    temps.slot_pool,
                    &mut slot_pool_len,
                    temps.slot_heads,
                    temps.slot_tails,
                    lvl * MAX_SLOTS_PER_LEVEL + s,
                    min_x,
                    max_x,
                );
                temps.level_slot_next[lvl] += 1;
                s
            } else {
                // Cap reached — degrade to slot 0 (heap pushes the
                // interval there too, keeping later decisions aligned).
                slot_push(
                    temps.slot_pool,
                    &mut slot_pool_len,
                    temps.slot_heads,
                    temps.slot_tails,
                    lvl * MAX_SLOTS_PER_LEVEL,
                    min_x,
                    max_x,
                );
                0
            };

            temps.node_slots[src_idx] = slot;
        }
    }

    // 3. Count jogging waypoints per level — only a waypoint whose column
    // differs from the NEXT chain column (next waypoint, or the layout-
    // target center) claims a routing row; straight pass-throughs are pure
    // verticals. Must match the emission filter in Step 9 and the heap
    // backend's jog rule exactly.
    temps.dummy_counts.fill(0);
    #[cfg_attr(not(feature = "ports"), allow(unused_mut, unused_variables))]
    let mut jog_n = 0usize;
    for (ei, (f_idx, t_idx)) in graph.edges_iter().enumerate() {
        if f_idx == t_idx {
            continue;
        }
        let ds = temps.dummy_offsets[ei] as usize;
        let de = (temps.dummy_offsets[ei + 1] as usize).min(temps.dummy_data.len());
        if de <= ds {
            continue;
        }
        let is_back = back_edges.get(ei).copied().unwrap_or(false);
        let layout_dst = if is_back { f_idx } else { t_idx };
        if layout_dst >= temps.real_coords.len() {
            continue;
        }
        // The chain's entry column: the lane when the target detours,
        // else the RESOLVED port (positioned, or the center) — matches
        // the heap backend and Step 9's emission filter.
        #[cfg(feature = "ports")]
        let target_x = {
            let plan = super::ports::plan_lookup(temps.detour_plans, ei);
            match plan {
                Some(d) if d.dst_lane != usize::MAX => d.dst_lane,
                _ => {
                    let positioned = temps.port_cross.get(ei).map_or(usize::MAX, |p| p.1);
                    if positioned != usize::MAX {
                        positioned
                    } else {
                        let (_, _, dx, dw) = temps.real_coords[layout_dst];
                        dx + dw / 2
                    }
                }
            }
        };
        #[cfg(not(feature = "ports"))]
        let target_x = {
            let (_, _, dx, dw) = temps.real_coords[layout_dst];
            dx + dw / 2
        };
        for i in ds..de {
            let (level, x) = temps.dummy_data[i];
            let next_x = if i + 1 < de {
                temps.dummy_data[i + 1].1 as usize
            } else {
                target_x
            };
            let lvl = level as usize;
            if x as usize != next_x && lvl < alloc_size {
                let k = temps.dummy_counts[lvl] as usize;
                temps.dummy_counts[lvl] += 1;
                // Jog bend rows share the band's slot rows (bend `k`
                // paints on slot index `1 + label row + k`); with
                // detours in play, record the interval so detour runs
                // are allocated clear of it (matches heap). The label
                // row is known only after the flags pass below, so the
                // allocator adds it.
                #[cfg(feature = "ports")]
                if jog_n < temps.jog_blocks.len() {
                    let (a, b) = (x as usize, next_x);
                    temps.jog_blocks[jog_n] = (lvl, k, a.min(b), a.max(b));
                    jog_n += 1;
                }
                #[cfg(not(feature = "ports"))]
                let _ = k;
            }
        }
    }

    // Per-level label-source flags: the label row is budgeted only in the
    // bands of levels that actually source a labeled edge. Mirrors the
    // heap backend.
    temps.level_labeled_src.fill(0);
    for (ei, (f_idx, t_idx)) in graph.edges_iter().enumerate() {
        if f_idx == t_idx || graph.edge_label(ei).is_empty() {
            continue;
        }
        let is_back = back_edges.get(ei).copied().unwrap_or(false);
        let layout_src = if is_back { t_idx } else { f_idx };
        if layout_src < temps.real_coords.len() {
            let lvl = temps.real_coords[layout_src].0 as usize;
            if lvl < alloc_size {
                temps.level_labeled_src[lvl] = 1;
            }
        }
    }

    // Detour rows: every detour run gets a slot with its EXACT interval
    // (never the fan-out bus); the up-run above a level-0 source lives
    // in the extra slot level past the last real one. Same order and
    // rule as the heap backend.
    #[cfg(feature = "ports")]
    if budget.any() {
        let real_coords: &[(usize, usize, usize, usize)] = &*temps.real_coords;
        let port_cross: &[(usize, usize)] = &*temps.port_cross;
        let resolved = |ei: usize, idx: usize, target: bool| -> usize {
            let positioned = port_cross
                .get(ei)
                .map_or(usize::MAX, |p| if target { p.1 } else { p.0 });
            if positioned != usize::MAX {
                positioned
            } else {
                let (_, _, base, _) = real_coords[idx];
                A::cross_center(
                    base,
                    A::cross_extent(graph.node_width(idx), graph.node_height(idx)),
                )
            }
        };
        let top_level = alloc_size;
        // The greedy pass registered each bus run as the CENTERS span,
        // which understates a skip edge's first run and a spread
        // port's; register the TRUE extents so detour runs see them
        // (matches heap; graphs with detours only).
        for (ei, (from_idx, to_idx)) in graph.edges_iter().enumerate() {
            if from_idx == to_idx
                || super::ports::plan_lookup(temps.detour_plans, ei).is_some_and(|d| d.active())
            {
                continue;
            }
            let is_back = back_edges.get(ei).copied().unwrap_or(false);
            let (src, dst) = if is_back {
                (to_idx, from_idx)
            } else {
                (from_idx, to_idx)
            };
            let from_x = resolved(ei, src, false);
            let to_x = resolved(ei, dst, true);
            let ds = temps.dummy_offsets[ei] as usize;
            let de = (temps.dummy_offsets[ei + 1] as usize).min(temps.dummy_data.len());
            let mut first_target = to_x;
            for i in ds..de {
                let x = temps.dummy_data[i].1 as usize;
                let next_x = if i + 1 < de {
                    temps.dummy_data[i + 1].1 as usize
                } else {
                    to_x
                };
                if x != next_x {
                    first_target = x;
                    break;
                }
            }
            if from_x == first_target {
                continue;
            }
            let level = real_coords[src].0;
            let slot = if temps.node_slots[src] != usize::MAX {
                temps.node_slots[src]
            } else {
                0
            };
            if level < alloc_size && slot < MAX_SLOTS_PER_LEVEL {
                if (temps.level_slot_next[level] as usize) <= slot {
                    temps.level_slot_next[level] = slot as Idx + 1;
                }
                slot_push(
                    temps.slot_pool,
                    &mut slot_pool_len,
                    temps.slot_heads,
                    temps.slot_tails,
                    level * MAX_SLOTS_PER_LEVEL + slot,
                    from_x.min(first_target),
                    from_x.max(first_target),
                );
            }
        }
        for pi in 0..temps.detour_plans.len() {
            let (ei, mut det) = temps.detour_plans[pi];
            if ei == usize::MAX || !det.active() {
                continue;
            }
            let (from_idx, to_idx) = graph.edge(ei);
            let is_back = back_edges.get(ei).copied().unwrap_or(false);
            let (src, dst) = if is_back {
                (to_idx, from_idx)
            } else {
                (from_idx, to_idx)
            };
            let (src_level, dst_level) = (real_coords[src].0, real_coords[dst].0);
            let from_x = resolved(ei, src, false);
            let to_x = resolved(ei, dst, true);
            let minmax = |a: usize, b: usize| (a.min(b), a.max(b));
            let mut col = from_x;
            if det.src_lane != usize::MAX {
                // An opposite-face exit needs its up-run row; a lateral
                // stub runs at the node's own row.
                if det.src_face.is_level() {
                    let (lo, hi) = minmax(from_x, det.src_lane);
                    let lvl = if src_level == 0 {
                        top_level
                    } else {
                        src_level - 1
                    };
                    det.up_slot = alloc_slot_csr(
                        temps.slot_pool,
                        &mut slot_pool_len,
                        temps.slot_heads,
                        temps.slot_tails,
                        temps.level_slot_next,
                        &temps.jog_blocks[..jog_n],
                        temps.level_labeled_src,
                        lvl,
                        lo,
                        hi,
                    );
                }
                col = det.src_lane;
            }
            let final_col = if det.dst_lane != usize::MAX {
                det.dst_lane
            } else {
                to_x
            };
            // The chain's first jogging waypoint, by the same rule as the
            // counting pass above.
            let ds = temps.dummy_offsets[ei] as usize;
            let de = (temps.dummy_offsets[ei + 1] as usize).min(temps.dummy_data.len());
            let mut first_target = final_col;
            for i in ds..de {
                let x = temps.dummy_data[i].1 as usize;
                let next_x = if i + 1 < de {
                    temps.dummy_data[i + 1].1 as usize
                } else {
                    final_col
                };
                if x != next_x {
                    first_target = x;
                    break;
                }
            }
            if col != first_target {
                let (lo, hi) = minmax(col, first_target);
                det.first_slot = alloc_slot_csr(
                    temps.slot_pool,
                    &mut slot_pool_len,
                    temps.slot_heads,
                    temps.slot_tails,
                    temps.level_slot_next,
                    &temps.jog_blocks[..jog_n],
                    temps.level_labeled_src,
                    src_level,
                    lo,
                    hi,
                );
            }
            if det.dst_lane != usize::MAX && det.dst_face.is_level() {
                let (lo, hi) = minmax(det.dst_lane, to_x);
                det.below_slot = alloc_slot_csr(
                    temps.slot_pool,
                    &mut slot_pool_len,
                    temps.slot_heads,
                    temps.slot_tails,
                    temps.level_slot_next,
                    &temps.jog_blocks[..jog_n],
                    temps.level_labeled_src,
                    dst_level,
                    lo,
                    hi,
                );
            }
            temps.detour_plans[pi].1 = det;
        }
    }
    // Rows above the first level for its upward exits: one per slot
    // plus the clearance line before the nodes.
    #[cfg(feature = "ports")]
    let top_extra =
        if temps.level_slot_next.len() > alloc_size && temps.level_slot_next[alloc_size] > 0 {
            EDGE_START_OFFSET + temps.level_slot_next[alloc_size] as usize
        } else {
            0
        };
    #[cfg(not(feature = "ports"))]
    let top_extra = 0usize;

    // 4. Compute per-level max node LEVEL extents and offsets.
    // Geometry stays `usize` — the configurable index type must never
    // hold extents (a 256-wide LR node would wrap to 0 under u8).
    let max_node_extents = &mut temps.level_max_extents[..alloc_size];
    max_node_extents.fill(1);
    for idx in 0..node_count {
        let level = temps.real_coords[idx].0;
        // Level-axis extent (Vertical: the node's height).
        let extent = A::level_extent(graph.node_width(idx), graph.node_height(idx));
        if level < alloc_size && extent > max_node_extents[level] {
            max_node_extents[level] = extent;
        }
    }

    temps.level_offsets.fill(0);
    let level_spacing: usize = config.level_spacing;

    // Compute subgraph Y extras (vertical border space)
    let (sg_initial_offset, sg_trailing_extra) = if graph.has_subgraphs() {
        compute_sg_level_extras::<A>(
            graph,
            temps.node_levels,
            max_level as usize,
            temps.sg_ranges,
            temps.sg_depths,
            temps.sg_y_extras,
        )
    } else {
        (0, 0)
    };
    #[cfg_attr(not(feature = "ports"), allow(unused_variables))]
    let top_base = sg_initial_offset;

    // Pure accumulation over per-level inputs — run once, and a second
    // time when the D8(b) label phase grows `sg_y_extras` (Horizontal
    // with subgraphs only).
    fn accumulate_offsets(
        level_offsets: &mut [usize],
        max_node_extents: &[usize],
        level_slot_next: &[Idx],
        dummy_counts: &[Idx],
        level_labeled_src: &[Idx],
        sg_y_extras: &[usize],
        has_subgraphs: bool,
        level_spacing: usize,
        sg_initial_offset: usize,
        sg_trailing_extra: usize,
        max_level: usize,
    ) -> usize {
        let mut current_offset = sg_initial_offset;
        for level in 0..=max_level {
            level_offsets[level] = current_offset;
            let node_height = max_node_extents[level];
            // Use actual geometry-aware slot count (not naive source count)
            let slot_count = level_slot_next[level] as usize;
            // Jog rows plus the bend row below the deepest jog (shared rule
            // with the heap backend).
            let diff = slot_count.max(super::geometry::passthrough_extent(
                dummy_counts[level] as usize,
            ));
            // Per-level overhead: the label row is budgeted only where a
            // labeled edge is sourced (shared rule with the heap backend).
            let routing_overhead = super::geometry::routing_overhead(level_labeled_src[level] != 0);
            let height = node_height + routing_overhead + diff.saturating_sub(1);
            current_offset += height;
            // Extra vertical gap between levels only — not after the last one,
            // which would pad the bottom of the canvas with blank rows.
            if level < max_level {
                current_offset += level_spacing;
            }
            // Add subgraph border space after this level
            if has_subgraphs && level < sg_y_extras.len() {
                current_offset += sg_y_extras[level];
            }
        }
        current_offset += sg_trailing_extra;
        level_offsets[max_level + 1] = current_offset;
        current_offset
    }
    let mut total_height = accumulate_offsets(
        temps.level_offsets,
        max_node_extents,
        temps.level_slot_next,
        temps.dummy_counts,
        temps.level_labeled_src,
        temps.sg_y_extras,
        graph.has_subgraphs(),
        level_spacing,
        sg_initial_offset + top_extra,
        sg_trailing_extra,
        max_level as usize,
    );

    // D8(b) second phase (matches heap): reserve label room on the
    // LEVEL axis. Deficits fold into `sg_y_extras` at each box's
    // closing level (max per level, like heap) and the accumulation
    // re-runs. Vertical claims are statically zero — it never re-runs.
    if A::LABEL_CLAIMS_LEVEL_AXIS && graph.has_subgraphs() {
        let mut any = false;
        // One pass over subgraphs, maxima accumulated per closing level.
        // `level_routing_floor` is not populated until the edge loop
        // (it is filled(0) there), so it serves as the per-level
        // scratch here — no new allocation, no O(levels × subgraphs)
        // scan (slices review).
        let label_extras = &mut temps.level_routing_floor[..alloc_size];
        label_extras.fill(0);
        for sg_idx in 0..graph.subgraph_count() {
            let (first, last) = temps.sg_ranges[sg_idx];
            if first == usize::MAX || last >= alloc_size {
                continue;
            }
            let need = A::label_level_extent(graph.subgraph_label(sg_idx));
            if need == 0 {
                continue;
            }
            let start = temps.level_offsets[first].saturating_sub(A::SG_PAD_LEVEL.0);
            let end = temps.level_offsets[last] + max_node_extents[last] + A::SG_PAD_LEVEL.1;
            let deficit = need.saturating_sub(end.saturating_sub(start));
            label_extras[last] = label_extras[last].max(deficit);
        }
        for (level, &extra) in label_extras.iter().enumerate() {
            if extra > 0 && level < temps.sg_y_extras.len() {
                temps.sg_y_extras[level] += extra;
                any = true;
            }
        }
        if any {
            total_height = accumulate_offsets(
                temps.level_offsets,
                max_node_extents,
                temps.level_slot_next,
                temps.dummy_counts,
                temps.level_labeled_src,
                temps.sg_y_extras,
                true,
                level_spacing,
                sg_initial_offset + top_extra,
                sg_trailing_extra,
                max_level as usize,
            );
        }
    }
    let total_height = total_height;

    // Step 9: Build LayoutIRArena
    // Include subgraph label bytes in total label allocation
    let sg_label_bytes = if graph.has_subgraphs() {
        let mut bytes = 0;
        for i in 0..graph.subgraph_count() {
            bytes += graph.subgraph_label(i).len();
        }
        bytes
    } else {
        0
    };
    // Reserve IR capacity for dummy nodes only when they are requested
    // (they also join the level lists, allocated from the same budget).
    let dummy_node_capacity = if config.include_dummy_nodes {
        temps.dummy_offsets[edge_count] as usize
    } else {
        0
    };
    // Custom payloads ride the IR label storage; the entry array is
    // sized by the declared-content count.
    let custom_payload_bytes: usize = graph
        .custom_nodes()
        .iter()
        .map(|entry| entry.payload_len)
        .sum();
    let mut builder = LayoutIRArenaBuilder::new_with_subgraphs(
        output_arena,
        node_count + dummy_node_capacity,
        edge_count,
        // Kept waypoints are a subset of the dummy chain entries; explicit
        // polylines (detours) stage their bends in the same pool.
        total_dummies.max(1) + budget.points,
        total_label_bytes + sg_label_bytes + custom_payload_bytes,
        max_level as usize + 1,
        sg_count,
        graph.custom_nodes().len(),
        self_loop_count,
    )
    .ok_or(GraphError::BuilderFailed)?;

    // max_width already includes the routing buffer and label/subgraph
    // margins (see its computation after refinement) — adding them again
    // here double-counted 12 columns vs the heap backend, caught by the
    // LayoutView equivalence tests.
    // ── Materialization point for canvas extents: (level, cross)
    // totals → (width, height).
    let (canvas_width, canvas_height) = A::materialize(total_height, max_width as usize);
    builder.set_dimensions(canvas_width, canvas_height);
    builder.set_level_count(max_level as usize + 1);

    // Add nodes
    for idx in 0..node_count {
        let (level, pos, cross, _) = temps.real_coords[idx];
        let (x, y) = A::materialize(temps.level_offsets[level as usize], cross as usize);
        let id = graph.node_id(idx);
        let label = graph.node_label(idx);

        let ir_idx = builder
            .add_node(
                id,
                label,
                x,
                y,
                // Physical extents from the declared dimensions — the
                // packed tuple's extent is the role-space cross extent.
                graph.node_width(idx),
                graph.node_height(idx),
                level as usize,
                pos as usize,
                if graph.node_is_implicit(idx) {
                    crate::ir::NodeKind::Implicit
                } else {
                    crate::ir::NodeKind::Explicit
                },
                usize::MAX,
                graph.node_content_tag(idx),
            )
            .ok_or(GraphError::ArenaOom)?;
        // Carry the node's declared painter/payload (sparse).
        if let Ok(pos) = graph
            .custom_nodes()
            .binary_search_by_key(&idx, |entry| entry.node_idx)
        {
            let entry = graph.custom_nodes()[pos];
            builder
                .add_custom(ir_idx, entry.painter, graph.custom_payload(&entry))
                .ok_or(GraphError::ArenaOom)?;
        }
        builder
            .add_node_to_level(level as usize, idx)
            .ok_or(GraphError::ArenaOom)?;
    }

    // Emit dummy nodes into the IR (opt-in; zero cost when disabled).
    // Mirrors the heap backend: synthetic ids counting down from
    // usize::MAX, width 1 at the drawn waypoint column, excluded from
    // id lookups, included in the level lists.
    if config.include_dummy_nodes {
        let mut synthetic = 0usize;
        for level in 0..=(max_level as usize) {
            let vstart = temps.vlevel_offsets[level] as usize;
            let vend = temps.vlevel_offsets[level + 1] as usize;
            for pos in vstart..vend {
                if !vnode_is_dummy(temps.vnode_data, pos) {
                    continue;
                }
                let edge_idx = vnode_payload(temps.vnode_data, pos) as usize;
                // The drawn column comes from this edge's waypoint chain
                // entry for this level (chains are level-sorted).
                let ds = temps.dummy_offsets[edge_idx] as usize;
                let de = (temps.dummy_offsets[edge_idx + 1] as usize).min(temps.dummy_data.len());
                let Some(&(_, x)) = temps.dummy_data[ds..de]
                    .iter()
                    .find(|&&(l, _)| l as usize == level)
                else {
                    continue;
                };
                let (x, y) = A::materialize(temps.level_offsets[level], x as usize);
                let id = usize::MAX - synthetic;
                synthetic += 1;
                let node_idx = builder
                    .add_node(
                        id,
                        "",
                        x,
                        y,
                        1,
                        1,
                        level,
                        pos - vstart,
                        crate::ir::NodeKind::Dummy,
                        edge_idx,
                        0,
                    )
                    .ok_or(GraphError::ArenaOom)?;
                builder
                    .add_node_to_level(level, node_idx)
                    .ok_or(GraphError::ArenaOom)?;
            }
        }
    }

    builder.finalize_levels();

    // Slots are pre-assigned by geometry-aware allocation in Step 8.
    // Only level_dummy_next needs reset for skip-level waypoint slot tracking.
    temps.level_dummy_next.fill(0);

    // Initialize routing floor tracking for edge-border avoidance
    temps.level_routing_floor.fill(0);

    // The direction flip is a per-layout fact — resolved once here,
    // exactly as the heap backend does.
    let level_flipped = super::ports::level_flipped::<A>(config.direction);

    // Access mutable buffers via temps
    let node_slots = &temps.node_slots;
    let level_dummy_next = &mut temps.level_dummy_next;
    let waypoint_scratch = &mut temps.waypoint_scratch;
    // ── Physical-space boundary ── edge paths below are computed and
    // emitted y-primary; LR-P1 rewrites routing in role space and
    // materializes per edge (with `flow_axis`, temp/08 D2).
    let level_offsets = &temps.level_offsets;
    let max_node_extents = &temps.level_max_extents;
    let level_labeled_src = &temps.level_labeled_src;
    let level_routing_floor = &mut temps.level_routing_floor;

    // Add edges
    for (edge_idx, (from_idx, to_idx)) in graph.edges_iter().enumerate() {
        // First kept waypoint's x — the label anchor for skip-level
        // paths (mirrors the heap backend's `waypoints[0].0`).
        let mut first_wp_x: Option<usize> = None;
        // Physical y-bounds of materialized waypoints (Horizontal only:
        // waypoint rows can lie outside the port-row span).
        let mut waypoint_y_bounds: Option<(usize, usize)> = None;
        #[cfg_attr(not(feature = "ports"), allow(unused_mut))]
        let mut ortho_y_bounds: Option<(usize, usize)> = None;
        #[cfg_attr(not(feature = "ports"), allow(unused_mut, unused_variables))]
        let mut explicit_label_col: Option<usize> = None;
        // Self-loops: mark the node and skip edge routing.
        // D5(a): the marker cell is layout geometry — one cell past the
        // node on the cross axis, at its level-leading line (matches
        // heap; Vertical: the legacy right-of-top-row cell).
        if from_idx == to_idx {
            // Preserve the loop as a record: identity (input index),
            // label, and the owning node — the routed list never sees
            // it, but the scene will.
            builder
                .add_self_loop(
                    graph.node_id(from_idx),
                    from_idx,
                    edge_idx,
                    if has_labeled_edges {
                        graph.edge_label(edge_idx)
                    } else {
                        ""
                    },
                )
                .ok_or(GraphError::BuilderFailed)?;
            let (lvl, _, cross, _) = temps.real_coords[from_idx];
            builder.set_self_loop_at(
                from_idx,
                A::materialize(
                    level_offsets[lvl as usize],
                    cross
                        + A::cross_extent(graph.node_width(from_idx), graph.node_height(from_idx)),
                ),
            );
            continue;
        }

        // For back edges, layout direction is reversed (to→from in level space).
        // We compute coordinates in layout order, then store semantic IDs in the IR.
        let is_back = back_edges.get(edge_idx).copied().unwrap_or(false);
        let (layout_src_idx, layout_dst_idx) = if is_back {
            (to_idx, from_idx)
        } else {
            (from_idx, to_idx)
        };

        let (src_level, _, src_x_base, _) = temps.real_coords[layout_src_idx];
        let (dst_level, _, dst_x_base, _) = temps.real_coords[layout_dst_idx];

        // Attachment resolution (ports sit on the node's DECLARED span
        // — the packed tuple extent may carry the D5(ii) marker
        // reserve; matches heap). Declared sides bind to declared
        // endpoints, so a reversal swaps the SIDES onto the layout
        // roles; `Auto` binds to the layout role itself.
        use super::ports::{Attachment, EndRole};
        let (src_side, dst_side) = graph.edge_ports(edge_idx);
        let (layout_src_side, layout_dst_side) = if is_back {
            (dst_side, src_side)
        } else {
            (src_side, dst_side)
        };
        // A positioned request overrides the center line; anything
        // else (Auto, or a face without routing yet) resolves as
        // before.
        let positioned = temps
            .port_cross
            .get(edge_idx)
            .copied()
            .unwrap_or((usize::MAX, usize::MAX));
        let from_x = if positioned.0 != usize::MAX {
            positioned.0
        } else {
            Attachment::resolve::<A>(
                layout_src_side,
                level_flipped,
                EndRole::Source,
                src_x_base,
                A::cross_extent(
                    graph.node_width(layout_src_idx),
                    graph.node_height(layout_src_idx),
                ),
            )
            .cross
        };
        let to_x = if positioned.1 != usize::MAX {
            positioned.1
        } else {
            Attachment::resolve::<A>(
                layout_dst_side,
                level_flipped,
                EndRole::Target,
                dst_x_base,
                A::cross_extent(
                    graph.node_width(layout_dst_idx),
                    graph.node_height(layout_dst_idx),
                ),
            )
            .cross
        };
        // The routing band starts after the level's FULL extent; the IR
        // endpoint sits on the source's port line (per-axis: Vertical =
        // band-trailing, Horizontal = own face). Matches heap.
        let band_trailing =
            level_offsets[src_level as usize] + max_node_extents[src_level as usize] - 1;
        let from_y = A::source_port_level(
            level_offsets[src_level as usize],
            A::level_extent(
                graph.node_width(layout_src_idx),
                graph.node_height(layout_src_idx),
            ),
            max_node_extents[src_level as usize],
        );
        let to_y = level_offsets[dst_level as usize];
        // A detouring end attaches on its node's OWN face line (matches
        // heap): the leading line for an upward exit, the trailing line
        // for an arrival from below.
        #[cfg(feature = "ports")]
        let detour = super::ports::plan_lookup(temps.detour_plans, edge_idx).filter(|d| d.active());
        #[cfg(not(feature = "ports"))]
        let detour: Option<super::ports::Detour> = None;
        // A lateral end sits on the node's OWN side cell: the face
        // column, at the row the spread assigned along the face.
        let lateral = |idx: usize, face: super::ports::Face, along: usize, level: usize| {
            let (_, _, base, _) = temps.real_coords[idx];
            let (w, h) = (graph.node_width(idx), graph.node_height(idx));
            let cross = if matches!(face, super::ports::Face::CrossTrailing) {
                base + A::cross_extent(w, h).saturating_sub(1)
            } else {
                base
            };
            let row = if along == usize::MAX {
                A::level_center(0, A::level_extent(w, h))
            } else {
                along
            };
            (cross, level_offsets[level] + row)
        };
        let (from_x, from_y, to_x, to_y) = match detour {
            Some(d) => {
                let (fx, fy) = if d.src_lane == usize::MAX {
                    (from_x, from_y)
                } else if d.src_face.is_level() {
                    (from_x, level_offsets[src_level as usize])
                } else {
                    lateral(layout_src_idx, d.src_face, positioned.0, src_level as usize)
                };
                let (tx, ty) = if d.dst_lane == usize::MAX {
                    (to_x, to_y)
                } else if d.dst_face.is_level() {
                    (
                        to_x,
                        level_offsets[dst_level as usize]
                            + A::level_extent(
                                graph.node_width(layout_dst_idx),
                                graph.node_height(layout_dst_idx),
                            )
                            - 1,
                    )
                } else {
                    lateral(layout_dst_idx, d.dst_face, positioned.1, dst_level as usize)
                };
                (fx, fy, tx, ty)
            }
            None => (from_x, from_y, to_x, to_y),
        };

        // Store original semantic IDs (not layout-direction IDs)
        let from_id = graph.node_id(from_idx);
        let to_id = graph.node_id(to_idx);

        // Get pre-assigned slot from geometry-aware allocation (Step 8)
        let slot = if dst_level > src_level && node_slots[layout_src_idx] != usize::MAX {
            node_slots[layout_src_idx]
        } else {
            0
        };

        // Edge routing starts one row below the source node. Reversed
        // edges' arrowheads on that row are protected by the arrow-cell
        // reservation in the slot allocator, not by shifting corners.
        let edge_start_row = EDGE_START_OFFSET;

        // 2-node cycle: A→B (forward) + B→A (reversed) sharing the same
        // column. Offset forward edge left by 1 and back edge right by 1
        // from center. Membership is precomputed in O(E log E) above.
        // Endpoint-shift separation only where nodes are cross-wide
        // (Vertical); Horizontal pairs keep the shared port — lane
        // separation is paint-time work (matches heap; temp/08 P3).
        let in_two_node_cycle = from_x == to_x
            && from_idx != to_idx
            && matches!(A::FLOW_AXIS, crate::ir::FlowAxis::Y)
            && temps.edge_in_two_cycle[edge_idx]
            && detour.is_none();

        // Applied only while BOTH shifted endpoints stay inside their
        // nodes' declared spans (matches heap): a resolved port on a
        // narrow custom node keeps its boundary cell.
        let (eff_from_x, eff_to_x) = if in_two_node_cycle {
            let delta: isize = if is_back { 1 } else { -1 };
            let inside = |x: usize, idx: usize| -> Option<usize> {
                let (_, _, base, _) = temps.real_coords[idx];
                let extent = A::cross_extent(graph.node_width(idx), graph.node_height(idx));
                let shifted = x.checked_add_signed(delta)?;
                (shifted >= base && shifted < base + extent).then_some(shifted)
            };
            match (inside(from_x, layout_src_idx), inside(to_x, layout_dst_idx)) {
                (Some(f), Some(t)) => (f, t),
                _ => (from_x, to_x),
            }
        } else {
            (from_x, to_x)
        };

        // The explicit polyline of a detouring edge (feature-gated with
        // the path shape itself); `None` routes the inferred way. Same
        // construction as the heap backend: runs on rows allocated for
        // their exact intervals, jog rows from the MultiSegment budget.
        #[cfg(feature = "ports")]
        let explicit: Option<EdgePathArena> = match detour {
            None => None,
            Some(det) => {
                let band_row = |level: usize, slot: usize| {
                    level_offsets[level] + max_node_extents[level] - 1 + edge_start_row + slot
                };
                let mut n = 0usize;
                // A run's end that IS an endpoint (a lateral stub leaves
                // the face cell itself) is no bend.
                let mut run = |row: usize, a: usize, b: usize| -> Result<(), GraphError> {
                    if a == b {
                        return Ok(());
                    }
                    for (cross, is_end) in [
                        (a, (a, row) == (from_x, from_y)),
                        (b, (b, row) == (to_x, to_y)),
                    ] {
                        if is_end {
                            continue;
                        }
                        if n >= waypoint_scratch.len() {
                            return Err(GraphError::ArenaOom);
                        }
                        waypoint_scratch[n] = (cross, row);
                        n += 1;
                    }
                    Ok(())
                };
                let mut col = from_x;
                if det.src_lane != usize::MAX {
                    if det.src_face.is_level() {
                        let up_row = if src_level == 0 {
                            top_base + top_extra - 1 - edge_start_row - det.up_slot
                        } else {
                            let lvl = src_level as usize - 1;
                            let row = band_row(lvl, det.up_slot);
                            level_routing_floor[lvl] = level_routing_floor[lvl].max(row);
                            row
                        };
                        run(up_row, from_x, det.src_lane)?;
                    } else {
                        // The lateral stub: out of the side face at the
                        // port's own row, straight onto the lane.
                        run(from_y, from_x, det.src_lane)?;
                    }
                    col = det.src_lane;
                }
                let final_col = if det.dst_lane != usize::MAX {
                    det.dst_lane
                } else {
                    to_x
                };
                let ds = temps.dummy_offsets[edge_idx] as usize;
                let de = (temps.dummy_offsets[edge_idx + 1] as usize).min(temps.dummy_data.len());
                // First jogging waypoint (same rule as the counting pass).
                let mut first_target = final_col;
                for i in ds..de {
                    let x = temps.dummy_data[i].1 as usize;
                    let next_x = if i + 1 < de {
                        temps.dummy_data[i + 1].1 as usize
                    } else {
                        final_col
                    };
                    if x != next_x {
                        first_target = x;
                        break;
                    }
                }
                explicit_label_col = Some(first_target);
                if col != first_target {
                    let lvl = src_level as usize;
                    let row = band_row(lvl, det.first_slot);
                    level_routing_floor[lvl] = level_routing_floor[lvl].max(row);
                    run(row, col, first_target)?;
                    col = first_target;
                }
                for i in ds..de {
                    let (level, x) = temps.dummy_data[i];
                    let (lvl, x) = (level as usize, x as usize);
                    let next = if i + 1 < de {
                        temps.dummy_data[i + 1].1 as usize
                    } else {
                        final_col
                    };
                    if x == next {
                        continue;
                    }
                    debug_assert_eq!(col, x);
                    let slot = if lvl < alloc_size {
                        let s = level_dummy_next[lvl];
                        level_dummy_next[lvl] += 1;
                        s
                    } else {
                        0
                    };
                    let wp_y = level_offsets[lvl]
                        + max_node_extents.get(lvl).copied().unwrap_or(1)
                        + usize::from(level_labeled_src[lvl] != 0)
                        + slot as usize;
                    if lvl < level_routing_floor.len() {
                        level_routing_floor[lvl] = level_routing_floor[lvl].max(wp_y + 1);
                    }
                    run(wp_y + 1, x, next)?;
                    col = next;
                }
                if det.dst_lane != usize::MAX {
                    if det.dst_face.is_level() {
                        let lvl = dst_level as usize;
                        let row = band_row(lvl, det.below_slot);
                        level_routing_floor[lvl] = level_routing_floor[lvl].max(row);
                        run(row, det.dst_lane, to_x)?;
                    } else {
                        // Down the lane to the port's row, then into the
                        // side face.
                        run(to_y, det.dst_lane, to_x)?;
                    }
                }
                let _ = col;
                // Materialize the role pairs and record the row span.
                let mut b_min = usize::MAX;
                let mut b_max = 0usize;
                for bend in waypoint_scratch[..n].iter_mut() {
                    *bend = A::materialize(bend.1, bend.0);
                    b_min = b_min.min(bend.1);
                    b_max = b_max.max(bend.1);
                }
                if n > 0 {
                    ortho_y_bounds = Some((b_min, b_max));
                }
                let (bends_start, bends_len) = builder
                    .add_waypoints(&waypoint_scratch[..n])
                    .ok_or(GraphError::ArenaOom)?;
                Some(EdgePathArena::Orthogonal {
                    bends_start,
                    bends_len,
                })
            }
        };
        #[cfg(not(feature = "ports"))]
        let explicit: Option<EdgePathArena> = None;
        let path = if let Some(explicit) = explicit {
            explicit
        } else if dst_level == src_level + 1 {
            if eff_from_x == eff_to_x {
                EdgePathArena::Direct
            } else {
                let hy = band_trailing + edge_start_row + slot;
                let src_lvl = src_level as usize;
                if src_lvl < level_routing_floor.len() {
                    level_routing_floor[src_lvl] = level_routing_floor[src_lvl].max(hy);
                }
                EdgePathArena::Corner { bend_at: hy }
            }
        } else if dst_level > src_level + 1 {
            let dummy_start = temps.dummy_offsets[edge_idx] as usize;
            let dummy_end = temps.dummy_offsets[edge_idx + 1] as usize;
            let dummy_count = dummy_end - dummy_start;

            if dummy_count > 0 && dummy_start < temps.dummy_data.len() {
                // Limit to scratch size
                let available = temps.dummy_data.len().saturating_sub(dummy_start);
                let raw_count = dummy_count.min(waypoint_scratch.len()).min(available);

                // Jog-aware: keep only waypoints whose column differs from
                // the next chain column (the bend to a new column paints
                // right below the kept row); straight pass-throughs are
                // longer verticals. Must match the row-counting pass in
                // Step 3 and the heap backend exactly.
                let mut waypoint_count = 0usize;
                for i in 0..raw_count {
                    let (level, x) = temps.dummy_data[dummy_start + i];
                    let next_x = if i + 1 < raw_count {
                        temps.dummy_data[dummy_start + i + 1].1 as usize
                    } else {
                        to_x
                    };
                    if x as usize == next_x {
                        continue;
                    }
                    let lvl_idx = level as usize;

                    // Assign a unique vertical slot for this edge at this level
                    let dummy_slot = if lvl_idx < alloc_size {
                        let s = level_dummy_next[lvl_idx];
                        level_dummy_next[lvl_idx] += 1;
                        s
                    } else {
                        0
                    };

                    // Calculate Y using level_offsets + max_node_height at intermediate level
                    let y_base = level_offsets[lvl_idx]
                        + max_node_extents.get(lvl_idx).copied().unwrap_or(1)
                        - 1;
                    // Waypoint rows budget the label offset only on levels
                    // that source a labeled edge — matches the heap path.
                    let wp_y = y_base
                        + edge_start_row
                        + usize::from(level_labeled_src[lvl_idx] != 0)
                        + dummy_slot as usize;
                    waypoint_scratch[waypoint_count] = (x as usize, wp_y);
                    waypoint_count += 1;
                    // Track routing floor: the row itself and the bend row
                    // right below it (every kept waypoint bends).
                    if lvl_idx < level_routing_floor.len() {
                        level_routing_floor[lvl_idx] = level_routing_floor[lvl_idx].max(wp_y + 1);
                    }
                }

                if waypoint_count == 0 {
                    // Fully straight chain: the reserved dummy columns line
                    // up with the target — plain vertical, or one bend in
                    // the source band. Matches the heap backend.
                    if eff_from_x == eff_to_x {
                        EdgePathArena::Direct
                    } else {
                        let hy = band_trailing + edge_start_row + slot;
                        let src_lvl = src_level as usize;
                        if src_lvl < level_routing_floor.len() {
                            level_routing_floor[src_lvl] = level_routing_floor[src_lvl].max(hy);
                        }
                        EdgePathArena::Corner { bend_at: hy }
                    }
                } else {
                    // Capture the ROLE cross of the first waypoint for the
                    // label anchor, then materialize the pairs into
                    // physical (x, y) — matches the heap ordering.
                    let first_wp_cross = waypoint_scratch[0].0;
                    for wp in waypoint_scratch[..waypoint_count].iter_mut() {
                        *wp = A::materialize(wp.1, wp.0);
                    }
                    if matches!(A::FLOW_AXIS, crate::ir::FlowAxis::X) {
                        let mut wp_min = usize::MAX;
                        let mut wp_max = 0usize;
                        for &(_, wy) in &waypoint_scratch[..waypoint_count] {
                            wp_min = wp_min.min(wy);
                            wp_max = wp_max.max(wy);
                        }
                        waypoint_y_bounds = Some((wp_min, wp_max));
                    }
                    if let Some((start, len)) =
                        builder.add_waypoints(&waypoint_scratch[..waypoint_count])
                    {
                        first_wp_x = Some(first_wp_cross);
                        let start_offset = (edge_start_row + slot).saturating_sub(1);

                        // Record the initial corner Y (first segment routing)
                        let initial_corner_y = band_trailing + 1 + start_offset;
                        let src_lvl = src_level as usize;
                        if src_lvl < level_routing_floor.len() {
                            level_routing_floor[src_lvl] =
                                level_routing_floor[src_lvl].max(initial_corner_y);
                        }

                        EdgePathArena::MultiSegment {
                            waypoints_start: start,
                            waypoints_len: len,
                            start_offset,
                        }
                    } else {
                        let hy = band_trailing + edge_start_row + slot;
                        let src_lvl = src_level as usize;
                        if src_lvl < level_routing_floor.len() {
                            level_routing_floor[src_lvl] = level_routing_floor[src_lvl].max(hy);
                        }
                        EdgePathArena::Corner { bend_at: hy }
                    }
                }
            } else {
                let hy = band_trailing + edge_start_row + slot;
                let src_lvl = src_level as usize;
                if src_lvl < level_routing_floor.len() {
                    level_routing_floor[src_lvl] = level_routing_floor[src_lvl].max(hy);
                }
                EdgePathArena::Corner { bend_at: hy }
            }
        } else {
            EdgePathArena::Direct
        };

        // Store edge label if present
        let edge_label_text = graph.edge_label(edge_idx);
        let (e_label_offset, e_label_len, e_label_x, e_label_y) = if !edge_label_text.is_empty() {
            if let Some((offset, len)) = builder.add_edge_label(edge_label_text) {
                // First row below the source level's routing block — shared
                // with the heap backend so label rows cannot drift. (A label
                // on a routing row collides with `─` and is skipped by the
                // renderer's collision check.)
                let l_y = band_trailing
                    + edge_label_offset(temps.level_slot_next[src_level as usize] as usize);
                let edge_x_at_label = match &path {
                    EdgePathArena::Direct => eff_from_x,
                    EdgePathArena::Corner { bend_at } => {
                        if l_y <= *bend_at {
                            eff_from_x
                        } else {
                            eff_to_x
                        }
                    }
                    #[cfg(feature = "ports")]
                    EdgePathArena::Orthogonal { .. } => explicit_label_col.unwrap_or(eff_from_x),
                    EdgePathArena::MultiSegment { start_offset, .. } => {
                        // Anchor on the first waypoint once the label row
                        // is past the first bend (heap-backend rule).
                        let bend_at = band_trailing + 1 + *start_offset;
                        match first_wp_x {
                            Some(wx) if l_y > bend_at => wx,
                            _ => eff_from_x,
                        }
                    }
                    EdgePathArena::SideChannel {
                        channel_at,
                        span_start,
                        ..
                    } => {
                        if l_y < *span_start {
                            eff_from_x
                        } else {
                            *channel_at
                        }
                    }
                    // Spline: fall back to source X
                    _ => eff_from_x,
                };
                // Materialize the anchor; label text spreads along
                // PHYSICAL x whichever role that is (temp/08 D9), so
                // centering (chars, not bytes — parity with the heap
                // backend for non-ASCII labels) and the canvas clamp
                // apply afterwards.
                let (anchor_x, anchor_y) = A::materialize(l_y, edge_x_at_label);
                let label_len_with_quotes = edge_label_text.chars().count() + 2;
                let l_x = anchor_x.saturating_sub(label_len_with_quotes / 2);
                let l_x = if l_x + label_len_with_quotes > canvas_width {
                    canvas_width.saturating_sub(label_len_with_quotes)
                } else {
                    l_x
                };
                (offset, len, l_x, anchor_y)
            } else {
                (0, 0, 0, 0)
            }
        } else {
            (0, 0, 0, 0)
        };

        // ── Materialization: role pairs → physical (x, y) — matches
        // the heap backend; the label logic above consumed role values.
        let (from_x_p, from_y_p) = A::materialize(from_y, eff_from_x);
        let (to_x_p, to_y_p) = A::materialize(to_y, eff_to_x);
        // Physical y-span for band culling: between the node faces in
        // Vertical (ink starts below/above them), the port-row span in
        // Horizontal (the P3 banding audit revisits).
        let (mut e_min_y, mut e_max_y) = if matches!(A::FLOW_AXIS, crate::ir::FlowAxis::Y) {
            (from_y_p + 1, to_y_p.saturating_sub(1))
        } else {
            (from_y_p.min(to_y_p), from_y_p.max(to_y_p))
        };
        // Waypoint rows can exceed the port span (slices review).
        if let Some((wp_min, wp_max)) = waypoint_y_bounds {
            e_min_y = e_min_y.min(wp_min);
            e_max_y = e_max_y.max(wp_max);
        }
        // An explicit polyline's ink can lie beyond both faces (a
        // detour): its span is the endpoints and every bend.
        if let Some((b_min, b_max)) = ortho_y_bounds {
            e_min_y = e_min_y.min(b_min).min(from_y_p.min(to_y_p));
            e_max_y = e_max_y.max(b_max).max(from_y_p.max(to_y_p));
        }
        // Attachments, by DECLARED end (matches heap): the requested
        // side, and the face the end actually took.
        let (from_port, to_port) = {
            use super::ports::{EndRole, PortAttachment};
            let (layout_src_face, layout_dst_face) = match detour {
                Some(d) => (
                    if d.src_lane != usize::MAX {
                        d.src_face
                    } else {
                        EndRole::Source.auto_face()
                    },
                    if d.dst_lane != usize::MAX {
                        d.dst_face
                    } else {
                        EndRole::Target.auto_face()
                    },
                ),
                None => (EndRole::Source.auto_face(), EndRole::Target.auto_face()),
            };
            let (from_face, to_face) = if is_back {
                (layout_dst_face, layout_src_face)
            } else {
                (layout_src_face, layout_dst_face)
            };
            (
                PortAttachment {
                    requested: src_side,
                    side: from_face.physical(A::FLOW_AXIS, level_flipped),
                },
                PortAttachment {
                    requested: dst_side,
                    side: to_face.physical(A::FLOW_AXIS, level_flipped),
                },
            )
        };
        builder.add_edge(LayoutEdgeArena {
            flow_axis: A::FLOW_AXIS,
            from_id,
            to_id,
            from_port,
            to_port,
            from_x: from_x_p,
            from_y: from_y_p,
            to_x: to_x_p,
            to_y: to_y_p,
            directed: true,
            reversed: is_back,
            path,
            edge_index: edge_idx,
            label_offset: e_label_offset,
            label_len: e_label_len,
            label_x: e_label_x,
            label_y: e_label_y,
            min_y: e_min_y,
            max_y: e_max_y,
        });
    }

    // Step 10: Compute subgraph bounding boxes and add to builder
    if graph.has_subgraphs() {
        let (sg_max_right, sg_max_bottom) = compute_sg_bounding_boxes::<A>(
            graph,
            temps.real_coords,
            temps.level_offsets,
            total_height,
            temps.sg_depths,
            temps.sg_envelopes,
            temps.level_routing_floor,
            &mut builder,
        );
        // The canvas must cover every border on BOTH physical axes: a
        // label-widened cluster box can extend past the node extent
        // `canvas_width` was derived from, and a materialized
        // Horizontal box (or a Vertical bottom border pushed off a
        // routing row) can extend past the height.
        if sg_max_right + 1 > canvas_width || sg_max_bottom > canvas_height {
            builder.set_dimensions(
                canvas_width.max(sg_max_right + 1),
                canvas_height.max(sg_max_bottom),
            );
        }
    }

    #[cfg(feature = "layout-vertical")]
    if config.direction == crate::graph::Direction::BottomUp {
        // Physical-space contract: IR coordinates match rendered cells.
        // Runs after the final set_dimensions — the mirror uses the height.
        builder.flip_vertical();
    }

    // RL is LR mirrored on x — the same contract, other axis. Gated
    // on the PROFILE (see the heap twin): mirroring a vertical layout
    // would be neither direction. Also pre-build, after the final
    // set_dimensions.
    #[cfg(feature = "layout-horizontal")]
    if config.direction == crate::graph::Direction::RightLeft
        && matches!(A::FLOW_AXIS, crate::ir::FlowAxis::X)
    {
        builder.flip_horizontal();
    }

    let mut ir = builder.build();
    ir.set_direction(config.direction);
    Ok(ir)
}

// Helpers for CSR layout (parallel implementation for CsrGraph)

fn alloc_layout_temps_csr<'b>(
    arena: &'b mut Arena<'_>,
    node_count: usize,
    edge_count: usize,
    sg_count: usize,
    node_levels: &'b mut [Idx],
    depth: usize,
    total_dummies: usize,
    max_level_width: usize,
    port_request_cap: usize,
    port_cross_len: usize,
    budget: super::ports::DetourBudget,
) -> Option<LayoutTemps<'b>> {
    // Per-level buffers hold exactly the graph's real depth (computed
    // by the caller before this allocation) — no fixed cap, no waste.
    let max_levels = depth.max(1);
    // Exact virtual-node counts, computed by the caller from level
    // spans — no estimates, no silent caps.
    let max_vnodes = node_count + total_dummies;
    // `medians` is indexed by position-within-level (level width);
    // `positions` doubles as a node-index → level-position map, so it
    // must span every node.
    let max_median_size = max_level_width.max(1);
    let max_positions_size = node_count.max(1);
    let max_dummy_waypoints = total_dummies.max(1);

    // Lane-pass buffers (temp/09 P4), sized from the same exact counts
    // the pass will see at runtime; the shared budget bounds them, and a
    // disabled pass allocates nothing.
    let lane_on = crate::algorithms::sugiyama::geometry::lane_pass_enabled(
        max_levels,
        edge_count,
        total_dummies,
    );
    let lane_gaps = max_levels.saturating_sub(1);
    let lane_chain_n = edge_count.min(total_dummies);
    let lane_comm_n = total_dummies + lane_chain_n;
    let lane_span_n = crate::algorithms::sugiyama::geometry::LANE_SPAN_CAP
        .min(edge_count + lane_comm_n + node_count + sg_count * max_levels + 16)
        * 2;
    let lane_cand_n = crate::algorithms::sugiyama::geometry::LANE_CAND_CAP.min(
        4 * (edge_count + lane_comm_n)
            + 2 * (node_count + sg_count * max_levels)
            + 8 * max_levels
            + 16,
    );
    let (lane_fixed_offsets_ptr, _) = if lane_on {
        arena.alloc_raw_uninit::<usize>(lane_gaps + 1)?
    } else {
        (core::ptr::null_mut(), 0)
    };
    let (lane_fixed_ptr, _) = if lane_on {
        arena.alloc_raw_uninit::<crate::algorithms::sugiyama::geometry::GapClaim>(edge_count)?
    } else {
        (core::ptr::null_mut(), 0)
    };
    let (lane_committed_offsets_ptr, _) = if lane_on {
        arena.alloc_raw_uninit::<usize>(lane_gaps + 1)?
    } else {
        (core::ptr::null_mut(), 0)
    };
    let (lane_cursors_ptr, _) = if lane_on {
        arena.alloc_raw_uninit::<usize>(lane_gaps.max(1))?
    } else {
        (core::ptr::null_mut(), 0)
    };
    let (lane_committed_ptr, _) = if lane_on {
        arena.alloc_raw_uninit::<crate::algorithms::sugiyama::geometry::GapClaim>(lane_comm_n)?
    } else {
        (core::ptr::null_mut(), 0)
    };
    let (lane_chains_ptr, _) = if lane_on {
        arena.alloc_raw_uninit::<(usize, usize, usize)>(lane_chain_n.max(1))?
    } else {
        (core::ptr::null_mut(), 0)
    };
    let (lane_spans_ptr, _) = if lane_on {
        arena.alloc_raw_uninit::<crate::algorithms::sugiyama::geometry::CrossSpan>(lane_span_n)?
    } else {
        (core::ptr::null_mut(), 0)
    };
    let (lane_cands_ptr, _) = if lane_on {
        arena.alloc_raw_uninit::<usize>(lane_cand_n)?
    } else {
        (core::ptr::null_mut(), 0)
    };
    let (lane_cand_offsets_ptr, _) = if lane_on {
        arena.alloc_raw_uninit::<usize>(max_levels + 1)?
    } else {
        (core::ptr::null_mut(), 0)
    };
    let (lane_dp_ptr, _) = if lane_on {
        arena.alloc_raw_uninit::<LaneDpEntry>(lane_cand_n)?
    } else {
        (core::ptr::null_mut(), 0)
    };

    let (edge_indices_ptr, _) = arena.alloc_raw_uninit::<(Idx, Idx)>(edge_count)?;
    let (vlevel_offsets_ptr, _) = arena.alloc_raw_uninit::<Idx>(max_levels + 1)?;
    let (level_counts_ptr, _) = arena.alloc_raw_uninit::<Idx>(max_levels)?;
    let (vnode_data_ptr, _) = arena.alloc_raw_uninit::<Idx>(max_vnodes * 2)?;
    let (x_coords_ptr, _) = arena.alloc_raw_uninit::<Coord>(max_vnodes)?;
    let (widths_ptr, _) = arena.alloc_raw_uninit::<Coord>(max_vnodes)?;
    let (real_coords_ptr, _) =
        arena.alloc_raw_uninit::<(usize, usize, usize, usize)>(node_count)?;
    let (dummy_offsets_ptr, _) = arena.alloc_raw_uninit::<Idx>(edge_count + 1)?;
    let (dummy_data_ptr, _) = arena.alloc_raw_uninit::<(Idx, Coord)>(max_dummy_waypoints)?;
    let (medians_ptr, _) = arena.alloc_raw_uninit::<(Idx, u32)>(max_median_size)?;
    let (positions_ptr, _) = arena.alloc_raw_uninit::<Idx>(max_positions_size)?;

    // Optimize allocs: boolean array
    // `bool` demands 0/1 bytes — allocate zeroed so the typed slice is
    // valid from the moment it exists (arena backing is arbitrary bytes).
    let (node_is_source_ptr, _) = arena.alloc_raw::<bool>(node_count)?;
    // Counters per level
    let (source_counts_ptr, _) = arena.alloc_raw_uninit::<Idx>(max_levels + 1)?;
    let (dummy_counts_ptr, _) = arena.alloc_raw_uninit::<Idx>(max_levels + 1)?;
    let (level_y_offsets_ptr, _) = arena.alloc_raw_uninit::<usize>(max_levels + 2)?;
    // Node slots
    let (node_slots_ptr, _) = arena.alloc_raw_uninit::<usize>(node_count)?;
    // Next slot counters — one more slot level when ports are declared:
    // the rows above the first level, for its upward exits.
    #[cfg(feature = "ports")]
    let slot_levels = max_levels + 1 + usize::from(budget.any());
    #[cfg(not(feature = "ports"))]
    let slot_levels = max_levels + 1;
    let (level_slot_next_ptr, _) = arena.alloc_raw_uninit::<Idx>(slot_levels)?;
    // Slot interval pool + per-(level, slot) list heads/tails
    // Detour runs add up to five intervals per DETOURING edge (arrow
    // reservation, true bus extent, up-run, first run, below-run).
    #[cfg(feature = "ports")]
    let slot_pool_size = 2 * edge_count + 1 + 5 * budget.edges;
    #[cfg(not(feature = "ports"))]
    let slot_pool_size = 2 * edge_count + 1;
    let slot_list_size = slot_levels * MAX_SLOTS_PER_LEVEL;
    let (slot_pool_ptr, _) = arena.alloc_raw_uninit::<(usize, usize, usize)>(slot_pool_size)?;
    let (slot_heads_ptr, _) = arena.alloc_raw_uninit::<usize>(slot_list_size)?;
    let (slot_tails_ptr, _) = arena.alloc_raw_uninit::<usize>(slot_list_size)?;
    let (level_dummy_next_ptr, _) = arena.alloc_raw_uninit::<Idx>(max_levels + 1)?;
    let (level_labeled_src_ptr, _) = arena.alloc_raw_uninit::<Idx>(max_levels + 1)?;
    let (two_cycle_order_ptr, _) = arena.alloc_raw_uninit::<Idx>(edge_count)?;
    let (edge_in_two_cycle_ptr, _) = arena.alloc_raw::<bool>(edge_count)?;
    // Port scratch, carved only for a graph that declared ports.
    // Requests are zero-initialized (all-zero is a valid request:
    // node 0, the first face, the first role) — never read uninit.
    let (port_requests_ptr, _) = if port_request_cap > 0 {
        arena.alloc_raw::<super::ports::FaceRequest>(port_request_cap)?
    } else {
        (core::ptr::null_mut(), 0)
    };
    let (port_cross_ptr, _) = if port_cross_len > 0 {
        arena.alloc_raw_uninit::<(usize, usize)>(port_cross_len)?
    } else {
        (core::ptr::null_mut(), 0)
    };
    // Sparse detour scratch, sized by the budget: plans and node flags
    // for the detouring edges and nodes, blockers on their levels, the
    // head-on records at those nodes, jog blocks for the chains.
    #[cfg(feature = "ports")]
    let (detour_plans_ptr, _) = if budget.edges > 0 {
        arena.alloc_raw_uninit::<(usize, super::ports::Detour)>(budget.edges)?
    } else {
        (core::ptr::null_mut(), 0)
    };
    #[cfg(feature = "ports")]
    let jog_cap = if budget.any() { total_dummies } else { 0 };
    #[cfg(feature = "ports")]
    let (jog_blocks_ptr, _) = if jog_cap > 0 {
        arena.alloc_raw_uninit::<(usize, usize, usize, usize)>(jog_cap)?
    } else {
        (core::ptr::null_mut(), 0)
    };
    #[cfg(feature = "ports")]
    let (lane_blockers_ptr, _) = if budget.blockers > 0 {
        arena.alloc_raw_uninit::<(usize, usize, usize, usize)>(budget.blockers)?
    } else {
        (core::ptr::null_mut(), 0)
    };
    #[cfg(not(feature = "ports"))]
    let _ = budget;
    // Explicit polylines stage two bends per run — the detour's three
    // runs plus one per jogging level.
    #[cfg(feature = "ports")]
    let waypoint_scratch_n = if budget.any() {
        2 * max_levels + 8
    } else {
        max_levels + 1
    };
    #[cfg(not(feature = "ports"))]
    let waypoint_scratch_n = max_levels + 1;
    let (waypoint_scratch_ptr, _) = arena.alloc_raw_uninit::<(usize, usize)>(waypoint_scratch_n)?;
    let (level_vdummy_counts_ptr, _) = arena.alloc_raw_uninit::<Idx>(max_levels + 1)?;
    let (level_max_extents_ptr, _) = arena.alloc_raw_uninit::<usize>(max_levels + 1)?;
    let (level_routing_floor_ptr, _) = arena.alloc_raw_uninit::<usize>(max_levels + 1)?;

    // Subgraph temporaries (0-length if no subgraphs)
    let sg_alloc = sg_count.max(1); // avoid 0-length allocations
    let (sg_ranges_ptr, _) = if sg_count > 0 {
        arena.alloc_raw_uninit::<(usize, usize)>(sg_alloc)?
    } else {
        (core::ptr::null_mut(), 0)
    };
    let (sg_depths_ptr, _) = if sg_count > 0 {
        arena.alloc_raw_uninit::<usize>(sg_alloc)?
    } else {
        (core::ptr::null_mut(), 0)
    };
    let (sg_envelopes_ptr, _) = if sg_count > 0 {
        arena.alloc_raw_uninit::<(usize, usize, usize, usize)>(sg_alloc)?
    } else {
        (core::ptr::null_mut(), 0)
    };
    let (sg_y_extras_ptr, _) = if sg_count > 0 {
        arena.alloc_raw_uninit::<usize>(max_levels + 1)?
    } else {
        (core::ptr::null_mut(), 0)
    };
    let (sg_frontier_a_ptr, _) = if sg_count > 0 {
        arena.alloc_raw_uninit::<usize>(max_levels + 2)?
    } else {
        (core::ptr::null_mut(), 0)
    };
    let (sg_frontier_b_ptr, _) = if sg_count > 0 {
        arena.alloc_raw_uninit::<usize>(max_levels + 2)?
    } else {
        (core::ptr::null_mut(), 0)
    };

    unsafe {
        Some(LayoutTemps {
            node_levels,
            edge_indices: core::slice::from_raw_parts_mut(edge_indices_ptr, edge_count),
            vlevel_offsets: core::slice::from_raw_parts_mut(vlevel_offsets_ptr, max_levels + 1),
            level_counts: core::slice::from_raw_parts_mut(level_counts_ptr, max_levels),
            vnode_data: core::slice::from_raw_parts_mut(vnode_data_ptr, max_vnodes * 2),
            x_coords: core::slice::from_raw_parts_mut(x_coords_ptr, max_vnodes),
            widths: core::slice::from_raw_parts_mut(widths_ptr, max_vnodes),
            real_coords: core::slice::from_raw_parts_mut(real_coords_ptr, node_count),
            dummy_offsets: core::slice::from_raw_parts_mut(dummy_offsets_ptr, edge_count + 1),

            node_is_source: core::slice::from_raw_parts_mut(node_is_source_ptr, node_count),
            source_counts: core::slice::from_raw_parts_mut(source_counts_ptr, max_levels + 1),
            dummy_counts: core::slice::from_raw_parts_mut(dummy_counts_ptr, max_levels + 1),
            level_offsets: core::slice::from_raw_parts_mut(level_y_offsets_ptr, max_levels + 2),
            node_slots: core::slice::from_raw_parts_mut(node_slots_ptr, node_count),
            level_slot_next: core::slice::from_raw_parts_mut(level_slot_next_ptr, slot_levels),
            level_labeled_src: core::slice::from_raw_parts_mut(
                level_labeled_src_ptr,
                max_levels + 1,
            ),
            two_cycle_order: core::slice::from_raw_parts_mut(two_cycle_order_ptr, edge_count),
            edge_in_two_cycle: core::slice::from_raw_parts_mut(edge_in_two_cycle_ptr, edge_count),
            port_requests: if port_request_cap > 0 {
                core::slice::from_raw_parts_mut(port_requests_ptr, port_request_cap)
            } else {
                &mut []
            },
            port_cross: if port_cross_len > 0 {
                core::slice::from_raw_parts_mut(port_cross_ptr, port_cross_len)
            } else {
                &mut []
            },
            #[cfg(feature = "ports")]
            detour_plans: if budget.edges > 0 {
                core::slice::from_raw_parts_mut(detour_plans_ptr, budget.edges)
            } else {
                &mut []
            },
            #[cfg(feature = "ports")]
            jog_blocks: if jog_cap > 0 {
                core::slice::from_raw_parts_mut(jog_blocks_ptr, jog_cap)
            } else {
                &mut []
            },
            #[cfg(feature = "ports")]
            lane_blockers: if budget.blockers > 0 {
                core::slice::from_raw_parts_mut(lane_blockers_ptr, budget.blockers)
            } else {
                &mut []
            },
            slot_pool: core::slice::from_raw_parts_mut(slot_pool_ptr, slot_pool_size),
            slot_heads: core::slice::from_raw_parts_mut(slot_heads_ptr, slot_list_size),
            slot_tails: core::slice::from_raw_parts_mut(slot_tails_ptr, slot_list_size),
            level_dummy_next: core::slice::from_raw_parts_mut(level_dummy_next_ptr, max_levels + 1),
            waypoint_scratch: core::slice::from_raw_parts_mut(
                waypoint_scratch_ptr,
                waypoint_scratch_n,
            ),
            level_max_extents: core::slice::from_raw_parts_mut(
                level_max_extents_ptr,
                max_levels + 1,
            ),
            level_vdummy_counts: core::slice::from_raw_parts_mut(
                level_vdummy_counts_ptr,
                max_levels + 1,
            ),
            level_routing_floor: core::slice::from_raw_parts_mut(
                level_routing_floor_ptr,
                max_levels + 1,
            ),
            dummy_data: core::slice::from_raw_parts_mut(dummy_data_ptr, max_dummy_waypoints),
            medians: core::slice::from_raw_parts_mut(medians_ptr, max_median_size),
            positions: core::slice::from_raw_parts_mut(positions_ptr, max_positions_size),
            sg_ranges: if sg_count > 0 {
                core::slice::from_raw_parts_mut(sg_ranges_ptr, sg_count)
            } else {
                &mut []
            },
            sg_depths: if sg_count > 0 {
                core::slice::from_raw_parts_mut(sg_depths_ptr, sg_count)
            } else {
                &mut []
            },
            lane_fixed_offsets: if lane_on {
                core::slice::from_raw_parts_mut(lane_fixed_offsets_ptr, lane_gaps + 1)
            } else {
                &mut []
            },
            lane_fixed: if lane_on {
                core::slice::from_raw_parts_mut(lane_fixed_ptr, edge_count)
            } else {
                &mut []
            },
            lane_committed_offsets: if lane_on {
                core::slice::from_raw_parts_mut(lane_committed_offsets_ptr, lane_gaps + 1)
            } else {
                &mut []
            },
            lane_cursors: if lane_on {
                core::slice::from_raw_parts_mut(lane_cursors_ptr, lane_gaps.max(1))
            } else {
                &mut []
            },
            lane_committed: if lane_on {
                core::slice::from_raw_parts_mut(lane_committed_ptr, lane_comm_n)
            } else {
                &mut []
            },
            lane_chains: if lane_on {
                core::slice::from_raw_parts_mut(lane_chains_ptr, lane_chain_n.max(1))
            } else {
                &mut []
            },
            lane_spans: if lane_on {
                core::slice::from_raw_parts_mut(lane_spans_ptr, lane_span_n)
            } else {
                &mut []
            },
            lane_cands: if lane_on {
                core::slice::from_raw_parts_mut(lane_cands_ptr, lane_cand_n)
            } else {
                &mut []
            },
            lane_cand_offsets: if lane_on {
                core::slice::from_raw_parts_mut(lane_cand_offsets_ptr, max_levels + 1)
            } else {
                &mut []
            },
            lane_dp: if lane_on {
                core::slice::from_raw_parts_mut(lane_dp_ptr, lane_cand_n)
            } else {
                &mut []
            },
            sg_envelopes: if sg_count > 0 {
                core::slice::from_raw_parts_mut(sg_envelopes_ptr, sg_count)
            } else {
                &mut []
            },
            sg_y_extras: if sg_count > 0 {
                core::slice::from_raw_parts_mut(sg_y_extras_ptr, max_levels + 1)
            } else {
                &mut []
            },
            sg_frontier_a: if sg_count > 0 {
                core::slice::from_raw_parts_mut(sg_frontier_a_ptr, max_levels + 2)
            } else {
                &mut []
            },
            sg_frontier_b: if sg_count > 0 {
                core::slice::from_raw_parts_mut(sg_frontier_b_ptr, max_levels + 2)
            } else {
                &mut []
            },
        })
    }
}

/// Three-color DFS back-edge detection for CsrGraph.
///
/// Identifies back edges (edges pointing to an ancestor on the DFS stack)
/// using a classic three-color algorithm: WHITE → GRAY (on stack) → BLACK (done).
/// All temporaries are allocated from `arena` — no heap allocation.
///
/// Self-loops (from == to) are unconditionally marked as back edges.
fn detect_back_edges_csr(graph: &CsrGraph<'_>, back_edges: &mut [bool], arena: &mut Arena<'_>) {
    let node_count = graph.node_count();
    let edge_count = graph.edge_count();

    for b in back_edges.iter_mut() {
        *b = false;
    }

    if node_count == 0 || edge_count == 0 {
        return;
    }

    // Mark self-loops immediately
    for ei in 0..edge_count {
        let (from, to) = graph.edge(ei);
        if from == to {
            back_edges[ei] = true;
        }
    }

    // Build edge-from CSR: for each source node, the list of outgoing edge indices.
    // Allocate from arena: offsets[node_count+1] + data[edge_count] + color[node_count] + stack[node_count]
    let Some((offsets_ptr, _)) = arena.alloc_raw::<u32>(node_count + 1) else {
        return;
    };
    let Some((edata_ptr, _)) = arena.alloc_raw::<u32>(edge_count) else {
        return;
    };
    let Some((color_ptr, _)) = arena.alloc_raw::<u8>(node_count) else {
        return;
    };
    // Stack entries: (node_index as u32, edge_iterator_position as u32)
    let Some((stack_ptr, _)) = arena.alloc_raw_uninit::<(u32, u32)>(node_count) else {
        return;
    };

    let offsets = unsafe { core::slice::from_raw_parts_mut(offsets_ptr, node_count + 1) };
    let edata = unsafe { core::slice::from_raw_parts_mut(edata_ptr, edge_count) };
    let color = unsafe { core::slice::from_raw_parts_mut(color_ptr, node_count) };
    let stack = unsafe { core::slice::from_raw_parts_mut(stack_ptr, node_count) };

    // Build edge-from CSR — count then fill
    // offsets already zeroed by alloc_raw
    for ei in 0..edge_count {
        let (from, _) = graph.edge(ei);
        if from < node_count {
            offsets[from + 1] += 1;
        }
    }
    for i in 1..=node_count {
        offsets[i] += offsets[i - 1];
    }
    // Fill cursors: the DFS stack is idle until the DFS starts, so its
    // first halves serve as per-node `u32` cursors — a node may have any
    // out-degree. (The `u8` color array once served here and wrapped
    // past 255 outgoing edges, corrupting the adjacency of any wider
    // fan-out.) `color` stays zeroed — WHITE — for the DFS.
    for slot in stack.iter_mut() {
        *slot = (0, 0);
    }
    for ei in 0..edge_count {
        let (from, _) = graph.edge(ei);
        if from < node_count {
            let pos = (offsets[from] + stack[from].0) as usize;
            edata[pos] = ei as u32;
            stack[from].0 += 1;
        }
    }

    const WHITE: u8 = 0;
    const GRAY: u8 = 1;
    // const BLACK: u8 = 2;

    // Explicit-stack DFS for each unvisited root
    for start in 0..node_count {
        if color[start] != WHITE {
            continue;
        }
        color[start] = GRAY;
        let mut stack_len: usize = 1;
        stack[0] = (start as u32, 0);

        while stack_len > 0 {
            let (node, ref mut ei_pos) = stack[stack_len - 1];
            let node_idx = node as usize;
            let edge_start = offsets[node_idx] as usize;
            let edge_end = offsets[node_idx + 1] as usize;
            let local_pos = *ei_pos as usize;

            if edge_start + local_pos < edge_end {
                let edge_idx = edata[edge_start + local_pos] as usize;
                stack[stack_len - 1].1 += 1; // advance iterator

                let (_, to) = graph.edge(edge_idx);
                if to < node_count {
                    match color[to] {
                        GRAY => {
                            back_edges[edge_idx] = true;
                        }
                        WHITE => {
                            color[to] = GRAY;
                            if stack_len < stack.len() {
                                stack[stack_len] = (to as u32, 0);
                                stack_len += 1;
                            }
                        }
                        _ => {} // BLACK — fully processed
                    }
                }
            } else {
                // All edges from this node exhausted
                color[node_idx] = 2; // BLACK
                stack_len -= 1;
            }
        }
    }
}

// ── Subgraph layout helpers (CSR) ────────────────────────────────────────

/// Resolve subgraph index for a virtual node in the CSR representation.
/// Real nodes use `graph.node_subgraph()`; dummy nodes return the subgraph
/// Walk the subgraph ancestry to find the root ancestor (the one with no parent).
/// Returns `None` for unaffiliated nodes.
fn root_subgraph_csr(graph: &CsrGraph<'_>, sg_idx: Option<usize>) -> Option<usize> {
    let mut cur = sg_idx;
    let mut root = sg_idx;
    while let Some(idx) = cur {
        if idx >= graph.subgraph_count() {
            break;
        }
        root = cur;
        cur = graph.subgraph_parent(idx);
    }
    root
}

/// Block-partition all levels so nodes in the same root-level subgraph tree
/// are contiguous. This is the CSR equivalent of `block_partition_level` in
/// the heap pipeline.
///
/// Uses a fixed-size scratch buffer (max 512 vnodes per level) to avoid
/// heap allocation.
fn block_partition_levels_csr(
    graph: &CsrGraph<'_>,
    vlevel_offsets: &[Idx],
    vnode_data: &mut [Idx],
    max_level: usize,
) {
    // Max block keys (subgraph roots + 1 for unaffiliated)
    const MAX_BLOCKS: usize = 65;
    const MAX_LEVEL_SIZE: usize = 512;

    // Scratch: (vnode_type, vnode_idx, root_sg_key) per vnode on a level
    // root_sg_key: 0..sg_count for subgraph roots, usize::MAX for unaffiliated
    let mut scratch: [(Idx, Idx, usize); MAX_LEVEL_SIZE] = [(0, 0, 0); MAX_LEVEL_SIZE];

    for level in 0..=max_level {
        if level + 1 >= vlevel_offsets.len() {
            break;
        }
        let start = vlevel_offsets[level] as usize;
        let end = vlevel_offsets[level + 1] as usize;
        let count = end - start;
        if count <= 1 || count > MAX_LEVEL_SIZE {
            continue;
        }

        // Collect vnodes with their root subgraph key
        for i in 0..count {
            let pos = start + i;
            let vtype = vnode_kind(vnode_data, pos);
            let vidx = vnode_payload(vnode_data, pos);
            let sg = vnode_subgraph_csr(graph, vtype, vidx);
            let root_sg = root_subgraph_csr(graph, sg);
            let key = root_sg.unwrap_or(usize::MAX);
            scratch[i] = (vtype, vidx, key);
        }

        // Collect unique block keys and their average position
        let mut block_keys = [usize::MAX; MAX_BLOCKS];
        let mut block_avg = [0.0f64; MAX_BLOCKS];
        let mut block_count = [0usize; MAX_BLOCKS];
        let mut num_blocks = 0usize;

        for i in 0..count {
            let key = scratch[i].2;
            // Find or insert key
            let mut found = usize::MAX;
            for b in 0..num_blocks {
                if block_keys[b] == key {
                    found = b;
                    break;
                }
            }
            if found == usize::MAX {
                if num_blocks >= MAX_BLOCKS {
                    continue;
                }
                found = num_blocks;
                block_keys[num_blocks] = key;
                num_blocks += 1;
            }
            block_avg[found] += i as f64;
            block_count[found] += 1;
        }

        // Compute averages
        for b in 0..num_blocks {
            if block_count[b] > 0 {
                block_avg[b] /= block_count[b] as f64;
            }
        }

        // Sort blocks by average position (insertion sort for small N)
        let mut block_order = [0usize; MAX_BLOCKS];
        for i in 0..num_blocks {
            block_order[i] = i;
        }
        for i in 1..num_blocks {
            let mut j = i;
            while j > 0 && block_avg[block_order[j]] < block_avg[block_order[j - 1]] {
                block_order.swap(j, j - 1);
                j -= 1;
            }
        }

        // Write back in block order
        let mut write_pos = 0usize;
        for bi in 0..num_blocks {
            let block_key = block_keys[block_order[bi]];
            for i in 0..count {
                if scratch[i].2 == block_key {
                    let pos = start + write_pos;
                    vnode_set(vnode_data, pos, scratch[i].0, scratch[i].1);
                    write_pos += 1;
                }
            }
        }
    }
}

/// only if both edge endpoints share the same subgraph.
fn vnode_subgraph_csr(graph: &CsrGraph<'_>, vnode_type: Idx, vnode_idx: Idx) -> Option<usize> {
    if vnode_type == 0 {
        // Real node
        graph.node_subgraph(vnode_idx as usize)
    } else {
        // Dummy node — vnode_idx is edge index
        let (from, to) = graph.edge(vnode_idx as usize);
        let fsg = graph.node_subgraph(from);
        let tsg = graph.node_subgraph(to);
        match (fsg, tsg) {
            (Some(a), Some(b)) if a == b => Some(a),
            _ => None,
        }
    }
}

/// Number of ancestors ABOVE a box (0 for a root box) — the CSR twin
/// of `subgraph::ancestor_count`.
fn sg_ancestors_csr(graph: &CsrGraph<'_>, sg_idx: usize) -> usize {
    let mut n = 0;
    let mut cur = graph.subgraph_parent(sg_idx);
    while let Some(p) = cur {
        n += 1;
        cur = graph.subgraph_parent(p);
    }
    n
}

/// Leading cross-axis margin a node inside `sg` needs (CSR twin of
/// `subgraph::leading_cross_pad`): the immediate box pad, plus one
/// label-side pad per ancestor for non-merging profiles.
fn leading_cross_pad_csr<A: Axis>(graph: &CsrGraph<'_>, sg: Option<usize>) -> usize {
    match sg {
        Some(sg_idx) if !A::NESTED_PADS_MERGE => {
            A::SG_PAD_CROSS.0 + sg_ancestors_csr(graph, sg_idx) * A::PARENT_CHILD_PAD_CROSS.0
        }
        Some(_) => A::SG_PAD_CROSS.0,
        None => 0,
    }
}

/// Insert horizontal subgraph padding into x_coords.
/// Returns the updated max_width.
fn subgraph_padding_csr<A: Axis>(
    graph: &CsrGraph<'_>,
    vlevel_offsets: &[Idx],
    vnode_data: &[Idx],
    x_coords: &mut [Coord],
    widths: &[Coord],
    max_level: Idx,
    node_spacing: usize,
) -> usize {
    let mut global_max_width = 0usize;

    for level in 0..=max_level as usize {
        if level + 1 >= vlevel_offsets.len() {
            break;
        }
        let start = vlevel_offsets[level] as usize;
        let end = vlevel_offsets[level + 1] as usize;
        if start >= end {
            continue;
        }

        let mut x = 0usize;

        // Left padding: flat cross-axis entry pad if first node is inside any subgraph
        // (matches heap's flat padding — bbox pass handles nesting expansion)
        let first_type = vnode_data.get(start * 2).copied().unwrap_or(0);
        let first_idx = vnode_data.get(start * 2 + 1).copied().unwrap_or(0);
        let first_sg = vnode_subgraph_csr(graph, first_type, first_idx);
        if let Some(sg_idx) = first_sg {
            x += A::SG_PAD_CROSS.0;
            // Non-merging profiles reserve the ancestry chain (matches
            // heap — label-bearing pads cannot share cells).
            if !A::NESTED_PADS_MERGE {
                x += sg_ancestors_csr(graph, sg_idx) * A::PARENT_CHILD_PAD_CROSS.0;
            }
        }

        for pos in start..end {
            if pos > start {
                let prev_type = vnode_kind(vnode_data, pos - 1);
                let prev_idx = vnode_payload(vnode_data, pos - 1);
                let curr_type = vnode_kind(vnode_data, pos);
                let curr_idx = vnode_payload(vnode_data, pos);
                let prev_sg = vnode_subgraph_csr(graph, prev_type, prev_idx);
                let curr_sg = vnode_subgraph_csr(graph, curr_type, curr_idx);
                if prev_sg != curr_sg {
                    // Flat padding per boundary transition (matches heap):
                    // one exit margin + one entry margin. The bbox pass
                    // handles depth-proportional expansion (merging
                    // profiles); non-merging ones reserve the chains.
                    x += A::SG_PAD_CROSS.1 + A::SG_PAD_CROSS.0;
                    if !A::NESTED_PADS_MERGE {
                        if let Some(id) = prev_sg {
                            x += sg_ancestors_csr(graph, id) * A::PARENT_CHILD_PAD_CROSS.1;
                        }
                        if let Some(id) = curr_sg {
                            x += sg_ancestors_csr(graph, id) * A::PARENT_CHILD_PAD_CROSS.0;
                        }
                    }
                }
            }
            if pos < x_coords.len() {
                x_coords[pos] = x as Coord;
            }
            let w = widths.get(pos).copied().unwrap_or(3) as usize;
            x += w + node_spacing;
        }

        // Right padding: flat cross-axis exit pad if last node is inside any subgraph
        let last_pos = end - 1;
        let last_type = vnode_kind(vnode_data, last_pos);
        let last_idx = vnode_payload(vnode_data, last_pos);
        let last_sg = vnode_subgraph_csr(graph, last_type, last_idx);
        let right_extra = match last_sg {
            Some(sg_idx) if !A::NESTED_PADS_MERGE => {
                A::SG_PAD_CROSS.1 + sg_ancestors_csr(graph, sg_idx) * A::PARENT_CHILD_PAD_CROSS.1
            }
            Some(_) => A::SG_PAD_CROSS.1,
            None => 0,
        };

        // Compute level width
        let mut level_max = 0usize;
        for pos in start..end {
            let px = x_coords.get(pos).copied().unwrap_or(0) as usize;
            let pw = widths.get(pos).copied().unwrap_or(3) as usize;
            let r = px + pw;
            if r > level_max {
                level_max = r;
            }
        }
        level_max += right_extra;
        if level_max > global_max_width {
            global_max_width = level_max;
        }
    }

    global_max_width
}

// ── X-refinement (CSR) ───────────────────────────────────────────────────

/// Compute median center-x of connected neighbors on an adjacent level.
/// Uses CSR adjacency lists (children/parents) for efficient neighbor lookup.
fn connected_median_csr(
    graph: &CsrGraph<'_>,
    vnode_data: &[Idx],
    x_coords: &[Coord],
    widths: &[Coord],
    pos: usize,
    adj_start: usize,
    adj_end: usize,
) -> Option<Coord> {
    let vtype = vnode_kind(vnode_data, pos);
    let vidx = vnode_payload(vnode_data, pos);
    let mut positions = [0 as Coord; 32];
    let mut pcount = 0usize;

    if vtype == 0 {
        // Real node — check children and parents on adj level
        let node_idx = vidx as usize;

        // Collect neighbor node indices and edge indices
        let children = graph.children(node_idx);
        let parents = graph.parents(node_idx);

        // Check real node neighbors and their dummy chain entries on adj level
        for ap in adj_start..adj_end {
            let at = vnode_kind(vnode_data, ap);
            let ai = vnode_payload(vnode_data, ap) as usize;
            if at == 0 {
                // Real node — check if it's a neighbor
                let is_neighbor = children.iter().any(|&c| c as usize == ai)
                    || parents.iter().any(|&p| p as usize == ai);
                if is_neighbor && pcount < 32 {
                    positions[pcount] = x_coords[ap].saturating_add(widths[ap] / 2);
                    pcount += 1;
                }
            } else {
                // Dummy node — check if edge connects to this node
                let edge_idx = ai;
                if edge_idx < graph.edge_count() {
                    let (from, to) = graph.edge(edge_idx);
                    if from == node_idx || to == node_idx {
                        if pcount < 32 {
                            positions[pcount] = x_coords[ap].saturating_add(widths[ap] / 2);
                            pcount += 1;
                        }
                    }
                }
            }
        }
    } else {
        // Dummy node — find same-edge dummy or real endpoints on adj level
        let edge_idx = vidx as usize;
        if edge_idx < graph.edge_count() {
            let (from, to) = graph.edge(edge_idx);
            for ap in adj_start..adj_end {
                let at = vnode_kind(vnode_data, ap);
                let ai = vnode_payload(vnode_data, ap) as usize;
                if (at == 1 && ai == edge_idx) || (at == 0 && (ai == from || ai == to)) {
                    if pcount < 32 {
                        positions[pcount] = x_coords[ap].saturating_add(widths[ap] / 2);
                        pcount += 1;
                    }
                }
            }
        }
    }

    if pcount == 0 {
        return None;
    }
    positions[..pcount].sort_unstable();
    let median = if pcount % 2 == 1 {
        positions[pcount / 2]
    } else {
        let mid = pcount / 2;
        (positions[mid - 1] + positions[mid]) / 2
    };
    Some(median)
}

/// Gap between adjacent nodes, accounting for subgraph boundaries.
fn gap_between_csr<A: Axis>(
    graph: &CsrGraph<'_>,
    vnode_data: &[Idx],
    pos_a: usize,
    pos_b: usize,
    node_spacing: Coord,
) -> Coord {
    let a_type = vnode_kind(vnode_data, pos_a);
    let a_idx = vnode_payload(vnode_data, pos_a);
    let b_type = vnode_kind(vnode_data, pos_b);
    let b_idx = vnode_payload(vnode_data, pos_b);
    let a_sg = vnode_subgraph_csr(graph, a_type, a_idx);
    let b_sg = vnode_subgraph_csr(graph, b_type, b_idx);
    if a_sg != b_sg && (a_sg.is_some() || b_sg.is_some()) {
        A::SG_GAP_CROSS as Coord
    } else {
        node_spacing
    }
}

/// Reclaim horizontal slack on each level (post-shift tightening).
/// CSR twin of `subgraph::tighten_levels`.
///
/// Sweeps each level in x order and moves every real node toward the
/// median center of its connected neighbors, strictly bounded by its
/// current level neighbors — so it can never widen a level, and the
/// rightmost node may only move left. `order_scratch` holds the x-order
/// permutation for one level (the `positions` crossing scratch fits).
fn tighten_levels_csr<A: Axis>(
    graph: &CsrGraph<'_>,
    real_coords: &mut [(usize, usize, usize, usize)], // (level, pos, x, width)
    max_level: usize,
    node_spacing: usize,
    order_scratch: &mut [Idx],
) {
    let node_count = graph.node_count().min(real_coords.len());
    if node_count < 2 {
        return;
    }
    // Bound the per-level insertion sort (quadratic in level size).
    const TIGHTEN_MAX_LEVEL_SIZE: usize = 1024;
    // Per-cluster extent snapshot lives on the stack; skip the pass for
    // graphs with more clusters than the snapshot holds (tightening is
    // cosmetic — skipping is always safe).
    const TIGHTEN_MAX_SUBGRAPHS: usize = 128;

    let sg_count = graph.subgraph_count();
    if sg_count > TIGHTEN_MAX_SUBGRAPHS {
        return;
    }

    for _sweep in 0..4 {
        let mut moved = false;

        // Snapshot each immediate cluster's member extent (min x, max right)
        // across all levels. Members may only move within it, so no cluster
        // bounding box can grow — growth would re-overlap sibling boxes that
        // fix_subgraph_overlaps_csr just separated.
        let mut extents = [(usize::MAX, 0usize); TIGHTEN_MAX_SUBGRAPHS];
        for ni in 0..node_count {
            if let Some(sg) = graph.node_subgraph(ni) {
                if sg < TIGHTEN_MAX_SUBGRAPHS {
                    let (_, _, x, w) = real_coords[ni];
                    extents[sg].0 = extents[sg].0.min(x);
                    extents[sg].1 = extents[sg].1.max(x + w);
                }
            }
        }
        for level in 0..=max_level {
            // Collect this level's real nodes into the scratch.
            let mut n = 0usize;
            for node_idx in 0..node_count {
                if real_coords[node_idx].0 == level {
                    if n >= order_scratch.len() || n >= TIGHTEN_MAX_LEVEL_SIZE {
                        n = usize::MAX;
                        break;
                    }
                    order_scratch[n] = node_idx as Idx;
                    n += 1;
                }
            }
            if n == usize::MAX || n == 0 {
                continue;
            }
            // Insertion sort by current x (stable).
            for k in 1..n {
                let mut j = k;
                while j > 0 {
                    let a = order_scratch[j - 1] as usize;
                    let b = order_scratch[j] as usize;
                    if (real_coords[a].2, order_scratch[j - 1])
                        > (real_coords[b].2, order_scratch[j])
                    {
                        order_scratch.swap(j - 1, j);
                        j -= 1;
                    } else {
                        break;
                    }
                }
            }

            for k in 0..n {
                let ni = order_scratch[k] as usize;
                let (_, _, x, w) = real_coords[ni];

                // Median center of connected neighbors (parents + children).
                let mut centers = [0usize; 32];
                let mut count = 0usize;
                for &c in graph.children(ni) {
                    let c = c as usize;
                    if c < node_count && count < 32 {
                        centers[count] = real_coords[c].2 + real_coords[c].3 / 2;
                        count += 1;
                    }
                }
                for &pa in graph.parents(ni) {
                    let pa = pa as usize;
                    if pa < node_count && count < 32 {
                        centers[count] = real_coords[pa].2 + real_coords[pa].3 / 2;
                        count += 1;
                    }
                }
                if count == 0 {
                    continue;
                }
                centers[..count].sort_unstable();
                let target_center = centers[count / 2];
                let desired = target_center.saturating_sub(w / 2);

                let my_sg = graph.node_subgraph(ni);
                let mut min_x = if k == 0 {
                    if my_sg.is_some() {
                        leading_cross_pad_csr::<A>(graph, my_sg)
                    } else {
                        0
                    }
                } else {
                    let prev = order_scratch[k - 1] as usize;
                    let prev_sg = graph.node_subgraph(prev);
                    let gap = if prev_sg != my_sg && (prev_sg.is_some() || my_sg.is_some()) {
                        A::SG_GAP_CROSS
                    } else {
                        node_spacing
                    };
                    real_coords[prev].2 + real_coords[prev].3 + gap
                };
                let mut max_x = if k + 1 < n {
                    let next = order_scratch[k + 1] as usize;
                    let next_sg = graph.node_subgraph(next);
                    let gap = if next_sg != my_sg && (next_sg.is_some() || my_sg.is_some()) {
                        A::SG_GAP_CROSS
                    } else {
                        node_spacing
                    };
                    real_coords[next].2.saturating_sub(gap + w)
                } else {
                    // Rightmost node: never move right (keeps canvas bounded).
                    x
                };
                if let Some(sg) = my_sg {
                    if sg < TIGHTEN_MAX_SUBGRAPHS {
                        let (ext_lo, ext_hi) = extents[sg];
                        min_x = min_x.max(ext_lo);
                        max_x = max_x.min(ext_hi.saturating_sub(w));
                    }
                }
                if max_x < min_x {
                    continue;
                }
                let new_x = desired.clamp(min_x, max_x);
                if new_x != x {
                    real_coords[ni].2 = new_x;
                    moved = true;
                }
            }
        }
        if !moved {
            break;
        }
    }
}

/// Project per-cluster x-envelopes and level ranges into `sg_envelopes`
/// (scratch layout per subgraph: left, right, first_level, last_level),
/// mirroring `compute_sg_bounding_boxes` x-math: member extent,
/// `A::SG_PAD_CROSS`, label minimum width, child → parent expansion
/// (deepest-first via `sg_depths`), label recheck.
fn project_sg_envelopes_csr<A: Axis>(
    graph: &CsrGraph<'_>,
    real_coords: &[(usize, usize, usize, usize)],
    node_count: usize,
    sg_count: usize,
    max_depth: usize,
    sg_envelopes: &mut [(usize, usize, usize, usize)],
    sg_depths: &[usize],
) {
    for e in sg_envelopes[..sg_count].iter_mut() {
        *e = (usize::MAX, 0, usize::MAX, 0);
    }

    for node_idx in 0..node_count {
        let Some(si) = graph.node_subgraph(node_idx) else {
            continue;
        };
        if si >= sg_count {
            continue;
        }
        let (level, _, x, w) = real_coords[node_idx];
        let r = x + w;
        let e = &mut sg_envelopes[si];
        if x < e.0 {
            e.0 = x;
        }
        if r > e.1 {
            e.1 = r;
        }
        // Level range covers self and all ancestors.
        let mut cur = Some(si);
        while let Some(i) = cur {
            if i < sg_count {
                let e = &mut sg_envelopes[i];
                if level < e.2 {
                    e.2 = level;
                }
                if level > e.3 {
                    e.3 = level;
                }
            }
            cur = graph.subgraph_parent(i);
        }
    }

    // Pad + label minimum (mirrors compute_sg_bounding_boxes pass 1.5).
    for si in 0..sg_count {
        let (l, r, _, _) = sg_envelopes[si];
        if l == usize::MAX {
            continue;
        }
        let left = l.saturating_sub(A::SG_PAD_CROSS.0);
        let mut right = r + A::SG_PAD_CROSS.1;
        let min_label_width = A::label_cross_extent(graph.subgraph_label(si));
        if right - left < min_label_width {
            right = left + min_label_width;
        }
        sg_envelopes[si].0 = left;
        sg_envelopes[si].1 = right;
    }

    // Child → parent expansion deepest-first (mirrors pass 2).
    let mut depth = max_depth;
    loop {
        for si in 0..sg_count {
            if sg_depths[si] != depth {
                continue;
            }
            let (cl, cr, _, _) = sg_envelopes[si];
            if cl == usize::MAX {
                continue;
            }
            if let Some(pi) = graph.subgraph_parent(si) {
                if pi >= sg_count {
                    continue;
                }
                // Shared parent-gap rule (geometry.rs): the child bbox
                // already carries its own cross pads; the parent adds
                // only its border column. (This projection previously
                // used the full box pad and silently disagreed with the
                // heap twin by one column per nesting level.)
                let exp_l = cl.saturating_sub(A::PARENT_CHILD_PAD_CROSS.0);
                let exp_r = cr + A::PARENT_CHILD_PAD_CROSS.1;
                let p = &mut sg_envelopes[pi];
                if p.0 == usize::MAX {
                    p.0 = exp_l;
                    p.1 = exp_r;
                } else {
                    if exp_l < p.0 {
                        p.0 = exp_l;
                    }
                    if exp_r > p.1 {
                        p.1 = exp_r;
                    }
                }
            }
        }
        if depth == 0 {
            break;
        }
        depth -= 1;
    }
    for si in 0..sg_count {
        let (l, r, _, _) = sg_envelopes[si];
        if l == usize::MAX {
            continue;
        }
        let min_label_width = A::label_cross_extent(graph.subgraph_label(si));
        if r - l < min_label_width {
            sg_envelopes[si].1 = l + min_label_width;
        }
    }
}

/// Last-resort same-level overlap repair — CSR twin of
/// `subgraph::repair_level_overlaps` (runs after every other x pass on
/// real coordinates, before dummy clearance).
///
/// Sweep each level in x order: push a real node right when it overlaps
/// its predecessor (restoring the pairwise gap rule for that pair), and
/// lift a level's leftmost clustered node back to its leading pad. A
/// layout with neither a node overlap nor a leading-pad violation
/// passes through unchanged. No level-size cap: this pass is a
/// correctness guarantee, not a cosmetic one, so every level is swept.
/// One global `sort_unstable` by `(level, x, index)` over the
/// node-count-sized `positions` scratch makes the whole pass
/// `O(N log N)` — no per-level rescan of all nodes, no allocation.
/// Returns how far the widest right edge grew.
fn repair_level_overlaps_csr<A: Axis>(
    graph: &CsrGraph<'_>,
    real_coords: &mut [(usize, usize, usize, usize)], // (level, pos, x, width)
    node_spacing: usize,
    order_scratch: &mut [Idx],
) -> usize {
    let node_count = graph
        .node_count()
        .min(real_coords.len())
        .min(order_scratch.len());
    if node_count < 2 {
        return 0;
    }

    let right_edge = |coords: &[(usize, usize, usize, usize)], n: usize| {
        let mut m = 0usize;
        for c in coords.iter().take(n) {
            m = m.max(c.2 + c.3);
        }
        m
    };
    let before = right_edge(real_coords, node_count);

    // One pass to fill, one sort: contiguous level runs, x order inside
    // each run (ties by index — deterministic).
    for (slot, i) in order_scratch[..node_count].iter_mut().zip(0..) {
        *slot = i as Idx;
    }
    order_scratch[..node_count].sort_unstable_by_key(|&i| {
        let (level, _, x, _) = real_coords[i as usize];
        (level, x, i)
    });

    let mut run_start = 0usize;
    while run_start < node_count {
        let level = real_coords[order_scratch[run_start] as usize].0;
        let mut run_end = run_start + 1;
        while run_end < node_count && real_coords[order_scratch[run_end] as usize].0 == level {
            run_end += 1;
        }

        // The saturation clamp can park a member below its cluster's
        // leading pad, where the box border would overwrite it; lift the
        // level's leftmost clustered node back to the pad first.
        let first = order_scratch[run_start] as usize;
        let first_sg = graph.node_subgraph(first);
        if first_sg.is_some() {
            let pad = leading_cross_pad_csr::<A>(graph, first_sg);
            if real_coords[first].2 < pad {
                real_coords[first].2 = pad;
            }
        }
        for k in run_start + 1..run_end {
            let prev = order_scratch[k - 1] as usize;
            let ni = order_scratch[k] as usize;
            let prev_end = real_coords[prev].2 + real_coords[prev].3;
            if real_coords[ni].2 < prev_end {
                let a = graph.node_subgraph(prev);
                let b = graph.node_subgraph(ni);
                let gap = if a != b && (a.is_some() || b.is_some()) {
                    A::SG_GAP_CROSS
                } else {
                    node_spacing
                };
                real_coords[ni].2 = prev_end + gap;
            }
        }
        run_start = run_end;
    }

    right_edge(real_coords, node_count).saturating_sub(before)
}

/// Compact root clusters and unaffiliated nodes leftward.
/// CSR twin of `subgraph::compact_clusters`.
///
/// Treats each root cluster as a rigid body and each unaffiliated node
/// as a singleton body, sweeps bodies left-to-right, and shifts each as
/// far left as the per-level frontier allows (envelope↔envelope keeps
/// [`Axis::SIBLING_GAP_CROSS`], envelope↔node 1, node↔node `node_spacing`).
/// Shift-left only. Per-level frontiers use depth-sized caller scratch
/// (no level cap). Body count is capped by a fixed stack table; graphs
/// with more bodies keep the first `MAX_BODIES` and skip the rest (the
/// pass is cosmetic, so partial application is always safe).
///
/// Returns the reclaimed canvas width (conservative minimum of node- and
/// envelope-extent reductions).
#[allow(clippy::too_many_arguments)]
fn compact_clusters_csr<A: Axis>(
    graph: &CsrGraph<'_>,
    real_coords: &mut [(usize, usize, usize, usize)],
    max_level: usize,
    node_spacing: usize,
    sg_envelopes: &mut [(usize, usize, usize, usize)],
    sg_depths: &mut [usize],
    vlevel_offsets: &[Idx],
    vnode_data: &[Idx],
    x_coords: &mut [Coord],
    frontier_env: &mut [usize],
    frontier_node: &mut [usize],
) -> usize {
    let sg_count = graph.subgraph_count();
    if sg_count == 0 || sg_count > sg_envelopes.len() || sg_count > sg_depths.len() {
        return 0;
    }
    let node_count = graph.node_count().min(real_coords.len());
    if node_count == 0 || frontier_env.len() <= max_level || frontier_node.len() <= max_level {
        return 0;
    }

    const MAX_BODIES: usize = 512;

    for i in 0..sg_count {
        sg_depths[i] = graph.sg_chain_depth(Some(i));
    }
    let max_depth = sg_depths[..sg_count].iter().copied().max().unwrap_or(0);

    project_sg_envelopes_csr::<A>(
        graph,
        real_coords,
        node_count,
        sg_count,
        max_depth,
        sg_envelopes,
        sg_depths,
    );

    let before_node_right = real_coords[..node_count]
        .iter()
        .map(|c| c.2 + c.3)
        .max()
        .unwrap_or(0);
    let before_env_right = sg_envelopes[..sg_count]
        .iter()
        .filter(|e| e.0 != usize::MAX)
        .map(|e| e.1)
        .max()
        .unwrap_or(0);

    let root_of = |mut i: usize| -> usize {
        while let Some(p) = graph.subgraph_parent(i) {
            i = p;
        }
        i
    };

    // A dummy belongs to a cluster only when both edge endpoints share
    // the same immediate subgraph (twin of `subgraph::vnode_subgraph`).
    let edge_count = graph.edge_count();
    let dummy_sg = |edge_idx: usize| -> Option<usize> {
        if edge_idx >= edge_count {
            return None;
        }
        let (f, t) = graph.edge(edge_idx);
        match (graph.node_subgraph(f), graph.node_subgraph(t)) {
            (Some(a), Some(b)) if a == b => Some(a),
            _ => None,
        }
    };

    // Bodies in left-to-right order: root clusters, unaffiliated real
    // nodes, and unaffiliated dummy waypoints. Dummies MUST participate —
    // leaving them out lets a cluster slide left over an edge chain, which
    // then renders running along (or on top of) the cluster's border.
    // (left, right, tag, a, b): tag 0 = cluster(a=sg idx), 1 = node(a=node
    // idx), 2 = dummy(a=level, b=pos). Fixed stack table.
    let mut bodies = [(0usize, 0usize, 0usize, 0usize, 0usize); MAX_BODIES];
    let mut n_bodies = 0usize;
    for si in 0..sg_count {
        if graph.subgraph_parent(si).is_none() {
            let (l, r, f, _) = sg_envelopes[si];
            if l == usize::MAX || f == usize::MAX {
                continue;
            }
            if n_bodies >= MAX_BODIES {
                return 0;
            }
            bodies[n_bodies] = (l, r, 0, si, 0);
            n_bodies += 1;
        }
    }
    for node_idx in 0..node_count {
        if graph.node_subgraph(node_idx).is_none() {
            let (_, _, x, w) = real_coords[node_idx];
            if n_bodies >= MAX_BODIES {
                return 0;
            }
            bodies[n_bodies] = (x, x + w, 1, node_idx, 0);
            n_bodies += 1;
        }
    }
    for level in 0..=max_level {
        let start = vlevel_offsets[level] as usize;
        let end = vlevel_offsets[level + 1] as usize;
        for pos in start..end {
            if !vnode_is_dummy(vnode_data, pos) {
                continue;
            }
            let edge_idx = vnode_payload(vnode_data, pos) as usize;
            if dummy_sg(edge_idx).is_some() {
                continue;
            }
            let Some(&x) = x_coords.get(pos) else {
                continue;
            };
            if n_bodies >= MAX_BODIES {
                return 0;
            }
            bodies[n_bodies] = (x as usize, x as usize + A::DUMMY_CROSS, 2, level, pos);
            n_bodies += 1;
        }
    }
    // Insertion sort by (left, right, tag, a).
    for k in 1..n_bodies {
        let mut j = k;
        while j > 0
            && (
                bodies[j - 1].0,
                bodies[j - 1].1,
                bodies[j - 1].2,
                bodies[j - 1].3,
            ) > (bodies[j].0, bodies[j].1, bodies[j].2, bodies[j].3)
        {
            bodies.swap(j - 1, j);
            j -= 1;
        }
    }

    // Per-level frontiers (usize::MAX = none yet), depth-sized scratch.
    let env_right: &mut [usize] = frontier_env;
    let node_right: &mut [usize] = frontier_node;
    env_right.fill(usize::MAX);
    node_right.fill(usize::MAX);

    for &(env_left, env_r, tag, a, b) in bodies[..n_bodies].iter() {
        match tag {
            0 => {
                let (_, _, first, last) = sg_envelopes[a];
                if first == usize::MAX {
                    continue;
                }
                let mut allowed = 0usize;
                for lvl in first..=last.min(max_level) {
                    if env_right[lvl] != usize::MAX {
                        allowed = allowed.max(env_right[lvl] + A::SIBLING_GAP_CROSS);
                    }
                    if node_right[lvl] != usize::MAX {
                        allowed = allowed.max(node_right[lvl] + A::ENVELOPE_CLEARANCE_CROSS);
                    }
                }
                let delta = env_left.saturating_sub(allowed);
                if delta > 0 {
                    for node_idx in 0..node_count {
                        if let Some(si) = graph.node_subgraph(node_idx) {
                            if si < sg_count && root_of(si) == a {
                                real_coords[node_idx].2 -= delta;
                            }
                        }
                    }
                    // Member dummies (both edge endpoints inside this
                    // cluster) are part of the rigid body too.
                    for level in 0..=max_level {
                        let start = vlevel_offsets[level] as usize;
                        let end = vlevel_offsets[level + 1] as usize;
                        for pos in start..end {
                            if !vnode_is_dummy(vnode_data, pos) {
                                continue;
                            }
                            let edge_idx = vnode_payload(vnode_data, pos) as usize;
                            let member = dummy_sg(edge_idx)
                                .is_some_and(|si| si < sg_count && root_of(si) == a);
                            if member {
                                if let Some(x) = x_coords.get_mut(pos) {
                                    *x = x.saturating_sub(delta.min(Coord::MAX as usize) as Coord);
                                }
                            }
                        }
                    }
                }
                let new_right = env_r - delta;
                for lvl in first..=last.min(max_level) {
                    if env_right[lvl] == usize::MAX || env_right[lvl] < new_right {
                        env_right[lvl] = new_right;
                    }
                }
            }
            1 => {
                let (lvl, _, x, w) = real_coords[a];
                if lvl >= env_right.len() {
                    continue;
                }
                let mut allowed = 0usize;
                if env_right[lvl] != usize::MAX {
                    allowed = allowed.max(env_right[lvl] + A::ENVELOPE_CLEARANCE_CROSS);
                }
                if node_right[lvl] != usize::MAX {
                    allowed = allowed.max(node_right[lvl] + node_spacing);
                }
                let delta = x.saturating_sub(allowed);
                if delta > 0 {
                    real_coords[a].2 = x - delta;
                }
                let r = x - delta + w;
                if node_right[lvl] == usize::MAX || node_right[lvl] < r {
                    node_right[lvl] = r;
                }
            }
            _ => {
                let (lvl, pos) = (a, b);
                let Some(&xc) = x_coords.get(pos) else {
                    continue;
                };
                let x = xc as usize;
                let mut allowed = 0usize;
                if env_right[lvl] != usize::MAX {
                    allowed = allowed.max(env_right[lvl] + A::ENVELOPE_CLEARANCE_CROSS);
                }
                if node_right[lvl] != usize::MAX {
                    allowed = allowed.max(node_right[lvl] + node_spacing);
                }
                let delta = x.saturating_sub(allowed);
                if delta > 0 {
                    x_coords[pos] = (x - delta).min(Coord::MAX as usize) as Coord;
                }
                let r = x - delta + A::DUMMY_CROSS;
                if node_right[lvl] == usize::MAX || node_right[lvl] < r {
                    node_right[lvl] = r;
                }
            }
        }
    }

    project_sg_envelopes_csr::<A>(
        graph,
        real_coords,
        node_count,
        sg_count,
        max_depth,
        sg_envelopes,
        sg_depths,
    );
    let after_node_right = real_coords[..node_count]
        .iter()
        .map(|c| c.2 + c.3)
        .max()
        .unwrap_or(0);
    let after_env_right = sg_envelopes[..sg_count]
        .iter()
        .filter(|e| e.0 != usize::MAX)
        .map(|e| e.1)
        .max()
        .unwrap_or(0);
    let node_reclaim = before_node_right.saturating_sub(after_node_right);
    let env_reclaim = before_env_right.saturating_sub(after_env_right);
    node_reclaim.min(env_reclaim)
}

/// Nudge dummy waypoints out of real node spans.
/// CSR twin of `subgraph::nudge_dummies_off_nodes` — see it for the
/// rationale (an edge may cross a subgraph border, never node text).
fn nudge_dummies_off_nodes_csr<A: Axis>(
    graph: &CsrGraph<'_>,
    real_coords: &[(usize, usize, usize, usize)],
    vlevel_offsets: &[Idx],
    vnode_data: &[Idx],
    x_coords: &mut [Coord],
    max_level: usize,
) {
    let node_count = graph.node_count().min(real_coords.len());
    for level in 0..=max_level {
        let start = vlevel_offsets[level] as usize;
        let end = vlevel_offsets[level + 1] as usize;
        for pos in start..end {
            if !vnode_is_dummy(vnode_data, pos) {
                continue;
            }
            let edge_idx = vnode_payload(vnode_data, pos) as usize;
            // The renderer draws this edge's flow segment at
            // x + dummy_draw_offset (axis-profiled; matches heap).
            let off = A::dummy_draw_offset(edge_idx);
            let Some(&x) = x_coords.get(pos) else {
                continue;
            };
            let mut col = x as usize + off;
            for _ in 0..8 {
                let mut hit = None;
                for node_idx in 0..node_count {
                    let (nl, _, nx, nw) = real_coords[node_idx];
                    if nl == level && col >= nx && col < nx + nw {
                        hit = Some((nx, nx + nw));
                        break;
                    }
                }
                let Some((sl, sr)) = hit else {
                    break;
                };
                let go_left = sl > off && (col - sl) < (sr - col);
                col = if go_left { sl - 1 } else { sr };
            }
            if col != x as usize + off {
                x_coords[pos] = ((col - off).min(Coord::MAX as usize)) as Coord;
            }
        }
    }
}

/// Push unaffiliated nodes clear of subgraph bounding-box envelopes
/// (cluster-width feedback). CSR twin of `subgraph::clear_external_overlaps`.
///
/// `subgraph_padding_csr` reserves space per level, but the border later
/// drawn from `compute_sg_bounding_boxes` is a *global* x-envelope: the
/// member extent across all levels, padded, widened to fit the label,
/// and expanded around children. This pass projects that envelope with
/// the same math and pushes overlapping external nodes right of it,
/// iterating (bounded rounds).
///
/// It runs on `real_coords` **after** `fix_subgraph_overlaps_csr` so it
/// sees the same coordinates the bounding boxes are computed from.
/// Only **unaffiliated real nodes** are pushed: members of other clusters
/// are left to the sibling-overlap repair (which moves whole clusters —
/// pushing them individually would stretch their envelope and cascade),
/// and dummies are never pushed (edges crossing a border render with
/// junction glyphs).
///
/// `sg_envelopes` and `sg_depths` are borrowed as scratch (both are
/// recomputed from scratch by their later users); `order_scratch` needs
/// room for the largest level's real-node count.
///
/// Returns the growth of the maximum node right edge (0 if nothing moved).
fn clear_external_overlaps_csr<A: Axis>(
    graph: &CsrGraph<'_>,
    real_coords: &mut [(usize, usize, usize, usize)], // (level, pos, x, width)
    max_level: usize,
    node_spacing: usize,
    sg_envelopes: &mut [(usize, usize, usize, usize)],
    sg_depths: &mut [usize],
    order_scratch: &mut [Idx],
    frontier_touched: &mut [usize],
    frontier_cursors: &mut [usize],
) -> usize {
    let sg_count = graph.subgraph_count();
    if sg_count == 0 || sg_count > sg_envelopes.len() || sg_count > sg_depths.len() {
        return 0;
    }
    let node_count = graph.node_count().min(real_coords.len());
    if node_count == 0 || frontier_touched.len() <= max_level || frontier_cursors.len() <= max_level
    {
        return 0;
    }

    for i in 0..sg_count {
        sg_depths[i] = graph.sg_chain_depth(Some(i));
    }
    let max_depth = sg_depths[..sg_count].iter().copied().max().unwrap_or(0);

    let before_max_right = real_coords[..node_count]
        .iter()
        .map(|c| c.2 + c.3)
        .max()
        .unwrap_or(0);

    for _round in 0..8 {
        project_sg_envelopes_csr::<A>(
            graph,
            real_coords,
            node_count,
            sg_count,
            max_depth,
            sg_envelopes,
            sg_depths,
        );

        // ── Push overlapping unaffiliated nodes right of each envelope ──
        let mut moved = false;
        // Depth-sized scratch: 0 = untouched, 1 = touched.
        let touched: &mut [usize] = frontier_touched;
        touched.fill(0);
        for si in 0..sg_count {
            let (left, right, first, last) = sg_envelopes[si];
            if left == usize::MAX || first == usize::MAX {
                continue;
            }
            let cursors: &mut [usize] = frontier_cursors;
            for c in cursors[first..=last.min(max_level)].iter_mut() {
                *c = right + A::ENVELOPE_CLEARANCE_CROSS;
            }
            for node_idx in 0..node_count {
                if graph.node_subgraph(node_idx).is_some() {
                    continue;
                }
                let (level, _, x, w) = real_coords[node_idx];
                if level < first || level > last || level >= cursors.len() {
                    continue;
                }
                if x < right && x + w > left {
                    real_coords[node_idx].2 = cursors[level];
                    cursors[level] += w + node_spacing;
                    moved = true;
                    touched[level] = 1;
                }
            }
        }
        if !moved {
            break;
        }

        // ── Re-establish min gaps on touched levels (push-right, x order) ──
        // The insertion sort is quadratic in level size, so bound the level
        // width it may run on; realistic subgraph levels are far smaller.
        const GAP_SWEEP_MAX_LEVEL_SIZE: usize = 1024;
        for level in 0..=max_level {
            if touched[level] == 0 {
                continue;
            }
            // Collect this level's real nodes into the scratch.
            let mut n = 0usize;
            for node_idx in 0..node_count {
                if real_coords[node_idx].0 == level {
                    if n >= order_scratch.len() {
                        n = usize::MAX;
                        break;
                    }
                    order_scratch[n] = node_idx as Idx;
                    n += 1;
                }
            }
            if !(2..=GAP_SWEEP_MAX_LEVEL_SIZE).contains(&n) {
                continue;
            }
            // Insertion sort by current x (stable).
            for k in 1..n {
                let mut j = k;
                while j > 0 {
                    let a = order_scratch[j - 1] as usize;
                    let b = order_scratch[j] as usize;
                    if (real_coords[a].2, order_scratch[j - 1])
                        > (real_coords[b].2, order_scratch[j])
                    {
                        order_scratch.swap(j - 1, j);
                        j -= 1;
                    } else {
                        break;
                    }
                }
            }
            for k in 1..n {
                let prev = order_scratch[k - 1] as usize;
                let cur = order_scratch[k] as usize;
                let prev_sg = graph.node_subgraph(prev);
                let cur_sg = graph.node_subgraph(cur);
                let gap = if prev_sg != cur_sg && (prev_sg.is_some() || cur_sg.is_some()) {
                    A::SG_GAP_CROSS
                } else {
                    node_spacing
                };
                let min_x = real_coords[prev].2 + real_coords[prev].3 + gap;
                if real_coords[cur].2 < min_x {
                    real_coords[cur].2 = min_x;
                }
            }
        }
    }

    let after_max_right = real_coords[..node_count]
        .iter()
        .map(|c| c.2 + c.3)
        .max()
        .unwrap_or(0);
    after_max_right.saturating_sub(before_max_right)
}

/// Left margin for a level (H_PAD if first node is in a subgraph).
fn left_margin_csr<A: Axis>(
    graph: &CsrGraph<'_>,
    vnode_data: &[Idx],
    vlevel_offsets: &[Idx],
    level: usize,
) -> Coord {
    if level + 1 >= vlevel_offsets.len() {
        return 0;
    }
    let start = vlevel_offsets[level] as usize;
    let end = vlevel_offsets[level + 1] as usize;
    if start >= end {
        return 0;
    }
    let vtype = vnode_kind(vnode_data, start);
    let vidx = vnode_payload(vnode_data, start);
    let sg = vnode_subgraph_csr(graph, vtype, vidx);
    if sg.is_some() {
        leading_cross_pad_csr::<A>(graph, sg) as Coord
    } else {
        0
    }
}

/// Refine x-coordinates by shifting nodes toward the median position of their
/// connected neighbors on adjacent levels. CSR equivalent of `refine_x_positions`.
fn refine_x_positions_csr<A: Axis>(
    graph: &CsrGraph<'_>,
    vlevel_offsets: &[Idx],
    vnode_data: &[Idx],
    x_coords: &mut [Coord],
    widths: &[Coord],
    max_level: usize,
    node_spacing: Coord,
) {
    const ITERATIONS: usize = 8;

    let num_levels = max_level + 1;
    if num_levels <= 1 {
        return;
    }

    // Helper: apply a single sweep (down or up) for one level
    fn sweep_level<A: Axis>(
        graph: &CsrGraph<'_>,
        vlevel_offsets: &[Idx],
        vnode_data: &[Idx],
        x_coords: &mut [Coord],
        widths: &[Coord],
        level: usize,
        adj_start: usize,
        adj_end: usize,
        node_spacing: Coord,
    ) {
        let start = vlevel_offsets[level] as usize;
        let end = vlevel_offsets[level + 1] as usize;
        let n = end - start;
        if n == 0 {
            return;
        }
        let margin = left_margin_csr::<A>(graph, vnode_data, vlevel_offsets, level);

        // Right-to-left pass
        for i in (0..n).rev() {
            let pos = start + i;
            if let Some(tc) =
                connected_median_csr(graph, vnode_data, x_coords, widths, pos, adj_start, adj_end)
            {
                let my_w = widths[pos];
                let target = tc.saturating_sub(my_w / 2);
                let min_x = if i == 0 {
                    margin
                } else {
                    let prev = start + i - 1;
                    x_coords[prev].saturating_add(widths[prev]).saturating_add(
                        gap_between_csr::<A>(graph, vnode_data, prev, pos, node_spacing),
                    )
                };
                let max_x = if i + 1 < n {
                    let next = start + i + 1;
                    x_coords[next].saturating_sub(my_w.saturating_add(gap_between_csr::<A>(
                        graph,
                        vnode_data,
                        pos,
                        next,
                        node_spacing,
                    )))
                } else {
                    Coord::MAX
                };
                x_coords[pos] = target.max(min_x).min(max_x);
            }
        }

        // Left-to-right pass
        for i in 0..n {
            let pos = start + i;
            if let Some(tc) =
                connected_median_csr(graph, vnode_data, x_coords, widths, pos, adj_start, adj_end)
            {
                let my_w = widths[pos];
                let target = tc.saturating_sub(my_w / 2);
                let min_x = if i == 0 {
                    margin
                } else {
                    let prev = start + i - 1;
                    x_coords[prev].saturating_add(widths[prev]).saturating_add(
                        gap_between_csr::<A>(graph, vnode_data, prev, pos, node_spacing),
                    )
                };
                let max_x = if i + 1 < n {
                    let next = start + i + 1;
                    x_coords[next].saturating_sub(my_w.saturating_add(gap_between_csr::<A>(
                        graph,
                        vnode_data,
                        pos,
                        next,
                        node_spacing,
                    )))
                } else {
                    Coord::MAX
                };
                x_coords[pos] = target.max(min_x).min(max_x);
            }
        }
    }

    for _iter in 0..ITERATIONS {
        // Down sweep: align with parents
        for level in 1..num_levels {
            if level + 1 >= vlevel_offsets.len() {
                break;
            }
            let adj_start = vlevel_offsets[level - 1] as usize;
            let adj_end = vlevel_offsets[level] as usize;
            sweep_level::<A>(
                graph,
                vlevel_offsets,
                vnode_data,
                x_coords,
                widths,
                level,
                adj_start,
                adj_end,
                node_spacing,
            );
        }

        // Up sweep: align with children
        if num_levels >= 2 {
            for level in (0..num_levels - 1).rev() {
                if level + 2 >= vlevel_offsets.len() {
                    break;
                }
                let adj_start = vlevel_offsets[level + 1] as usize;
                let adj_end = if level + 2 < vlevel_offsets.len() {
                    vlevel_offsets[level + 2] as usize
                } else {
                    vlevel_offsets[level + 1] as usize
                };
                sweep_level::<A>(
                    graph,
                    vlevel_offsets,
                    vnode_data,
                    x_coords,
                    widths,
                    level,
                    adj_start,
                    adj_end,
                    node_spacing,
                );
            }
        }
    }
}

// ── X-compaction (CSR) ───────────────────────────────────────────────────

/// Compact subgraph nodes toward their subgraph centroid using cascading push.
/// CSR equivalent of `compact_subgraphs`.
fn compact_subgraphs_csr<A: Axis>(
    graph: &CsrGraph<'_>,
    vlevel_offsets: &[Idx],
    vnode_data: &[Idx],
    x_coords: &mut [Coord],
    widths: &[Coord],
    max_level: usize,
    node_spacing: Coord,
) {
    let sg_count = graph.subgraph_count();
    if sg_count == 0 {
        return;
    }

    // Collect members per subgraph and compute centroid, then push toward it.
    // Use fixed-size arrays since subgraph count is bounded.
    const MAX_SG: usize = 64;
    const MAX_MEMBERS: usize = 512;

    for sg_idx in 0..sg_count.min(MAX_SG) {
        // Collect all vnode positions belonging to this subgraph
        let mut members = [(0usize, 0usize); MAX_MEMBERS]; // (level_idx, flat_pos)
        let mut member_count = 0usize;
        let mut centroid_sum = 0u64;
        let mut centroid_n = 0usize;

        for level in 0..=max_level {
            if level + 1 >= vlevel_offsets.len() {
                break;
            }
            let start = vlevel_offsets[level] as usize;
            let end = vlevel_offsets[level + 1] as usize;
            for pos in start..end {
                let vtype = vnode_kind(vnode_data, pos);
                let vidx = vnode_payload(vnode_data, pos);
                if vnode_subgraph_csr(graph, vtype, vidx) == Some(sg_idx) {
                    if member_count < MAX_MEMBERS {
                        members[member_count] = (level, pos);
                        member_count += 1;
                        // Only count real nodes for centroid
                        if vtype == 0 {
                            let cx = x_coords[pos] as u64 + widths[pos] as u64 / 2;
                            centroid_sum += cx;
                            centroid_n += 1;
                        }
                    }
                }
            }
        }

        if member_count <= 1 {
            continue;
        }

        // Use real-node centroid if available, otherwise all members
        let centroid = if centroid_n > 0 {
            (centroid_sum / centroid_n as u64) as Coord
        } else {
            let sum: u64 = (0..member_count)
                .map(|i| x_coords[members[i].1] as u64 + widths[members[i].1] as u64 / 2)
                .sum();
            (sum / member_count as u64) as Coord
        };

        // Sort members by distance from centroid (farthest first)
        let mut by_dist = [(0usize, 0usize, 0 as Coord); MAX_MEMBERS];
        for i in 0..member_count {
            let (level, pos) = members[i];
            let cx = x_coords[pos].saturating_add(widths[pos] / 2);
            let dist = cx.abs_diff(centroid);
            by_dist[i] = (level, pos, dist);
        }
        // Sort by distance descending (insertion sort)
        for i in 1..member_count {
            let mut j = i;
            while j > 0 && by_dist[j].2 > by_dist[j - 1].2 {
                by_dist.swap(j, j - 1);
                j -= 1;
            }
        }

        for mi in 0..member_count {
            let (level, pos, dist) = by_dist[mi];
            if dist < A::SG_GAP_CROSS as Coord {
                continue;
            }

            let start = vlevel_offsets[level] as usize;
            let end = vlevel_offsets[level + 1] as usize;
            let i = pos - start; // position index within level
            let n = end - start;

            let my_w = widths[pos];
            let my_cx = x_coords[pos].saturating_add(my_w / 2);
            let target_x = centroid.saturating_sub(my_w / 2);

            // Compute constraints
            let min_x =
                if i == 0 {
                    let vtype = vnode_kind(vnode_data, start);
                    let vidx = vnode_payload(vnode_data, start);
                    if vnode_subgraph_csr(graph, vtype, vidx).is_some() {
                        2
                    } else {
                        0
                    }
                } else {
                    let prev = start + i - 1;
                    x_coords[prev].saturating_add(widths[prev]).saturating_add(
                        gap_between_csr::<A>(graph, vnode_data, prev, pos, node_spacing),
                    )
                };
            let max_x = if i + 1 < n {
                let next = start + i + 1;
                x_coords[next].saturating_sub(my_w.saturating_add(gap_between_csr::<A>(
                    graph,
                    vnode_data,
                    pos,
                    next,
                    node_spacing,
                )))
            } else {
                Coord::MAX
            };

            let simple_x = target_x.max(min_x).min(max_x);

            let simple_ok = (my_cx > centroid && simple_x < x_coords[pos])
                || (my_cx < centroid && simple_x > x_coords[pos]);

            if simple_ok {
                x_coords[pos] = simple_x;
            } else if my_cx > centroid && target_x < min_x {
                // Cascading push left
                let vtype = vnode_kind(vnode_data, pos);
                if vtype == 0 {
                    let push_target = (x_coords[pos] + target_x) / 2;
                    x_coords[pos] = push_target;
                    let mut k = i;
                    while k > 0 {
                        let cur = start + k;
                        let prev = start + k - 1;
                        let g = gap_between_csr::<A>(graph, vnode_data, prev, cur, node_spacing);
                        let needed = x_coords[cur].saturating_sub(widths[prev].saturating_add(g));
                        if x_coords[prev] <= needed {
                            break;
                        }
                        let margin = if k - 1 == 0 {
                            let vt = vnode_kind(vnode_data, start);
                            let vi = vnode_payload(vnode_data, start);
                            if vnode_subgraph_csr(graph, vt, vi).is_some() {
                                2
                            } else {
                                0
                            }
                        } else {
                            0
                        };
                        x_coords[prev] = needed.max(margin);
                        k -= 1;
                    }
                }
            } else if my_cx < centroid && target_x > max_x {
                // Cascading push right
                let vtype = vnode_kind(vnode_data, pos);
                if vtype == 0 {
                    let push_target = (x_coords[pos] + target_x) / 2;
                    x_coords[pos] = push_target;
                    let mut k = i;
                    while k + 1 < n {
                        let cur = start + k;
                        let next = start + k + 1;
                        let g = gap_between_csr::<A>(graph, vnode_data, cur, next, node_spacing);
                        let needed = x_coords[cur].saturating_add(widths[cur]).saturating_add(g);
                        if x_coords[next] >= needed {
                            break;
                        }
                        x_coords[next] = needed;
                        k += 1;
                    }
                }
            }
        }
    }
}

// ── Sibling subgraph overlap repair (CSR) ────────────────────────────────

/// Check if `node_idx` belongs to `target_sg` or any of its descendants.
fn node_in_sg_subtree(graph: &CsrGraph<'_>, node_idx: usize, target_sg: usize) -> bool {
    if let Some(mut sg) = graph.node_subgraph(node_idx) {
        loop {
            if sg == target_sg {
                return true;
            }
            match graph.subgraph_parent(sg) {
                Some(p) => sg = p,
                None => return false,
            }
        }
    }
    false
}

/// CSR equivalent of `fix_subgraph_overlaps` in subgraph.rs.
///
/// Detects and fixes horizontal overlaps between sibling subgraph bounding
/// boxes after centering. Uses only pre-allocated scratch buffers (no heap).
///
/// * `sg_envelopes` — scratch for bbox data: `(left, right, shift, 0)`.
/// * `sg_depths` — scratch for nesting depths.
/// * `scratch` — scratch for per-level node sorting (`node_slots`, `>= node_count`).
fn fix_subgraph_overlaps_csr<A: Axis>(
    graph: &CsrGraph<'_>,
    real_coords: &mut [(usize, usize, usize, usize)],
    sg_envelopes: &mut [(usize, usize, usize, usize)],
    sg_depths: &mut [usize],
    scratch: &mut [usize],
) -> usize {
    let sg_count = graph.subgraph_count();
    if sg_count < 2 {
        return 0;
    }
    let node_count = graph.node_count().min(real_coords.len());

    let cross_sg_gap: usize = A::SG_GAP_CROSS;

    // Fill nesting depths
    for i in 0..sg_count {
        sg_depths[i] = graph.sg_chain_depth(Some(i));
    }
    let max_depth = sg_depths[..sg_count].iter().copied().max().unwrap_or(0);

    // Compute level range per subgraph (constant across rounds).
    let mut sg_min_level = [usize::MAX; 128];
    let mut sg_max_level = [0usize; 128];
    for ni in 0..node_count {
        if let Some(sg_idx) = graph.node_subgraph(ni) {
            if sg_idx < sg_count && sg_idx < 128 {
                let level = real_coords[ni].0;
                if level < sg_min_level[sg_idx] {
                    sg_min_level[sg_idx] = level;
                }
                if level > sg_max_level[sg_idx] {
                    sg_max_level[sg_idx] = level;
                }
            }
        }
    }
    // Propagate child level ranges to parents (bottom-up)
    for depth in (0..=max_depth).rev() {
        for sg_idx in 0..sg_count.min(128) {
            if sg_depths[sg_idx] != depth {
                continue;
            }
            if let Some(pidx) = graph.subgraph_parent(sg_idx) {
                if pidx < 128 {
                    let (cl, cr) = (sg_min_level[sg_idx], sg_max_level[sg_idx]);
                    if cl == usize::MAX {
                        continue;
                    }
                    if cl < sg_min_level[pidx] {
                        sg_min_level[pidx] = cl;
                    }
                    if cr > sg_max_level[pidx] {
                        sg_max_level[pidx] = cr;
                    }
                }
            }
        }
    }

    let mut total_extra = 0usize;

    for _round in 0..8 {
        // ── Compute padded bbox (left, right) per subgraph ──
        for e in sg_envelopes[..sg_count].iter_mut() {
            *e = (usize::MAX, 0, 0, 0);
        }
        for ni in 0..node_count {
            if let Some(sg_idx) = graph.node_subgraph(ni) {
                if sg_idx < sg_count {
                    let (_, _, x, width) = real_coords[ni];
                    let right = x + width;
                    let (ref mut mn, ref mut mx, _, _) = sg_envelopes[sg_idx];
                    if x < *mn {
                        *mn = x;
                    }
                    if right > *mx {
                        *mx = right;
                    }
                }
            }
        }
        // Propagate children → parents (bottom-up)
        // Minimal gap: the child bbox already includes its own cross
        // pads, so the parent only needs its border column (matches heap).
        for depth in (0..=max_depth).rev() {
            for sg_idx in 0..sg_count {
                if sg_depths[sg_idx] != depth {
                    continue;
                }
                if let Some(pidx) = graph.subgraph_parent(sg_idx) {
                    let (cx, cr, _, _) = sg_envelopes[sg_idx];
                    if cx == usize::MAX {
                        continue;
                    }
                    let exp_l = cx.saturating_sub(A::PARENT_CHILD_PAD_CROSS.0);
                    let exp_r = cr + A::PARENT_CHILD_PAD_CROSS.1;
                    let (ref mut pl, ref mut pr, _, _) = sg_envelopes[pidx];
                    if exp_l < *pl {
                        *pl = exp_l;
                    }
                    if exp_r > *pr {
                        *pr = exp_r;
                    }
                }
            }
        }
        // Final padding + label-width expansion
        for sg_idx in 0..sg_count {
            let (mn, mx, _, _) = sg_envelopes[sg_idx];
            if mn == usize::MAX {
                continue;
            }
            let left = mn.saturating_sub(A::SG_PAD_CROSS.0);
            let mut right = mx + A::SG_PAD_CROSS.1;
            let label_w = A::label_cross_extent(graph.subgraph_label(sg_idx));
            if right.saturating_sub(left) < label_w {
                right = left + label_w;
            }
            sg_envelopes[sg_idx] = (left, right, 0, 0);
        }

        // ── Right-frontier sweep per parent group ──
        let mut any_shifted = false;

        // Iterate over each unique parent.  Sentinel 0 = top-level (None),
        // sentinel 1..=sg_count = parent index 0..sg_count-1.
        for parent_sentinel in 0..sg_count + 1 {
            let parent: Option<usize> = if parent_sentinel == 0 {
                None
            } else {
                Some(parent_sentinel - 1)
            };

            // Collect siblings (stack array, max 128)
            let mut siblings = [0usize; 128];
            let mut sib_count = 0usize;
            for sg_idx in 0..sg_count {
                if sg_envelopes[sg_idx].0 == usize::MAX {
                    continue;
                }
                if graph.subgraph_parent(sg_idx) == parent {
                    if sib_count < 128 {
                        siblings[sib_count] = sg_idx;
                        sib_count += 1;
                    }
                }
            }
            if sib_count < 2 {
                continue;
            }

            // Insertion-sort siblings by bbox left
            for i in 1..sib_count {
                let key = siblings[i];
                let key_left = sg_envelopes[key].0;
                let mut j = i;
                while j > 0 && sg_envelopes[siblings[j - 1]].0 > key_left {
                    siblings[j] = siblings[j - 1];
                    j -= 1;
                }
                siblings[j] = key;
            }

            // Level-aware pairwise sweep: only separate siblings whose
            // rendered level ranges share at least one level.
            let mut processed = [(0usize, 0usize, 0usize, 0usize); 128]; // (sg_idx, eff_right, min_l, max_l)
            let mut proc_count = 0usize;

            for s in 0..sib_count {
                let sg_idx = siblings[s];
                let (left, right, _, _) = sg_envelopes[sg_idx];
                let cur_min_l = sg_min_level[sg_idx.min(127)];
                let cur_max_l = sg_max_level[sg_idx.min(127)];

                let mut eff_frontier = 0usize;
                let mut has_level_overlap = false;
                for p in 0..proc_count {
                    let (_, prev_right, prev_min_l, prev_max_l) = processed[p];
                    let overlaps = prev_min_l <= cur_max_l && cur_min_l <= prev_max_l;
                    if overlaps && prev_right > eff_frontier {
                        eff_frontier = prev_right;
                        has_level_overlap = true;
                    }
                }

                if has_level_overlap && eff_frontier + A::SIBLING_GAP_CROSS > left {
                    let shift = eff_frontier + A::SIBLING_GAP_CROSS - left;
                    for ni in 0..node_count {
                        if node_in_sg_subtree(graph, ni, sg_idx) {
                            real_coords[ni].2 += shift;
                        }
                    }
                    total_extra += shift;
                    any_shifted = true;
                    if proc_count < 128 {
                        processed[proc_count] = (sg_idx, right + shift, cur_min_l, cur_max_l);
                        proc_count += 1;
                    }
                } else {
                    if proc_count < 128 {
                        processed[proc_count] = (sg_idx, right, cur_min_l, cur_max_l);
                        proc_count += 1;
                    }
                }
            }
        }

        if !any_shifted {
            break;
        }

        // ── Per-level collision repair ──
        let max_level = real_coords[..node_count]
            .iter()
            .map(|c| c.0)
            .max()
            .unwrap_or(0);
        for level in 0..=max_level {
            // Collect nodes on this level into scratch[]
            let mut count = 0usize;
            for ni in 0..node_count {
                if real_coords[ni].0 == level && count < scratch.len() {
                    scratch[count] = ni;
                    count += 1;
                }
            }
            // Insertion-sort by x
            for i in 1..count {
                let key = scratch[i];
                let key_x = real_coords[key].2;
                let mut j = i;
                while j > 0 && real_coords[scratch[j - 1]].2 > key_x {
                    scratch[j] = scratch[j - 1];
                    j -= 1;
                }
                scratch[j] = key;
            }
            // Fix collisions
            for j in 1..count {
                let prev = scratch[j - 1];
                let curr = scratch[j];
                let need_sg_gap = match (graph.node_subgraph(prev), graph.node_subgraph(curr)) {
                    (Some(a), Some(b)) => a != b,
                    _ => false,
                };
                let gap = if need_sg_gap { cross_sg_gap } else { 3 };
                let needed = real_coords[prev].2 + real_coords[prev].3 + gap;
                if real_coords[curr].2 < needed {
                    real_coords[curr].2 = needed;
                }
            }
        }
    }

    total_extra
}

/// Compute per-level Y extras for subgraph borders.
/// Populates `sg_ranges`, `sg_depths`, `sg_y_extras` in temps and returns
/// (initial_offset, trailing_extra).
fn compute_sg_level_extras<A: Axis>(
    graph: &CsrGraph<'_>,
    node_levels: &[Idx],
    max_level: usize,
    sg_ranges: &mut [(usize, usize)],
    sg_depths: &mut [usize],
    sg_y_extras: &mut [usize],
) -> (usize, usize) {
    let sg_count = graph.subgraph_count();
    if sg_count == 0 {
        sg_y_extras.fill(0);
        return (0, 0);
    }

    // 1. For each subgraph, find (first_level, last_level) from member nodes
    for r in sg_ranges.iter_mut() {
        *r = (usize::MAX, 0);
    }
    for node_idx in 0..graph.node_count() {
        if let Some(sg_idx) = graph.node_subgraph(node_idx) {
            if sg_idx < sg_count {
                let lvl = node_levels[node_idx] as usize;
                let (ref mut first, ref mut last) = sg_ranges[sg_idx];
                if lvl < *first {
                    *first = lvl;
                }
                if lvl > *last {
                    *last = lvl;
                }
            }
        }
    }

    // 2. Propagate child ranges to parents (bottom-up)
    let mut changed = true;
    while changed {
        changed = false;
        for i in 0..sg_count {
            let (cf, cl) = sg_ranges[i];
            if cf == usize::MAX {
                continue;
            } // no nodes
            if let Some(pi) = graph.subgraph_parent(i) {
                if pi < sg_count {
                    let (ref mut pf, ref mut pl) = sg_ranges[pi];
                    if *pf == usize::MAX {
                        *pf = cf;
                        *pl = cl;
                        changed = true;
                    } else {
                        if cf < *pf {
                            *pf = cf;
                            changed = true;
                        }
                        if cl > *pl {
                            *pl = cl;
                            changed = true;
                        }
                    }
                }
            }
        }
    }

    // 3. Compute nesting depths
    for i in 0..sg_count {
        let mut depth = 0;
        let mut cur = graph.subgraph_parent(i);
        while let Some(pid) = cur {
            depth += 1;
            if pid >= sg_count {
                break;
            }
            cur = graph.subgraph_parent(pid);
        }
        sg_depths[i] = depth;
    }

    // Helper: count stacked closing borders at a boundary
    let stacked_closing = |sg_idx: usize, boundary_level: usize| -> usize {
        let mut count = 1;
        let mut cur = graph.subgraph_parent(sg_idx);
        while let Some(pid) = cur {
            if pid >= sg_count {
                break;
            }
            let (f, l) = sg_ranges[pid];
            if f != usize::MAX && l == boundary_level {
                count += 1;
                cur = graph.subgraph_parent(pid);
                continue;
            }
            break;
        }
        count
    };
    let stacked_opening = |sg_idx: usize, boundary_level: usize| -> usize {
        let mut count = 1;
        let mut cur = graph.subgraph_parent(sg_idx);
        while let Some(pid) = cur {
            if pid >= sg_count {
                break;
            }
            let (f, _l) = sg_ranges[pid];
            if f == boundary_level {
                count += 1;
                cur = graph.subgraph_parent(pid);
                continue;
            }
            break;
        }
        count
    };

    // 4. Initial offset: max stacked opening borders at level 0
    let mut max_open_at_0 = 0usize;
    for i in 0..sg_count {
        let (f, _) = sg_ranges[i];
        if f == 0 {
            let d = stacked_opening(i, 0);
            if d > max_open_at_0 {
                max_open_at_0 = d;
            }
        }
    }
    let initial_offset = max_open_at_0 * A::SG_PAD_LEVEL.0;

    // 5. Per-boundary extras
    sg_y_extras.fill(0);
    for boundary_after in 0..max_level {
        let next_level = boundary_after + 1;

        let mut max_close = 0usize;
        let mut max_open = 0usize;

        for i in 0..sg_count {
            let (f, l) = sg_ranges[i];
            if f == usize::MAX {
                continue;
            }
            if l == boundary_after {
                let d = stacked_closing(i, boundary_after);
                if d > max_close {
                    max_close = d;
                }
            }
            if f == next_level {
                let d = stacked_opening(i, next_level);
                if d > max_open {
                    max_open = d;
                }
            }
        }

        if boundary_after < sg_y_extras.len() {
            sg_y_extras[boundary_after] =
                max_close * A::SG_PAD_LEVEL.1 + max_open * A::SG_PAD_LEVEL.0;
        }
    }

    // 6. Trailing extra
    let mut max_close_at_end = 0usize;
    for i in 0..sg_count {
        let (f, l) = sg_ranges[i];
        if f == usize::MAX {
            continue;
        }
        if l == max_level {
            let d = stacked_closing(i, max_level);
            if d > max_close_at_end {
                max_close_at_end = d;
            }
        }
    }
    let trailing_extra = max_close_at_end * A::SG_PAD_LEVEL.1;

    (initial_offset, trailing_extra)
}

/// Compute subgraph bounding boxes and add them to the builder.
/// Uses sg_envelopes as scratch space.
/// `level_routing_floor` contains the max Y used by edge routing at each level,
/// so bottom borders can be placed below the routing area.
/// Returns the maximum physical right and bottom edges across all
/// subgraphs, so the caller can grow BOTH canvas dimensions to cover
/// every border.
fn compute_sg_bounding_boxes<A: Axis>(
    graph: &CsrGraph<'_>,
    real_coords: &[(usize, usize, usize, usize)], // (level, pos, x, width)
    level_offsets: &[usize],
    total_height: usize,
    sg_depths: &[usize],
    sg_envelopes: &mut [(usize, usize, usize, usize)],
    level_routing_floor: &[usize],
    builder: &mut LayoutIRArenaBuilder<'_>,
) -> (usize, usize) {
    let sg_count = graph.subgraph_count();
    if sg_count == 0 {
        return (0, 0);
    }

    // Pass 1: compute node envelope per subgraph
    // Also track max level per subgraph for routing floor lookup
    let mut sg_max_level = [0usize; 64];
    for e in sg_envelopes.iter_mut() {
        *e = (usize::MAX, usize::MAX, 0, 0); // (min_x, min_y, max_x, max_y)
    }

    for node_idx in 0..graph.node_count() {
        if let Some(sg_idx) = graph.node_subgraph(node_idx) {
            if sg_idx >= sg_count {
                continue;
            }
            if node_idx >= real_coords.len() {
                continue;
            }
            let (level, _, x, width) = real_coords[node_idx];
            let y = level_offsets.get(level).copied().unwrap_or(0);
            // Member LEVEL extent from declared dimensions (matches heap).
            let node_max_y =
                y + A::level_extent(graph.node_width(node_idx), graph.node_height(node_idx));
            let node_max_x = x + width;

            if sg_idx < 64 {
                sg_max_level[sg_idx] = sg_max_level[sg_idx].max(level);
            }

            let (ref mut min_x, ref mut min_y, ref mut max_x, ref mut max_y) = sg_envelopes[sg_idx];
            if x < *min_x {
                *min_x = x;
            }
            if y < *min_y {
                *min_y = y;
            }
            if node_max_x > *max_x {
                *max_x = node_max_x;
            }
            if node_max_y > *max_y {
                *max_y = node_max_y;
            }
        }
    }

    // Pass 1.5: Convert envelopes to padded bboxes
    for sg_idx in 0..sg_count {
        let (min_x, min_y, max_x, max_y) = sg_envelopes[sg_idx];
        if min_x == usize::MAX {
            continue;
        } // no nodes

        let x = min_x.saturating_sub(A::SG_PAD_CROSS.0);
        let y = min_y.saturating_sub(A::SG_PAD_LEVEL.0);
        let right = max_x + A::SG_PAD_CROSS.1;

        // Place bottom border below edge routing area if possible.
        let last_level = if sg_idx < 64 { sg_max_level[sg_idx] } else { 0 };
        let routing_floor = if last_level < level_routing_floor.len() {
            level_routing_floor[last_level]
        } else {
            0
        };
        let base_bottom = max_y + A::SG_PAD_LEVEL.1;
        // Ensure bottom border row (base_bottom - 1) is below the routing floor
        let bottom = if routing_floor > 0 && base_bottom.saturating_sub(1) <= routing_floor {
            (routing_floor + 2).min(total_height) // +2: 1 blank row + border row
        } else {
            base_bottom.min(total_height)
        };

        // D8: the label's claim, per axis (matches heap pass 1.5).
        let label = graph.subgraph_label(sg_idx);
        let min_label_width = A::label_cross_extent(label);
        let width = right.saturating_sub(x);
        let right = if width < min_label_width {
            x + min_label_width
        } else {
            right
        };
        let min_label_level = A::label_level_extent(label);
        let bottom = if bottom.saturating_sub(y) < min_label_level {
            y + min_label_level
        } else {
            bottom
        };

        sg_envelopes[sg_idx] = (x, y, right, bottom);
    }

    // Pass 2: propagate children to parents (bottom-up by depth)
    // Process deepest first. Since depth array is already computed, sort by depth desc.
    // Use simple bubble iteration (sg_count is small)
    let mut order = [0usize; 64];
    let effective_sg = sg_count.min(64);
    for i in 0..effective_sg {
        order[i] = i;
    }
    // Sort by depth descending (simple insertion sort for small N)
    for i in 1..effective_sg {
        let mut j = i;
        while j > 0 && sg_depths[order[j]] > sg_depths[order[j - 1]] {
            order.swap(j, j - 1);
            j -= 1;
        }
    }

    for oi in 0..effective_sg {
        let sg_idx = order[oi];
        if let Some(parent_idx) = graph.subgraph_parent(sg_idx) {
            if parent_idx >= sg_count {
                continue;
            }
            let (cx, cy, cr, cb) = sg_envelopes[sg_idx];
            if cx == usize::MAX {
                continue;
            }
            // The child box already includes its own cross-axis pads;
            // the parent adds only its border column (shared rule with heap —
            // using the full pad here made CSR parents one column wider per side).
            let expanded = (
                cx.saturating_sub(A::PARENT_CHILD_PAD_CROSS.0),
                cy.saturating_sub(A::PARENT_CHILD_PAD_LEVEL.0),
                cr + A::PARENT_CHILD_PAD_CROSS.1,
                cb + A::PARENT_CHILD_PAD_LEVEL.1,
            );
            let (ref mut px, ref mut py, ref mut pr, ref mut pb) = sg_envelopes[parent_idx];
            if *px == usize::MAX {
                *px = expanded.0;
                *py = expanded.1;
                *pr = expanded.2;
                *pb = expanded.3;
            } else {
                if expanded.0 < *px {
                    *px = expanded.0;
                }
                if expanded.1 < *py {
                    *py = expanded.1;
                }
                if expanded.2 > *pr {
                    *pr = expanded.2;
                }
                if expanded.3 > *pb {
                    *pb = expanded.3;
                }
            }
        }
    }

    // Post-process: shift borders that overlap with edge routing rows.
    // Since we can't use HashSet in no_std, check against all level_routing_floor values.
    let has_routing = level_routing_floor.iter().any(|&f| f > 0);
    if has_routing {
        for sg_idx in 0..effective_sg {
            let (x, y, right, bottom) = sg_envelopes[sg_idx];
            if x == usize::MAX {
                continue;
            }
            let mut new_y = y;
            let mut new_bottom = bottom;

            // Top border is at row `y`. Check if it matches any routing floor.
            if y > 0 {
                for &floor in level_routing_floor.iter() {
                    if floor > 0 && floor == y {
                        new_y = y - 1;
                        break;
                    }
                }
            }

            // Bottom border is at row `bottom - 1`. Check if it matches any routing floor.
            let bottom_row = bottom.saturating_sub(1);
            for &floor in level_routing_floor.iter() {
                if floor > 0 && floor == bottom_row {
                    new_bottom = bottom + 1;
                    break;
                }
            }

            if new_y != y || new_bottom != bottom {
                sg_envelopes[sg_idx] = (x, new_y, right, new_bottom);
            }
        }
    }

    // Add subgraph bounding boxes to builder — materialize the role
    // rect into physical IR (`x`/`right` cross-axis, `y`/`bottom`
    // level-axis; identity for Vertical).
    let mut max_right = 0usize;
    let mut max_bottom = 0usize;
    for sg_idx in 0..effective_sg {
        let (x, y, right, bottom) = sg_envelopes[sg_idx];
        if x == usize::MAX {
            continue;
        }
        let (px, py) = A::materialize(y, x);
        let (pr, pb) = A::materialize(bottom, right);
        let sg_id = graph.subgraph_id(sg_idx);
        let parent_id = graph.subgraph_parent(sg_idx).map(|p| graph.subgraph_id(p));
        let label = graph.subgraph_label(sg_idx);
        builder.add_subgraph(
            sg_id,
            parent_id,
            label,
            px,
            py,
            pr.saturating_sub(px),
            pb.saturating_sub(py),
        );
        max_right = max_right.max(pr);
        max_bottom = max_bottom.max(pb);
    }
    (max_right, max_bottom)
}

fn calculate_levels_csr(graph: &CsrGraph<'_>, levels: &mut [Idx], back_edges: &[bool]) -> Idx {
    for l in levels.iter_mut() {
        *l = 0;
    }

    let mut changed = true;
    let mut passes = 0;
    while changed && passes < levels.len() {
        changed = false;
        passes += 1;

        for (ei, (from, to)) in graph.edges_iter().enumerate() {
            // Skip self-loops
            if from == to {
                continue;
            }
            // For back edges, flip direction so cycles don't prevent convergence
            let is_back = back_edges.get(ei).copied().unwrap_or(false);
            let (src, dst) = if is_back { (to, from) } else { (from, to) };
            // Saturate: unbroken cycles (CycleBreaking::None) can push
            // levels past Idx::MAX before the pass cap stops relaxation.
            let new_level = levels[src].saturating_add(1);
            if new_level > levels[dst] {
                levels[dst] = new_level;
                changed = true;
            }
        }
    }
    // True (unclamped) max: the caller checks it against buffer capacity
    // and rejects too-deep graphs; clamping here would silently corrupt.
    levels.iter().copied().max().unwrap_or(0)
}

fn build_virtual_levels_csr(
    graph: &CsrGraph<'_>,
    node_levels: &[Idx],
    vlevel_offsets: &mut [Idx],
    level_counts: &mut [Idx],
    vnode_data: &mut [Idx],
    max_level: Idx,
    back_edges: &[bool],
) -> (Idx, Idx) {
    // Logic identical to DAG version but iterating graph.edges_iter()
    for c in level_counts.iter_mut() {
        *c = 0;
    }

    for &level in node_levels.iter() {
        let level_usize = level as usize;
        if level_usize < level_counts.len() {
            level_counts[level_usize] += 1;
        }
    }

    for (ei, (from, to)) in graph.edges_iter().enumerate() {
        // For back edges, layout direction is reversed
        let is_back = back_edges.get(ei).copied().unwrap_or(false);
        let (layout_from, layout_to) = if is_back { (to, from) } else { (from, to) };
        let from_level = node_levels[layout_from] as usize;
        let to_level = node_levels[layout_to] as usize;
        if to_level > from_level + 1 {
            for level in (from_level + 1)..to_level {
                if level < level_counts.len() {
                    level_counts[level] += 1;
                }
            }
        }
    }

    vlevel_offsets[0] = 0;
    let effective_max_level = (max_level as usize).min(level_counts.len().saturating_sub(1));
    for level in 0..=effective_max_level {
        vlevel_offsets[level + 1] = vlevel_offsets[level] + level_counts[level];
    }

    for c in level_counts.iter_mut() {
        *c = 0;
    }

    for (idx, &level) in node_levels.iter().enumerate() {
        let level_usize = level as usize;
        if level_usize <= effective_max_level {
            let pos = (vlevel_offsets[level_usize] + level_counts[level_usize]) as usize;
            // Bounds check for safety - skip if buffer exhausted
            if pos * 2 + 1 >= vnode_data.len() {
                continue;
            }
            vnode_set(vnode_data, pos, 0, idx as Idx); // Real
            level_counts[level_usize] += 1;
        }
    }

    for (edge_idx, (from, to)) in graph.edges_iter().enumerate() {
        // For back edges, layout direction is reversed
        let is_back = back_edges.get(edge_idx).copied().unwrap_or(false);
        let (layout_from, layout_to) = if is_back { (to, from) } else { (from, to) };
        let from_level = node_levels[layout_from] as usize;
        let to_level = node_levels[layout_to] as usize;
        if to_level > from_level + 1 {
            for level in (from_level + 1)..to_level {
                if level <= effective_max_level {
                    let pos = (vlevel_offsets[level] + level_counts[level]) as usize;
                    // Bounds check for safety - skip if buffer exhausted
                    if pos * 2 + 1 >= vnode_data.len() {
                        continue;
                    }
                    vnode_set(vnode_data, pos, 1, edge_idx as Idx); // Dummy
                    level_counts[level] += 1;
                }
            }
        }
    }

    let total = vlevel_offsets[effective_max_level + 1];
    let max_size = level_counts.iter().copied().max().unwrap_or(0);
    (total, max_size)
}

fn assign_x_coords_csr<A: Axis>(
    graph: &CsrGraph<'_>,
    vlevel_offsets: &[Idx],
    vnode_data: &[Idx],
    x_coords: &mut [Coord],
    widths: &mut [Coord],
    max_level: Idx,
    node_spacing: Coord,
) -> Coord {
    let mut max_width: Coord = 0;
    let max_pos = x_coords.len();
    let max_vnode_idx = vnode_data.len() / 2;

    for level in 0..=(max_level as usize) {
        let start = vlevel_offsets[level] as usize;
        let end = (vlevel_offsets[level + 1] as usize)
            .min(max_pos)
            .min(max_vnode_idx);
        let mut x: Coord = 0;

        for pos in start..end {
            // Bounds check
            if pos * 2 + 1 >= vnode_data.len() {
                break;
            }
            let vnode_type = vnode_kind(vnode_data, pos);
            let vnode_idx = vnode_payload(vnode_data, pos) as usize;

            let width: Coord = if vnode_type == 0 {
                // Cross-axis extent (Vertical: the stored width).
                let ext =
                    A::cross_extent(graph.node_width(vnode_idx), graph.node_height(vnode_idx));
                // D5(ii): reserve the self-loop marker cell at
                // `node_spacing == 0` (matches heap; inert otherwise).
                if node_spacing == 0 && graph.children(vnode_idx).contains(&(vnode_idx as u32)) {
                    (ext + 1) as Coord
                } else {
                    ext as Coord
                }
            } else {
                // Dummy clearance — shared constant, matches heap mode.
                A::DUMMY_CROSS as Coord
            };

            if pos < x_coords.len() {
                x_coords[pos] = x;
                widths[pos] = width;
            }
            x += width + node_spacing;
        }

        if end > start && end - 1 < x_coords.len() {
            let last_x = x_coords[end - 1];
            let last_width = widths[end - 1];
            max_width = max_width.max(last_x + last_width);
        }
    }
    max_width
}

fn build_real_coords_csr(
    vlevel_offsets: &[Idx],
    vnode_data: &[Idx],
    x_coords: &[Coord],
    widths: &[Coord],
    real_coords: &mut [(usize, usize, usize, usize)],
    max_level: Idx,
    max_width: Coord,
    center: bool,
) {
    let max_pos = x_coords.len();
    let max_vnode_idx = vnode_data.len() / 2;

    // Logic identical to DAG version (no graph access needed, just array processing)
    for level in 0..=(max_level as usize) {
        let start = vlevel_offsets[level] as usize;
        let end = (vlevel_offsets[level + 1] as usize)
            .min(max_pos)
            .min(max_vnode_idx);
        if end <= start {
            continue;
        }

        let level_width: usize = if end > start && end - 1 < x_coords.len() {
            x_coords[end - 1] as usize + widths[end - 1] as usize
        } else {
            0
        };
        let offset: usize = if center && (max_width as usize) > level_width {
            (max_width as usize - level_width) / 2
        } else {
            0
        };

        for pos in start..end {
            // Bounds check
            if pos * 2 + 1 >= vnode_data.len() || pos >= x_coords.len() {
                break;
            }
            let vnode_type = vnode_kind(vnode_data, pos);
            let vnode_idx = vnode_payload(vnode_data, pos) as usize;

            if vnode_type == 0 && vnode_idx < real_coords.len() {
                let x = x_coords[pos] as usize + offset;
                let width = widths[pos] as usize;
                let level_pos = pos - start;
                real_coords[vnode_idx] = (level, level_pos, x, width);
            }
        }
    }
}

// ── Fan-aware chain-lane allocation, CSR mirror (temp/09 P4) ─────────────
//
// Branch-for-branch port of `heap::allocate_chain_lanes` +
// `heap::chain_lane_dp` onto caller-arena buffers. Every decision rule —
// budget, ordering, exemptions, candidate generation, tie-breaks — is
// either shared code in `geometry.rs` or a literal mirror; byte parity
// with the heap backend is the contract, pinned by the parity suite.

/// One chain, in layout orientation (back edges already flipped).
#[derive(Clone, Copy)]
struct LaneChain {
    #[allow(dead_code)] // parity of shape with the heap ChainPlan
    ei: usize,
    s_idx: usize,
    d_idx: usize,
    s_level: usize,
    t_level: usize,
    s_cross: usize,
    t_cross: usize,
}

/// Transition cost for the §4.7 DP: `(crossings, lane changes,
/// displacement, extent)`; the first three add along a path, extent maxes.
type LaneCostCsr = (usize, usize, usize, usize);

fn lane_cost_add(a: LaneCostCsr, b: LaneCostCsr) -> LaneCostCsr {
    (a.0 + b.0, a.1 + b.1, a.2 + b.2, a.3.max(b.3))
}

/// Chain endpoints in layout orientation.
fn lane_layout_ends(
    graph: &CsrGraph<'_>,
    back_edges: &[bool],
    ei: usize,
) -> Option<(usize, usize)> {
    let (from_idx, to_idx) = graph.edge(ei);
    if from_idx == to_idx {
        return None;
    }
    Some(if back_edges.get(ei).copied().unwrap_or(false) {
        (to_idx, from_idx)
    } else {
        (from_idx, to_idx)
    })
}

/// §4.5: a claim is exempt for a chain only in its endpoint gaps —
/// shared source trunk in gap(S, S+1), shared target merge in gap(T-1, T).
fn lane_exempt(
    graph: &CsrGraph<'_>,
    back_edges: &[bool],
    ch: &LaneChain,
    claim_edge: usize,
    gap: usize,
) -> bool {
    let Some((cs, cd)) = lane_layout_ends(graph, back_edges, claim_edge) else {
        return false;
    };
    (gap == ch.s_level && cs == ch.s_idx) || (gap + 1 == ch.t_level && cd == ch.d_idx)
}

/// The chain's ideal line: interpolation between its endpoint centers.
fn lane_ideal_at(ch: &LaneChain, l: usize) -> usize {
    let span = ch.t_level - ch.s_level;
    let step = l - ch.s_level;
    if ch.t_cross >= ch.s_cross {
        ch.s_cross + (ch.t_cross - ch.s_cross) * step / span
    } else {
        ch.s_cross - (ch.s_cross - ch.t_cross) * step / span
    }
}

/// Stream one gap's filtered claims (fixed, then committed-so-far)
/// through `f` — the same multiset the heap backend builds.
#[allow(clippy::too_many_arguments)]
fn lane_for_filtered(
    graph: &CsrGraph<'_>,
    back_edges: &[bool],
    ch: &LaneChain,
    fixed_offsets: &[usize],
    fixed: &[crate::algorithms::sugiyama::geometry::GapClaim],
    committed_offsets: &[usize],
    cursors: &[usize],
    committed: &[crate::algorithms::sugiyama::geometry::GapClaim],
    gap: usize,
    f: &mut dyn FnMut(crate::algorithms::sugiyama::geometry::CrossSpan),
) {
    for c in fixed[fixed_offsets[gap]..fixed_offsets[gap + 1]].iter() {
        if !lane_exempt(graph, back_edges, ch, c.edge_idx, gap) {
            f(c.span);
        }
    }
    for c in committed[committed_offsets[gap]..cursors[gap]].iter() {
        if !lane_exempt(graph, back_edges, ch, c.edge_idx, gap) {
            f(c.span);
        }
    }
}

/// Stream one level's obstacle spans (node bodies, then cluster
/// envelopes) through `f` — the heap backend's `level_obstacles[lvl]`.
fn lane_for_level_obstacles(
    has_subgraphs: bool,
    real_coords: &[(usize, usize, usize, usize)],
    sg_envelopes: &[(usize, usize, usize, usize)],
    n_levels: usize,
    lvl: usize,
    f: &mut dyn FnMut(crate::algorithms::sugiyama::geometry::CrossSpan),
) {
    use crate::algorithms::sugiyama::geometry::CrossSpan;
    for &(l, _, x, w) in real_coords.iter() {
        if l == lvl {
            f(CrossSpan {
                lo: x,
                hi: x + w.saturating_sub(1),
            });
        }
    }
    if has_subgraphs {
        for &(l, r, first, last) in sg_envelopes.iter() {
            if l == usize::MAX || first > last {
                continue; // empty cluster (sentinel)
            }
            // Exclusive right → inclusive, mirroring the heap wrapper.
            let hi = r.saturating_sub(1).max(l);
            if first <= lvl && lvl <= last.min(n_levels - 1) {
                f(CrossSpan { lo: l, hi });
            }
        }
    }
}

/// CSR mirror of `allocate_chain_lanes` (temp/09 §4). Rewrites
/// `dummy_data` cross coordinates in place; returns the reach (greatest
/// coordinate plus waypoint body) so the caller can widen the canvas —
/// the flip must see the same width (§4.8).
fn allocate_chain_lanes_csr<A: Axis>(
    graph: &CsrGraph<'_>,
    back_edges: &[bool],
    n_levels: usize,
    temps: &mut LayoutTemps<'_>,
) -> usize {
    use crate::algorithms::sugiyama::geometry::{
        CrossSpan, GapClaim, LANE_SPAN_CAP, free_gap_containing, lane_pass_enabled, merge_fan,
    };

    if n_levels < 2 {
        return 0;
    }
    let edge_count = graph.edge_count();
    let total_dummies = temps.dummy_offsets[edge_count.min(temps.dummy_offsets.len() - 1)] as usize;
    // Shared budget — identical to the heap backend's gate and to the
    // predicate the temps builder sized these buffers under.
    if !lane_pass_enabled(n_levels, edge_count, total_dummies) || temps.lane_spans.is_empty() {
        return 0;
    }
    let n_gaps = n_levels - 1;
    let clearance = A::SIBLING_GAP_CROSS;
    let body = A::DUMMY_CROSS.saturating_sub(1);
    let has_sg = graph.has_subgraphs();

    let LayoutTemps {
        real_coords,
        dummy_offsets,
        dummy_data,
        sg_envelopes,
        lane_fixed_offsets,
        lane_fixed,
        lane_committed_offsets,
        lane_cursors,
        lane_committed,
        lane_chains,
        lane_spans,
        lane_cands,
        lane_cand_offsets,
        lane_dp,
        ..
    } = temps;
    let real_coords: &[(usize, usize, usize, usize)] = real_coords;
    let sg_envelopes: &[(usize, usize, usize, usize)] = sg_envelopes;

    // ── Fixed claims (§4.1): adjacent-level real-to-real sweeps,
    //    two-pass CSR fill grouped by gap. ──
    for o in lane_fixed_offsets[..=n_gaps].iter_mut() {
        *o = 0;
    }
    for ei in 0..edge_count {
        let Some((s, d)) = lane_layout_ends(graph, back_edges, ei) else {
            continue;
        };
        let (sl, dl) = (real_coords[s].0, real_coords[d].0);
        if sl.abs_diff(dl) == 1 {
            lane_fixed_offsets[sl.min(dl) + 1] += 1;
        }
    }
    for g in 0..n_gaps {
        lane_fixed_offsets[g + 1] += lane_fixed_offsets[g];
    }
    lane_cursors[..n_gaps].copy_from_slice(&lane_fixed_offsets[..n_gaps]);
    for ei in 0..edge_count {
        let Some((s, d)) = lane_layout_ends(graph, back_edges, ei) else {
            continue;
        };
        let (sl, _, sc, sw) = real_coords[s];
        let (dl, _, dc, dw) = real_coords[d];
        if sl.abs_diff(dl) != 1 {
            continue;
        }
        let g = sl.min(dl);
        lane_fixed[lane_cursors[g]] = GapClaim {
            span: CrossSpan::between(sc + sw / 2, dc + dw / 2),
            edge_idx: ei,
        };
        lane_cursors[g] += 1;
    }

    // ── Chains in allocation order (§4.4): ascending
    //    (target_level, span, edge) — shortest first, so the
    //    farthest-travelling chain is pushed outermost. ──
    let mut chain_count = 0usize;
    for ei in 0..edge_count {
        let ds = dummy_offsets[ei] as usize;
        let de = dummy_offsets[ei + 1] as usize;
        if de <= ds {
            continue;
        }
        let Some((s, d)) = lane_layout_ends(graph, back_edges, ei) else {
            continue;
        };
        let (sl, dl) = (real_coords[s].0, real_coords[d].0);
        if dl <= sl {
            continue;
        }
        lane_chains[chain_count] = (dl, dl - sl, ei);
        chain_count += 1;
    }
    lane_chains[..chain_count].sort_unstable();

    // ── Committed-claim regions: one claim per crossed gap per chain,
    //    so the grouped layout is precomputable before placement. ──
    for o in lane_committed_offsets[..=n_gaps].iter_mut() {
        *o = 0;
    }
    for &(t, span, _) in lane_chains[..chain_count].iter() {
        for g in (t - span)..t {
            lane_committed_offsets[g + 1] += 1;
        }
    }
    for g in 0..n_gaps {
        lane_committed_offsets[g + 1] += lane_committed_offsets[g];
    }
    lane_cursors[..n_gaps].copy_from_slice(&lane_committed_offsets[..n_gaps]);

    let mut reach = 0usize;
    // Global work purse (§4.7, claim-comparison units): both backends
    // charge the same amounts at the same points in the same chain
    // order, so they exhaust identically.
    let mut dp_budget = crate::algorithms::sugiyama::geometry::LANE_WORK_BUDGET;

    for ci in 0..chain_count {
        let (t_level, span_levels, ei) = lane_chains[ci];
        let s_level = t_level - span_levels;
        let Some((s_idx, d_idx)) = lane_layout_ends(graph, back_edges, ei) else {
            continue;
        };
        let (_, _, sc, sw) = real_coords[s_idx];
        let (_, _, dc, dw) = real_coords[d_idx];
        let ch = LaneChain {
            ei,
            s_idx,
            d_idx,
            s_level,
            t_level,
            s_cross: sc + sw / 2,
            t_cross: dc + dw / 2,
        };
        let ds = dummy_offsets[ei] as usize;
        let de = dummy_offsets[ei + 1] as usize;
        let wp_count = de - ds;

        // Per-chain span budget, counted before building (mirrors heap).
        let mut span_need = 0usize;
        for gap in s_level..t_level {
            lane_for_filtered(
                graph,
                back_edges,
                &ch,
                lane_fixed_offsets,
                lane_fixed,
                lane_committed_offsets,
                lane_cursors,
                lane_committed,
                gap,
                &mut |_| span_need += 1,
            );
        }
        for lvl in (s_level + 1)..t_level {
            lane_for_level_obstacles(
                has_sg,
                real_coords,
                sg_envelopes,
                n_levels,
                lvl,
                &mut |_| span_need += 1,
            );
        }

        let contiguous = wp_count == span_levels.saturating_sub(1)
            && dummy_data[ds..de]
                .iter()
                .enumerate()
                .all(|(i, &(l, _))| l as usize == s_level + 1 + i);

        let mut have_placed = false;

        if contiguous && wp_count > 0 && span_need <= LANE_SPAN_CAP && dp_budget >= span_need {
            // Charge the union/candidate stream work up front (mirrors
            // the heap); an exhausted purse skips the whole attempt.
            dp_budget -= span_need;
            // ── §4.3: one lane for the whole chain ──
            let mut n_union = 0usize;
            for gap in s_level..t_level {
                lane_for_filtered(
                    graph,
                    back_edges,
                    &ch,
                    lane_fixed_offsets,
                    lane_fixed,
                    lane_committed_offsets,
                    lane_cursors,
                    lane_committed,
                    gap,
                    &mut |sp| {
                        lane_spans[n_union] = sp;
                        n_union += 1;
                    },
                );
            }
            for lvl in (s_level + 1)..t_level {
                lane_for_level_obstacles(
                    has_sg,
                    real_coords,
                    sg_envelopes,
                    n_levels,
                    lvl,
                    &mut |sp| {
                        lane_spans[n_union] = sp;
                        n_union += 1;
                    },
                );
            }
            let un = merge_fan(&mut lane_spans[..n_union], clearance);

            // Endpoint components (§4.3.2), each against its own gap,
            // built in the tail region past the union.
            let comp_of = |lane_spans: &mut [CrossSpan], gap: usize, at: usize| {
                let base = un;
                let mut k = 0usize;
                lane_for_filtered(
                    graph,
                    back_edges,
                    &ch,
                    lane_fixed_offsets,
                    lane_fixed,
                    lane_committed_offsets,
                    lane_cursors,
                    lane_committed,
                    gap,
                    &mut |sp| {
                        lane_spans[base + k] = sp;
                        k += 1;
                    },
                );
                let kn = merge_fan(&mut lane_spans[base..base + k], clearance);
                free_gap_containing(&lane_spans[base..base + kn], at)
            };
            let s_comp = comp_of(lane_spans, ch.s_level, ch.s_cross);
            let t_comp = comp_of(lane_spans, ch.t_level - 1, ch.t_cross);

            let walk_cost = crate::algorithms::sugiyama::geometry::lane_scan_work(0, un, wp_count);
            let can_walk = dp_budget >= walk_cost;
            if can_walk {
                dp_budget -= walk_cost;
            }
            if let (true, Some((slo, shi)), Some((tlo, thi))) = (can_walk, s_comp, t_comp) {
                let lo_bound = slo.max(tlo);
                let hi_bound = match (shi, thi) {
                    (Some(a), Some(b)) => Some(a.min(b)),
                    (Some(a), None) | (None, Some(a)) => Some(a),
                    (None, None) => None,
                };
                // Ideals into the candidate buffer (disjoint in time
                // from its DP use); lower median, ties smaller.
                for (i, lvl) in ((s_level + 1)..t_level).enumerate() {
                    lane_cands[i] = lane_ideal_at(&ch, lvl);
                }
                let ideals = &mut lane_cands[..wp_count];
                ideals.sort_unstable();
                let median = ideals[(wp_count - 1) / 2];

                let mut best: Option<(usize, usize)> = None;
                {
                    let ideals = &lane_cands[..wp_count];
                    let union_fan = &lane_spans[..un];
                    let mut consider = |p: usize| {
                        // A lane beyond Coord range does not exist —
                        // for either backend (heap mirrors this).
                        if !crate::algorithms::sugiyama::geometry::lane_admissible(p) {
                            return;
                        }
                        let dd: usize = ideals.iter().map(|&i| i.abs_diff(p)).sum();
                        if best.is_none_or(|(bd, bp)| dd < bd || (dd == bd && p < bp)) {
                            best = Some((dd, p));
                        }
                    };
                    let mut cursor = 0usize;
                    for s in union_fan.iter() {
                        if s.lo > cursor {
                            let (flo, fhi) = (cursor, s.lo - 1);
                            let lo = flo.max(lo_bound);
                            let hi = hi_bound.map_or(fhi, |h| fhi.min(h));
                            if lo <= hi {
                                consider(median.clamp(lo, hi));
                            }
                        }
                        cursor = s.hi.saturating_add(1);
                    }
                    if cursor != usize::MAX || union_fan.is_empty() {
                        let lo = cursor.max(lo_bound);
                        match hi_bound {
                            Some(h) if lo <= h => consider(median.clamp(lo, h)),
                            None => consider(median.max(lo)),
                            _ => {}
                        }
                    }
                }

                if let Some((_, lane)) = best {
                    // `consider` refused anything past LANE_MAX_CROSS,
                    // so the cast is exact — no clamp into occupied space.
                    for slot in dummy_data[ds..de].iter_mut() {
                        slot.1 = lane as Coord;
                    }
                    have_placed = true;
                }
            }

            if !have_placed {
                have_placed = chain_lane_dp_csr::<A>(
                    graph,
                    back_edges,
                    &ch,
                    n_levels,
                    has_sg,
                    real_coords,
                    sg_envelopes,
                    lane_fixed_offsets,
                    lane_fixed,
                    lane_committed_offsets,
                    lane_cursors,
                    lane_committed,
                    lane_spans,
                    lane_cands,
                    lane_cand_offsets,
                    lane_dp,
                    dummy_data,
                    ds,
                    span_need,
                    &mut dp_budget,
                );
            }
        }
        let _ = have_placed; // packed coordinates stand when placement failed

        // Commit the final segment spans — placed or packed alike — so
        // later chains route around the real geometry (§4.1).
        let mut prev = ch.s_cross;
        for i in 0..wp_count {
            let c = dummy_data[ds + i].1 as usize;
            let gap = s_level + i;
            lane_committed[lane_cursors[gap]] = GapClaim {
                span: CrossSpan {
                    lo: prev.min(c),
                    hi: prev.max(c) + body,
                },
                edge_idx: ei,
            };
            lane_cursors[gap] += 1;
            reach = reach.max(c + A::DUMMY_CROSS);
            prev = c;
        }
        lane_committed[lane_cursors[t_level - 1]] = GapClaim {
            span: CrossSpan {
                lo: prev.min(ch.t_cross),
                hi: prev.max(ch.t_cross) + body,
            },
            edge_idx: ei,
        };
        lane_cursors[t_level - 1] += 1;
    }

    reach
}

/// CSR mirror of `heap::chain_lane_dp` (§4.7). Returns whether a
/// placement was written into `dummy_data`.
#[allow(clippy::too_many_arguments)]
fn chain_lane_dp_csr<A: Axis>(
    graph: &CsrGraph<'_>,
    back_edges: &[bool],
    ch: &LaneChain,
    n_levels: usize,
    has_sg: bool,
    real_coords: &[(usize, usize, usize, usize)],
    sg_envelopes: &[(usize, usize, usize, usize)],
    lane_fixed_offsets: &[usize],
    lane_fixed: &[crate::algorithms::sugiyama::geometry::GapClaim],
    lane_committed_offsets: &[usize],
    lane_cursors: &[usize],
    lane_committed: &[crate::algorithms::sugiyama::geometry::GapClaim],
    lane_spans: &mut [crate::algorithms::sugiyama::geometry::CrossSpan],
    lane_cands: &mut [usize],
    lane_cand_offsets: &mut [usize],
    lane_dp: &mut [LaneDpEntry],
    dummy_data: &mut [(Idx, Coord)],
    ds: usize,
    span_need: usize,
    dp_budget: &mut usize,
) -> bool {
    use crate::algorithms::sugiyama::geometry::{merge_fan, nearest_outside};

    let span_levels = ch.t_level - ch.s_level;
    let wp_count = span_levels - 1;
    let body = A::DUMMY_CROSS.saturating_sub(1);
    let clearance = A::SIBLING_GAP_CROSS;
    let cand_cap = lane_cands.len();
    // Candidate generation streams the same claims the union did —
    // charge it before doing it (mirrors the heap).
    if *dp_budget < span_need {
        return false;
    }
    *dp_budget -= span_need;

    let crossings = |gap: usize, a: usize, b: usize| -> usize {
        let (lo, hi) = (a.min(b), a.max(b));
        let mut n = 0usize;
        for c in lane_fixed[lane_fixed_offsets[gap]..lane_fixed_offsets[gap + 1]].iter() {
            if !lane_exempt(graph, back_edges, ch, c.edge_idx, gap)
                && c.span.lo <= hi + body
                && lo <= c.span.hi.saturating_add(body)
            {
                n += 1;
            }
        }
        for c in lane_committed[lane_committed_offsets[gap]..lane_cursors[gap]].iter() {
            if !lane_exempt(graph, back_edges, ch, c.edge_idx, gap)
                && c.span.lo <= hi + body
                && lo <= c.span.hi.saturating_add(body)
            {
                n += 1;
            }
        }
        n
    };

    // ── Candidates per interior level (mirrors the heap generation:
    //    both gaps' free boundaries, level-obstacle free boundaries,
    //    probes clamped against gaps ∪ obstacles, filtered by the
    //    level's obstacles, sorted, deduped). ──
    let mut total = 0usize;
    lane_cand_offsets[0] = 0;
    for (li, lvl) in ((ch.s_level + 1)..ch.t_level).enumerate() {
        let cstart = total;
        let push = |cands: &mut [usize], total: &mut usize, v: usize| -> bool {
            if *total >= cand_cap {
                return false;
            }
            cands[*total] = v;
            *total += 1;
            true
        };

        // Free boundaries of each adjacent gap (claims merged with
        // clearance), then of the level obstacles; regions carved
        // sequentially from lane_spans.
        let mut base = 0usize;
        let mut overflow = false;
        for gap in [ch.s_level + li, ch.s_level + li + 1] {
            let mut k = 0usize;
            lane_for_filtered(
                graph,
                back_edges,
                ch,
                lane_fixed_offsets,
                lane_fixed,
                lane_committed_offsets,
                lane_cursors,
                lane_committed,
                gap,
                &mut |sp| {
                    lane_spans[base + k] = sp;
                    k += 1;
                },
            );
            let m = merge_fan(&mut lane_spans[base..base + k], clearance);
            let mut cursor = 0usize;
            for i in 0..m {
                let s = lane_spans[base + i];
                if s.lo > cursor {
                    overflow |= !push(lane_cands, &mut total, cursor);
                    overflow |= !push(lane_cands, &mut total, s.lo - 1);
                }
                cursor = s.hi.saturating_add(1);
            }
            if cursor != usize::MAX {
                overflow |= !push(lane_cands, &mut total, cursor);
            }
            base += m; // keep this gap's merged fan for the probe union
        }
        // Level obstacles: merged with clearance; contributes candidates
        // AND the retain filter below.
        let obs_base = base;
        let mut k = 0usize;
        lane_for_level_obstacles(
            has_sg,
            real_coords,
            sg_envelopes,
            n_levels,
            lvl,
            &mut |sp| {
                lane_spans[obs_base + k] = sp;
                k += 1;
            },
        );
        let on = merge_fan(&mut lane_spans[obs_base..obs_base + k], clearance);
        {
            let mut cursor = 0usize;
            for i in 0..on {
                let s = lane_spans[obs_base + i];
                if s.lo > cursor {
                    overflow |= !push(lane_cands, &mut total, cursor);
                    overflow |= !push(lane_cands, &mut total, s.lo - 1);
                }
                cursor = s.hi.saturating_add(1);
            }
            if cursor != usize::MAX {
                overflow |= !push(lane_cands, &mut total, cursor);
            }
        }
        // Probes, clamped clear of both gaps AND this level: union the
        // three merged regions (already widened) with zero clearance.
        let both_base = obs_base + on;
        let both_len = both_base; // regions [0..both_base] are the three merged fans
        for i in 0..both_len {
            lane_spans[both_base + i] = lane_spans[i];
        }
        let bn = merge_fan(&mut lane_spans[both_base..both_base + both_len], 0);
        for probe in [ch.s_cross, ch.t_cross, lane_ideal_at(ch, lvl)] {
            if let Some(p) = nearest_outside(&lane_spans[both_base..both_base + bn], probe, None) {
                overflow |= !push(lane_cands, &mut total, p);
            }
        }
        if overflow {
            return false; // shared per-chain DP budget: keep packed
        }
        // Filter by the level's own obstacles AND representability
        // (`LANE_MAX_CROSS`), then sort + dedup. The budget above was
        // charged on RAW pushes — the heap backend meters the same
        // quantity, so the two exhaust identically.
        {
            let obs = &lane_spans[obs_base..obs_base + on];
            let mut w = cstart;
            for r in cstart..total {
                let p = lane_cands[r];
                if crate::algorithms::sugiyama::geometry::lane_admissible(p)
                    && !obs.iter().any(|s| s.contains(p))
                {
                    lane_cands[w] = p;
                    w += 1;
                }
            }
            total = w;
        }
        let seg = &mut lane_cands[cstart..total];
        seg.sort_unstable();
        let mut w = cstart;
        for r in cstart..total {
            if w == cstart || lane_cands[r] != lane_cands[w - 1] {
                lane_cands[w] = lane_cands[r];
                w += 1;
            }
        }
        total = w;
        if total == cstart {
            return false; // no representable candidate — keep packed (§4.6)
        }
        lane_cand_offsets[li + 1] = total;
    }

    // §4.7 work budget: transitions ARE claim scans, so each row
    // product is weighted by its gap's filtered-claim count (mirrors
    // the heap's `lane_dp_work` inputs exactly).
    {
        let claims_in = |gap: usize| -> usize {
            let mut n = 0usize;
            for c in lane_fixed[lane_fixed_offsets[gap]..lane_fixed_offsets[gap + 1]].iter() {
                if !lane_exempt(graph, back_edges, ch, c.edge_idx, gap) {
                    n += 1;
                }
            }
            for c in lane_committed[lane_committed_offsets[gap]..lane_cursors[gap]].iter() {
                if !lane_exempt(graph, back_edges, ch, c.edge_idx, gap) {
                    n += 1;
                }
            }
            n
        };
        let row = |li: usize| lane_cand_offsets[li + 1] - lane_cand_offsets[li];
        let mut work = row(0).saturating_mul(claims_in(ch.s_level) + 1);
        for li in 1..wp_count {
            work = work.saturating_add(
                row(li - 1)
                    .saturating_mul(row(li))
                    .saturating_mul(claims_in(ch.s_level + li) + 1),
            );
        }
        work = work.saturating_add(row(wp_count - 1).saturating_mul(claims_in(ch.t_level - 1) + 1));
        if work > *dp_budget {
            return false; // purse exhausted — keep packed (both backends)
        }
        *dp_budget -= work;
    }

    // ── DP fill: per candidate, best predecessor (ascending scan with
    //    strict `<` keeps the first — smaller coordinate, earlier index). ──
    for li in 0..wp_count {
        let lvl = ch.s_level + 1 + li;
        let ideal = lane_ideal_at(ch, lvl);
        let (cs, ce) = (lane_cand_offsets[li], lane_cand_offsets[li + 1]);
        for idx in cs..ce {
            let c = lane_cands[idx];
            let step = |from: usize, gap: usize| -> LaneCostCsr {
                (
                    crossings(gap, from, c),
                    usize::from(from != c),
                    ideal.abs_diff(c),
                    c + A::DUMMY_CROSS,
                )
            };
            let entry = if li == 0 {
                (step(ch.s_cross, ch.s_level), usize::MAX)
            } else {
                let (ps, pe) = (lane_cand_offsets[li - 1], lane_cand_offsets[li]);
                let mut b: Option<(LaneCostCsr, usize)> = None;
                for pi in ps..pe {
                    let pc = lane_cands[pi];
                    let total_cost = lane_cost_add(lane_dp[pi].0, step(pc, ch.s_level + li));
                    if b.is_none_or(|(bc, _)| total_cost < bc) {
                        b = Some((total_cost, pi));
                    }
                }
                match b {
                    Some(x) => x,
                    None => return false,
                }
            };
            lane_dp[idx] = entry;
        }
    }

    // Close with the final segment into the target.
    let (ls, le) = (lane_cand_offsets[wp_count - 1], lane_cand_offsets[wp_count]);
    let mut best_end: Option<(LaneCostCsr, usize)> = None;
    for idx in ls..le {
        let c = lane_cands[idx];
        let tail = (
            crossings(ch.t_level - 1, c, ch.t_cross),
            usize::from(c != ch.t_cross),
            0,
            0,
        );
        let total_cost = lane_cost_add(lane_dp[idx].0, tail);
        if best_end.is_none_or(|(bc, _)| total_cost < bc) {
            best_end = Some((total_cost, idx));
        }
    }
    let Some((_, mut idx)) = best_end else {
        return false;
    };
    for li in (0..wp_count).rev() {
        // Candidates were filtered to LANE_MAX_CROSS — the cast is exact.
        dummy_data[ds + li].1 = lane_cands[idx] as Coord;
        idx = lane_dp[idx].1;
    }
    true
}

/// Build dummy positions for skip-level edges from virtual level positions (CSR version).
/// This extracts the actual x-coordinates assigned during layout, ensuring edges
/// route around nodes based on the natural layout ordering.
fn build_dummy_positions_csr<A: Axis>(
    graph: &CsrGraph<'_>,
    vlevel_offsets: &[Idx],
    vnode_data: &[Idx],
    x_coords: &[Coord],
    widths: &[Coord],
    dummy_offsets: &mut [Idx],
    dummy_data: &mut [(Idx, Coord)],
    max_level: Idx,
    max_width: Coord,
    center: bool,
) {
    let edge_count = graph.edge_count();

    // Classic in-place CSR construction — the offsets buffer doubles as
    // the per-edge write cursor, so there is no fixed edge cap and no
    // extra memory.
    // First pass: count dummies per edge into offsets[e + 1].
    for i in 0..=edge_count.min(dummy_offsets.len().saturating_sub(1)) {
        dummy_offsets[i] = 0;
    }
    for level in 0..=(max_level as usize) {
        let start = vlevel_offsets[level] as usize;
        let end = vlevel_offsets[level + 1] as usize;
        for pos in start..end {
            if vnode_kind(vnode_data, pos) == 1 {
                let edge_idx = vnode_payload(vnode_data, pos) as usize;
                if edge_idx + 1 < dummy_offsets.len() {
                    dummy_offsets[edge_idx + 1] += 1;
                }
            }
        }
    }
    // Prefix-sum into start offsets.
    for i in 1..=edge_count {
        dummy_offsets[i] += dummy_offsets[i - 1];
    }

    // Second pass: write dummy data in level order (important for waypoints)
    for level in 0..=(max_level as usize) {
        let start = vlevel_offsets[level] as usize;
        let end = vlevel_offsets[level + 1] as usize;

        // Calculate centering offset for this level
        let level_width = if end > start {
            x_coords[end - 1] as usize + widths[end - 1] as usize
        } else {
            0
        };
        let offset = if center && (max_width as usize) > level_width {
            ((max_width as usize) - level_width) / 2
        } else {
            0
        };

        for pos in start..end {
            let vnode_type = vnode_kind(vnode_data, pos);
            if vnode_type == 1 {
                let edge_idx = vnode_payload(vnode_data, pos) as usize;

                let base_x = x_coords[pos] as usize + offset;
                let edge_offset = A::dummy_draw_offset(edge_idx);
                let x = base_x + edge_offset;

                if edge_idx < edge_count {
                    let write_idx = dummy_offsets[edge_idx] as usize;
                    if write_idx < dummy_data.len() {
                        dummy_data[write_idx] = (level as Idx, x as Coord);
                        dummy_offsets[edge_idx] += 1;
                    }
                }
            }
        }
    }
    // Each cursor ended at the next edge's start — shift back down.
    for i in (1..=edge_count).rev() {
        dummy_offsets[i] = dummy_offsets[i - 1];
    }
    dummy_offsets[0] = 0;
}

// ---------- Crossing reduction for CSR path ----------

/// Crossing reduction operating on flat virtual-level arrays, specialized for CsrGraph.
///
/// Mirrors `Graph::reduce_crossings_arena` but uses CsrGraph adjacency (`children`/`parents`
/// returning `&[u32]`) instead of heap `Vec<Vec<usize>>`.
#[allow(clippy::too_many_arguments)]
fn reduce_crossings_csr(
    graph: &CsrGraph<'_>,
    crossing_pipeline: &[CrossingReducer],
    vlevel_offsets: &[Idx],
    vnode_data: &mut [Idx],
    max_level: usize,
    medians: &mut [(Idx, u32)],
    positions: &mut [Idx],
    edge_indices: &[(Idx, Idx)],
    level_vdummy_counts: &[Idx],
) {
    // One-time init: positions is alloc_raw_uninit, fill with sentinel
    for p in positions.iter_mut() {
        *p = Idx::MAX;
    }

    for reducer in crossing_pipeline {
        match reducer {
            CrossingReducer::Median(passes) => {
                for _ in 0..*passes {
                    // Top-down pass
                    for level in 1..=max_level {
                        median_reorder_csr_level(
                            graph,
                            vlevel_offsets,
                            vnode_data,
                            edge_indices,
                            level,
                            level - 1,
                            true,
                            medians,
                            positions,
                            level_vdummy_counts,
                        );
                    }
                    // Bottom-up pass
                    for level in (0..max_level).rev() {
                        median_reorder_csr_level(
                            graph,
                            vlevel_offsets,
                            vnode_data,
                            edge_indices,
                            level,
                            level + 1,
                            false,
                            medians,
                            positions,
                            level_vdummy_counts,
                        );
                    }
                }
            }
            CrossingReducer::AdjacentExchange(passes) => {
                for _ in 0..*passes {
                    for level in 1..=max_level {
                        adjacent_exchange_csr_level(
                            graph,
                            vlevel_offsets,
                            vnode_data,
                            edge_indices,
                            level,
                            level - 1,
                            true,
                            positions,
                            level_vdummy_counts,
                        );
                    }
                    for level in (0..max_level).rev() {
                        adjacent_exchange_csr_level(
                            graph,
                            vlevel_offsets,
                            vnode_data,
                            edge_indices,
                            level,
                            level + 1,
                            false,
                            positions,
                            level_vdummy_counts,
                        );
                    }
                }
            }
        }
    }
}

/// Median-heuristic reorder of one level (CSR version).
#[allow(clippy::too_many_arguments)]
fn median_reorder_csr_level(
    graph: &CsrGraph<'_>,
    vlevel_offsets: &[Idx],
    vnode_data: &mut [Idx],
    edge_indices: &[(Idx, Idx)],
    level: usize,
    adj_level: usize,
    use_parents: bool,
    medians: &mut [(Idx, u32)],
    positions: &mut [Idx],
    level_vdummy_counts: &[Idx],
) {
    let cur_start = vlevel_offsets[level] as usize;
    let cur_end = vlevel_offsets[level + 1] as usize;
    let count = cur_end - cur_start;
    if count < 2 {
        return;
    }

    let adj_start = vlevel_offsets[adj_level] as usize;
    let adj_end = vlevel_offsets[adj_level + 1] as usize;

    let adj_has_dummies =
        adj_level < level_vdummy_counts.len() && level_vdummy_counts[adj_level] > 0;

    // Build position map for real nodes in adjacent level (sparse-clear optimized)
    let adj_size = adj_end - adj_start;
    let mut written_buf: [usize; 512] = [0; 512];
    let mut written_count: usize = 0;
    let use_sparse_clear = adj_size <= 512;

    if !use_sparse_clear {
        for p in positions.iter_mut() {
            *p = Idx::MAX;
        }
    }
    for adj_pos in adj_start..adj_end {
        if adj_pos * 2 + 1 >= vnode_data.len() {
            break;
        }
        if vnode_is_real(vnode_data, adj_pos) {
            let node_idx = vnode_payload(vnode_data, adj_pos) as usize;
            if node_idx < positions.len() {
                positions[node_idx] = (adj_pos - adj_start) as Idx;
                if use_sparse_clear && written_count < 512 {
                    written_buf[written_count] = node_idx;
                    written_count += 1;
                }
            }
        }
    }

    // Compute median for each node on this level
    for i in 0..count {
        let pos = cur_start + i;
        if pos * 2 + 1 >= vnode_data.len() {
            medians[i] = (i as Idx, (i as u32) << 10);
            continue;
        }
        let vtype = vnode_kind(vnode_data, pos);
        let vidx = vnode_payload(vnode_data, pos) as usize;

        let mut neigh: [usize; 16] = [0; 16];
        let mut neigh_count: usize = 0;

        if vtype == 0 {
            // Real node — CsrGraph adjacency
            let neighbours = if use_parents {
                graph.parents(vidx)
            } else {
                graph.children(vidx)
            };
            for &n_idx in neighbours {
                let n = n_idx as usize;
                if n < positions.len() && positions[n] != Idx::MAX && neigh_count < 16 {
                    neigh[neigh_count] = positions[n] as usize;
                    neigh_count += 1;
                }
            }
            if adj_has_dummies {
                for adj_pos in adj_start..adj_end {
                    if adj_pos * 2 + 1 >= vnode_data.len() {
                        break;
                    }
                    if vnode_is_dummy(vnode_data, adj_pos) {
                        let eidx = vnode_payload(vnode_data, adj_pos) as usize;
                        if eidx < edge_indices.len() {
                            let (from_idx, to_idx) = edge_indices[eidx];
                            if (from_idx as usize == vidx || to_idx as usize == vidx)
                                && neigh_count < 16
                            {
                                neigh[neigh_count] = adj_pos - adj_start;
                                neigh_count += 1;
                            }
                        }
                    }
                }
            }
        } else if vidx < edge_indices.len() {
            // Dummy node
            let (from_idx, to_idx) = edge_indices[vidx];
            for &endpoint in &[from_idx as usize, to_idx as usize] {
                if endpoint < positions.len() && positions[endpoint] != Idx::MAX && neigh_count < 16
                {
                    neigh[neigh_count] = positions[endpoint] as usize;
                    neigh_count += 1;
                }
            }
            if adj_has_dummies {
                for adj_pos in adj_start..adj_end {
                    if adj_pos * 2 + 1 >= vnode_data.len() {
                        break;
                    }
                    if vnode_is_dummy(vnode_data, adj_pos)
                        && vnode_payload(vnode_data, adj_pos) as usize == vidx
                        && neigh_count < 16
                    {
                        neigh[neigh_count] = adj_pos - adj_start;
                        neigh_count += 1;
                        break;
                    }
                }
            }
        }

        let median_fixed = if neigh_count == 0 {
            (i as u32) << 10
        } else {
            neigh[..neigh_count].sort_unstable();
            if neigh_count % 2 == 1 {
                (neigh[neigh_count / 2] as u32) << 10
            } else {
                let mid = neigh_count / 2;
                let sum = neigh[mid - 1] + neigh[mid];
                (sum as u32) * 512
            }
        };
        medians[i] = (i as Idx, median_fixed);
    }

    // Sort by median (unstable: no alloc needed, fine for layout positions)
    medians[..count].sort_unstable_by_key(|m| m.1);

    // Gather sorted vnode_data into medians buffer
    for j in 0..count {
        let orig_pos = medians[j].0 as usize;
        let src = cur_start + orig_pos;
        let vtype = vnode_kind(vnode_data, src);
        let vidx = vnode_payload(vnode_data, src) as u32;
        medians[j] = (vtype, vidx);
    }

    // Write sorted data back
    for j in 0..count {
        let dst = cur_start + j;
        vnode_set(vnode_data, dst, medians[j].0, medians[j].1 as Idx);
    }

    // Sparse-clear
    if use_sparse_clear {
        for i in 0..written_count {
            positions[written_buf[i]] = Idx::MAX;
        }
    }
}

/// Adjacent exchange on one level (CSR version).
#[allow(clippy::too_many_arguments)]
fn adjacent_exchange_csr_level(
    graph: &CsrGraph<'_>,
    vlevel_offsets: &[Idx],
    vnode_data: &mut [Idx],
    edge_indices: &[(Idx, Idx)],
    level: usize,
    adj_level: usize,
    use_parents: bool,
    positions: &mut [Idx],
    level_vdummy_counts: &[Idx],
) {
    let cur_start = vlevel_offsets[level] as usize;
    let cur_end = vlevel_offsets[level + 1] as usize;
    let count = cur_end - cur_start;
    if count < 2 {
        return;
    }

    let adj_has_dummies =
        adj_level < level_vdummy_counts.len() && level_vdummy_counts[adj_level] > 0;

    let adj_start = vlevel_offsets[adj_level] as usize;
    let adj_end = vlevel_offsets[adj_level + 1] as usize;

    // Build position map (sparse-clear optimized)
    let adj_size = adj_end - adj_start;
    let mut written_buf: [usize; 512] = [0; 512];
    let mut written_count: usize = 0;
    let use_sparse_clear = adj_size <= 512;

    if !use_sparse_clear {
        for p in positions.iter_mut() {
            *p = Idx::MAX;
        }
    }
    for adj_pos in adj_start..adj_end {
        if adj_pos * 2 + 1 >= vnode_data.len() {
            break;
        }
        if vnode_is_real(vnode_data, adj_pos) {
            let node_idx = vnode_payload(vnode_data, adj_pos) as usize;
            if node_idx < positions.len() {
                positions[node_idx] = (adj_pos - adj_start) as Idx;
                if use_sparse_clear && written_count < 512 {
                    written_buf[written_count] = node_idx;
                    written_count += 1;
                }
            }
        }
    }

    let mut u_neigh: [usize; 16] = [0; 16];
    let mut v_neigh: [usize; 16] = [0; 16];

    for i in 0..count - 1 {
        let u_pos = cur_start + i;
        let v_pos = cur_start + i + 1;
        if u_pos * 2 + 1 >= vnode_data.len() || v_pos * 2 + 1 >= vnode_data.len() {
            break;
        }

        let mut u_count = 0;
        let mut v_count = 0;

        gather_csr_neighbours(
            graph,
            vnode_data,
            edge_indices,
            positions,
            u_pos,
            adj_start,
            adj_end,
            use_parents,
            adj_has_dummies,
            &mut u_neigh,
            &mut u_count,
        );
        gather_csr_neighbours(
            graph,
            vnode_data,
            edge_indices,
            positions,
            v_pos,
            adj_start,
            adj_end,
            use_parents,
            adj_has_dummies,
            &mut v_neigh,
            &mut v_count,
        );

        let mut cross_uv: usize = 0;
        let mut cross_vu: usize = 0;
        for &a in &u_neigh[..u_count] {
            for &b in &v_neigh[..v_count] {
                if a > b {
                    cross_uv += 1;
                } else if a < b {
                    cross_vu += 1;
                }
            }
        }

        if cross_vu < cross_uv {
            let u_type = vnode_kind(vnode_data, u_pos);
            let u_idx = vnode_payload(vnode_data, u_pos);
            vnode_set(
                vnode_data,
                u_pos,
                vnode_kind(vnode_data, v_pos),
                vnode_payload(vnode_data, v_pos),
            );
            vnode_set(vnode_data, v_pos, u_type, u_idx);
        }
    }

    if use_sparse_clear {
        for i in 0..written_count {
            positions[written_buf[i]] = Idx::MAX;
        }
    }
}

/// Gather neighbour positions for a single vnode (CSR version).
#[inline]
#[allow(clippy::too_many_arguments)]
fn gather_csr_neighbours(
    graph: &CsrGraph<'_>,
    vnode_data: &[Idx],
    edge_indices: &[(Idx, Idx)],
    positions: &[Idx],
    pos: usize,
    adj_start: usize,
    adj_end: usize,
    use_parents: bool,
    adj_has_dummies: bool,
    out: &mut [usize; 16],
    out_count: &mut usize,
) {
    *out_count = 0;
    let vtype = vnode_kind(vnode_data, pos);
    let vidx = vnode_payload(vnode_data, pos) as usize;

    if vtype == 0 {
        // Real node — CsrGraph adjacency (returns &[u32])
        let neighbours = if use_parents {
            graph.parents(vidx)
        } else {
            graph.children(vidx)
        };
        for &n_idx in neighbours {
            let n = n_idx as usize;
            if n < positions.len() && positions[n] != Idx::MAX && *out_count < 16 {
                out[*out_count] = positions[n] as usize;
                *out_count += 1;
            }
        }
        if adj_has_dummies {
            for adj_pos in adj_start..adj_end {
                if adj_pos * 2 + 1 >= vnode_data.len() {
                    break;
                }
                if vnode_is_dummy(vnode_data, adj_pos) {
                    let eidx = vnode_payload(vnode_data, adj_pos) as usize;
                    if eidx < edge_indices.len() {
                        let (from_idx, to_idx) = edge_indices[eidx];
                        if (from_idx as usize == vidx || to_idx as usize == vidx) && *out_count < 16
                        {
                            out[*out_count] = (adj_pos - adj_start) as usize;
                            *out_count += 1;
                        }
                    }
                }
            }
        }
    } else if vidx < edge_indices.len() {
        // Dummy node
        let (from_idx, to_idx) = edge_indices[vidx];
        for &endpoint in &[from_idx as usize, to_idx as usize] {
            if endpoint < positions.len() && positions[endpoint] != Idx::MAX && *out_count < 16 {
                out[*out_count] = positions[endpoint] as usize;
                *out_count += 1;
            }
        }
        if adj_has_dummies {
            for adj_pos in adj_start..adj_end {
                if adj_pos * 2 + 1 >= vnode_data.len() {
                    break;
                }
                if vnode_is_dummy(vnode_data, adj_pos)
                    && vnode_payload(vnode_data, adj_pos) as usize == vidx
                    && *out_count < 16
                {
                    out[*out_count] = (adj_pos - adj_start) as usize;
                    *out_count += 1;
                    break;
                }
            }
        }
    }
}

// ── Graph::estimate_layout_arena_size ─────────────────────────────────────────
#[cfg(feature = "alloc")]
use crate::graph::Graph;
#[cfg(feature = "alloc")]
use alloc::vec;

#[cfg(feature = "alloc")]
impl<'a> Graph<'a> {
    /// Layout-temp bytes for the port pass — requests (two per edge)
    /// and per-edge positions — only when a port was declared (and
    /// only with the `ports` feature at all).
    /// Layout-temp bytes for the port pass — requests (two per edge)
    /// and per-edge positions whenever a port was declared, plus the
    /// detour scratch sized by the [`DetourBudget`]: the marks the
    /// budget pass leaves, the sparse plan/node/blocker/head-on tables,
    /// jog blocks, the extra slot intervals and level, and the wider
    /// bend-staging scratch. Zero without the `ports` feature.
    #[allow(clippy::too_many_arguments)]
    fn port_scratch_bytes(
        &self,
        budget: crate::algorithms::sugiyama::ports::DetourBudget,
        node_count: usize,
        depth: usize,
        edge_count: usize,
        max_levels: usize,
        dummies: usize,
    ) -> usize {
        #[cfg(feature = "ports")]
        {
            if self.edge_ports.is_empty() {
                return 0;
            }
            let item = |count: usize, size: usize| count.saturating_mul(size).saturating_add(8);
            let declared = item(
                edge_count.saturating_mul(2),
                core::mem::size_of::<crate::algorithms::sugiyama::ports::FaceRequest>(),
            )
            .saturating_add(item(edge_count, core::mem::size_of::<(usize, usize)>()))
            .saturating_add(item(node_count.max(1), core::mem::size_of::<bool>()))
            .saturating_add(item(depth.max(1), core::mem::size_of::<bool>()));
            if !budget.any() {
                return declared;
            }
            declared
                .saturating_add(item(
                    budget.edges,
                    core::mem::size_of::<(usize, crate::algorithms::sugiyama::ports::Detour)>(),
                ))
                .saturating_add(item(
                    dummies,
                    core::mem::size_of::<(usize, usize, usize, usize)>(),
                ))
                .saturating_add(item(
                    budget.blockers,
                    core::mem::size_of::<(usize, usize, usize, usize)>(),
                ))
                .saturating_add(item(
                    budget.edges.saturating_mul(5),
                    core::mem::size_of::<(usize, usize, usize)>(),
                ))
                .saturating_add(item(2 * MAX_SLOTS_PER_LEVEL, core::mem::size_of::<usize>()))
                .saturating_add(item(1, core::mem::size_of::<Idx>()))
                .saturating_add(item(
                    max_levels.saturating_add(7),
                    core::mem::size_of::<(usize, usize)>(),
                ))
        }
        #[cfg(not(feature = "ports"))]
        {
            let _ = (budget, node_count, depth, edge_count, max_levels, dummies);
            0
        }
    }

    /// Estimate the arena buffer size needed for `compute_layout_arena()`
    /// under [`LayoutConfig::standard()`]. Use
    /// [`Self::estimate_layout_arena_size_with`] when rendering with a
    /// non-default configuration (notably `include_dummy_nodes`, which
    /// grows the IR output).
    pub fn estimate_layout_arena_size(&self) -> usize {
        self.estimate_layout_arena_size_with(
            &crate::algorithms::sugiyama::config::LayoutConfig::standard(),
        )
    }

    /// Estimate the arena buffer size needed for `compute_layout_arena()`
    /// under the given configuration.
    ///
    /// Performs a cheap O(N+E) level computation to measure the actual
    /// dummy count, then sums the full allocation manifest of the layout
    /// pass: every temp buffer, the IR output (including subgraphs, all
    /// label storage, and — when `include_dummy_nodes` is set — the
    /// emitted dummy nodes). All arithmetic saturates.
    pub fn estimate_layout_arena_size_with(
        &self,
        config: &crate::algorithms::sugiyama::config::LayoutConfig<'_>,
    ) -> usize {
        let node_count = self.nodes.len();
        let edge_count = self.edges.len();
        let sg_count = self.subgraphs.len();
        // Every label the layout copies into the output arena: node,
        // edge, and subgraph labels alike.
        let label_bytes: usize = self
            .nodes
            .iter()
            .map(|(_, l)| l.len())
            .chain(self.edges.iter().map(|&(_, _, l)| l.map_or(0, |t| t.len())))
            .chain(self.subgraphs.iter().map(|sg| sg.label.len()))
            // Custom payloads ride the IR label storage — twice: once
            // in the CSR carry, once in the IR (both from this arena
            // budget when the caller shares one arena; itemized here
            // for the IR side, the CSR side is estimate_csr_arena_size).
            .chain(self.node_custom.iter().map(|entry| entry.2.len()))
            .fold(0usize, |a, b| a.saturating_add(b));
        // Sparse custom entry array in the IR output (+ alignment).
        let custom_entry_bytes: usize = self
            .node_custom
            .len()
            .saturating_mul(core::mem::size_of::<crate::ir::arena::CustomNodeArena>())
            .saturating_add(if self.node_custom.is_empty() { 0 } else { 16 });
        // ── Level assignment for sizing: mirror the layout's own ──
        // Under `CycleBreaking::DepthFirst` (the default), run the SAME
        // back-edge detection and back-flipped relaxation the layout
        // runs, so depth and dummy count are EXACT for every graph —
        // cyclic included. The previous unflipped relaxation undercounted
        // cyclic graphs: an ordered cycle 0→1→…→N-1→0 relaxes to zero
        // dummies, but breaking reverses the closing edge into a
        // span-(N-1) chain with N-2 waypoints, and every dummy-derived
        // manifest term (vnodes, level widths, waypoints, lane scratch,
        // emitted dummy IR) was sized from the undercount.
        //
        // Under `CycleBreaking::None` the unflipped relaxation stays: if
        // it converges the graph is acyclic and it equals the layout's;
        // if not, the layout itself fails with `ExceedsMaxLevels`, and
        // node-count bounds keep the estimate a ceiling on the way there.
        let actual_dummies: usize;
        let max_levels: usize;
        let lane_dummy_bound: usize;
        // The levels and reversal the layout will use — kept for the
        // detour budget below.
        let levels: alloc::vec::Vec<usize>;
        let back_flags: alloc::vec::Vec<bool>;
        {
            use crate::algorithms::sugiyama::config::CycleBreaking;
            let count_from = |lvl: &[usize], back: &[bool]| -> usize {
                let mut dummies = 0usize;
                for (ei, &(from_id, to_id, _)) in self.edges.iter().enumerate() {
                    if from_id == to_id {
                        continue;
                    }
                    let (Some(fi), Some(ti)) = (self.node_index(from_id), self.node_index(to_id))
                    else {
                        continue;
                    };
                    let is_back = back.get(ei).copied().unwrap_or(false);
                    let (s, d) = if is_back { (ti, fi) } else { (fi, ti) };
                    let span = lvl[d].abs_diff(lvl[s]);
                    if span > 1 {
                        dummies += span - 1;
                    }
                }
                dummies
            };
            match config.cycle_breaking() {
                CycleBreaking::DepthFirst => {
                    let back = self.detect_back_edges();
                    let mut lvl = vec![0usize; node_count];
                    for (idx, l) in self.calculate_levels_with_back_edges(&back) {
                        if idx < node_count {
                            lvl[idx] = l;
                        }
                    }
                    max_levels = lvl.iter().copied().max().unwrap_or(0) + 1;
                    actual_dummies = count_from(&lvl, &back);
                    lane_dummy_bound = actual_dummies; // exact
                    levels = lvl;
                    back_flags = back;
                }
                _ => {
                    let mut relaxed = vec![0u32; node_count];
                    let edge_idx: alloc::vec::Vec<(usize, usize)> = self
                        .edges
                        .iter()
                        .map(|&(from_id, to_id, _)| {
                            let fi = self.node_index(from_id).unwrap_or(usize::MAX);
                            let ti = self.node_index(to_id).unwrap_or(usize::MAX);
                            (fi, ti)
                        })
                        .collect();
                    let mut changed = true;
                    let mut passes = 0;
                    while changed && passes < node_count {
                        changed = false;
                        passes += 1;
                        for &(fi, ti) in &edge_idx {
                            if fi != usize::MAX && ti != usize::MAX && fi != ti {
                                let nl = relaxed[fi].saturating_add(1);
                                if nl > relaxed[ti] {
                                    relaxed[ti] = nl;
                                    changed = true;
                                }
                            }
                        }
                    }
                    max_levels = if changed {
                        node_count.max(1)
                    } else {
                        relaxed.iter().copied().max().unwrap_or(0) as usize + 1
                    };
                    let lvl: alloc::vec::Vec<usize> = relaxed.iter().map(|&l| l as usize).collect();
                    actual_dummies = count_from(&lvl, &[]);
                    levels = lvl;
                    back_flags = alloc::vec::Vec::new();
                    lane_dummy_bound = if changed {
                        // Did not converge: the layout will fail, but keep
                        // the estimate a ceiling on the way there.
                        actual_dummies.max(edge_count.saturating_mul(node_count.saturating_sub(1)))
                    } else {
                        actual_dummies
                    };
                }
            }
        }

        // The detour budget, from the same facts the layout will have
        // (levels, reversal, declared sides) — exact under the default
        // cycle breaking, a ceiling otherwise.
        #[cfg(feature = "ports")]
        let budget = if self.edge_ports.is_empty() {
            crate::algorithms::sugiyama::ports::DetourBudget::NONE
        } else {
            let (axis, flipped) = crate::algorithms::sugiyama::ports::frame(config.direction);
            let mut level_real = vec![0usize; max_levels.max(1)];
            let mut level_dummy = vec![0usize; max_levels.max(1)];
            for &l in &levels {
                if l < level_real.len() {
                    level_real[l] += 1;
                }
            }
            let endpoints = |ei: usize| -> (usize, usize) {
                let (from_id, to_id, _) = self.edges[ei];
                (
                    self.node_index(from_id).unwrap_or(usize::MAX),
                    self.node_index(to_id).unwrap_or(usize::MAX),
                )
            };
            for ei in 0..edge_count {
                let (f, t) = endpoints(ei);
                if f == t || f == usize::MAX || t == usize::MAX {
                    continue;
                }
                let (lo, hi) = (levels[f].min(levels[t]), levels[f].max(levels[t]));
                for slot in level_dummy.iter_mut().take(hi).skip(lo + 1) {
                    *slot += 1;
                }
            }
            let mut node_marks = vec![false; node_count];
            let mut level_marks = vec![false; max_levels.max(1)];
            crate::algorithms::sugiyama::ports::detour_budget(
                edge_count,
                &endpoints,
                &|ei| self.edge_ports.get(ei).copied().unwrap_or_default(),
                &|ei| back_flags.get(ei).copied().unwrap_or(false),
                &|n| levels.get(n).copied().unwrap_or(0),
                axis,
                flipped,
                &level_real,
                &level_dummy,
                &mut node_marks,
                &mut level_marks,
            )
        };
        #[cfg(not(feature = "ports"))]
        let budget = {
            let _ = (levels, back_flags);
            crate::algorithms::sugiyama::ports::DetourBudget::NONE
        };

        let max_vnodes = (node_count + actual_dummies).min(MAX_NODES);
        // Medians scratch is sized by level width, which dummy-heavy
        // graphs can push past the real-node count.
        let max_level_size = max_vnodes;
        let max_dummy_waypoints = (actual_dummies + 16).min(MAX_NODES);

        // The full temp-arena allocation manifest, in the order the
        // layout pass carves it (saturating throughout).
        let item = |count: usize, size: usize| count.saturating_mul(size);
        // Lane-pass buffers (temp/09 P4): clamped mirrors of the builder's
        // formulas. Clamping (not gating) is what keeps the estimate an
        // upper bound even when this cheap relaxation and the real layout
        // disagree about depth or dummy count (cyclic graphs): the builder
        // sizes from exact values which the shared caps bound identically.
        let lane_bytes: usize = if lane_dummy_bound > 0
            && edge_count <= crate::algorithms::sugiyama::geometry::LANE_PASS_MAX_WORK
        {
            use crate::algorithms::sugiyama::geometry::{
                CrossSpan, GapClaim, LANE_CAND_CAP, LANE_PASS_MAX_WORK, LANE_SPAN_CAP,
            };
            let d = lane_dummy_bound.min(LANE_PASS_MAX_WORK);
            let c = edge_count.min(d);
            let comm = d + c;
            let gaps = max_levels.saturating_sub(1);
            let span_n = LANE_SPAN_CAP
                .min(edge_count + comm + node_count + sg_count.saturating_mul(max_levels) + 16)
                * 2;
            let cand_n = LANE_CAND_CAP.min(
                4 * (edge_count + comm)
                    + 2 * (node_count + sg_count.saturating_mul(max_levels))
                    + 8 * max_levels
                    + 16,
            );
            item(2 * (gaps + 1) + gaps.max(1), core::mem::size_of::<usize>())
                .saturating_add(item(edge_count + comm, core::mem::size_of::<GapClaim>()))
                .saturating_add(item(
                    c.max(1),
                    core::mem::size_of::<(usize, usize, usize)>(),
                ))
                .saturating_add(item(span_n, core::mem::size_of::<CrossSpan>()))
                .saturating_add(item(cand_n, core::mem::size_of::<usize>()))
                .saturating_add(item(max_levels + 1, core::mem::size_of::<usize>()))
                .saturating_add(item(
                    cand_n,
                    core::mem::size_of::<((usize, usize, usize, usize), usize)>(),
                ))
        } else {
            0
        };
        let temps_size = [
            item(edge_count.max(1), core::mem::size_of::<bool>()), // back_edges
            item(node_count, core::mem::size_of::<Idx>()),         // node_levels
            item(
                2 * max_levels.saturating_add(2),
                core::mem::size_of::<usize>(),
            ), // level_real + level_dummy
            item(edge_count, core::mem::size_of::<(Idx, Idx)>()),  // edge_indices
            item(max_levels.saturating_add(2), core::mem::size_of::<Idx>()), // vlevel_offsets
            item(max_levels.saturating_add(1), core::mem::size_of::<Idx>()), // level_counts
            item(max_vnodes.saturating_mul(2), core::mem::size_of::<Idx>()), // vnode_data
            item(max_vnodes, core::mem::size_of::<Coord>()),       // x_coords
            item(max_vnodes, core::mem::size_of::<Coord>()),       // widths
            item(
                node_count,
                core::mem::size_of::<(usize, usize, usize, usize)>(),
            ), // real_coords
            item(edge_count.saturating_add(1), core::mem::size_of::<Idx>()), // dummy_offsets
            item(max_dummy_waypoints, core::mem::size_of::<(Idx, Coord)>()), // dummy_data
            item(max_level_size, core::mem::size_of::<(Idx, u32)>()), // medians
            item(node_count.max(1), core::mem::size_of::<Idx>()),  // positions
            item(node_count, core::mem::size_of::<bool>()),        // node_is_source
            item(max_levels.saturating_add(1), core::mem::size_of::<Idx>()), // source_counts
            item(max_levels.saturating_add(1), core::mem::size_of::<Idx>()), // dummy_counts
            item(max_levels.saturating_add(2), core::mem::size_of::<usize>()), // level_offsets
            item(node_count, core::mem::size_of::<usize>()),       // node_slots
            item(max_levels.saturating_add(1), core::mem::size_of::<Idx>()), // level_slot_next
            item(
                2 * edge_count.saturating_add(1),
                3 * core::mem::size_of::<usize>(),
            ), // slot_pool
            item(
                2 * max_levels
                    .saturating_add(1)
                    .saturating_mul(MAX_SLOTS_PER_LEVEL),
                core::mem::size_of::<usize>(),
            ), // slot heads + tails
            item(max_levels.saturating_add(1), core::mem::size_of::<Idx>()), // level_dummy_next
            item(max_levels.saturating_add(1), core::mem::size_of::<Idx>()), // level_labeled_src
            item(edge_count, core::mem::size_of::<Idx>()),         // two_cycle_order
            item(edge_count, core::mem::size_of::<bool>()),        // edge_in_two_cycle
            // Port scratch — requests (two per edge) and per-edge
            // positions — exists only when a port was declared.
            self.port_scratch_bytes(
                budget,
                node_count,
                max_levels,
                edge_count,
                max_levels,
                actual_dummies,
            ),
            item(
                max_levels.saturating_add(1),
                core::mem::size_of::<(usize, usize)>(),
            ), // waypoint_scratch
            item(max_levels.saturating_add(1), core::mem::size_of::<Idx>()), // level_vdummy_counts
            item(max_levels.saturating_add(1), core::mem::size_of::<usize>()), // level_max_extents
            item(max_levels.saturating_add(1), core::mem::size_of::<usize>()), // level_routing_floor
            // Subgraph temporaries (allocated only when clustered, but
            // an estimate must cover the clustered case).
            item(sg_count, core::mem::size_of::<(usize, usize)>()), // sg_ranges
            item(sg_count, core::mem::size_of::<usize>()),          // sg_depths
            item(
                sg_count,
                core::mem::size_of::<(usize, usize, usize, usize)>(),
            ), // sg_envelopes
            item(max_levels.saturating_add(1), core::mem::size_of::<usize>()), // sg_y_extras
            item(
                2 * max_levels.saturating_add(2),
                core::mem::size_of::<usize>(),
            ), // sg frontier scratch
            4096, // per-allocation alignment slack + margin
        ]
        .into_iter()
        .fold(0usize, |a, b| a.saturating_add(b));

        // IR output: with dummy emission enabled, dummies become real
        // IR nodes (and level-list entries) in the output arena.
        let ir_nodes = if config.include_dummy_nodes {
            node_count.saturating_add(actual_dummies)
        } else {
            node_count
        };
        let max_ir_waypoints = max_dummy_waypoints.saturating_add(budget.points);
        let ir_size = crate::ir::arena::estimate_layout_arena_size_with_subgraphs(
            ir_nodes,
            edge_count,
            label_bytes,
            max_ir_waypoints,
            sg_count,
        );

        temps_size
            .saturating_add(lane_bytes)
            .saturating_add(ir_size)
            .saturating_add(custom_entry_bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::arena::Arena;
    use crate::graph::csr::CsrGraphBuilder;

    /// Helper: build a CsrGraph from edges (node labels auto-generated A, B, C, ...)
    fn build_csr_graph<'a>(
        arena: &'a mut Arena<'a>,
        node_count: usize,
        edges: &[(usize, usize)],
    ) -> CsrGraph<'a> {
        let label_bytes = node_count * 2; // single-char labels
        let mut builder = CsrGraphBuilder::new(arena, node_count, edges.len(), label_bytes, 0)
            .expect("builder alloc");
        for i in 0..node_count {
            let label = &[b'A' + i as u8];
            let label_str = core::str::from_utf8(label).unwrap();
            builder.add_node(i, label_str);
        }
        for &(from, to) in edges {
            builder.add_edge(from, to);
        }
        builder.build().expect("csr build")
    }

    /// The arena estimate covers the per-node parent counters `build`
    /// carves: an exactly estimated arena builds a two-thousand-node
    /// star (small graphs hid the missing term under the slack).
    #[test]
    fn exact_estimate_builds_a_wide_star() {
        const N: usize = 2_001;
        let size = CsrGraph::required_arena_size(N, N - 1, N);
        let mut buf = alloc::vec![0u8; size];
        let mut arena = Arena::new(&mut buf);
        let mut builder = CsrGraphBuilder::new(&mut arena, N, N - 1, N, 0).expect("builder");
        for i in 0..N {
            builder.add_node(i, "L");
        }
        for leaf in 1..N {
            builder.add_edge(0, leaf);
        }
        let graph = builder
            .build()
            .expect("exact estimate covers the parent counters");
        assert_eq!(graph.edge_count(), N - 1);
    }

    #[test]
    fn test_detect_back_edges_acyclic() {
        let mut buf = [0u8; 8192];
        let mut arena = Arena::new(&mut buf);
        let graph = build_csr_graph(&mut arena, 3, &[(0, 1), (1, 2)]);

        let mut back_edges = [false; 2];
        let mut dfs_buf = [0u8; 4096];
        let mut dfs_arena = Arena::new(&mut dfs_buf);
        detect_back_edges_csr(&graph, &mut back_edges, &mut dfs_arena);

        assert!(!back_edges[0], "0→1 should not be a back edge");
        assert!(!back_edges[1], "1→2 should not be a back edge");
    }

    #[test]
    fn test_detect_back_edges_simple_cycle() {
        let mut buf = [0u8; 8192];
        let mut arena = Arena::new(&mut buf);
        // A→B→C→A (edge 2 is the back edge)
        let graph = build_csr_graph(&mut arena, 3, &[(0, 1), (1, 2), (2, 0)]);

        let mut back_edges = [false; 3];
        let mut dfs_buf = [0u8; 4096];
        let mut dfs_arena = Arena::new(&mut dfs_buf);
        detect_back_edges_csr(&graph, &mut back_edges, &mut dfs_arena);

        // Exactly one edge should be marked as back edge (the cycle-closing one)
        let back_count: usize = back_edges.iter().filter(|&&b| b).count();
        assert_eq!(back_count, 1, "exactly 1 back edge in A→B→C→A");
        // The DFS from 0: visits 0→1→2, then 2→0 targets GRAY node → back edge
        assert!(back_edges[2], "edge 2→0 should be the back edge");
    }

    #[test]
    fn test_detect_back_edges_self_loop() {
        let mut buf = [0u8; 8192];
        let mut arena = Arena::new(&mut buf);
        let graph = build_csr_graph(&mut arena, 2, &[(0, 0), (0, 1)]);

        let mut back_edges = [false; 2];
        let mut dfs_buf = [0u8; 4096];
        let mut dfs_arena = Arena::new(&mut dfs_buf);
        detect_back_edges_csr(&graph, &mut back_edges, &mut dfs_arena);

        assert!(back_edges[0], "self-loop should be a back edge");
        assert!(!back_edges[1], "0→1 should not be a back edge");
    }

    #[test]
    fn test_cyclic_graph_levels_converge() {
        let mut buf = [0u8; 8192];
        let mut arena = Arena::new(&mut buf);
        // A→B→C→A
        let graph = build_csr_graph(&mut arena, 3, &[(0, 1), (1, 2), (2, 0)]);

        let mut back_edges = [false; 3];
        let mut dfs_buf = [0u8; 4096];
        let mut dfs_arena = Arena::new(&mut dfs_buf);
        detect_back_edges_csr(&graph, &mut back_edges, &mut dfs_arena);

        let mut levels = [0 as Idx; 3];
        let max_level = calculate_levels_csr(&graph, &mut levels, &back_edges);

        // With back edge 2→0 reversed, effective DAG is A→B→C
        // Levels: A=0, B=1, C=2
        assert_eq!(max_level, 2);
        assert_eq!(levels[0], 0, "A should be level 0");
        assert_eq!(levels[1], 1, "B should be level 1");
        assert_eq!(levels[2], 2, "C should be level 2");
    }

    #[test]
    fn test_cyclic_csr_layout_no_panic() {
        // A→B→C→A: full layout pipeline should complete without panic
        let mut graph_buf = [0u8; 8192];
        let mut graph_arena = Arena::new(&mut graph_buf);
        let graph = build_csr_graph(&mut graph_arena, 3, &[(0, 1), (1, 2), (2, 0)]);

        let config = LayoutConfig::standard();
        let mut temp_buf = [0u8; 65536];
        let mut temp_arena = Arena::new(&mut temp_buf);
        let mut out_buf = [0u8; 65536];
        let mut out_arena = Arena::new(&mut out_buf);

        let ir = compute_layout_arena_csr(&graph, &config, &mut temp_arena, &mut out_arena);
        assert!(ir.is_ok(), "layout of cyclic graph should succeed");

        let ir = ir.unwrap();
        assert_eq!(ir.node_count(), 3);
        assert!(
            ir.edge_count() >= 2,
            "should have at least 2 edges (self-loops skipped)"
        );

        // Check that the reversed edge is marked
        let mut found_reversed = false;
        for i in 0..ir.edge_count() {
            if ir.edge(i).reversed {
                found_reversed = true;
            }
        }
        assert!(
            found_reversed,
            "cyclic graph should have at least one reversed edge"
        );
    }

    /// Helper: chain graph 0→1→…→n-1 (one node per level, depth = n).
    #[cfg(not(feature = "arena-idx-u8"))]
    fn build_chain_csr<'a>(arena: &'a mut Arena<'a>, n: usize) -> CsrGraph<'a> {
        let mut builder = CsrGraphBuilder::new(arena, n, n - 1, n, 0).expect("builder alloc");
        for i in 0..n {
            builder.add_node(i, "n").expect("add node");
        }
        for i in 0..n - 1 {
            builder.add_edge(i, i + 1).expect("add edge");
        }
        builder.build().expect("csr build")
    }

    /// Regression: a chain deeper than the per-level buffer capacity
    /// (256 levels) must return `ExceedsMaxLevels`, not index out of
    /// bounds in crossing reduction. Original report used a 20k-node
    /// chain; any depth past 256 triggers the same OOB.
    /// arena-idx-u8 is excluded: >255 nodes already fails `ExceedsMaxNodes`.
    #[cfg(not(feature = "arena-idx-u8"))]
    #[test]
    fn test_deep_chain_lays_out_with_depth_sized_buffers() {
        // Per-level buffers are sized from the graph's real depth — a
        // 300-level chain (past the old fixed 256 cap) must lay out.
        const N: usize = 300;
        let mut graph_buf = vec![0u8; 128 * 1024];
        let mut graph_arena = Arena::new(&mut graph_buf);
        let graph = build_chain_csr(&mut graph_arena, N);

        let config = LayoutConfig::standard();
        let mut temp_buf = vec![0u8; 1024 * 1024];
        let mut temp_arena = Arena::new(&mut temp_buf);
        let mut out_buf = vec![0u8; 512 * 1024];
        let mut out_arena = Arena::new(&mut out_buf);

        let ir = compute_layout_arena_csr(&graph, &config, &mut temp_arena, &mut out_arena)
            .expect("a deep chain lays out with depth-sized buffers");
        assert_eq!(ir.nodes().len(), N);
        assert_eq!(
            ir.nodes().iter().map(|n| n.level).max(),
            Some(N - 1),
            "one node per level, depth = N"
        );
    }

    #[test]
    fn test_unbroken_cycle_errors_cleanly() {
        // With CycleBreaking::None a cycle pumps level relaxation past
        // any DAG-possible depth; the depth > node_count guard must
        // reject it as ExceedsMaxLevels — never panic, never allocate
        // per-level buffers from a saturated depth.
        let mut graph_buf = vec![0u8; 64 * 1024];
        let mut graph_arena = Arena::new(&mut graph_buf);
        let mut builder = CsrGraphBuilder::new(&mut graph_arena, 2, 2, 8, 0).expect("builder");
        builder.add_node(1, "A").expect("node");
        builder.add_node(2, "B").expect("node");
        builder.add_edge(0, 1).expect("edge");
        builder.add_edge(1, 0).expect("edge");
        let graph = builder.build().expect("csr build");

        let mut config = LayoutConfig::standard();
        let crate::algorithms::sugiyama::config::AlgorithmConfig::Sugiyama {
            cycle_breaking, ..
        } = &mut config.algorithm;
        *cycle_breaking = CycleBreaking::None;
        let mut temp_buf = vec![0u8; 256 * 1024];
        let mut temp_arena = Arena::new(&mut temp_buf);
        let mut out_buf = vec![0u8; 128 * 1024];
        let mut out_arena = Arena::new(&mut out_buf);

        let err = compute_layout_arena_csr(&graph, &config, &mut temp_arena, &mut out_arena)
            .expect_err("unbroken cycle must error, not saturate");
        assert!(
            matches!(err, GraphError::ExceedsMaxLevels { .. }),
            "expected ExceedsMaxLevels, got {err:?}"
        );
    }

    /// Boundary: a chain of exactly 256 levels fills the per-level
    /// buffers to capacity and must still lay out successfully.
    #[cfg(not(feature = "arena-idx-u8"))]
    #[test]
    fn test_chain_at_level_capacity_ok() {
        const N: usize = 256;
        let mut graph_buf = vec![0u8; 128 * 1024];
        let mut graph_arena = Arena::new(&mut graph_buf);
        let graph = build_chain_csr(&mut graph_arena, N);

        let config = LayoutConfig::standard();
        let mut temp_buf = vec![0u8; 512 * 1024];
        let mut temp_arena = Arena::new(&mut temp_buf);
        let mut out_buf = vec![0u8; 512 * 1024];
        let mut out_arena = Arena::new(&mut out_buf);

        let ir = compute_layout_arena_csr(&graph, &config, &mut temp_arena, &mut out_arena)
            .expect("256-level chain should lay out");
        assert_eq!(ir.node_count(), N);
    }

    #[test]
    fn test_cyclic_csr_renders_without_panic() {
        // A→B→C→A: full pipeline through to rendering
        let mut graph_buf = [0u8; 8192];
        let mut graph_arena = Arena::new(&mut graph_buf);
        let graph = build_csr_graph(&mut graph_arena, 3, &[(0, 1), (1, 2), (2, 0)]);

        let config = LayoutConfig::standard();
        let mut temp_buf = [0u8; 65536];
        let mut temp_arena = Arena::new(&mut temp_buf);
        let mut out_buf = [0u8; 65536];
        let mut out_arena = Arena::new(&mut out_buf);

        let ir = compute_layout_arena_csr(&graph, &config, &mut temp_arena, &mut out_arena)
            .expect("layout should succeed");

        let opts = crate::render::engine::RenderOptions::plain();
        let mut render_arena_buf = vec![0u8; ir.estimate_render_arena_size(&opts)];
        let render_arena = Arena::new(&mut render_arena_buf);
        let mut render_buf = vec![0u8; ir.estimate_render_output_size(&opts)];
        let rendered = ir.render_to_bytes(&opts, &render_arena, &mut render_buf);
        assert!(rendered.is_ok(), "rendering should succeed: {rendered:?}");
        let len = rendered.unwrap();
        assert!(len > 0, "should produce non-empty output");
    }

    #[cfg(feature = "layout-vertical")]
    #[test]
    fn test_diamond_with_back_edge() {
        // Diamond: A→B, A→C, B→D, C→D, plus back edge D→A
        let mut graph_buf = [0u8; 16384];
        let mut graph_arena = Arena::new(&mut graph_buf);
        let graph = build_csr_graph(
            &mut graph_arena,
            4,
            &[(0, 1), (0, 2), (1, 3), (2, 3), (3, 0)],
        );

        let config = LayoutConfig::standard();
        let mut temp_buf = [0u8; 65536];
        let mut temp_arena = Arena::new(&mut temp_buf);
        let mut out_buf = [0u8; 65536];
        let mut out_arena = Arena::new(&mut out_buf);

        let ir = compute_layout_arena_csr(&graph, &config, &mut temp_arena, &mut out_arena)
            .expect("diamond+backedge layout should succeed");

        assert_eq!(ir.node_count(), 4);

        // Verify levels make sense: A at top, D at bottom (back edge D→A reversed)
        let node_a = ir.node(0);
        let node_d = ir.node(3);
        assert!(node_a.y < node_d.y, "A should be above D");
    }

    /// Regression test: cyclic graph via Graph::to_csr() path
    /// (existing tests use CsrGraphBuilder directly)
    #[test]
    fn test_cyclic_via_to_csr_layout_and_render() {
        use crate::graph::Graph;

        let mut dag = Graph::new();
        dag.add_node(1, "A");
        dag.add_node(2, "B");
        dag.add_node(3, "C");
        dag.add_edge(1, 2, None);
        dag.add_edge(2, 3, None);
        dag.add_edge(3, 1, None); // cycle

        let csr_size = dag.estimate_csr_arena_size() * 2;
        let mut csr_buf = vec![0u8; csr_size];
        let mut csr_arena = Arena::new(&mut csr_buf);
        let csr = dag.to_csr(&mut csr_arena).expect("to_csr");

        assert_eq!(csr.node_count(), 3);
        assert_eq!(csr.edge_count(), 3);

        let config = LayoutConfig::standard();
        let mut temp_buf = vec![0u8; 128 * 1024];
        let mut out_buf = vec![0u8; 128 * 1024];
        let mut temp_arena = Arena::new(&mut temp_buf);
        let mut out_arena = Arena::new(&mut out_buf);

        let ir = compute_layout_arena_csr(&csr, &config, &mut temp_arena, &mut out_arena)
            .expect("layout should succeed");

        assert_eq!(ir.node_count(), 3);

        let opts = crate::render::engine::RenderOptions::plain();
        let mut render_arena_buf = vec![0u8; ir.estimate_render_arena_size(&opts)];
        let render_arena = Arena::new(&mut render_arena_buf);
        let mut render_buf = vec![0u8; ir.estimate_render_output_size(&opts)];
        let rendered = ir.render_to_bytes(&opts, &render_arena, &mut render_buf);
        assert!(rendered.is_ok(), "render should succeed: {rendered:?}");
    }

    #[cfg(feature = "layout-vertical")]
    #[test]
    fn test_two_node_cycle_layout() {
        use crate::graph::Graph;

        let mut dag = Graph::new();
        dag.add_node(1, "Ping");
        dag.add_node(2, "Pong");
        dag.add_edge(1, 2, None);
        dag.add_edge(2, 1, None);

        let csr_size = dag.estimate_csr_arena_size() * 2;
        let mut csr_buf = vec![0u8; csr_size];
        let mut csr_arena = Arena::new(&mut csr_buf);
        let csr = dag.to_csr(&mut csr_arena).expect("to_csr");

        let config = LayoutConfig::standard();
        let mut temp_buf = vec![0u8; 128 * 1024];
        let mut out_buf = vec![0u8; 128 * 1024];
        let mut temp_arena = Arena::new(&mut temp_buf);
        let mut out_arena = Arena::new(&mut out_buf);

        let ir = compute_layout_arena_csr(&csr, &config, &mut temp_arena, &mut out_arena)
            .expect("layout should succeed");

        assert_eq!(ir.node_count(), 2);
        assert_eq!(ir.edge_count(), 2);

        // The two edges should be offset from each other (not overlapping)
        let e0 = ir.edge(0);
        let e1 = ir.edge(1);
        assert_ne!(
            e0.from_x, e1.from_x,
            "2-node cycle edges must not share the same column"
        );

        // The forward edge should be left of the back-edge
        let (fwd, bck) = if e0.reversed { (e1, e0) } else { (e0, e1) };
        assert!(
            fwd.from_x < bck.from_x,
            "forward edge should be left of back-edge"
        );

        // Rendering should succeed
        let opts = crate::render::engine::RenderOptions::plain();
        let mut render_arena_buf = vec![0u8; ir.estimate_render_arena_size(&opts)];
        let render_arena = Arena::new(&mut render_arena_buf);
        let mut render_buf = vec![0u8; ir.estimate_render_output_size(&opts)];
        let rendered = ir.render_to_bytes(&opts, &render_arena, &mut render_buf);
        assert!(rendered.is_ok(), "render should succeed: {rendered:?}");
    }

    #[cfg(feature = "layout-vertical")]
    #[test]
    fn test_self_loop_renders_indicator() {
        use crate::graph::Graph;

        let mut dag = Graph::new();
        dag.add_node(1, "Loop");
        dag.add_edge(1, 1, None);

        let csr_size = dag.estimate_csr_arena_size() * 2;
        let mut csr_buf = vec![0u8; csr_size];
        let mut csr_arena = Arena::new(&mut csr_buf);
        let csr = dag.to_csr(&mut csr_arena).expect("to_csr");

        let config = LayoutConfig::standard();
        let mut temp_buf = vec![0u8; 128 * 1024];
        let mut out_buf = vec![0u8; 128 * 1024];
        let mut temp_arena = Arena::new(&mut temp_buf);
        let mut out_arena = Arena::new(&mut out_buf);

        let ir = compute_layout_arena_csr(&csr, &config, &mut temp_arena, &mut out_arena)
            .expect("layout should succeed");

        assert_eq!(ir.node_count(), 1);
        assert!(ir.node(0).has_self_loop, "self-loop node should be marked");

        // Rendered output should contain ↺
        let opts = crate::render::engine::RenderOptions::plain();
        let mut render_arena_buf = vec![0u8; ir.estimate_render_arena_size(&opts)];
        let render_arena = Arena::new(&mut render_arena_buf);
        let mut render_buf = vec![0u8; ir.estimate_render_output_size(&opts)];
        let len = ir
            .render_to_bytes(&opts, &render_arena, &mut render_buf)
            .unwrap();
        let output = core::str::from_utf8(&render_buf[..len]).unwrap();
        assert!(
            output.contains('↺'),
            "rendered output should contain self-loop indicator ↺"
        );
        assert!(
            output.contains("[Loop]↺"),
            "↺ should appear right after the node bracket"
        );
    }

    #[cfg(feature = "layout-vertical")]
    #[test]
    fn test_geometry_aware_slot_sharing() {
        // Fan-out: Root splits to Left and Right, then they merge.
        // The Root→Left corner and Root→Right corner come from the same
        // source (Root) and share a slot via source-bus rule.
        // Additionally: Left→Merge and Right→Merge come from different
        // sources at level 1. Their horizontal spans point inward (toward
        // Merge). With geometry-aware allocation, if the spans don't overlap,
        // they share a slot — resulting in a more compact layout.
        use crate::graph::Graph;

        let mut dag = Graph::new();
        dag.add_node(1, "Root");
        dag.add_node(2, "Left");
        dag.add_node(3, "Right");
        dag.add_node(4, "Merge");
        dag.add_edge(1, 2, None);
        dag.add_edge(1, 3, None);
        dag.add_edge(2, 4, None);
        dag.add_edge(3, 4, None);

        let csr_size = dag.estimate_csr_arena_size() * 2;
        let mut csr_buf = vec![0u8; csr_size];
        let mut csr_arena = Arena::new(&mut csr_buf);
        let csr = dag.to_csr(&mut csr_arena).expect("to_csr");

        let config = LayoutConfig::standard();
        let mut temp_buf = vec![0u8; 128 * 1024];
        let mut out_buf = vec![0u8; 128 * 1024];
        let mut temp_arena = Arena::new(&mut temp_buf);
        let mut out_arena = Arena::new(&mut out_buf);

        let ir = compute_layout_arena_csr(&csr, &config, &mut temp_arena, &mut out_arena)
            .expect("layout should succeed");

        // Verify rendering works and produces expected characters
        let opts = crate::render::engine::RenderOptions::plain();
        let mut render_arena_buf = vec![0u8; ir.estimate_render_arena_size(&opts)];
        let render_arena = Arena::new(&mut render_arena_buf);
        let mut render_buf = vec![0u8; ir.estimate_render_output_size(&opts)];
        let len = ir
            .render_to_bytes(&opts, &render_arena, &mut render_buf)
            .unwrap();
        let output = core::str::from_utf8(&render_buf[..len]).unwrap();

        // Should contain all node labels
        assert!(output.contains("[Root]"));
        assert!(output.contains("[Left]"));
        assert!(output.contains("[Right]"));
        assert!(output.contains("[Merge]"));
        // Should contain arrow indicators and edge corners
        assert!(output.contains('↓'), "should contain down arrows");
        assert!(
            output.contains('┌') || output.contains('└'),
            "should contain corner characters for non-vertical edges"
        );

        // Count the total height: geometry-aware should produce compact output
        let line_count = output.lines().count();
        // Diamond with 4 nodes should be at most ~10 lines with compressed slots
        assert!(
            line_count <= 12,
            "layout should be compact: got {} lines",
            line_count
        );
    }

    #[test]
    fn test_csr_single_subgraph_produces_border() {
        // Build: A→B, both in subgraph "cluster"
        let mut buf = [0u8; 32768];
        let mut arena = Arena::new(&mut buf);
        let sg_label_bytes = 7; // "cluster"
        let label_bytes = 4 + sg_label_bytes; // A+B node labels + sg label
        let mut builder = CsrGraphBuilder::new_with_subgraphs(&mut arena, 2, 1, label_bytes, 1, 0)
            .expect("builder");
        builder.add_node(0, "A");
        builder.add_node(1, "B");
        builder.add_edge(0, 1);
        let sg = builder.add_subgraph(0, "cluster").expect("sg");
        builder.set_node_subgraph(0, sg);
        builder.set_node_subgraph(1, sg);
        let graph = builder.build().expect("build");

        assert_eq!(graph.subgraph_count(), 1);
        assert_eq!(graph.subgraph_label(0), "cluster");

        // Layout + render
        let config = LayoutConfig::default();
        let mut temp_buf = [0u8; 65536];
        let mut temp_arena = Arena::new(&mut temp_buf);
        let mut out_buf = [0u8; 65536];
        let mut out_arena = Arena::new(&mut out_buf);

        let ir = compute_layout_arena_csr(&graph, &config, &mut temp_arena, &mut out_arena)
            .expect("layout");

        assert_eq!(ir.subgraph_count(), 1);
        let sg_info = &ir.subgraphs()[0];
        assert!(sg_info.width > 0, "subgraph should have width");
        assert!(sg_info.height > 0, "subgraph should have height");

        // Render to text
        let opts = crate::render::engine::RenderOptions::plain();
        let mut render_arena_buf = vec![0u8; ir.estimate_render_arena_size(&opts)];
        let render_arena = Arena::new(&mut render_arena_buf);
        let mut render_buf = vec![0u8; ir.estimate_render_output_size(&opts)];
        let bytes = ir
            .render_to_bytes(&opts, &render_arena, &mut render_buf)
            .expect("render");
        let output = core::str::from_utf8(&render_buf[..bytes]).expect("utf8");

        // Should contain border characters
        assert!(output.contains('╔'), "top-left border missing");
        assert!(output.contains('╗'), "top-right border missing");
        assert!(output.contains('╚'), "bottom-left border missing");
        assert!(output.contains('╝'), "bottom-right border missing");
        assert!(output.contains('║'), "side border missing");
        assert!(output.contains('═'), "horizontal border missing");
        // Label should appear
        assert!(output.contains("cluster"), "subgraph label missing");
        // Nodes should still be present
        assert!(output.contains("[A]"), "node A missing");
        assert!(output.contains("[B]"), "node B missing");
    }

    #[test]
    fn test_csr_subgraph_via_to_csr() {
        // Use the Graph→to_csr path which copies subgraph data
        use crate::graph::Graph;

        let mut g = Graph::new();
        g.add_node(1, "X");
        g.add_node(2, "Y");
        g.add_edge(1, 2, None);
        let sg = g.add_subgraph("box");
        g.put_nodes(&[1, 2]).inside(sg).unwrap();

        let mut csr_buf = [0u8; 32768];
        let mut csr_arena = Arena::new(&mut csr_buf);
        let csr = g.to_csr(&mut csr_arena).expect("to_csr");

        assert_eq!(csr.subgraph_count(), 1);
        assert_eq!(csr.subgraph_label(0), "box");

        let config = LayoutConfig::default();
        let mut temp_buf = [0u8; 65536];
        let mut temp_arena = Arena::new(&mut temp_buf);
        let mut out_buf = [0u8; 65536];
        let mut out_arena = Arena::new(&mut out_buf);

        let ir = compute_layout_arena_csr(&csr, &config, &mut temp_arena, &mut out_arena)
            .expect("layout");

        assert!(ir.subgraph_count() >= 1, "IR should have subgraph");
        assert!(ir.subgraphs()[0].width > 0);
        assert!(ir.subgraphs()[0].height > 0);
    }

    #[test]
    fn test_csr_no_subgraphs_unchanged() {
        // Verify that the subgraph code path doesn't affect graphs without subgraphs
        let mut buf = [0u8; 16384];
        let mut arena = Arena::new(&mut buf);
        let graph = build_csr_graph(&mut arena, 3, &[(0, 1), (1, 2)]);

        assert_eq!(graph.subgraph_count(), 0);
        assert!(!graph.has_subgraphs());

        let config = LayoutConfig::default();
        let mut temp_buf = [0u8; 65536];
        let mut temp_arena = Arena::new(&mut temp_buf);
        let mut out_buf = [0u8; 65536];
        let mut out_arena = Arena::new(&mut out_buf);

        let ir = compute_layout_arena_csr(&graph, &config, &mut temp_arena, &mut out_arena)
            .expect("layout");

        assert_eq!(ir.subgraph_count(), 0);
        assert!(!ir.has_subgraphs());

        // Render should work fine
        let opts = crate::render::engine::RenderOptions::plain();
        let mut render_arena_buf = vec![0u8; ir.estimate_render_arena_size(&opts)];
        let render_arena = Arena::new(&mut render_arena_buf);
        let mut render_buf = vec![0u8; ir.estimate_render_output_size(&opts)];
        let bytes = ir
            .render_to_bytes(&opts, &render_arena, &mut render_buf)
            .expect("render");
        let output = core::str::from_utf8(&render_buf[..bytes]).expect("utf8");
        assert!(output.contains("[A]"));
        assert!(output.contains("[B]"));
        assert!(output.contains("[C]"));
    }

    // ── Spacing config (regression: fields were silently ignored) ──────

    /// A fans out to B/C/D (three adjacent nodes on level 1), converging on E.
    #[cfg(feature = "layout-vertical")]
    const SPACING_TEST_EDGES: &[(usize, usize)] = &[(0, 1), (0, 2), (0, 3), (1, 4), (2, 4), (3, 4)];

    #[cfg(feature = "layout-vertical")]
    fn layout_with_config<'b>(
        config: &LayoutConfig<'_>,
        graph_buf: &mut [u8],
        temp_buf: &mut [u8],
        out_arena: &'b mut Arena<'b>,
    ) -> LayoutIRArena<'b> {
        let mut graph_arena = Arena::new(graph_buf);
        let graph = build_csr_graph(&mut graph_arena, 5, SPACING_TEST_EDGES);
        let mut temp_arena = Arena::new(temp_buf);
        compute_layout_arena_csr(&graph, config, &mut temp_arena, out_arena).expect("layout")
    }

    #[cfg(feature = "layout-vertical")]
    #[test]
    fn test_node_spacing_config_applied_csr() {
        for spacing in [3usize, 8] {
            let mut config = LayoutConfig::standard();
            config.node_spacing = spacing;
            let mut graph_buf = [0u8; 8192];
            let mut temp_buf = [0u8; 65536];
            let mut out_buf = [0u8; 65536];
            let mut out_arena = Arena::new(&mut out_buf);
            let ir = layout_with_config(&config, &mut graph_buf, &mut temp_buf, &mut out_arena);

            let mut boxes: [(usize, usize); 3] = [(0, 0); 3];
            let mut count = 0;
            for n in ir.nodes() {
                if n.level == 1 {
                    boxes[count] = (n.x, n.width);
                    count += 1;
                }
            }
            assert_eq!(count, 3);
            boxes.sort_unstable();
            for pair in boxes.windows(2) {
                let gap = pair[1].0 - (pair[0].0 + pair[0].1);
                assert_eq!(
                    gap, spacing,
                    "gap between adjacent nodes should equal node_spacing={spacing}"
                );
            }
        }
    }

    #[cfg(feature = "layout-vertical")]
    #[test]
    fn test_level_spacing_config_applied_csr() {
        let base_config = LayoutConfig::standard();
        let mut graph_buf = [0u8; 8192];
        let mut temp_buf = [0u8; 65536];
        let mut out_buf = [0u8; 65536];
        let mut out_arena = Arena::new(&mut out_buf);
        let base = layout_with_config(&base_config, &mut graph_buf, &mut temp_buf, &mut out_arena);

        let mut spaced_config = LayoutConfig::standard();
        spaced_config.level_spacing = 2;
        let mut graph_buf2 = [0u8; 8192];
        let mut temp_buf2 = [0u8; 65536];
        let mut out_buf2 = [0u8; 65536];
        let mut out_arena2 = Arena::new(&mut out_buf2);
        let spaced = layout_with_config(
            &spaced_config,
            &mut graph_buf2,
            &mut temp_buf2,
            &mut out_arena2,
        );

        let y_at = |ir: &LayoutIRArena<'_>, level: usize| {
            ir.nodes().iter().find(|n| n.level == level).unwrap().y
        };
        // Each of the two inter-level gaps grows by level_spacing.
        assert_eq!(y_at(&spaced, 1), y_at(&base, 1) + 2);
        assert_eq!(y_at(&spaced, 2), y_at(&base, 2) + 4);
        // No trailing gap after the last level: total height grows by
        // exactly (levels - 1) * level_spacing.
        assert_eq!(spaced.height(), base.height() + 4);
    }

    // ── Cluster-width feedback (regression: external nodes rendered
    //    inside subgraph borders) ────────────────────────────────────────

    /// Build a one-subgraph graph, lay it out, and assert the nodes with
    /// the given ids stay clear of every subgraph box.
    fn assert_externals_clear_csr(
        sg_label: &str,
        nodes: &[(&str, bool)],
        edges: &[(usize, usize)],
        external_ids: &[usize],
    ) {
        let mut graph_buf = [0u8; 16384];
        let mut graph_arena = Arena::new(&mut graph_buf);
        let mut b = CsrGraphBuilder::new_with_subgraphs(&mut graph_arena, 16, 16, 256, 4, 0)
            .expect("builder");
        let sg = b.add_subgraph(0, sg_label).expect("sg");
        for (i, (label, _)) in nodes.iter().enumerate() {
            b.add_node(i, label).expect("node");
        }
        for (i, (_, inside)) in nodes.iter().enumerate() {
            if *inside {
                b.set_node_subgraph(i, sg).expect("assign");
            }
        }
        for &(f, t) in edges {
            b.add_edge(f, t).expect("edge");
        }
        let graph = b.build().expect("build");

        let config = LayoutConfig::standard();
        let mut temp_buf = [0u8; 65536];
        let mut temp_arena = Arena::new(&mut temp_buf);
        let mut out_buf = [0u8; 65536];
        let mut out_arena = Arena::new(&mut out_buf);
        let ir = compute_layout_arena_csr(&graph, &config, &mut temp_arena, &mut out_arena)
            .expect("layout");

        for sg in ir.subgraphs() {
            assert!(
                sg.x + sg.width <= ir.width(),
                "canvas clips subgraph border (right {} > width {})",
                sg.x + sg.width,
                ir.width(),
            );
            for n in ir.nodes().iter().filter(|n| external_ids.contains(&n.id)) {
                let x_overlap = n.x < sg.x + sg.width && n.x + n.width > sg.x;
                let y_overlap = n.y >= sg.y && n.y < sg.y + sg.height;
                assert!(
                    !(x_overlap && y_overlap),
                    "external node id={} overlaps subgraph box",
                    n.id,
                );
            }
        }
    }

    #[test]
    fn test_book_length_label_capped_csr() {
        let long_label = "L".repeat(300);
        let mut graph_buf = [0u8; 16384];
        let mut graph_arena = Arena::new(&mut graph_buf);
        let mut b = CsrGraphBuilder::new_with_subgraphs(&mut graph_arena, 4, 4, 512, 2, 0)
            .expect("builder");
        let sg = b.add_subgraph(0, &long_label).expect("sg");
        b.add_node(0, "A").expect("node");
        b.add_node(1, "B").expect("node");
        b.set_node_subgraph(0, sg).expect("assign");
        b.set_node_subgraph(1, sg).expect("assign");
        b.add_edge(0, 1).expect("edge");
        let graph = b.build().expect("build");

        let config = LayoutConfig::standard();
        let mut temp_buf = [0u8; 65536];
        let mut temp_arena = Arena::new(&mut temp_buf);
        let mut out_buf = [0u8; 65536];
        let mut out_arena = Arena::new(&mut out_buf);
        let ir = compute_layout_arena_csr(&graph, &config, &mut temp_arena, &mut out_arena)
            .expect("layout");

        let info = &ir.subgraphs()[0];
        assert!(
            info.width <= 40,
            "label must not widen the box past the cap (got {})",
            info.width,
        );
        assert!(
            ir.width() < 100,
            "canvas must not scale with label length (got {})",
            ir.width(),
        );
    }

    #[test]
    fn test_label_widened_subgraph_clear_of_externals_csr() {
        assert_externals_clear_csr(
            "VeryLongSubgraphLabelHere",
            &[("X", true), ("E", false), ("X2", true), ("E2", false)],
            &[(0, 2), (1, 3)],
            &[1, 3],
        );
    }

    #[test]
    fn test_cross_level_envelope_clear_of_externals_csr() {
        assert_externals_clear_csr(
            "C",
            &[
                ("WideMemberNodeAAA", true),
                ("WideMemberNodeBBB", true),
                ("m", true),
                ("ext", false),
            ],
            &[(0, 2), (1, 2), (0, 3)],
            &[3],
        );
    }
}
