//! Typed, run-scoped diagnostics.
//!
//! Fatal failures stay in `Result<T, E>` — an `Err` is authoritative
//! and never copied here. Everything NON-fatal the library wants to
//! tell you (a placeholder was auto-created, a label had nowhere to
//! go) travels as typed data through one narrow channel:
//!
//! - a [`Diagnostic`] is an owned, lifetime-free record with a stable
//!   machine code, a severity, and a semantic [`DiagnosticKind`] whose
//!   subjects use USER-FACING identity (node ids, input edge indices —
//!   never scene positions);
//! - a [`DiagnosticSink`] decides storage: collect ([`VecDiagnostics`],
//!   [`SliceDiagnostics`]), count ([`CountingDiagnostics`]), forward
//!   ([`FnDiagnostics`]), or explicitly discard
//!   ([`IgnoreDiagnostics`]) — the library never selects stderr;
//! - a [`DiagnosticRun`] owns one sink for one logical operation run,
//!   lends [`DiagnosticContext`]s to context-taking entry points, and
//!   [`finish`](DiagnosticRun::finish)es into a [`Report`] pairing the
//!   outcome with everything collected.
//!
//! Emission is best-effort and can never fail an operation: a bounded
//! sink that fills up counts what it dropped, and storage exhaustion
//! never turns a warning into an unrelated failure. Diagnostics are
//! deterministic public output: both layout backends emit the same
//! codes, subjects, and order for the same graph.
//!
//! The compatibility promise: existing codes are never repurposed and
//! never change severity or subject meaning; new kinds and codes are
//! additive; match on [`DiagnosticKind`] non-exhaustively.

/// How serious a diagnostic is. Only the classes the channel carries
/// today are defined; further classes are added when a diagnostic
/// actually uses them.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// The operation succeeded; something is worth knowing.
    Warning,
    /// A validation failure reported alongside a primary error.
    Error,
}

/// What happened — flat semantic kinds, organized by meaning rather
/// than by the pipeline stage that currently detects them (a condition
/// may move between stages without changing its public identity).
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticKind {
    /// An edge referenced an undeclared node and the graph
    /// auto-created a placeholder under the IMPLICIT missing-node
    /// policy (an explicitly chosen policy is declared intent and does
    /// not warn — the [`EdgeInsertion`](crate::EdgeInsertion) receipt
    /// is the record either way).
    PlaceholderCreated {
        /// The auto-created node's id.
        node: usize,
    },
    /// An edge label found no inline position and the overflow policy
    /// says omit — the label appears nowhere in the output.
    LabelOmitted {
        /// The edge's input index (insertion order — the same identity
        /// style callbacks and `EdgeView::input_index` use; self-loops
        /// count in this numbering).
        edge: usize,
    },
    /// `crossing_reduction_passes` was unreasonably large (possibly a
    /// negative value cast to `usize`) and was clamped.
    CrossingPassesClamped {
        /// The requested pass count.
        requested: usize,
        /// What it was clamped to.
        clamped_to: usize,
    },
    /// `crossing_reduction_passes` is high enough for diminishing
    /// returns; the value was kept.
    CrossingPassesExcessive {
        /// The requested (and kept) pass count.
        requested: usize,
    },
}

/// What a diagnostic is about, normalized across kinds: the stable
/// half of the `(code, subject)` identity. Always USER-FACING
/// identity — node ids and input edge indices, never scene positions
/// or backend-internal slots.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticSubject {
    /// A node, by its user-facing id.
    Node(usize),
    /// An edge, by its input index (insertion order — self-loops
    /// count in this numbering).
    Edge(usize),
    /// Graph-wide configuration rather than a single element.
    Configuration,
}

/// One diagnostic event: an owned, lifetime-free record. Construction
/// is crate-internal; consumers read [`kind`](Self::kind),
/// [`code`](Self::code), [`severity`](Self::severity),
/// [`subject`](Self::subject), and [`hint`](Self::hint).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Diagnostic {
    kind: DiagnosticKind,
}

