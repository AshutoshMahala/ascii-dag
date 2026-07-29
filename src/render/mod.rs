//! ASCII rendering for DAG visualization.
//!
//! This module provides ASCII-art rendering capabilities for directed acyclic graphs,
//! including horizontal, vertical, and cycle visualization modes.

#[cfg(feature = "alloc")]
pub mod ascii;
pub mod chars;
pub mod colors;
pub mod engine;
#[cfg(feature = "alloc")]
mod legacy;
