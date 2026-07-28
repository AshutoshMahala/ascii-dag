//! Plan/scratch storage — one growable-buffer shape over two memory
//! sources (temp/05 N2, temp/06 §8b).
//!
//! Every plan structure and painter scratch buffer is a [`PlanBuf`]:
//! heap-backed under `alloc`, or carved from a caller-provided [`Arena`]
//! on the no-alloc path. Capacities are computed exactly before any
//! carve, so the build code never needs reallocation — the two sources
//! share one fill path, keeping the engine's single-code-path rule.
//!
//! Carve failures map to the WDP `Render` component: the caller decides
//! the failure domain (`Plan` for plan structures, `Canvas` for band
//! compositing memory) at the carve site.

use crate::graph::arena::Arena;
use crate::GraphError;

/// A bounded push buffer over caller or heap memory.
pub(crate) enum PlanBuf<'a, T: Copy + Default> {
    /// Heap-backed (allocates on push, capacity pre-reserved).
    #[cfg(feature = "alloc")]
    Heap(alloc::vec::Vec<T>),
    /// Arena-carved: fixed capacity, explicit length.
    Slice { data: &'a mut [T], len: usize },
}

impl<'a, T: Copy + Default> PlanBuf<'a, T> {
    /// Heap-backed buffer with `capacity` reserved.
    #[cfg(feature = "alloc")]
    pub(crate) fn heap(capacity: usize) -> Self {
        PlanBuf::Heap(alloc::vec::Vec::with_capacity(capacity))
    }

    /// Carve `capacity` elements from `arena`; `on_oom` names the WDP
    /// failure domain of this buffer.
    pub(crate) fn carve(
        arena: &Arena<'a>,
        capacity: usize,
        on_oom: GraphError,
    ) -> Result<Self, GraphError> {
        match arena.alloc_slice_default::<T>(capacity) {
            Some(data) => Ok(PlanBuf::Slice { data, len: 0 }),
            None => Err(on_oom),
        }
    }

    /// A zero-filled fixed-size buffer (`len == capacity == n`), for
    /// index-addressed planes rather than push use.
    #[cfg(feature = "alloc")]
    pub(crate) fn heap_zeroed(n: usize) -> Self {
        PlanBuf::Heap(alloc::vec![T::default(); n])
    }

    /// Arena variant of [`PlanBuf::heap_zeroed`].
    pub(crate) fn carve_zeroed(
        arena: &Arena<'a>,
        n: usize,
        on_oom: GraphError,
    ) -> Result<Self, GraphError> {
        match arena.alloc_slice_default::<T>(n) {
            Some(data) => Ok(PlanBuf::Slice { data, len: n }),
            None => Err(on_oom),
        }
    }

    /// Append. Capacities are exact by construction; overflowing a
    /// carved buffer is an engine bug, not a user error.
    #[inline]
    pub(crate) fn push(&mut self, value: T) {
        match self {
            #[cfg(feature = "alloc")]
            PlanBuf::Heap(v) => v.push(value),
            PlanBuf::Slice { data, len } => {
                debug_assert!(*len < data.len(), "PlanBuf capacity underestimated");
                if *len < data.len() {
                    data[*len] = value;
                    *len += 1;
                }
            }
        }
    }

    #[inline]
    pub(crate) fn as_slice(&self) -> &[T] {
        match self {
            #[cfg(feature = "alloc")]
            PlanBuf::Heap(v) => v.as_slice(),
            PlanBuf::Slice { data, len } => &data[..*len],
        }
    }

    #[inline]
    pub(crate) fn as_mut_slice(&mut self) -> &mut [T] {
        match self {
            #[cfg(feature = "alloc")]
            PlanBuf::Heap(v) => v.as_mut_slice(),
            PlanBuf::Slice { data, len } => &mut data[..*len],
        }
    }

    #[inline]
    pub(crate) fn len(&self) -> usize {
        self.as_slice().len()
    }

    /// Drop all elements, keeping capacity (per-band scratch reuse).
    #[inline]
    pub(crate) fn clear(&mut self) {
        match self {
            #[cfg(feature = "alloc")]
            PlanBuf::Heap(v) => v.clear(),
            PlanBuf::Slice { len, .. } => *len = 0,
        }
    }

