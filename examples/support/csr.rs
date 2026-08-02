//! Shared `--csr` mode for examples: render the same graph through the
//! arena/no-alloc pipeline (Graph → CSR → arena IR → engine) instead of
//! the heap path. Output is byte-identical — pinned by the parity
//! suite; the flag exists so every example demonstrates both backends.
//! (`Graph::render` conveniences — horizontal chains, cycle banners —
//! are heap-path niceties; `--csr` shows the pipeline's canonical
//! vertical layout.)
//!
//! Usage in an example:
//! ```ignore
//! #[path = "support/csr.rs"]
//! mod csr;
//! // ...build g, pick options...
//! csr::print(&g, &options); // honors --csr, else heap path
//! ```
#![allow(dead_code)]

use ascii_dag::{Graph, RenderOptions};

/// True when `--csr` was passed on the command line.
pub fn requested() -> bool {
    std::env::args().any(|a| a == "--csr")
}

/// Render `g` through the CSR/arena pipeline.
#[cfg(feature = "arena")]
pub fn render(g: &Graph<'_>, options: &RenderOptions) -> String {
    let mut config = ascii_dag::LayoutConfig::standard();
    config.direction = g.direction();
    render_with(g, options, &config)
}

/// Like [`render`], with a caller-supplied layout config (e.g. to set
/// `include_dummy_nodes`).
#[cfg(feature = "arena")]
pub fn render_with(
    g: &Graph<'_>,
    options: &RenderOptions,
    config: &ascii_dag::LayoutConfig<'_>,
) -> String {
    use ascii_dag::graph::arena::Arena;
    let mut csr_buf = vec![0u8; g.estimate_csr_arena_size() * 2];
    let mut csr_arena = Arena::new(&mut csr_buf);
    let csr = g.to_csr(&mut csr_arena).expect("CSR conversion");
    // Size with the config: `include_dummy_nodes` grows the IR.
    let size = (g.estimate_layout_arena_size_with(config) * 2).max(256 * 1024);
    let mut temp_buf = vec![0u8; size];
    let mut out_buf = vec![0u8; size];
    let mut temp_arena = Arena::new(&mut temp_buf);
    let mut out_arena = Arena::new(&mut out_buf);
    let ir = csr
        .compute_layout_arena(config, &mut temp_arena, &mut out_arena)
        .expect("CSR layout");
    ir.render_string(options)
}

#[cfg(not(feature = "arena"))]
pub fn render(_g: &Graph<'_>, _options: &RenderOptions) -> String {
    eprintln!(
        "--csr needs the arena feature: cargo run --example <name> --features arena -- --csr"
    );
    std::process::exit(2)
}

#[cfg(not(feature = "arena"))]
pub fn render_with(
    g: &Graph<'_>,
    options: &RenderOptions,
    _config: &ascii_dag::LayoutConfig<'_>,
) -> String {
    render(g, options)
}

/// Print `g` rendered with `options` — through the CSR pipeline when
/// `--csr` was passed, the heap pipeline otherwise.
pub fn print(g: &Graph<'_>, options: &RenderOptions) {
    if requested() {
        println!("{}", render(g, options));
    } else {
        println!("{}", g.compute_layout().render_string(options));
    }
}
