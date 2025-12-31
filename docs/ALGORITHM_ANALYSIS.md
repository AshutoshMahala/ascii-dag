# ASCII DAG Rendering Algorithm Analysis

## Current Algorithm: Sugiyama-Style Layout

### 4-Pass Approach
1. **Level Assignment** - Fixed-point iteration, nodes placed at `max(parent_level) + 1`
2. **Crossing Reduction** - Median heuristic (4 iterations top-down/bottom-up)
3. **Coordinate Assignment** - Left-to-right with centering under parents
4. **Edge Routing** - Manhattan-style with convergence/divergence detection

### Identified Issues

#### 1. Mixed Convergence/Divergence (Cross Edges)

**Current Behavior**: Falls back to simple vertical lines
```
[A1]   [A2]
  │      │
  ↓      ↓
[B1]   [B2]
```

**Root Cause**: `draw_connections_sugiyama` checks:
```rust
if has_convergence && !has_divergence { ... }
else if has_divergence && !has_convergence { ... }
else { draw_simple_manhattan(...) }  // BOTH case drops to simple
```

**Solution Options**:
1. **Layer decomposition**: Split mixed connections into multiple routing layers
2. **Bundled edges**: Route edges through virtual waypoints
3. **ASCII limitation acceptance**: Document that K2,2+ bipartite graphs simplify

#### 2. Skip-Level Edge Rendering

**Current Behavior**: Vertical line overlaps with node column
**Root Cause**: Pass-through position uses source node's center:
```rust
node_x_coords[src_idx] - src_min_x + src_offset + width / 2
```

**Solution**: Allocate virtual columns for skip edges:
- Insert "dummy nodes" at intermediate levels
- Or use separate column tracking for skip edge routing

#### 3. Fixed Spacing in Coordinate Assignment

**Current**:
```rust
x += width + 3;  // Always 3 chars between nodes
```

**Better Approach**:
```rust
// Calculate spacing based on edge complexity
let edge_count = count_edges_between(current_level, next_level);
let spacing = BASE_SPACING + (edge_count / 2);
```

## Recommended Algorithm Improvements

### Option A: Dummy Node Insertion (Classic Sugiyama)

For edges spanning multiple levels, insert invisible dummy nodes:
```
A ──> D becomes A ──> [d1] ──> [d2] ──> D
```

Benefits:
- Proper space allocation for long edges
- Standard Sugiyama approach
- Better crossing minimization

Implementation:
```rust
fn insert_dummy_nodes(&mut self) {
    for (from, to) in self.edges.clone() {
        let from_level = self.get_level(from);
        let to_level = self.get_level(to);
        
        if to_level - from_level > 1 {
            // Insert dummies at each intermediate level
            let mut prev = from;
            for level in (from_level + 1)..to_level {
                let dummy = self.create_dummy_node(level);
                self.add_edge(prev, dummy);
                prev = dummy;
            }
            self.add_edge(prev, to);
            self.remove_edge(from, to);
        }
    }
}
```

### Option B: Port-Based Routing

Instead of center-to-center connections, use ports:
- Each node has left/center/right ports
- Edges connect to specific ports
- Reduces crossing at node boundaries

```
   [A]           
  ╱   ╲          
 ↓     ↓         
[B]   [C]        
```

### Option C: Orthogonal Edge Routing (Current + Fix)

Keep current approach but fix the mixed case:

```rust
fn draw_mixed_manhattan(&self, connections: &[(usize, usize)], ...) {
    // Sort connections by source, then by target
    // Draw in layers: vertical drop -> horizontal segment -> vertical drop
    
    // Layer 1: All sources drop vertically
    // Layer 2: Horizontal routing (may need multiple sub-layers for crossings)
    // Layer 3: Vertical arrival at targets
}
```

## ASCII Art Limitations

Some patterns are inherently difficult in ASCII:
- Diagonal lines require `/` and `\` which don't align well
- True edge crossings need special characters like `╳` or `┼`
- Multiple edges at same position are indistinguishable

### Acceptable Simplifications
- K(n,m) bipartite graphs simplified to parallel verticals
- Edge bundles shown as single lines with count annotation
- Very dense graphs shown as adjacency matrix instead

## Performance Considerations

Current: O(n * m * iterations) for crossing reduction
- n = nodes, m = edges
- 4 iterations is reasonable

Potential optimizations:
- Early termination when no improvement
- Sparse matrix for large graphs
- Incremental updates for dynamic graphs
