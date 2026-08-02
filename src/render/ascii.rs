//! ASCII rendering implementation for DAG visualization.

use crate::graph::{Graph, RenderMode};
use alloc::{string::String, vec::Vec};
use core::fmt::Write;

// Box drawing characters (Unicode)
const ARROW_RIGHT: char = '→';
pub(crate) const CYCLE_ARROW: char = '⇄'; // For cycle detection

impl<'a> Graph<'a> {
    /// Render the DAG to an ASCII string.
    ///
    /// # Examples
    ///
    /// ```
    /// use ascii_dag::graph::Graph;
    ///
    /// let dag = Graph::from_edges(
    ///     &[(1, "Start"), (2, "End")],
    ///     &[(1, 2)]
    /// );
    ///
    /// let output = dag.render();
    /// println!("{}", output);
    /// ```
    pub fn render(&self) -> String {
        let mut buf = String::with_capacity(self.estimate_size());
        self.render_to(&mut buf);
        buf
    }

    /// Render the DAG into a provided buffer (zero-allocation).
    ///
    /// # Examples
    ///
    /// ```
    /// use ascii_dag::graph::Graph;
    ///
    /// let dag = Graph::from_edges(
    ///     &[(1, "A")],
    ///     &[]
    /// );
    ///
    /// let mut buffer = String::new();
    /// dag.render_to(&mut buffer);
    /// assert!(!buffer.is_empty());
    /// ```
    pub fn render_to(&self, output: &mut String) {
        if self.nodes.is_empty() {
            output.push_str("Empty DAG");
            return;
        }

        // Check for cycles
        if self.has_cycle() {
            self.render_cycle(output);
            return;
        }

        // Determine render mode. The simple-chain shortcut is a
        // TopDown-only convenience: it traverses source→target and
        // paints `→` unconditionally, so it cannot honor a rank
        // direction. Under `Auto`, anything other than `TopDown` goes
        // through the layout pipeline, which does. An EXPLICIT
        // `RenderMode::Horizontal` still overrides — asking for the
        // chain form is a deliberate choice.
        let is_chain = self.is_simple_chain();
        let mode = match self.render_mode {
            RenderMode::Auto => {
                if is_chain && self.direction == crate::graph::Direction::TopDown {
                    RenderMode::Horizontal
                } else {
                    RenderMode::Vertical
                }
            }
            other => other,
        };

        if mode == RenderMode::Horizontal {
            self.render_horizontal(output);
            return;
        }

        // Vertical mode renders through the unified engine.
        let ir = self.compute_layout();
        let _ = ir.render_with(&crate::render::engine::RenderOptions::plain(), output);
    }

    /// Render a graph with cycles (not a valid DAG, but useful for error visualization).
    fn render_cycle(&self, output: &mut String) {
        writeln!(output, "⚠️  CYCLE DETECTED - Not a valid DAG").ok();
        writeln!(output).ok();

        // Find the cycle using DFS
        if let Some(cycle_nodes) = self.find_cycle_path() {
            writeln!(output, "Cyclic dependency chain:").ok();

            for (i, node_id) in cycle_nodes.iter().enumerate() {
                if let Some(&(id, label)) = self.nodes.iter().find(|(nid, _)| nid == node_id) {
                    self.write_node(output, id, label);

                    if i < cycle_nodes.len() - 1 {
                        write!(output, " → ").ok();
                    } else {
                        // Last node, show it cycles back
                        if let Some(&(first_id, first_label)) =
                            self.nodes.iter().find(|(nid, _)| nid == &cycle_nodes[0])
                        {
                            write!(output, " {} ", CYCLE_ARROW).ok();
                            self.write_node(output, first_id, first_label);
                        }
                    }
                }
            }
            writeln!(output).ok();
            writeln!(output).ok();
            writeln!(
                output,
                "This creates an infinite loop in error dependencies."
            )
            .ok();
        } else {
            writeln!(output, "Complex cycle detected in graph.").ok();
        }
    }

    /// Check if this is a simple chain (A → B → C, no branching).
    fn is_simple_chain(&self) -> bool {
        if self.nodes.is_empty() {
            return false;
        }

        let components = self.find_connected_components();
        if components.len() > 1 {
            return false;
        }

        for idx in 0..self.nodes.len() {
            if self.parents_count(idx) > 1 || self.children_count(idx) > 1 {
                return false;
            }
        }

        true
    }

    /// Render in horizontal mode: [A] → [B] → [C]
    fn render_horizontal(&self, output: &mut String) {
        let roots: Vec<_> = self
            .nodes
            .iter()
            .filter(|(id, _)| self.get_parents(*id).is_empty())
            .collect();

        if roots.is_empty() {
            output.push_str("(no root)");
            return;
        }

        let mut current_id = roots[0].0;
        let mut visited = Vec::new();

        loop {
            visited.push(current_id);

            if let Some(&(id, label)) = self.nodes.iter().find(|(nid, _)| *nid == current_id) {
                self.write_node(output, id, label);
            }

            let children = self.get_children(current_id);

            if children.is_empty() {
                break;
            }

            write!(output, " {} ", ARROW_RIGHT).ok();

            current_id = children[0];

            if visited.contains(&current_id) {
                break;
            }
        }

        writeln!(output).ok();
    }
}
