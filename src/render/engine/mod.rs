//! Unified render engine (temp/05 requirements, temp/06 design).
//!
//! One paint path serving both IRs — the render layer has no "backends".
//! The engine composes **semantic cells** (what a cell means, not which
//! glyph shows it) and projects them through a charset decode table at
//! emission, so Unicode and ASCII are equal outputs of one canvas.
//!
//! ```text
//! LayoutView (both IRs)                            [RW1]
//!       ↓
//! RenderPlan  — styles, spatial index, labels      [RW2]
//!       ↓
//! Band compositor — semantic cells, Z-order        [RW3+]
//!       ↓
//! Emission — charset decode, colors, writer        [RW3+]
//! ```
//!
//! # Organization rules (temp/05 N6b)
//!
//! One concern per file; growth by addition (a new charset is a new file
//! in `charset/`); internals are `pub(crate)`; the public surface is
//! exported only from this module; soft guardrail ~600 lines per file.
//!
//! **Staging note:** the engine integrates with the public API at RW3.
//! Until then it is exercised by its unit tests only; the module-level
//! `dead_code` allowance below is removed at integration.

#![allow(dead_code)] // staged: removed at RW3 integration

pub(crate) mod cell;
pub(crate) mod charset;
pub(crate) mod color;

pub use charset::Charset;
pub use color::{CellColor, ColorMode};
