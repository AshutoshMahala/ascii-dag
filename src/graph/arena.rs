//! Bump/arena allocator for `no_std` and embedded environments.
//!
//! This module provides a simple arena allocator that can be used
//! for temporary allocations during graph layout and rendering.
//!
//! # Trade-offs
//!
//! | Aspect | Arena | Heap |
//! |--------|-------|------|
//! | Speed | ⚡ Very fast (pointer bump) | Slower (bookkeeping) |
//! | Memory | Uses ~5-30x CSR estimate | Uses only what's needed |
//! | `no_std` | ✅ Works | ❌ Needs allocator |
//! | Predictability | Must estimate upfront | Allocates on demand |
//!
//! # Usage
//!
//! ```
//! use ascii_dag::graph::arena::Arena;
//!
//! // Provide a buffer (stack or static)
//! let mut buffer = [0u8; 64 * 1024]; // 64 KB
//! let mut arena = Arena::new(&mut buffer);
//!
//! // Allocate slices from the arena
//! let nums: &mut [usize] = arena.alloc_slice_default(100).unwrap();
//! nums[0] = 42;
//!
//! // Reset for reuse (O(1)).
//! // SAFETY: no allocation from the arena is used after this point.
//! unsafe { arena.reset() };
//! ```

use core::cell::Cell;
use core::marker::PhantomData;
use core::mem::{align_of, size_of};

/// A simple bump allocator backed by a user-provided buffer.
///
/// All allocations are contiguous and cannot be individually freed.
/// Call `reset()` to free all allocations at once (O(1) operation).
pub struct Arena<'a> {
    start: *mut u8,
    end: *mut u8,
    ptr: Cell<*mut u8>,
    alloc_count: Cell<usize>,
    _marker: PhantomData<&'a mut [u8]>,
}

impl<'a> Arena<'a> {
    /// Create a new arena backed by the provided buffer.
    ///
    /// The buffer can be stack-allocated, static, or heap-allocated.
    #[inline]
    pub fn new(buffer: &'a mut [u8]) -> Self {
        let range = buffer.as_mut_ptr_range();
        Self {
            start: range.start,
            end: range.end,
            ptr: Cell::new(range.start),
            alloc_count: Cell::new(0),
            _marker: PhantomData,
        }
    }

    /// Allocate raw memory for `count` items of type T.
    ///
    /// Returns a pointer and does NOT borrow the arena mutably past the call.
    /// This allows multiple allocations before converting to slices.
    /// Memory is zeroed by default. Use `alloc_raw_uninit` for uninitialized memory.
    ///
    /// # Safety
    ///
    /// The returned pointer is valid for `count` items of type T.
    /// The caller must ensure:
    /// - The arena outlives any slices created from this pointer
    /// - The memory is properly initialized before reading
    /// - No other code accesses this memory region while slices are held
    ///
    /// # Example
    ///
    /// ```
    /// use ascii_dag::graph::arena::Arena;
    ///
    /// let mut buffer = [0u8; 1024];
    /// let mut arena = Arena::new(&mut buffer);
    ///
    /// // Allocate multiple regions
    /// let (ptr1, len1) = arena.alloc_raw::<usize>(10).unwrap();
    /// let (ptr2, len2) = arena.alloc_raw::<usize>(20).unwrap();
    ///
    /// // Convert to slices (unsafe because we're asserting exclusive access)
    /// unsafe {
    ///     let slice1 = core::slice::from_raw_parts_mut(ptr1, len1);
    ///     let slice2 = core::slice::from_raw_parts_mut(ptr2, len2);
    ///     slice1[0] = 42;
    ///     slice2[0] = 99;
    /// }
    /// ```
    #[inline]
    pub fn alloc_raw<T: Copy>(&mut self, count: usize) -> Option<(*mut T, usize)> {
        self.alloc_raw_inner::<T>(count, true)
    }

