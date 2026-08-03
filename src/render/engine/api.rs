//! Public render surface on the IR types (temp/06 §9, M5/M8, R4/R5).
//!
//! The streaming writer is primary: `render_with` feeds any
//! `core::fmt::Write`. `render_string` is the owned std convenience;
//! `render_to_bytes` + the estimate functions are the zero-allocation
//! surface (caller arena and byte buffer, R4.3). Both IR types carry
//! the same methods — the engine underneath is one paint path, so the
//! render layer has no "backends" (N1).
//!
//! An external-backend trait (the design's M8 "Renderer" seam) is
//! deliberately not part of 0.10: it returns when a real consumer
//! defines its shape.

use super::config::RenderOptions;
use super::plan::{HitResult, RenderPlan};
use crate::GraphError;
use crate::graph::arena::Arena;
use crate::ir::arena::LayoutIRArena;

#[cfg(feature = "alloc")]
use crate::ir::LayoutIR;

macro_rules! render_surface {
    ($ir:ty) => {
        impl $ir {
            /// Stream this layout into any writer. `options` decides
            /// everything: colors when `color_mode != None` (plus the
            /// legend when `legend` is set), the charset, direction of
            /// banding, and per-element styles.
            #[cfg(feature = "alloc")]
            pub fn render_with<W: core::fmt::Write>(
                &self,
                options: &RenderOptions,
                out: &mut W,
            ) -> core::fmt::Result {
                super::render_into(self, options, out)
            }

            /// Owned-`String` convenience over [`Self::render_with`].
            #[cfg(feature = "alloc")]
            pub fn render_string(&self, options: &RenderOptions) -> alloc::string::String {
                let mut out = alloc::string::String::new();
                let _ = super::render_into(self, options, &mut out);
                out
            }

            /// Render into a caller byte buffer with all working memory
            /// carved from `arena` — the zero-allocation surface.
            /// Returns the bytes written. Undersized memory reports the
            /// WDP `Render` component: plan storage `E.Render.Plan.026`,
            /// band canvas/scratch `E.Render.Canvas.026`, output buffer
            /// `E.Render.Sink.026`. Size the arena with
            /// [`Self::estimate_render_arena_size`] and the buffer with
            /// [`Self::estimate_render_output_size`].
            pub fn render_to_bytes(
                &self,
                options: &RenderOptions,
                arena: &Arena<'_>,
                out: &mut [u8],
            ) -> Result<usize, GraphError> {
                super::render_to_bytes(self, options, arena, out)
            }

            /// Bytes of arena [`Self::render_to_bytes`] needs for this
            /// layout under `options`.
            pub fn estimate_render_arena_size(&self, options: &RenderOptions) -> usize {
                super::estimate_render_arena_size(self, options)
            }

            /// Upper bound on the bytes [`Self::render_to_bytes`] can
            /// write for this layout under `options`.
            pub fn estimate_render_output_size(&self, options: &RenderOptions) -> usize {
                super::estimate_render_output_size(self, options)
            }

            /// Build the introspectable [`RenderPlan`] for this layout
            /// (resolved styles, label placement, band partition). The
            /// plan is reusable across renders of the same layout.
            #[cfg(feature = "alloc")]
            pub fn render_plan(&self, options: &RenderOptions) -> RenderPlan<'static> {
                RenderPlan::build(self, options)
            }

            /// Arena-backed `render_plan` for no-alloc callers;
            /// exhaustion reports `E.Render.Plan.026`.
            pub fn render_plan_in<'buf>(
                &self,
                options: &RenderOptions,
                arena: &Arena<'buf>,
            ) -> Result<RenderPlan<'buf>, GraphError> {
                RenderPlan::build_in(self, options, arena)
            }

            /// What occupies the rendered cell at `(x, y)` under `plan`?
            /// Nodes win over edges, edges over subgraph boxes,
            /// matching the visual z-order.
            pub fn hit_test(&self, plan: &RenderPlan<'_>, x: usize, y: usize) -> HitResult {
                plan.element_at(self, x, y)
            }
        }
    };
}

#[cfg(feature = "alloc")]
render_surface!(LayoutIR<'_>);
render_surface!(LayoutIRArena<'_>);
