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
//! use ascii_dag::arena::Arena;
//!
//! // Provide a buffer (stack or static)
//! let mut buffer = [0u8; 64 * 1024]; // 64 KB
//! let mut arena = Arena::new(&mut buffer);
//!
//! // Allocate slices from the arena
//! let nums: &mut [usize] = arena.alloc_slice_default(100).unwrap();
//! nums[0] = 42;
//!
//! // Reset for reuse (O(1))
//! arena.reset();
//! ```

use core::mem::{align_of, size_of};

/// A simple bump allocator backed by a user-provided buffer.
///
/// All allocations are contiguous and cannot be individually freed.
/// Call `reset()` to free all allocations at once (O(1) operation).
pub struct Arena<'a> {
    buffer: &'a mut [u8],
    offset: usize,
    alloc_count: usize,
}

impl<'a> Arena<'a> {
    /// Create a new arena backed by the provided buffer.
    ///
    /// The buffer can be stack-allocated, static, or heap-allocated.
    #[inline]
    pub fn new(buffer: &'a mut [u8]) -> Self {
        Self {
            buffer,
            offset: 0,
            alloc_count: 0,
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
    /// use ascii_dag::arena::Arena;
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
    fn alloc_raw_inner<T: Copy>(&mut self, count: usize, zero: bool) -> Option<(*mut T, usize)> {
        if count == 0 {
            return Some((core::ptr::null_mut(), 0));
        }

        let size = size_of::<T>() * count;
        let align = align_of::<T>();

        // Align the offset
        let aligned_offset = (self.offset + align - 1) & !(align - 1);

        if aligned_offset + size > self.buffer.len() {
            return None; // Out of memory
        }

        // Only zero if requested
        if zero {
            self.buffer[aligned_offset..aligned_offset + size].fill(0);
        }

        let ptr = self.buffer[aligned_offset..].as_mut_ptr() as *mut T;
        self.offset = aligned_offset + size;
        self.alloc_count += 1;

        Some((ptr, count))
    }

    /// Allocate a slice of `count` items, initialized to zero.
    ///
    /// Returns `None` if there isn't enough space in the arena.
    #[inline]
    pub fn alloc_slice_zeroed<T: Copy>(&mut self, count: usize) -> Option<&mut [T]> {
        self.alloc_slice_inner::<T>(count, true)
    }

    /// Allocate a slice of `count` items WITHOUT initialization.
    ///
    /// This is faster but memory contains garbage. Use when you will
    /// immediately overwrite all values (e.g., in a loop).
    ///
    /// # Safety
    ///
    /// Caller MUST initialize all elements before reading them.
    #[inline]
    pub fn alloc_slice_uninit<T: Copy>(&mut self, count: usize) -> Option<&mut [T]> {
        self.alloc_slice_inner::<T>(count, false)
    }

    #[inline]
    fn alloc_slice_inner<T: Copy>(&mut self, count: usize, zero: bool) -> Option<&mut [T]> {
        if count == 0 {
            return Some(&mut []);
        }

        let size = size_of::<T>() * count;
        let align = align_of::<T>();

        // Align the offset
        let aligned_offset = (self.offset + align - 1) & !(align - 1);

        if aligned_offset + size > self.buffer.len() {
            return None; // Out of memory
        }

        // Only zero if requested
        if zero {
            self.buffer[aligned_offset..aligned_offset + size].fill(0);
        }

        let ptr = self.buffer[aligned_offset..].as_mut_ptr() as *mut T;
        self.offset = aligned_offset + size;
        self.alloc_count += 1;

        // Safety: we've bounds-checked and aligned the memory
        Some(unsafe { core::slice::from_raw_parts_mut(ptr, count) })
    }

    /// Allocate a slice of `count` items, initialized to their default value.
    ///
    /// Returns `None` if there isn't enough space in the arena.
    #[inline]
    pub fn alloc_slice_default<T: Copy + Default>(&mut self, count: usize) -> Option<&mut [T]> {
        if count == 0 {
            return Some(&mut []);
        }

        let size = size_of::<T>() * count;
        let align = align_of::<T>();

        // Align the offset
        let aligned_offset = (self.offset + align - 1) & !(align - 1);

        if aligned_offset + size > self.buffer.len() {
            return None; // Out of memory
        }

        let ptr = self.buffer[aligned_offset..].as_mut_ptr() as *mut T;
        self.offset = aligned_offset + size;
        self.alloc_count += 1;

        // Safety: we've bounds-checked and aligned the memory
        let slice = unsafe { core::slice::from_raw_parts_mut(ptr, count) };

        // Initialize to default
        for item in slice.iter_mut() {
            *item = T::default();
        }

        Some(slice)
    }

    /// Allocate space for a single value, returning a mutable reference.
    #[inline]
    pub fn alloc<T: Copy + Default>(&mut self) -> Option<&mut T> {
        self.alloc_slice_default::<T>(1).map(|s| &mut s[0])
    }

    /// Reset the arena, freeing all allocations.
    ///
    /// This is an O(1) operation - it just resets the bump pointer.
    #[inline]
    pub fn reset(&mut self) {
        self.offset = 0;
        self.alloc_count = 0;
    }

    /// Returns the number of bytes currently used.
    #[inline]
    pub fn used(&self) -> usize {
        self.offset
    }

    /// Returns the total capacity of the arena.
    #[inline]
    pub fn capacity(&self) -> usize {
        self.buffer.len()
    }

    /// Returns the number of bytes remaining.
    #[inline]
    pub fn remaining(&self) -> usize {
        self.buffer.len() - self.offset
    }

    /// Returns the number of allocations made since the last reset.
    #[inline]
    pub fn alloc_count(&self) -> usize {
        self.alloc_count
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
    pub fn new(arena: &mut Arena<'a>, capacity: usize) -> Option<Self> {
        // We need to transmute the lifetime - the arena ensures the memory is valid
        let data = arena.alloc_slice_default::<T>(capacity)?;
        // Safety: we're extending the lifetime to match the arena's buffer lifetime
        let data = unsafe { core::slice::from_raw_parts_mut(data.as_mut_ptr(), capacity) };
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
        let mut arena = Arena::new(&mut buffer);

        let nums: &mut [usize] = arena.alloc_slice_default(10).unwrap();
        assert_eq!(nums.len(), 10);
        assert_eq!(nums[0], 0);

        nums[0] = 42;
        assert_eq!(nums[0], 42);

        assert!(arena.used() > 0);
        assert_eq!(arena.alloc_count(), 1);

        arena.reset();
        assert_eq!(arena.used(), 0);
        assert_eq!(arena.alloc_count(), 0);
    }

    #[test]
    fn test_arena_out_of_memory() {
        let mut buffer = [0u8; 64];
        let mut arena = Arena::new(&mut buffer);

        // This should fail - not enough space for 100 usizes
        let result: Option<&mut [usize]> = arena.alloc_slice_default(100);
        assert!(result.is_none());
    }

    #[test]
    fn test_arena_vec() {
        let mut buffer = [0u8; 1024];
        let mut arena = Arena::new(&mut buffer);

        let mut vec: ArenaVec<i32> = ArenaVec::new(&mut arena, 10).unwrap();
        assert!(vec.push(1));
        assert!(vec.push(2));
        assert!(vec.push(3));

        assert_eq!(vec.len(), 3);
        assert_eq!(vec[0], 1);
        assert_eq!(vec[1], 2);
        assert_eq!(vec[2], 3);
    }
}
