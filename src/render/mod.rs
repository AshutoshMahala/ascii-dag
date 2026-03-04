//! ASCII rendering for DAG visualization.
//!
//! This module provides ASCII-art rendering capabilities for directed acyclic graphs,
//! including horizontal, vertical, and cycle visualization modes.

#[cfg(feature = "alloc")]
pub mod ascii;
pub mod chars;
#[cfg(feature = "alloc")]
pub(crate) mod classic;
pub mod colors;
#[cfg(feature = "alloc")]
pub(crate) mod connections;
#[cfg(feature = "alloc")]
pub mod scanline;
#[cfg(feature = "alloc")]
pub(crate) mod virtual_build;
#[cfg(feature = "alloc")]
pub(crate) mod virtual_render;