impl Diagnostic {
    pub(crate) fn new(kind: DiagnosticKind) -> Self {
        Self { kind }
    }

    /// What happened.
    pub fn kind(&self) -> &DiagnosticKind {
        &self.kind
    }

    /// The permanent machine identity (a WDP code such as
    /// `W.Render.Label.031`): never repurposed, never a different
    /// severity, never a different subject interpretation.
    pub fn code(&self) -> &'static str {
        match self.kind {
            DiagnosticKind::PlaceholderCreated { .. } => crate::errors::WARN_NODE_AUTO_CREATED,
            DiagnosticKind::LabelOmitted { .. } => crate::errors::WARN_LABEL_INVISIBLE,
            DiagnosticKind::CrossingPassesClamped { .. } => crate::errors::WARN_CONFIG_CLAMPED,
            DiagnosticKind::CrossingPassesExcessive { .. } => crate::errors::WARN_CONFIG_EXCESSIVE,
        }
    }

    /// Severity, derived from the kind — never a free-floating field.
    pub fn severity(&self) -> Severity {
        match self.kind {
            DiagnosticKind::PlaceholderCreated { .. }
            | DiagnosticKind::LabelOmitted { .. }
            | DiagnosticKind::CrossingPassesClamped { .. }
            | DiagnosticKind::CrossingPassesExcessive { .. } => Severity::Warning,
        }
    }

    /// The user-facing subject this diagnostic is about — the stable
    /// half of the `(code, subject)` identity, normalized so generic
    /// consumers (dedup keys, grouping, suppression lists) need not
    /// understand every [`DiagnosticKind`] variant, including ones
    /// added after they were written.
    pub fn subject(&self) -> DiagnosticSubject {
        match self.kind {
            DiagnosticKind::PlaceholderCreated { node } => DiagnosticSubject::Node(node),
            DiagnosticKind::LabelOmitted { edge } => DiagnosticSubject::Edge(edge),
            DiagnosticKind::CrossingPassesClamped { .. }
            | DiagnosticKind::CrossingPassesExcessive { .. } => DiagnosticSubject::Configuration,
        }
    }

    /// Actionable, static advice for this kind of event.
    pub fn hint(&self) -> &'static str {
        match self.kind {
            DiagnosticKind::PlaceholderCreated { .. } => {
                "Call add_node(id, \"label\") before add_edge(), or declare the \
                 missing-node policy explicitly with set_missing_node_policy()"
            }
            DiagnosticKind::LabelOmitted { .. } => {
                "Set LabelOverflow::Legend to list unplaced labels below the graph"
            }
            DiagnosticKind::CrossingPassesClamped { .. } => {
                "Pass a small positive count (a negative value cast to usize \
                 arrives enormous and is clamped)"
            }
            DiagnosticKind::CrossingPassesExcessive { .. } => {
                "Values above 20 have diminishing returns and may be slow"
            }
        }
    }
}

impl core::fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "[{}] ", self.code())?;
        match self.kind {
            DiagnosticKind::PlaceholderCreated { node } => {
                write!(f, "node {node} auto-created as a placeholder")
            }
            DiagnosticKind::LabelOmitted { edge } => {
                write!(f, "the label of edge {edge} will not be rendered")
            }
            DiagnosticKind::CrossingPassesClamped {
                requested,
                clamped_to,
            } => {
                write!(
                    f,
                    "crossing_reduction_passes={requested} clamped to {clamped_to}"
                )
            }
            DiagnosticKind::CrossingPassesExcessive { requested } => {
                write!(f, "crossing_reduction_passes={requested} is high")
            }
        }
    }
}

