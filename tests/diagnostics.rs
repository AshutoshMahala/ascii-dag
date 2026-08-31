//! The diagnostics contract, end to end: typed events into a caller's
//! run, receipts at mutation sites, no stderr anywhere — and the
//! release-gate proof that one diagnostic run survives successful AND
//! failed planning.

#![cfg(feature = "layout-vertical")]

use ascii_dag::{
    CountingDiagnostics, Diagnostic, DiagnosticKind, DiagnosticRef, DiagnosticRun,
    DiagnosticSubject, Graph, MissingNodePolicy, PlanOptions, ScenePlanner, Severity,
    SliceDiagnostics, VecDiagnostics,
};

/// A short label that places plus one that fits nowhere — under the
/// default Omit policy the second becomes a LabelOmitted diagnostic.
fn omitting_graph() -> Graph<'static> {
    let mut g = Graph::new();
    g.add_node(1usize, "A");
    g.add_node(2usize, "B");
    g.add_node(3usize, "C");
    g.add_edge(
        1usize,
        2usize,
        Some("an-extremely-long-label-that-cannot-possibly-fit-inline-anywhere-at-all"),
    );
    g.add_edge(1usize, 3usize, Some("ok"));
    g
}

/// THE release-gate proof: a run collects across plans, stays usable
/// after a scene drops, keeps its pre-failure events through a FAILED
/// plan, and finishes into a report either way.
#[test]
fn run_survives_successful_and_failed_planning() {
    let g = omitting_graph();
    let ir = g.compute_layout();
    let options = PlanOptions::new();

    let mut run = DiagnosticRun::new(VecDiagnostics::default());

    // Successful plan: the omitted label lands in the run; the scene
    // drops and the run keeps collecting.
    let mut planner = ScenePlanner::new();
    {
        let scene = planner
            .plan(&ir, &options)
            .compute(&mut run.context())
            .unwrap();
        assert!(scene.width() > 0);
    }
    assert_eq!(run.counts().warnings(), 1);

    // Failed plan: a fixed workspace far too small. The error is
    // authoritative through Result; the run's earlier events survive.
    let mut tiny = [0u8; 8];
    let mut fixed = ScenePlanner::new_in(&mut tiny);
    let err = fixed
        .plan(&ir, &options)
        .compute(&mut run.context())
        .err()
        .expect("8 bytes cannot hold a plan");

    let report = run.finish::<(), _>(Err(err));
    assert!(report.outcome().is_err());
    // The unified stream: retained events first, then the primary
    // failure projected from the outcome — a presenter iterating
    // `diagnostics()` alone misses nothing, while storage never
    // holds the error twice.
    let stream: Vec<DiagnosticRef<'_, _>> = report.diagnostics().collect();
    assert!(
        matches!(
            stream.as_slice(),
            [DiagnosticRef::Retained(d), DiagnosticRef::Failure(_)]
                if matches!(d.kind(), DiagnosticKind::LabelOmitted { .. })
        ),
        "pre-failure events intact, failure projected once: {stream:?}"
    );
    assert_eq!(
        report.retained_diagnostics().count(),
        1,
        "the error is never copied into storage"
    );
    assert_eq!(report.dropped_diagnostics(), 0);
}

/// LabelOmitted carries the INPUT edge index — self-loops count in
/// that numbering — and both backends emit identical kind sequences.
#[test]
#[cfg(feature = "arena")]
fn omitted_labels_are_input_indexed_and_backend_deterministic() {
    // Loop FIRST so input and scene indices diverge.
    let mut g = Graph::new();
    g.add_node(1usize, "A");
    g.add_node(2usize, "B");
    g.add_edge(1usize, 1usize, Some("retry")); // input 0: loop, never places
    g.add_edge(
        1usize,
        2usize,
        Some("an-extremely-long-label-that-cannot-possibly-fit-inline-anywhere-at-all"),
    ); // input 1: unplaceable

    let collect = |ir: &dyn Fn(&mut DiagnosticRun<VecDiagnostics>)| {
        let mut run = DiagnosticRun::new(VecDiagnostics::default());
        ir(&mut run);
        let (_, sink) = run.finish::<(), ()>(Ok(())).into_parts();
        sink.entries().iter().map(|d| *d.kind()).collect::<Vec<_>>()
    };

    let heap_ir = g.compute_layout();
    let heap_kinds = collect(&|run| {
        let mut planner = ScenePlanner::new();
        planner
            .plan(&heap_ir, &PlanOptions::new())
            .compute(&mut run.context())
            .unwrap();
    });

    use ascii_dag::LayoutConfig;
    use ascii_dag::graph::arena::Arena;
    let mut csr_buf = vec![0u8; g.estimate_csr_arena_size() * 2];
    let mut csr_arena = Arena::new(&mut csr_buf);
    let csr = g.to_csr(&mut csr_arena).unwrap();
    let size = (g.estimate_layout_arena_size() * 2).max(256 * 1024);
    let mut temp_buf = vec![0u8; size];
    let mut out_buf = vec![0u8; size];
    let mut temp_arena = Arena::new(&mut temp_buf);
    let mut out_arena = Arena::new(&mut out_buf);
    let arena_ir = csr
        .compute_layout_arena(&LayoutConfig::standard(), &mut temp_arena, &mut out_arena)
        .unwrap();
    let arena_kinds = collect(&|run| {
        let mut planner = ScenePlanner::new();
        planner
            .plan(&arena_ir, &PlanOptions::new())
            .compute(&mut run.context())
            .unwrap();
    });

    assert_eq!(heap_kinds, arena_kinds, "backend diagnostic parity");
    // Routed labels emit first (in list order), then loop labels —
    // both by INPUT index.
    assert_eq!(
        heap_kinds,
        vec![
            DiagnosticKind::LabelOmitted { edge: 1 },
            DiagnosticKind::LabelOmitted { edge: 0 },
        ]
    );
}

