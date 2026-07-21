// The hero graph — README showcase exercising every major feature:
// nested subgraphs, cross-cluster edges, skip-level edges, edge labels,
// a self-cycle, and a reversed (back) edge.
//
// SINGLE SOURCE OF TRUTH: examples/hero.rs and the golden snapshot test
// in tests/layout_output.rs both pull this in via include!. Change the
// graph here, then regenerate the golden file:
//   cargo run --example hero 2>/dev/null > tests/golden/hero.txt
fn hero_graph() -> ascii_dag::Graph<'static> {
    let mut g = ascii_dag::Graph::new();

    // ── Nodes ────────────────────────────────────────────────────
    g.add_node(1, "Client");
    g.add_node(2, "Gateway");
    g.add_node(3, "Users");
    g.add_node(4, "Orders");
    g.add_node(5, "DB");
    g.add_node(6, "Queue");
    g.add_node(7, "Mailer");
    g.add_node(8, "Dash");

    // ── Edges (with labels) ──────────────────────────────────────
    g.add_edge(1, 2, Some("http")); // Client → Gateway
    g.add_edge(2, 3, None); // Gateway → Users
    g.add_edge(2, 4, None); // Gateway → Orders
    g.add_edge(3, 5, Some("read")); // Users → DB
    g.add_edge(4, 5, Some("write")); // Orders → DB
    g.add_edge(4, 6, Some("emit")); // Orders → Queue
    g.add_edge(6, 7, Some("notify")); // Queue → Mailer
    g.add_edge(5, 8, Some("sync")); // DB → Dash
    g.add_edge(7, 8, None); // Mailer → Dash
    g.add_edge(1, 8, Some("trace")); // Client → Dash (deep skip-level!)

    // Self-cycle: Gateway retries on failure
    g.add_edge(2, 2, Some("retry"));

    // Reversed edge: Dash feeds back to Gateway (back-edge / cycle)
    g.add_edge(8, 2, Some("feedback"));

    // ── Subgraphs ────────────────────────────────────────────────
    let svc = g.add_subgraph("Services");
    g.put_nodes(&[3, 4])
        .inside(svc)
        .expect("place nodes in Services");

    let data = g.add_subgraph("Data");
    g.put_nodes(&[5, 6])
        .inside(data)
        .expect("place nodes in Data");

    // Nested: Async inside Data
    let async_sg = g.add_subgraph("Async");
    g.put_nodes(&[6])
        .inside(async_sg)
        .expect("place Queue in Async");
    g.put_subgraphs(&[async_sg])
        .inside(data)
        .expect("nest Async in Data");

    g
}
