//! `TerminalRenderer` — retained terminal emission over a [`Scene`].
//!
//! The scene-pipeline shape of `render_with`/`render_to_bytes`: plan
//! once with a [`ScenePlanner`](super::scene::ScenePlanner), then
//! render the scene repeatedly — same emission options or different
//! ones, same renderer — with zero replanning and, at steady state,
//! zero allocation. Emission goes through exactly the same
//! plan→compose→emit path as the one-step wrappers, so output is
//! byte-identical to them for matching options (pinned by test).
//!
//! The renderer runs the LEAN terminal profile: cells only, a color
//! plane only for colored emission, no ownership plane — the semantic
//! composer's introspection cost never becomes part of the terminal
//! surface. Workspace behavior mirrors the composer: one retained
//! chunk, a fresh bump arena per render, growth on the heap, a
//! documented byte-unit error in fixed-workspace mode.

use super::cell::Cell;
use super::color::{CellColor, ColorMode};
use super::compose::PaintScratch;
use super::composer::CompositionRequirements;
use super::config::{ComposeBudget, EmitOptions};
use super::emit::ByteSink;
use super::plan::RenderPlan;
use super::scene::{Scene, ViewRef};
use super::view::LayoutView;
use crate::GraphError;
use crate::graph::arena::Arena;

/// Dispatch one expression over both lens backends.
macro_rules! with_view {
    ($scene:expr, $v:ident => $e:expr) => {
        match *$scene.view() {
            #[cfg(feature = "alloc")]
            ViewRef::Heap($v) => $e,
            ViewRef::Arena($v) => $e,
        }
    };
}

enum Workspace<'ws> {
    /// Growable retained chunk (never shrinks; steady state allocates
    /// nothing).
    #[cfg(feature = "alloc")]
    Heap(alloc::vec::Vec<u8>),
    /// Caller-provided fixed chunk: misfits are documented errors.
    Fixed(&'ws mut [u8]),
}

/// Renders [`Scene`]s to terminal text, retaining its workspace across
/// renders and scenes.
///
/// Emission options are construction state here (a renderer IS "how I
/// write"); planning options never are — one scene serves any number
/// of renderers, and [`GraphError::RenderSinkFailed`] surfaces writer
/// failures that the bare `fmt::Result` wrappers cannot name.
pub struct TerminalRenderer<'ws> {
    ws: Workspace<'ws>,
    emit: EmitOptions,
    band_rows_cap: usize,
}

#[cfg(feature = "alloc")]
impl TerminalRenderer<'static> {
    /// Heap renderer, presized for `requirements` under `emit`;
    /// renders of larger scenes grow the workspace to their
    /// high-water mark. Unfittable requirements presize nothing and
    /// every render then reports
    /// [`GraphError::RenderWorkspaceTooSmall`].
    pub fn new(emit: &EmitOptions, requirements: CompositionRequirements) -> Self {
        let colored = !matches!(emit.color_mode, ColorMode::None);
        Self {
            ws: Workspace::Heap(alloc::vec![
                0u8;
                requirements.terminal_bytes(colored).unwrap_or(0)
            ]),
            emit: *emit,
            band_rows_cap: requirements.band_rows_cap,
        }
    }
}

impl<'ws> TerminalRenderer<'ws> {
    /// No-alloc renderer over a caller-provided workspace. Fails at
    /// construction — byte units, nothing carved — when
    /// `requirements` do not fit under `emit`.
    pub fn new_in(
        emit: &EmitOptions,
        requirements: CompositionRequirements,
        workspace: &'ws mut [u8],
    ) -> Result<Self, GraphError> {
        let colored = !matches!(emit.color_mode, ColorMode::None);
        let needed = requirements.terminal_bytes(colored).unwrap_or(usize::MAX);
        if needed > workspace.len() {
            return Err(GraphError::RenderWorkspaceTooSmall {
                needed_bytes: needed,
                got_bytes: workspace.len(),
            });
        }
        Ok(Self {
            ws: Workspace::Fixed(workspace),
            emit: *emit,
            band_rows_cap: requirements.band_rows_cap,
        })
    }

    /// Render `scene` into any writer. A writer failure surfaces as
    /// [`GraphError::RenderSinkFailed`] — rendering state is
    /// unaffected and the render may be retried.
    pub fn render<W: core::fmt::Write>(
        &mut self,
        scene: &Scene<'_, '_>,
        out: &mut W,
    ) -> Result<(), GraphError> {
        self.render_impl(scene, out, |_| GraphError::RenderSinkFailed)
    }