/// Receipts serve the call site; the diagnostic serves the run — and
/// only under the IMPLICIT policy. Declared intent is silent.
#[test]
fn receipts_and_placeholder_policy() {
    // Implicit policy: receipt AND a replayed diagnostic.
    let mut g = Graph::new();
    g.add_node(1usize, "A");
    let receipt = g.add_edge(1usize, 9usize, None); // 9 undeclared
    assert_eq!(receipt.edge, 0);
    assert!(!receipt.created_source);
    assert!(receipt.created_target);

    let mut run = DiagnosticRun::new(VecDiagnostics::default());
    let _ir = g.layout().compute(&mut run.context());
    let (_, sink) = run.finish::<(), ()>(Ok(())).into_parts();
    assert!(
        sink.entries()
            .iter()
            .any(|d| matches!(d.kind(), DiagnosticKind::PlaceholderCreated { node: 9 })),
        "implicit auto-create is diagnostic-worthy"
    );

    // Explicit policy: the receipt is the sole record.
    let mut g = Graph::new();
    g.set_missing_node_policy(MissingNodePolicy::AutoCreate);
    g.add_node(1usize, "A");
    let receipt = g.add_edge(1usize, 9usize, None);
    assert!(receipt.created_target);
    let mut run = DiagnosticRun::new(VecDiagnostics::default());
    let _ir = g.layout().compute(&mut run.context());
    assert_eq!(
        run.counts().warnings(),
        0,
        "declared intent is not suspicious"
    );
}

/// Bounded sinks keep the deterministic prefix and count the rest;
/// counting sinks tally without storing; the envelope exposes stable
/// code, severity, and hint.
#[test]
fn sinks_and_envelope() {
    let g = omitting_graph();
    let ir = g.compute_layout();

    // Bounded slice sink, positive capacity: retains the
    // deterministic prefix, counts the overflow, never fails the
    // operation. (`MaybeUninit` storage: a `Diagnostic` has no
    // vacuous public constructor to fill an array with.)
    let two = {
        let mut g = omitting_graph();
        g.add_edge(3usize, 3usize, Some("loop labels never place inline"));
        g
    };
    let ir2 = two.compute_layout();
    let mut storage = [core::mem::MaybeUninit::<Diagnostic>::uninit(); 1];
    let mut run = DiagnosticRun::new(SliceDiagnostics::new(&mut storage));
    let mut planner = ScenePlanner::new();
    planner
        .plan(&ir2, &PlanOptions::new())
        .compute(&mut run.context())
        .unwrap();
    assert_eq!(run.counts().warnings(), 2, "counts are run-owned");
    let report = run.finish::<(), ()>(Ok(()));
    assert_eq!(report.dropped_diagnostics(), 1);
    let retained: Vec<&Diagnostic> = report.retained_diagnostics().collect();
    assert_eq!(retained.len(), 1, "capacity-1 slice keeps the prefix");
    assert!(
        matches!(retained[0].kind(), DiagnosticKind::LabelOmitted { edge: 0 }),
        "the FIRST event is the retained one"
    );

    // Counting sink.
    let mut run = DiagnosticRun::new(CountingDiagnostics::default());
    let mut planner = ScenePlanner::new();
    planner
        .plan(&ir, &PlanOptions::new())
        .compute(&mut run.context())
        .unwrap();
    let (_, sink) = run.finish::<(), ()>(Ok(())).into_parts();
    assert_eq!(sink.warnings(), 1);

    // The envelope: stable code, derived severity, actionable hint.
    let mut run = DiagnosticRun::new(VecDiagnostics::default());
    let mut planner = ScenePlanner::new();
    planner
        .plan(&ir, &PlanOptions::new())
        .compute(&mut run.context())
        .unwrap();
    let (_, sink) = run.finish::<(), ()>(Ok(())).into_parts();
    let d = sink.entries()[0];
    assert_eq!(d.code(), "W.Render.Label.031");
    assert_eq!(d.severity(), Severity::Warning);
    assert!(d.hint().contains("LabelOverflow::Legend"));
    assert!(format!("{d}").starts_with("[W.Render.Label.031]"));
}