/// Where diagnostics go. Object-safe; `emit` is best-effort and can
/// never fail the operation that produced the diagnostic.
pub trait DiagnosticSink {
    /// Record one diagnostic (by value — records are owned and
    /// lifetime-free).
    fn emit(&mut self, diagnostic: Diagnostic);
    /// How many diagnostics this sink could not retain (bounded sinks;
    /// `0` for unbounded and non-retaining sinks).
    fn dropped(&self) -> usize {
        0
    }
    /// The retained diagnostics, for sinks that store them (empty for
    /// counting/ignoring sinks).
    fn entries(&self) -> &[Diagnostic] {
        &[]
    }
}

/// Explicit discard: zero storage. The way quiet conveniences opt out
/// — visibly, in their implementation — rather than any API silently
/// falling back to a log.
#[derive(Debug, Default)]
pub struct IgnoreDiagnostics;

impl DiagnosticSink for IgnoreDiagnostics {
    fn emit(&mut self, _diagnostic: Diagnostic) {}
}

/// Counts by severity without retaining entries.
#[derive(Debug, Default)]
pub struct CountingDiagnostics {
    warnings: usize,
    errors: usize,
}

impl CountingDiagnostics {
    /// Warnings seen.
    pub fn warnings(&self) -> usize {
        self.warnings
    }
    /// Validation errors seen.
    pub fn errors(&self) -> usize {
        self.errors
    }
}

impl DiagnosticSink for CountingDiagnostics {
    fn emit(&mut self, diagnostic: Diagnostic) {
        match diagnostic.severity() {
            Severity::Warning => self.warnings += 1,
            Severity::Error => self.errors += 1,
            #[allow(unreachable_patterns)] // future severities count as-yet-unclassified
            _ => {}
        }
    }
}

/// Writes into caller-provided storage; when full, keeps the
/// deterministic prefix and counts the rest as dropped.
///
/// Storage is `MaybeUninit` because a [`Diagnostic`] cannot be
/// constructed by callers (there is no vacuous "nothing happened"
/// record to fill an array with) — and `Diagnostic` is `Copy`, so a
/// buffer is one array literal away:
///
/// ```
/// use core::mem::MaybeUninit;
/// use ascii_dag::{Diagnostic, SliceDiagnostics};
///
/// let mut storage = [MaybeUninit::<Diagnostic>::uninit(); 8];
/// let sink = SliceDiagnostics::new(&mut storage);
/// assert!(sink.entries().is_empty());
/// ```
#[derive(Debug)]
pub struct SliceDiagnostics<'a> {
    storage: &'a mut [core::mem::MaybeUninit<Diagnostic>],
    len: usize,
    dropped: usize,
}

impl<'a> SliceDiagnostics<'a> {
    /// Collect into `storage` (capacity = its length).
    pub fn new(storage: &'a mut [core::mem::MaybeUninit<Diagnostic>]) -> Self {
        Self {
            storage,
            len: 0,
            dropped: 0,
        }
    }
}

impl SliceDiagnostics<'_> {
    /// The retained diagnostics (the deterministic prefix when full).
    pub fn entries(&self) -> &[Diagnostic] {
        // SAFETY: `emit` wrote `storage[i]` before ever growing `len`
        // past `i`, `len` never shrinks, and `MaybeUninit<Diagnostic>`
        // has the same layout as `Diagnostic` — so the first `len`
        // slots are initialized.
        unsafe { core::slice::from_raw_parts(self.storage.as_ptr().cast::<Diagnostic>(), self.len) }
    }
}

impl DiagnosticSink for SliceDiagnostics<'_> {
    fn emit(&mut self, diagnostic: Diagnostic) {
        if self.len < self.storage.len() {
            self.storage[self.len].write(diagnostic);
            self.len += 1;
        } else {
            self.dropped += 1;
        }
    }
    fn dropped(&self) -> usize {
        self.dropped
    }
    fn entries(&self) -> &[Diagnostic] {
        SliceDiagnostics::entries(self)
    }
}

/// `alloc` convenience: collects everything, reusable capacity.
#[cfg(feature = "alloc")]
#[derive(Debug, Default)]
pub struct VecDiagnostics {
    entries: alloc::vec::Vec<Diagnostic>,
}

