# Diagnostics

The library never writes warnings to stderr. Errors remain in `Result`;
non-fatal conditions travel through a caller-owned diagnostic sink.
Neither diagnostics nor their storage belong inside the IR or scene.

## Choose the operation boundary

| Entry | What you receive |
|---|---|
| `graph.layout().quiet()` | The IR, with layout diagnostics discarded |
| `graph.layout().reported()` | An owned layout report; no planning has happened |
| `planner.plan(&ir, &options).quiet()` | `Result<Scene, GraphError>`, with planning diagnostics discarded |
| `planner.plan(&ir, &options).reported()` | An owned planning report; no earlier layout diagnostics are included |
| Either builder's `.compute(&mut cx)` | The stage's usual outcome, with diagnostics sent to your run |

The heap layout outcome is currently infallible; planning and arena
layout can fail. `.reported()` needs `alloc`. The context-taking forms
work with either collecting or non-collecting sinks.

For one report covering several stages, create one `DiagnosticRun`, lend
its context to each stage, and call `finish(outcome)` at your boundary.
`finish` packages the result you supply; it does not run a stage or
discover more diagnostics. A successful result is the success signal:
success may have warnings, and failure may have earlier warnings too.

## Layout, planning, and rendering in one report

This complete example uses default features. The undeclared node produces
a layout warning; the self-loop label produces a planning warning under
the default omit policy. The retained renderer emits the diagnosed scene
without building another plan.

```rust
use ascii_dag::{
    DiagnosticRef, DiagnosticRun, Graph, GraphError, RenderOptions,
    ScenePlanner, TerminalRenderer, VecDiagnostics,
};

let mut graph = Graph::new();
graph.add_node(1, "Job");
graph.add_edge(1, 2, None); // implicit placeholder
graph.add_edge(1, 1, Some("retry")); // no inline loop-label slot

let options = RenderOptions::plain();
let mut run = DiagnosticRun::new(VecDiagnostics::default());
let ir = graph.layout().compute(&mut run.context());
let mut planner = ScenePlanner::new();

// Keep `?` inside this closure: the outer scope finishes the report
// on success OR failure, retaining the warnings collected so far.
let outcome = (|| -> Result<String, GraphError> {
    let scene = planner.plan(&ir, &options.plan).compute(&mut run.context())?;
    let requirements = scene.composition_requirements(&options.compose);
    let mut renderer = TerminalRenderer::new(&options.emit, requirements);
    let mut output = String::new();
    renderer.render(&scene, &mut output)?;
    Ok(output)
})();
let report = run.finish(outcome);

// Presentation belongs to the application. A TUI could populate a
// notification panel here instead of printing.
for item in report.diagnostics() {
    match item {
        DiagnosticRef::Retained(d) => eprintln!("{}: {d} ({})", d.code(), d.hint()),
        DiagnosticRef::Failure(e) => eprintln!("{}: {e} ({})", e.code(), e.hint()),
        DiagnosticRef::Cause(e) => eprintln!("caused by: {e}"),
    }
}
let (outcome, _sink) = report.into_parts();
if let Ok(output) = outcome {
    print!("{output}");
}
```

To exercise failure presentation, replace the heap planner with
`let mut storage = []; let mut planner = ScenePlanner::new_in(&mut storage);`.
Construction succeeds; the plan terminal returns `RenderPlanOom`. The
report still contains the earlier layout warning and projects the primary
failure exactly once. A planning failure need not include label warnings:
planning may have failed before reaching their detection sites.

`Report::diagnostics()` yields retained records, then the primary error,
then any causes exposed by the error type's `ProjectedFailure` impl.
`GraphError` has no cause chain; `ErrorChain` can supply one. External
error types can implement `ProjectedFailure` too.

`warnings()` filters retained warnings only. `retained_diagnostics()` is
the sink's storage view; neither includes the primary error. Always
handle `outcome()` or `into_parts().0`, even if you choose not to display
diagnostics. With a bounded sink, also inspect `dropped_diagnostics()`:
the unified stream cannot reproduce records that were not retained.

## Pick storage to match the target

| Sink | Behavior |
|---|---|
| `VecDiagnostics` | Retains records in growable heap storage |
| `SliceDiagnostics` | Retains into caller storage; counts records dropped when full |
| `CountingDiagnostics` | Counts events without retaining their details |
| `FnDiagnostics` | Forwards each event to your callback without retaining it |
| `IgnoreDiagnostics` | Explicitly discards events |

A bounded collector does not need an allocator:

```rust
use ascii_dag::{Diagnostic, DiagnosticRun, SliceDiagnostics};
use core::mem::MaybeUninit;

let mut storage = [MaybeUninit::<Diagnostic>::uninit(); 16];
let mut run = DiagnosticRun::new(SliceDiagnostics::new(&mut storage));
// Lend `run.context()` to the stages; finish with their final Result.
let _cx = run.context();
```

In a no-alloc pipeline, pass a context to
`CsrGraph::compute_layout_arena_reporting`, then to
`ScenePlanner::new_in(...).plan(...).compute(...)`. Use the same run for
both, and finish it even when either stage fails. The diagnostic slice
is separate from graph, layout, scene, composition and output storage.
Filling it never turns a warning into an operation failure; the primary
error remains in `Result` regardless of sink capacity.

## Quiet paths and receipts

`compute_layout()` / `compute_layout_with_config()` are quiet layout
conveniences. `render_string`, `render_with` and `render_to_bytes` are
quiet planning/rendering conveniences. Calling them on an IR obtained
from a reported layout does not recover planning warnings. Keep and emit
the scene you already planned with a diagnostic context instead.

An unchanged standing condition reports again on a new diagnostic-aware
run. Repainting an existing scene does not rerun layout or planning.
Diagnostics use stable codes and user-facing subjects (node ids or input
edge indices); scene edge indices are a separate numbering convention.

Point events belong to insertion receipts: `add_node` returns
`NodeInsertion`, and `add_edge` returns an `EdgeHandle` whose `.receipt()`
detaches an `EdgeInsertion`. These tell you about replacements and newly
created endpoints immediately. The graph's still-undeclared placeholders
can also be a standing layout condition; the graph does not keep a queue
of past insertion events for a future report.
