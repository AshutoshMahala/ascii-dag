//! ASCII rendering for DAG visualization.
//!
//! This module provides ASCII-art rendering capabilities for directed acyclic graphs,
//! including horizontal, vertical, and cycle visualization modes.

pub mod ascii;
pub mod chars;
pub(crate) mod classic;
pub mod colors;
pub(crate) mod connections;
pub mod scanline;
pub(crate) mod virtual_build;
pub(crate) mod virtual_render;
