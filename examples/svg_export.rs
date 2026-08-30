//! Semantic consumption: a mini SVG exporter over the scene API.
//!
//! The point of this example is what it does NOT do: it never renders
//! terminal text and never touches a charset. It plans once, then
//! reads the scene's element views — resolved geometry, colors,
//! weights, label slots — and projects them into SVG shapes. The same
//! scene could drive a TUI, a canvas, or an editor overlay; the
//! terminal emitters are just one more consumer of the same answers.
//!
//!   cargo run --example svg_export > graph.svg

use ascii_dag::render::colors::Palette;
use ascii_dag::{
    EdgePathView, Graph, LabelSlot, LineWeight, NodeKind, RenderOptions, ScenePlanner,
};
use std::fmt::Write;

/// One character cell in pixels.
const CW: usize = 10;
const CH: usize = 20;

fn px(cell: (usize, usize)) -> (usize, usize) {
    (cell.0 * CW + CW / 2, cell.1 * CH + CH / 2)
}

fn css(color: ascii_dag::CellColor) -> String {
    match color.as_rgb() {
        Some((r, g, b)) => format!("rgb({r},{g},{b})"),
        None => "currentColor".into(),
    }
}

fn main() {
    let mut g = Graph::new();
    g.add_node(1usize, "ingest");
    g.add_node(2usize, "parse");
    g.add_node(3usize, "check");
    g.add_node(4usize, "emit");
    g.add_edge(1usize, 2usize, Some("raw"));
    g.add_edge(2usize, 3usize, None);
    g.add_edge(2usize, 4usize, Some("fast path"));
    g.add_edge(3usize, 4usize, None);
    let sg = g.add_subgraph("backend");
    g.put_nodes(&[2, 3]).inside(sg).unwrap();

    let ir = g.compute_layout();

    // Plan once (style callbacks run exactly here); read many.
    let mut planner = ScenePlanner::new();
    let scene = planner
        .plan(&ir, &RenderOptions::colored(Palette::Ansi).plan)
        .expect("plan");

    let mut svg = String::new();
    let (w, h) = (scene.width() * CW, scene.height() * CH + 24);
    let _ = writeln!(
        svg,
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {w} {h}" font-family="monospace" font-size="13">"#
    );

    // Clusters first (background), from resolved subgraph views.
    for s in scene.subgraphs() {
        let _ = writeln!(
            svg,
            r#"  <rect x="{}" y="{}" width="{}" height="{}" fill="none" stroke="{}" stroke-dasharray="4 3" rx="6"/>"#,
            s.x * CW,
            s.y * CH,
            s.width * CW,
            s.height * CH,
            css(s.color),
        );
        let _ = writeln!(
            svg,
            r#"  <text x="{}" y="{}" opacity="0.7">{}</text>"#,
            s.x * CW + 8,
            s.y * CH + 15,
            s.label,
        );
    }

    // Edges: routed geometry as polylines, resolved style as stroke.
    // (This graph flows top-down, so every trunk is Y-axis; a general
    // exporter would branch on `e.flow_axis` for the horizontal
    // reading of `Corner`/`SideChannel`.)
    for e in scene.edges() {
        let mut pts = vec![px(e.from)];
        match e.path {
            EdgePathView::Direct => {}
            EdgePathView::Corner { bend_at } => {
                pts.push(px((e.from.0, bend_at)));
                pts.push(px((e.to.0, bend_at)));
            }
            EdgePathView::SideChannel {
                channel_at,
                span_start,
                span_end,
            } => {
                pts.push(px((e.from.0, span_start)));
                pts.push(px((channel_at, span_start)));
                pts.push(px((channel_at, span_end)));
                pts.push(px((e.to.0, span_end)));
            }
            EdgePathView::MultiSegment { waypoints, .. } => {
                pts.extend(waypoints.iter().map(|&p| px(p)));
            }
            _ => {}
        }
        pts.push(px(e.to));
        let points: Vec<String> = pts.iter().map(|(x, y)| format!("{x},{y}")).collect();
        let dash = match e.weight {
            LineWeight::Dashed => r#" stroke-dasharray="5 4""#,
            _ => "",
        };
        let _ = writeln!(
            svg,
            r#"  <polyline points="{}" fill="none" stroke="{}"{dash} marker-end="url(#arrow)"/>"#,
            points.join(" "),
            css(e.color),
        );
        // Labels go where the PLAN put them: inline at their resolved
        // cell, or collected below if they overflowed to the legend.
        if let Some(label) = e.label {
            if let LabelSlot::Inline { x, y, .. } = label.slot {
                let _ = writeln!(
                    svg,
                    r#"  <text x="{}" y="{}" fill="{}">{}</text>"#,
                    x * CW,
                    y * CH + 14,
                    css(label.color),
                    label.text,
                );
            }
        }
    }

    // Nodes above edges (the same z-order hit-testing uses).
    for n in scene.nodes() {
        if matches!(n.kind, NodeKind::Dummy) {
            continue;
        }
        let _ = writeln!(
            svg,
            r#"  <rect x="{}" y="{}" width="{}" height="{}" fill="white" stroke="black" rx="3"/>"#,
            n.x * CW,
            n.y * CH,
            n.width * CW,
            n.height.max(1) * CH,
        );
        let _ = writeln!(
            svg,
            r#"  <text x="{}" y="{}" text-anchor="middle">{}</text>"#,
            (n.x + n.width / 2) * CW,
            n.y * CH + 15,
            n.label,
        );
    }

    // Overflowed labels: the legend list, below the canvas.
    for (i, e) in scene.legend().enumerate() {
        if let Some(label) = e.label {
            let _ = writeln!(
                svg,
                r#"  <text x="4" y="{}" fill="{}">{} → {}: {}</text>"#,
                scene.height() * CH + 16 + i * 16,
                css(label.color),
                e.from_id,
                e.to_id,
                label.text,
            );
        }
    }

    let _ = writeln!(
        svg,
        r#"  <defs><marker id="arrow" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse"><path d="M 0 0 L 10 5 L 0 10 z"/></marker></defs>"#
    );
    let _ = writeln!(svg, "</svg>");
    println!("{svg}");
}