#[cfg(feature = "alloc")]
impl VecDiagnostics {
    /// The retained diagnostics, in discovery order.
    pub fn entries(&self) -> &[Diagnostic] {
        &self.entries
    }
}

#[cfg(feature = "alloc")]
impl DiagnosticSink for VecDiagnostics {
    fn emit(&mut self, diagnostic: Diagnostic) {
        self.entries.push(diagnostic);
    }
    fn entries(&self) -> &[Diagnostic] {
        &self.entries
    }
}

/// Callback adapter: forward each diagnostic to a closure.
pub struct FnDiagnostics<F: FnMut(Diagnostic)>(pub F);

impl<F: FnMut(Diagnostic)> DiagnosticSink for FnDiagnostics<F> {
    fn emit(&mut self, diagnostic: Diagnostic) {
        (self.0)(diagnostic);
    }
}

/// Run-wide tallies, kept by the run regardless of sink choice.
#[derive(Debug, Default, Clone, Copy)]
pub struct DiagnosticCounts {
    warnings: usize,
    errors: usize,
}

impl DiagnosticCounts {
    /// Warnings emitted this run.
    pub fn warnings(&self) -> usize {
        self.warnings
    }
    /// Validation errors emitted this run.
    pub fn errors(&self) -> usize {
        self.errors
    }
}

/// One logical operation run: owns the sink and the run-wide counts,
/// lends [`DiagnosticContext`]s to any number of phase calls, and
/// [`finish`](Self::finish)es into a [`Report`]. Starting a new run
/// starts a new diagnostic set.
#[derive(Debug, Default)]
pub struct DiagnosticRun<S> {
    sink: S,
    counts: DiagnosticCounts,
}

impl<S: DiagnosticSink> DiagnosticRun<S> {
    /// A run collecting into `sink`.
    pub fn new(sink: S) -> Self {
        Self {
            sink,
            counts: DiagnosticCounts::default(),
        }
    }

    /// Borrow a context for one or more phase calls. The borrow ends
    /// when the context drops; the run keeps accumulating across
    /// contexts.
    pub fn context(&mut self) -> DiagnosticContext<'_> {
        DiagnosticContext {
            sink: &mut self.sink,
            counts: &mut self.counts,
        }
    }

    /// Run-wide tallies so far.
    pub fn counts(&self) -> DiagnosticCounts {
        self.counts
    }

    /// Package the run: the operation's outcome plus everything the
    /// sink retained. The `Err` inside `outcome` is authoritative and
    /// is NOT duplicated into the diagnostics.
    pub fn finish<T, E>(self, outcome: Result<T, E>) -> Report<T, E, S> {
        let dropped = self.sink.dropped();
        Report {
            outcome,
            diagnostics: self.sink,
            dropped_diagnostics: dropped,
        }
    }
}

/// A temporary borrow of a [`DiagnosticRun`], passed through
/// context-taking entry points. Internal code emits through this
/// without becoming generic over the sink.
pub struct DiagnosticContext<'a> {
    sink: &'a mut dyn DiagnosticSink,
    counts: &'a mut DiagnosticCounts,
}

impl DiagnosticContext<'_> {
    pub(crate) fn emit(&mut self, kind: DiagnosticKind) {
        self.emit_diagnostic(Diagnostic::new(kind));
    }

    pub(crate) fn emit_diagnostic(&mut self, diagnostic: Diagnostic) {
        match diagnostic.severity() {
            Severity::Warning => self.counts.warnings += 1,
            Severity::Error => self.counts.errors += 1,
            #[allow(unreachable_patterns)]
            _ => {}
        }
        self.sink.emit(diagnostic);
    }
}

/// One operation, packaged: the outcome and the diagnostics collected
/// alongside it. The primary `Err` stays in the outcome (authoritative,
/// never duplicated into storage).
#[derive(Debug)]
pub struct Report<T, E, D> {
    outcome: Result<T, E>,
    diagnostics: D,
    dropped_diagnostics: usize,
}