    /// Render `scene` into a caller byte buffer (the no-alloc sink
    /// shape); returns the bytes written. An undersized buffer reports
    /// [`GraphError::RenderOutputTooSmall`].
    pub fn render_into(
        &mut self,
        scene: &Scene<'_, '_>,
        out: &mut [u8],
    ) -> Result<usize, GraphError> {
        let mut sink = ByteSink::new(out);
        // ByteSink's only failure mode is running out of buffer.
        match self.render_impl(scene, &mut sink, |_| GraphError::RenderOutputTooSmall) {
            Ok(()) => Ok(sink.written()),
            Err(e) => Err(e),
        }
    }

    fn render_impl<W: core::fmt::Write>(
        &mut self,
        scene: &Scene<'_, '_>,
        out: &mut W,
        sink_err: impl Fn(core::fmt::Error) -> GraphError,
    ) -> Result<(), GraphError> {
        let colored = !matches!(self.emit.color_mode, ColorMode::None);
        let cap = self.band_rows_cap;
        let emit = self.emit;
        with_view!(scene, v => {
            let req = scene.composition_requirements(&ComposeBudget::new().with_band_rows_cap(cap));
            let Some(needed) = req.terminal_bytes(colored) else {
                let got_bytes = match &self.ws {
                    #[cfg(feature = "alloc")]
                    Workspace::Heap(buf) => buf.len(),
                    Workspace::Fixed(buf) => buf.len(),
                };
                return Err(GraphError::RenderWorkspaceTooSmall {
                    needed_bytes: usize::MAX,
                    got_bytes,
                });
            };
            let chunk: &mut [u8] = match &mut self.ws {
                #[cfg(feature = "alloc")]
                Workspace::Heap(buf) => {
                    if needed > buf.len() {
                        buf.resize(needed, 0); // the only allocating event
                    }
                    buf.as_mut_slice()
                }
                Workspace::Fixed(buf) => {
                    if needed > buf.len() {
                        return Err(GraphError::RenderWorkspaceTooSmall {
                            needed_bytes: needed,
                            got_bytes: buf.len(),
                        });
                    }
                    &mut buf[..]
                }
            };
            let got_bytes = chunk.len();
            let arena = Arena::new(chunk);
            render_core(v, scene.plan(), &emit, &req, &arena, got_bytes, out)
                .map_err(|e| match e {
                    RenderFailure::Workspace(err) => err,
                    RenderFailure::Sink(fmt_err) => sink_err(fmt_err),
                })
        })
    }
}

impl Scene<'_, '_> {
    /// Upper bound on the bytes rendering this scene under `emit` can
    /// produce — sizes the caller's buffer for
    /// [`TerminalRenderer::render_into`]. The output-bytes third of
    /// the sizing split (scene storage:
    /// `estimate_scene_size` on the layout; workspace:
    /// [`CompositionRequirements`](super::composer::CompositionRequirements)).
    pub fn estimate_output_size(&self, emit: &EmitOptions) -> usize {
        let colored = !matches!(emit.color_mode, ColorMode::None);
        with_view!(self, v => super::emit::estimate_output_size(v, colored, emit.render_legend))
    }
}

/// Internal failure split: workspace carving vs the caller's sink.
enum RenderFailure {
    Workspace(GraphError),
    Sink(core::fmt::Error),
}

fn render_core<V: LayoutView, W: core::fmt::Write>(
    view: &V,
    plan: &RenderPlan<'_>,
    emit: &EmitOptions,
    req: &CompositionRequirements,
    arena: &Arena<'_>,
    got_bytes: usize,
    out: &mut W,
) -> Result<(), RenderFailure> {
    let colored = !matches!(emit.color_mode, ColorMode::None);
    let area = req.width * req.band_rows;
    let oom = || {
        RenderFailure::Workspace(GraphError::RenderWorkspaceTooSmall {
            needed_bytes: req.terminal_bytes(colored).unwrap_or(usize::MAX),
            got_bytes,
        })
    };
    let mut scratch = PaintScratch::carve(view, plan, colored, req.band_rows, arena)
        .map_err(RenderFailure::Workspace)?;
    let cells = arena.alloc_slice_default::<Cell>(area).ok_or_else(oom)?;
    let colors = if colored {
        Some(
            arena
                .alloc_slice_default::<CellColor>(area)
                .ok_or_else(oom)?,
        )
    } else {
        None
    };
    super::emit_bands(
        view,
        plan,
        emit,
        req.band_rows_cap,
        &mut scratch,
        cells,
        colors,
        out,
    )
    .map_err(RenderFailure::Sink)
}

