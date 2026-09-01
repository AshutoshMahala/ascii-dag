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

/// Mutation-context diagnostics are standing CONDITIONS re-derived
/// per run — never stored, never consumed: like a compiler warning,
/// each reports on every diagnostic-aware run until the condition is
/// FIXED, and quiet paths (`.quiet()`, `compute_layout`) neither
/// leak state nor change what a later run sees.
#[test]
fn mutation_diagnostics_reflect_current_conditions() {
    let mut g = Graph::new();
    g.add_node(1usize, "A");
    g.set_crossing_reduction_passes(25); // high → advice condition

    let report = g.layout().reported();
    assert_eq!(report.warnings().count(), 1);
    let report = g.layout().reported();
    assert_eq!(
        report.warnings().count(),
        1,
        "the condition still holds, so it reports again"
    );

    // Quiet paths have no diagnostic state to consume or leave stale.
    let _ = g.layout().quiet();
    let _ = g.compute_layout();
    let report = g.layout().reported();
    assert_eq!(
        report.warnings().count(),
        1,
        "quiet runs change nothing — there is nothing to consume"
    );

    // Fixing the configuration clears the condition...
    g.set_crossing_reduction_passes(3);
    assert_eq!(g.layout().reported().warnings().count(), 0);

    // ...and each setter call describes the CURRENT value only.
    g.set_crossing_reduction_passes(50_000); // absurd → clamped
    g.set_crossing_reduction_passes(25); // replaced by the advice note
    let report = g.layout().reported();
    let kinds: Vec<DiagnosticKind> = report.warnings().map(|d| *d.kind()).collect();
    assert_eq!(
        kinds,
        vec![DiagnosticKind::CrossingPassesExcessive { requested: 25 }],
        "a condition slot, not an event log"
    );

    // A direct pipeline replaces the shim value wholesale.
    g.set_crossing_reduction_passes(50_000);
    g.set_crossing_pipeline(ascii_dag::STANDARD);
    assert_eq!(g.layout().reported().warnings().count(), 0);
}

/// The passes note describes the graph-owned configuration — a
/// `.with_config(...)` override replaces that configuration for the
/// run, so the note must not fire against a config the run never
/// uses.
#[test]
fn config_override_suppresses_graph_passes_note() {
    use ascii_dag::LayoutConfig;
    let mut g = Graph::new();
    g.add_node(1usize, "A");
    g.set_crossing_reduction_passes(50_000);

    assert_eq!(
        g.layout().reported().warnings().count(),
        1,
        "graph-owned config selected: the condition applies"
    );
    let standard = LayoutConfig::standard();
    assert_eq!(
        g.layout()
            .with_config(&standard)
            .reported()
            .warnings()
            .count(),
        0,
        "an override replaces the configuration the note describes"
    );
}

/// The builder chain maintains the condition slot exactly like the
/// setters: a direct pipeline replaces the shim value wholesale.
#[test]
fn builder_chain_clears_stale_pass_condition() {
    let mut g = Graph::new()
        .with_crossing_reduction_passes(50_000)
        .with_crossing_pipeline(ascii_dag::STANDARD);
    g.add_node(1usize, "A");
    assert_eq!(
        g.layout().reported().warnings().count(),
        0,
        "the clamped shim value no longer exists"
    );
}

/// The two crossing-passes conditions carry DISTINCT stable codes —
/// a `(code, subject)` dedup key never collapses them.
#[test]
fn clamped_and_excessive_have_distinct_identities() {
    let mut g = Graph::new();
    g.add_node(1usize, "A");
    g.set_crossing_reduction_passes(50_000);
    let clamped = g.layout().reported();
    let clamped_code = clamped.warnings().next().expect("clamp condition").code();

    g.set_crossing_reduction_passes(25);
    let excessive = g.layout().reported();
    let excessive_code = excessive
        .warnings()
        .next()
        .expect("advice condition")
        .code();

    assert_eq!(clamped_code, "W.Graph.Dag.003");
    assert_eq!(excessive_code, "W.Graph.Dag.033");
    assert_ne!(clamped_code, excessive_code);
}

/// `Graph` keeps its auto traits: diagnostics never put interior
/// mutability (or any storage) on the graph.
#[test]
fn graph_stays_send_and_sync() {
    fn assert_auto<T: Send + Sync>() {}
    assert_auto::<Graph<'static>>();
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

/// The placeholder condition is derived from live graph state, in
/// deterministic node-insertion order — fixing the condition (either
/// way) silences it, whenever the fix happens.
#[test]
fn placeholder_condition_clears_when_fixed() {
    let mut g = Graph::new();
    g.add_node(1usize, "A");
    g.add_edge(1usize, 9usize, None); // implicit placeholder
    g.add_edge(1usize, 7usize, None); // another
    let kinds: Vec<DiagnosticKind> = g
        .layout()
        .reported()
        .warnings()
        .map(|d| *d.kind())
        .collect();
    assert_eq!(
        kinds,
        vec![
            DiagnosticKind::PlaceholderCreated { node: 9 },
            DiagnosticKind::PlaceholderCreated { node: 7 },
        ],
        "node insertion order, deterministic"
    );

    // Fix one by promotion: the receipt records the replacement, and
    // the condition narrows to the remaining placeholder.
    let receipt = g.add_node(9usize, "Nine");
    assert!(receipt.replaced);
    assert!(!receipt.replaced_involving_auto);
    let kinds: Vec<DiagnosticKind> = g
        .layout()
        .reported()
        .warnings()
        .map(|d| *d.kind())
        .collect();
    assert_eq!(kinds, vec![DiagnosticKind::PlaceholderCreated { node: 7 }]);

    // Fix the rest by declaring the policy — even AFTER creation:
    // declared intent silences, whenever it is declared.
    g.set_missing_node_policy(MissingNodePolicy::AutoCreate);
    assert_eq!(g.layout().reported().warnings().count(), 0);
}

/// AUTO-involved replacement is a point EVENT, not a condition — its
/// record is the `NodeInsertion` receipt at the call site.
#[test]
fn auto_replacement_is_the_receipts_business() {
    use ascii_dag::AUTO;
    let mut g = Graph::new();
    let auto = g.add_node(AUTO, "first"); // graph assigns an id
    let receipt = g.add_node(auto.id(), "usurper"); // explicit id hits it
    assert!(receipt.replaced);
    assert!(
        receipt.replaced_involving_auto,
        "explicit id overwrote an auto-numbered node"
    );

    // No AUTO involved → an ordinary, unflagged replacement.
    g.add_node(5usize, "five");
    let receipt = g.add_node(5usize, "five-again");
    assert!(receipt.replaced);
    assert!(!receipt.replaced_involving_auto);

    // Nothing reaches the run: the graph stores no event history.
    assert_eq!(g.layout().reported().warnings().count(), 0);
}