    /// Allocate raw memory for `count` items of type T WITHOUT zeroing.
    ///
    /// This is faster than `alloc_raw` but the memory contains garbage.
    /// Use when you will immediately overwrite all values.
    ///
    /// # Safety
    ///
    /// Same as `alloc_raw`, plus: caller MUST initialize all memory before reading.
    #[inline]
    pub fn alloc_raw_uninit<T: Copy>(&mut self, count: usize) -> Option<(*mut T, usize)> {
        self.alloc_raw_inner::<T>(count, false)
    }

    #[inline]
    fn alloc_raw_inner<T: Copy>(&self, count: usize, zero: bool) -> Option<(*mut T, usize)> {
        if count == 0 {
            return Some((core::ptr::NonNull::<T>::dangling().as_ptr(), 0));
        }

        // Checked arithmetic throughout: an adversarial `count` must
        // produce `None`, never a wrapped-around (too small) size that
        // safe callers would then write past.
        let size = size_of::<T>().checked_mul(count)?;
        let align = align_of::<T>();

        // Current pointer
        let current = self.ptr.get() as usize;

        // Align the pointer
        let aligned_ptr = current.checked_add(align - 1)? & !(align - 1);
        let new_ptr = aligned_ptr.checked_add(size)?;

        // Check bounds
        if new_ptr > self.end as usize {
            return None; // Out of memory
        }

        let ptr = aligned_ptr as *mut u8;

        // Only zero if requested
        if zero {
            unsafe {
                core::ptr::write_bytes(ptr, 0, size);
            }
        }

        // Update state
        self.ptr.set(new_ptr as *mut u8);
        self.alloc_count.set(self.alloc_count.get() + 1);

        Some((ptr as *mut T, count))
    }