/// Owned heap-side report.
#[cfg(feature = "alloc")]
pub type OwnedReport<T, E> = Report<T, E, VecDiagnostics>;

/// Report borrowing caller storage (no-alloc).
pub type BorrowedReport<'a, T, E> = Report<T, E, SliceDiagnostics<'a>>;

/// A failure type the unified [`Report::diagnostics`] stream can
/// project: the authoritative primary error, plus a walkable causal
/// chain when the type carries one. Implemented for the crate's error
/// types; external `E` types opt in by implementing it (the default
/// `cause` reports no chain).
pub trait ProjectedFailure {
    /// The next cause beneath this failure, if any.
    fn cause(&self) -> Option<&Self> {
        None
    }
}

impl ProjectedFailure for crate::GraphError {}

#[cfg(feature = "alloc")]
impl ProjectedFailure for crate::errors::ErrorChain {
    fn cause(&self) -> Option<&Self> {
        self.cause()
    }
}

impl ProjectedFailure for core::convert::Infallible {}

/// One item in the unified presentation stream of
/// [`Report::diagnostics`]: retained sidecar records first (discovery
/// order), then the authoritative primary failure — projected, never
/// copied into storage — then its causes, marked as causes.
#[derive(Debug, Clone, Copy)]
pub enum DiagnosticRef<'a, E> {
    /// A retained sidecar diagnostic.
    Retained(&'a Diagnostic),
    /// The primary failure from the outcome (authoritative — this is
    /// a projection of the `Err`, which remains in the outcome).
    Failure(&'a E),
    /// A cause beneath the primary failure.
    Cause(&'a E),
}

/// Walks a [`ProjectedFailure`] cause chain.
struct Causes<'a, E> {
    next: Option<&'a E>,
}

impl<'a, E: ProjectedFailure> Iterator for Causes<'a, E> {
    type Item = &'a E;
    fn next(&mut self) -> Option<&'a E> {
        let current = self.next?;
        self.next = current.cause();
        Some(current)
    }
}

impl<T, E, D: DiagnosticSink> Report<T, E, D> {
    /// The operation's outcome.
    pub fn outcome(&self) -> Result<&T, &E> {
        self.outcome.as_ref()
    }

    /// One logical stream for presenters: every retained diagnostic
    /// (deterministic discovery order), then the primary failure
    /// projected from the outcome, then its causal chain — so a
    /// presenter iterating this method alone misses nothing. The
    /// failure is a projection: it stays authoritative in
    /// [`outcome`](Self::outcome) and is never copied into storage.
    pub fn diagnostics(&self) -> impl Iterator<Item = DiagnosticRef<'_, E>>
    where
        E: ProjectedFailure,
    {
        let failure = self.outcome.as_ref().err();
        self.retained_diagnostics()
            .map(DiagnosticRef::Retained)
            .chain(failure.map(DiagnosticRef::Failure))
            .chain(
                Causes {
                    next: failure.and_then(ProjectedFailure::cause),
                }
                .map(DiagnosticRef::Cause),
            )
    }

    /// Only what the sink retained, in deterministic discovery order —
    /// the storage view (tests, tooling). The primary failure is never
    /// among these; [`diagnostics`](Self::diagnostics) is the complete
    /// presentation stream.
    pub fn retained_diagnostics(&self) -> impl Iterator<Item = &Diagnostic> {
        self.diagnostics.entries().iter()
    }

    /// Retained warnings only.
    pub fn warnings(&self) -> impl Iterator<Item = &Diagnostic> {
        self.retained_diagnostics()
            .filter(|d| matches!(d.severity(), Severity::Warning))
    }

    /// How many diagnostics a bounded sink could not retain. The
    /// primary error is never among them.
    pub fn dropped_diagnostics(&self) -> usize {
        self.dropped_diagnostics
    }

    /// Split into the outcome and the sink (with whatever it retained).
    pub fn into_parts(self) -> (Result<T, E>, D) {
        (self.outcome, self.diagnostics)
    }
}
