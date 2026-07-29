//! Configurable index types for arena-based layout.
//!
//! This module provides compile-time selection of index sizes to optimize
//! memory usage on constrained embedded targets.
//!
//! # Feature Flags
//!
//! | Feature | Index Type | Max Value | Memory per Index |
//! |---------|-----------|-----------|------------------|
//! | `arena-idx-u8` | `u8` | 255 | 1 byte |
//! | `arena-idx-u16` | `u16` | 65,535 | 2 bytes |
//! | `arena-idx-u32` | `u32` | 4,294,967,295 | 4 bytes |
//!
//! If no index feature is selected, defaults to `u32`.

/// The index type used for arena allocations.
/// Configurable via feature flags for memory optimization.
/// Priority: arena-idx-u8 > arena-idx-u16 > u32 (default)
#[cfg(feature = "arena-idx-u8")]
pub type Idx = u8;

/// 16-bit index type (`arena-idx-u16`): up to 65,535 nodes/edges.
#[cfg(all(feature = "arena-idx-u16", not(feature = "arena-idx-u8")))]
pub type Idx = u16;

#[cfg(all(
    feature = "arena",
    not(feature = "arena-idx-u8"),
    not(feature = "arena-idx-u16")
))]
/// Default 32-bit index type when no specific size is selected.
pub type Idx = u32;

// Fallback for when arena feature is enabled but no specific idx feature
#[cfg(all(
    not(feature = "arena"),
    not(feature = "arena-idx-u8"),
    not(feature = "arena-idx-u16")
))]
/// Fallback 32-bit index type.
pub type Idx = u32;

/// Maximum number of nodes/edges supported by the current index type.
pub const MAX_NODES: usize = Idx::MAX as usize;

/// Maximum number of levels supported — the index type's capacity.
///
/// Depth can never exceed node count (every level holds at least one
/// node), so the per-feature node capacity is also the natural level
/// bound. Per-level layout buffers are sized from each graph's real
/// depth, not from this constant.
pub const MAX_LEVELS: usize = MAX_NODES;

/// Coordinate type - always needs full width for x/y positions.
/// On embedded, x coordinates can exceed 255 easily with wide graphs.
pub type Coord = u16;

/// Maximum coordinate value.
pub const MAX_COORD: usize = Coord::MAX as usize;

/// Trait for converting between index types and usize.
pub trait IdxConv: Copy + Default {
    /// Convert a `usize` to this index type, returning `None` on overflow.
    fn from_usize(v: usize) -> Option<Self>;
    /// Convert this index type to `usize`.
    fn to_usize(self) -> usize;
}

impl IdxConv for u8 {
    #[inline]
    fn from_usize(v: usize) -> Option<Self> {
        if v <= u8::MAX as usize {
            Some(v as u8)
        } else {
            None
        }
    }

    #[inline]
    fn to_usize(self) -> usize {
        self as usize
    }
}

impl IdxConv for u16 {
    #[inline]
    fn from_usize(v: usize) -> Option<Self> {
        if v <= u16::MAX as usize {
            Some(v as u16)
        } else {
            None
        }
    }

    #[inline]
    fn to_usize(self) -> usize {
        self as usize
    }
}

impl IdxConv for u32 {
    #[inline]
    fn from_usize(v: usize) -> Option<Self> {
        if v <= u32::MAX as usize {
            Some(v as u32)
        } else {
            None
        }
    }

    #[inline]
    fn to_usize(self) -> usize {
        self as usize
    }
}

/// Returns the size in bytes of one Idx element.
#[inline]
pub const fn idx_size() -> usize {
    core::mem::size_of::<Idx>()
}

/// Returns the size in bytes of one Coord element.
#[inline]
pub const fn coord_size() -> usize {
    core::mem::size_of::<Coord>()
}