    /// Allocate a slice of `count` items, initialized to zero bytes.
    ///
    /// Returns `None` if there isn't enough space in the arena.
    ///
    /// # Safety
    ///
    /// The all-zero bit pattern must be a valid value of `T` (true for
    /// integers, floats, raw pointers-as-options, and `#[repr(C)]`
    /// aggregates of such; NOT guaranteed for arbitrary enums, `bool`
    /// wrappers, or references). Prefer [`Self::alloc_slice_default`],
    /// which is safe for any `T: Copy + Default`.
    #[inline]
    pub unsafe fn alloc_slice_zeroed<T: Copy>(&self, count: usize) -> Option<&'a mut [T]> {
        self.alloc_slice_inner::<T>(count, true)
    }

    /// Allocate a slice of `count` items WITHOUT initialization.
    ///
    /// This is faster but memory contains garbage. Use when you will
    /// immediately overwrite all values (e.g., in a loop).
    ///
    /// # Safety
    ///
    /// The returned slice refers to uninitialized memory. The caller
    /// must write every element before any read, and `T` must tolerate
    /// the transient invalid contents (in strict terms: prefer raw
    /// pointer writes via [`Self::alloc_raw_uninit`] for types with
    /// validity invariants such as enums or `bool`).
    #[inline]
    pub unsafe fn alloc_slice_uninit<T: Copy>(&self, count: usize) -> Option<&'a mut [T]> {
        self.alloc_slice_inner::<T>(count, false)
    }

    #[inline]
    fn alloc_slice_inner<T: Copy>(&self, count: usize, zero: bool) -> Option<&'a mut [T]> {
        if count == 0 {
            return Some(unsafe {
                core::slice::from_raw_parts_mut(core::ptr::NonNull::dangling().as_ptr(), 0)
            });
        }

        let (ptr, _size) = self.alloc_raw_inner::<T>(count, zero)?;

        // SAFETY: we've allocated valid memory and bound it to lifetime 'a
        // The bump pointer ensures disjointness from future allocations.
        // We rely on the borrow checker to ensure the arena itself outlives 'a (which is true since we borrow 'a mut [u8])
        Some(unsafe { core::slice::from_raw_parts_mut(ptr, count) })
    }

    /// Allocate a slice of `count` items, initialized to their default value.
    ///
    /// Returns `None` if there isn't enough space in the arena.
    ///
    /// Every element is written through a raw pointer **before** the
    /// `&mut [T]` is formed, so no reference to uninitialized (or
    /// invalid-bit-pattern) memory ever exists — sound for any
    /// `T: Copy + Default`, including enums.
    #[inline]
    pub fn alloc_slice_default<T: Copy + Default>(&self, count: usize) -> Option<&'a mut [T]> {
        if count == 0 {
            // SAFETY: a zero-length slice from a dangling, well-aligned
            // pointer is always valid.
            return Some(unsafe {
                core::slice::from_raw_parts_mut(core::ptr::NonNull::dangling().as_ptr(), 0)
            });
        }
        let (ptr, _size) = self.alloc_raw_inner::<T>(count, false)?;
        // SAFETY: `ptr` is valid for `count` writes of `T` (freshly
        // reserved, aligned, in-bounds); writing through the raw
        // pointer requires no validity of the old bytes.
        unsafe {
            for i in 0..count {
                ptr.add(i).write(T::default());
            }
            // SAFETY: all `count` elements are now initialized; the
            // bump pointer guarantees disjointness from every other
            // allocation, and the backing buffer outlives `'a`.
            Some(core::slice::from_raw_parts_mut(ptr, count))
        }
    }

    /// Allocate space for a single value, returning a mutable reference.
    #[inline]
    pub fn alloc<T: Copy + Default>(&self) -> Option<&'a mut T> {
        self.alloc_slice_default::<T>(1).map(|s| &mut s[0])
    }

    /// Reset the arena, freeing all allocations.
    ///
    /// This is an O(1) operation - it just resets the bump pointer.
    ///
    /// # Safety
    ///
    /// Every slice or reference previously carved from this arena
    /// becomes dangling-equivalent: later allocations will reuse the
    /// same memory while old `&mut` borrows may still be live (the
    /// borrow checker cannot see this). The caller must guarantee that
    /// **no allocation from this arena is used again** after the call.
    #[inline]
    pub unsafe fn reset(&self) {
        self.ptr.set(self.start);
        self.alloc_count.set(0);
    }

    /// Save the current allocation position (watermark).
    ///
    /// Use with [`Self::restore_position`] to implement arena "bands" — allocate
    /// scratch buffers, do work, then rewind the arena pointer so the
    /// scratch region can be reused by the next phase.
    ///
    /// # Example
    /// ```ignore
    /// let mark = arena.save_position();
    /// let scratch = arena.alloc_slice_zeroed::<u8>(1024).unwrap();
    /// // ... use scratch ...
    /// arena.restore_position(mark); // scratch memory is now reusable
    /// ```
    #[inline]
    pub fn save_position(&self) -> usize {
        self.ptr.get() as usize - self.start as usize
    }

    /// Restore the allocation pointer to a previously saved position.
    ///
    /// All allocations made *after* the saved position are invalidated.
    ///
    /// # Safety
    ///
    /// Allocations carved after the watermark alias memory that later
    /// allocations will reuse. The caller must guarantee that no slice
    /// or reference obtained from this arena after the corresponding
    /// [`Self::save_position`] call is ever used again.
    #[inline]
    pub unsafe fn restore_position(&self, saved: usize) {
        let target = (self.start as usize + saved) as *mut u8;
        debug_assert!(target as usize <= self.end as usize);
        debug_assert!(target as usize >= self.start as usize);
        self.ptr.set(target);
    }

    /// Returns the number of bytes currently used.
    #[inline]
    pub fn used(&self) -> usize {
        (self.ptr.get() as usize) - (self.start as usize)
    }

    /// Returns the total capacity of the arena.
    #[inline]
    pub fn capacity(&self) -> usize {
        (self.end as usize) - (self.start as usize)
    }

    /// Returns the number of bytes remaining.
    #[inline]
    pub fn remaining(&self) -> usize {
        (self.end as usize) - (self.ptr.get() as usize)
    }

    /// Returns the number of allocations made since the last reset.
    #[inline]
    pub fn alloc_count(&self) -> usize {
        self.alloc_count.get()
    }
}