/// The plan run's report terminal packages the outcome — success or
/// failure — with everything the run collected.
#[test]
fn plan_reported_packages_either_outcome() {
    let g = omitting_graph();
    let ir = g.compute_layout();
    let options = PlanOptions::new();

    let mut planner = ScenePlanner::new();
    let report = planner.plan(&ir, &options).reported();
    assert!(report.outcome().is_ok());
    assert_eq!(report.warnings().count(), 1, "the omitted label");

    let mut tiny = [0u8; 8];
    let mut fixed = ScenePlanner::new_in(&mut tiny);
    let report = fixed.plan(&ir, &options).reported();
    assert!(
        report.outcome().is_err(),
        "the error is the outcome, not a diagnostic"
    );
}

/// The fluent report terminal owns a complete operation report.
#[test]
fn layout_reported_owns_the_run() {
    let mut g = Graph::new();
    g.add_node(1usize, "A");
    g.add_edge(1usize, 9usize, None); // implicit placeholder
    let report = g.layout().reported();
    assert!(report.outcome().is_ok());
    assert_eq!(
        report
            .warnings()
            .filter(|d| matches!(d.kind(), DiagnosticKind::PlaceholderCreated { node: 9 }))
            .count(),
        1
    );
}

/// Mutation diagnostics are delivered to the FIRST layout-run
/// terminal and consumed by it: later runs report only later
/// mutations, and a quiet run visibly discards rather than defers.
#[test]
fn mutation_diagnostics_deliver_exactly_once() {
    let mut g = Graph::new();
    g.add_node(1usize, "A");
    g.set_crossing_reduction_passes(25); // high → advice

    let report = g.layout().reported();
    assert_eq!(report.warnings().count(), 1);
    let report = g.layout().reported();
    assert_eq!(
        report.warnings().count(),
        0,
        "one delivery — no permanent graph history"
    );

    g.set_crossing_reduction_passes(30);
    let _ = g.layout().quiet();
    let report = g.layout().reported();
    assert_eq!(
        report.warnings().count(),
        0,
        "quiet explicitly discards outstanding events, it does not defer them"
    );

    g.set_crossing_reduction_passes(40);
    let report = g.layout().reported();
    assert_eq!(
        report.warnings().count(),
        1,
        "new mutations surface on the next run"
    );
}

/// `(code, subject)` is the generic identity: subjects normalize
/// across kinds, so consumers group and dedup without matching every
/// (non-exhaustive) variant.
#[test]
fn subjects_normalize_without_kind_matching() {
    let mut g = Graph::new();
    g.add_node(1usize, "A");
    g.add_edge(1usize, 9usize, None); // implicit placeholder → Node(9)
    g.set_crossing_reduction_passes(50_000); // clamp → Configuration

    let mut run = DiagnosticRun::new(VecDiagnostics::default());
    let ir = g.layout().compute(&mut run.context());
    let mut planner = ScenePlanner::new();
    let omitting = omitting_graph().compute_layout();
    drop(ir);
    planner
        .plan(&omitting, &PlanOptions::new())
        .compute(&mut run.context())
        .unwrap(); // omitted label → Edge(0)
    let (_, sink) = run.finish::<(), ()>(Ok(())).into_parts();
    let subjects: Vec<DiagnosticSubject> = sink.entries().iter().map(|d| d.subject()).collect();
    assert!(subjects.contains(&DiagnosticSubject::Node(9)));
    assert!(subjects.contains(&DiagnosticSubject::Configuration));
    assert!(subjects.contains(&DiagnosticSubject::Edge(0)));
}

/// Configuration clamps and AUTO-involved replacement are typed
/// diagnostics on the graph, delivered to the next diagnostic-aware
/// layout run.
#[test]
fn mutation_diagnostics_replay_at_layout() {
    let mut g = Graph::new();
    g.add_node(1usize, "A");
    g.add_node(1usize, "A2"); // replace, no AUTO involved → silent
    g.set_crossing_reduction_passes(50_000); // absurd → clamped
    g.set_crossing_reduction_passes(25); // high → advice

    let mut run = DiagnosticRun::new(VecDiagnostics::default());
    let _ir = g.layout().compute(&mut run.context());
    let (_, sink) = run.finish::<(), ()>(Ok(())).into_parts();
    let kinds: Vec<DiagnosticKind> = sink.entries().iter().map(|d| *d.kind()).collect();
    assert_eq!(
        kinds,
        vec![
            DiagnosticKind::CrossingPassesClamped {
                requested: 50_000,
                clamped_to: 0,
            },
            DiagnosticKind::CrossingPassesExcessive { requested: 25 },
        ]
    );
}