#[cfg(all(test, feature = "std", feature = "layout-vertical"))]
mod tests {
    use super::super::config::{PlanOptions, RenderOptions};
    use super::super::scene::ScenePlanner;
    use super::*;
    use crate::graph::Graph;
    use crate::render::colors::Palette;

    fn graph() -> Graph<'static> {
        let mut g = Graph::new();
        g.add_node(1usize, "Start");
        g.add_node(2usize, "Middle");
        g.add_node(3usize, "End");
        g.add_edge(1usize, 2usize, Some("go"));
        g.add_edge(2usize, 3usize, None);
        let sg = g.add_subgraph("Stage");
        g.put_nodes(&[2]).inside(sg).unwrap();
        g
    }

    /// ONE scene, four emission modes, zero replanning — and every
    /// output byte-identical to the one-step wrapper under the same
    /// options (the wrappers and the renderer share one emission
    /// path).
    #[test]
    fn one_scene_serves_every_emission_mode() {
        let g = graph();
        let ir = g.compute_layout();
        let plan_options = RenderOptions::colored(Palette::Ansi).plan;
        let mut planner = ScenePlanner::new();
        let scene = planner.plan(&ir, &plan_options).unwrap();
        let req = scene.composition_requirements(&ComposeBudget::new());

        let emits = [
            RenderOptions::plain().emit,
            RenderOptions::ascii().emit,
            RenderOptions::colored(Palette::Ansi).emit,
            RenderOptions::ascii_colored(Palette::Ansi).emit,
        ];
        for emit in emits {
            let mut renderer = TerminalRenderer::new(&emit, req);
            let mut out = String::new();
            renderer.render(&scene, &mut out).unwrap();

            let wrapper_options = RenderOptions {
                plan: plan_options,
                emit,
                compose: ComposeBudget::new(),
            };
            assert_eq!(
                out,
                ir.render_string(&wrapper_options),
                "renderer vs wrapper under {emit:?}"
            );

            // The byte surface agrees too.
            let mut buf = vec![0u8; out.len() + 64];
            let n = renderer.render_into(&scene, &mut buf).unwrap();
            assert_eq!(core::str::from_utf8(&buf[..n]).unwrap(), out);
        }
    }

    /// Fixed-workspace mode renders fitting scenes and reports byte
    /// units on misfits — at construction and per render.
    #[test]
    fn fixed_workspace_renders_and_reports_misfits() {
        let g = graph();
        let ir = g.compute_layout();
        let mut planner = ScenePlanner::new();
        let scene = planner.plan(&ir, &PlanOptions::new()).unwrap();
        let req = scene.composition_requirements(&ComposeBudget::new());
        let emit = RenderOptions::plain().emit;

        let needed = req.terminal_bytes(false).unwrap();
        let mut ws = vec![0u8; needed];
        let mut renderer = TerminalRenderer::new_in(&emit, req, &mut ws).unwrap();
        let mut out = String::new();
        renderer.render(&scene, &mut out).unwrap();
        assert_eq!(out, ir.render_string(&RenderOptions::plain()));

        let mut tiny = vec![0u8; 8];
        assert!(matches!(
            TerminalRenderer::new_in(&emit, req, &mut tiny),
            Err(GraphError::RenderWorkspaceTooSmall { .. })
        ));
    }

    /// A failing writer surfaces as the documented sink error — the
    /// renderer stays usable afterwards.
    #[test]
    fn failing_sink_reports_render_sink_failed() {
        struct FailAfter(usize);
        impl core::fmt::Write for FailAfter {
            fn write_str(&mut self, s: &str) -> core::fmt::Result {
                if s.len() > self.0 {
                    return Err(core::fmt::Error);
                }
                self.0 -= s.len();
                Ok(())
            }
        }

        let g = graph();
        let ir = g.compute_layout();
        let mut planner = ScenePlanner::new();
        let scene = planner.plan(&ir, &PlanOptions::new()).unwrap();
        let req = scene.composition_requirements(&ComposeBudget::new());
        let mut renderer = TerminalRenderer::new(&RenderOptions::plain().emit, req);

        let err = renderer.render(&scene, &mut FailAfter(3)).unwrap_err();
        assert!(matches!(err, GraphError::RenderSinkFailed));
        assert_eq!(err.code(), "E.Render.Sink.032");

        // Retry with a healthy sink succeeds.
        let mut out = String::new();
        renderer.render(&scene, &mut out).unwrap();
        assert_eq!(out, ir.render_string(&RenderOptions::plain()));

        // The byte sink keeps its own error: undersized buffer.
        let mut tiny = [0u8; 4];
        assert!(matches!(
            renderer.render_into(&scene, &mut tiny),
            Err(GraphError::RenderOutputTooSmall)
        ));
    }
}