/// A growable vector backed by an arena.
///
/// Unlike `Vec`, this cannot grow beyond its initial capacity
/// and does not deallocate when dropped.
pub struct ArenaVec<'a, T> {
    data: &'a mut [T],
    len: usize,
}

impl<'a, T: Copy + Default> ArenaVec<'a, T> {
    /// Create a new ArenaVec with the given capacity.
    #[inline]
    pub fn new(arena: &Arena<'a>, capacity: usize) -> Option<Self> {
        // No unsafe transmute needed! alloc_slice_default now returns &'a mut [T] directly
        let data = arena.alloc_slice_default::<T>(capacity)?;
        Some(Self { data, len: 0 })
    }

    /// Push an item onto the vector.
    ///
    /// Returns `false` if the vector is at capacity.
    #[inline]
    pub fn push(&mut self, value: T) -> bool {
        if self.len >= self.data.len() {
            return false;
        }
        self.data[self.len] = value;
        self.len += 1;
        true
    }

    /// Returns the number of items in the vector.
    #[inline]
    pub fn len(&self) -> usize {
        self.len
    }

    /// Returns true if the vector is empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Returns the capacity of the vector.
    #[inline]
    pub fn capacity(&self) -> usize {
        self.data.len()
    }

    /// Returns a slice of the current items.
    #[inline]
    pub fn as_slice(&self) -> &[T] {
        &self.data[..self.len]
    }

    /// Returns a mutable slice of the current items.
    #[inline]
    pub fn as_mut_slice(&mut self) -> &mut [T] {
        &mut self.data[..self.len]
    }

    /// Clear the vector.
    #[inline]
    pub fn clear(&mut self) {
        self.len = 0;
    }
}

impl<'a, T> core::ops::Deref for ArenaVec<'a, T> {
    type Target = [T];

    fn deref(&self) -> &Self::Target {
        &self.data[..self.len]
    }
}

impl<'a, T> core::ops::DerefMut for ArenaVec<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.data[..self.len]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_arena_basic() {
        let mut buffer = [0u8; 1024];
        let arena = Arena::new(&mut buffer);

        let nums: &mut [usize] = arena.alloc_slice_default(10).unwrap();
        assert_eq!(nums.len(), 10);
        assert_eq!(nums[0], 0);

        nums[0] = 42;
        assert_eq!(nums[0], 42);

        assert!(arena.used() > 0);
        assert_eq!(arena.alloc_count(), 1);

        // SAFETY: `nums` is not used after this point.
        unsafe { arena.reset() };
        assert_eq!(arena.used(), 0);
        assert_eq!(arena.alloc_count(), 0);
    }

    #[test]
    fn test_arena_out_of_memory() {
        let mut buffer = [0u8; 64];
        let arena = Arena::new(&mut buffer);

        // This should fail - not enough space for 100 usizes
        let result: Option<&mut [usize]> = arena.alloc_slice_default(100);
        assert!(result.is_none());
    }

    #[test]
    fn test_arena_vec() {
        let mut buffer = [0u8; 1024];
        let arena = Arena::new(&mut buffer);

        let mut vec: ArenaVec<i32> = ArenaVec::new(&arena, 10).unwrap();
        assert!(vec.push(1));
        assert!(vec.push(2));
        assert!(vec.push(3));

        assert_eq!(vec.len(), 3);
        assert_eq!(vec[0], 1);
        assert_eq!(vec[1], 2);
        assert_eq!(vec[2], 3);
    }
}