    /// Reset every element to default, keeping the length (plane reuse).
    #[inline]
    pub(crate) fn refill_default(&mut self) {
        for v in self.as_mut_slice() {
            *v = T::default();
        }
    }
}

/// A max-heap over a `PlanBuf` — the no-alloc replacement for
/// `BinaryHeap` in the run-flush sweep. Standard sift up/down over the
/// buffer's slice; `T`'s `Ord` decides priority.
pub(crate) struct SliceHeap<'h, 'a, T: Copy + Default + Ord> {
    buf: &'h mut PlanBuf<'a, T>,
}

impl<'h, 'a, T: Copy + Default + Ord> SliceHeap<'h, 'a, T> {
    /// Wrap `buf` as an empty heap (clears it).
    pub(crate) fn new(buf: &'h mut PlanBuf<'a, T>) -> Self {
        buf.clear();
        SliceHeap { buf }
    }

    #[inline]
    pub(crate) fn push(&mut self, value: T) {
        self.buf.push(value);
        let s = self.buf.as_mut_slice();
        let mut i = s.len() - 1;
        while i > 0 {
            let parent = (i - 1) / 2;
            if s[parent] < s[i] {
                s.swap(parent, i);
                i = parent;
            } else {
                break;
            }
        }
    }

    #[inline]
    pub(crate) fn peek(&self) -> Option<&T> {
        self.buf.as_slice().first()
    }

    pub(crate) fn pop(&mut self) -> Option<T> {
        let s = self.buf.as_mut_slice();
        if s.is_empty() {
            return None;
        }
        let n = s.len();
        s.swap(0, n - 1);
        let top = match self.buf {
            #[cfg(feature = "alloc")]
            PlanBuf::Heap(v) => v.pop(),
            PlanBuf::Slice { data, len } => {
                *len -= 1;
                Some(data[*len])
            }
        };
        let s = self.buf.as_mut_slice();
        let n = s.len();
        let mut i = 0;
        loop {
            let (l, r) = (2 * i + 1, 2 * i + 2);
            let mut largest = i;
            if l < n && s[largest] < s[l] {
                largest = l;
            }
            if r < n && s[largest] < s[r] {
                largest = r;
            }
            if largest == i {
                break;
            }
            s.swap(i, largest);
            i = largest;
        }
        top
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn planbuf_slice_push_and_clear() {
        let mut backing = [0u8; 256];
        let arena = Arena::new(&mut backing);
        let mut buf: PlanBuf<'_, u32> =
            PlanBuf::carve(&arena, 4, GraphError::RenderPlanOom).unwrap();
        buf.push(3);
        buf.push(1);
        assert_eq!(buf.as_slice(), &[3, 1]);
        buf.clear();
        assert_eq!(buf.len(), 0);
        // Exhaustion maps to the caller's WDP domain.
        let huge: Result<PlanBuf<'_, u64>, _> =
            PlanBuf::carve(&arena, 1 << 20, GraphError::RenderCanvasTooSmall { needed: 0, got: 0 });
        assert!(matches!(
            huge,
            Err(GraphError::RenderCanvasTooSmall { .. })
        ));
    }

    #[test]
    fn slice_heap_orders_max_first() {
        let mut backing = [0u8; 512];
        let arena = Arena::new(&mut backing);
        let mut buf: PlanBuf<'_, (u32, usize)> =
            PlanBuf::carve(&arena, 8, GraphError::RenderPlanOom).unwrap();
        let mut heap = SliceHeap::new(&mut buf);
        for v in [3u32, 7, 1, 9, 4] {
            heap.push((v, v as usize));
        }
        let mut drained = alloc::vec::Vec::new();
        while let Some((v, _)) = heap.pop() {
            drained.push(v);
        }
        assert_eq!(drained, alloc::vec![9, 7, 4, 3, 1]);
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn planbuf_heap_matches_slice_behavior() {
        let mut h: PlanBuf<'static, u32> = PlanBuf::heap(2);
        h.push(5);
        h.push(6);
        h.push(7); // heap variant may exceed initial capacity
        assert_eq!(h.as_slice(), &[5, 6, 7]);
        let mut z: PlanBuf<'static, u32> = PlanBuf::heap_zeroed(3);
        z.as_mut_slice()[1] = 9;
        z.refill_default();
        assert_eq!(z.as_slice(), &[0, 0, 0]);
    }
}
