//! A bump arena for syntax trees.
//!
//! Trees reference the arena (and, for plain identifiers and strings,
//! the source text) instead of owning heap allocations: parsing a source
//! costs one bump pointer per node and no per-node constructor or
//! destructor, and dropping the arena frees everything at once. Values
//! allocated here must not own memory and must not implement `Drop`
//! (all AST types below satisfy that: references and `Copy` data only).
//!
//! Allocation takes `&self` (as bumpalo's does): the bump discipline
//! hands out disjoint memory, so several allocations can be alive at
//! once through one shared borrow — that is what lets a parser hold
//! `&Arena` and build nodes throughout a recursive descent.

use std::cell::{Cell, RefCell};

/// Chunks double in size, starting small so short sources do not pay for
/// long ones; the arena is single-use (create one per parse).
pub struct Arena {
    chunks: RefCell<Vec<Box<[u8]>>>,
    used: Cell<usize>,
}

const FIRST_CHUNK: usize = 2048;

impl Arena {
    pub fn new() -> Arena {
        Arena { chunks: RefCell::new(Vec::new()), used: Cell::new(0) }
    }

    /// The raw pointer to space for `size` bytes aligned to `align`.
    fn take(&self, size: usize, align: usize) -> *mut u8 {
        let mut chunks = self.chunks.borrow_mut();
        let base = chunks.last().map_or(std::ptr::null::<u8>(), |c| c.as_ptr());
        let start = (base as usize + self.used.get() + align - 1) & !(align - 1);
        let end = start + size;
        let chunk_end = base as usize + chunks.last().map_or(0, |c| c.len());
        if end > chunk_end || base.is_null() {
            let next = chunks.last().map_or(FIRST_CHUNK, |c| c.len() * 2).max(size.max(align));
            chunks.push(vec![0u8; next].into_boxed_slice());
            self.used.set(0);
            let b = chunks.last().unwrap().as_ptr();
            let s = (b as usize + align - 1) & !(align - 1);
            self.used.set(s - b as usize + size);
            return s as *mut u8;
        }
        self.used.set(end - base as usize);
        start as *mut u8
    }

    /// Place one value in the arena.
    pub fn alloc<T>(&self, value: T) -> &mut T {
        let ptr = self.take(std::mem::size_of::<T>(), std::mem::align_of::<T>()) as *mut T;
        unsafe {
            ptr.write(value);
            &mut *ptr
        }
    }

    /// Place a copy of a slice in the arena (`T` must be `Copy`: the
    /// source is dropped by the caller as usual, the copy is never
    /// dropped).
    pub fn alloc_slice<T: Copy>(&self, vals: &[T]) -> &[T] {
        if vals.is_empty() {
            return &[];
        }
        let size = std::mem::size_of::<T>() * vals.len();
        let ptr = self.take(size, std::mem::align_of::<T>()) as *mut T;
        unsafe {
            std::ptr::copy_nonoverlapping(vals.as_ptr(), ptr, vals.len());
            std::slice::from_raw_parts_mut(ptr, vals.len())
        }
    }

    /// Copy bytes into the arena.
    pub fn bytes(&self, b: &[u8]) -> &[u8] {
        if b.is_empty() {
            return &[];
        }
        let ptr = self.take(b.len(), 1);
        unsafe {
            std::ptr::copy_nonoverlapping(b.as_ptr(), ptr, b.len());
            std::slice::from_raw_parts_mut(ptr, b.len())
        }
    }

    /// Copy a string into the arena.
    pub fn str(&self, s: &str) -> &str {
        let b = self.bytes(s.as_bytes());
        // The bytes came from a &str and are unchanged.
        unsafe { std::str::from_utf8_unchecked(b) }
    }
}

impl Default for Arena {
    fn default() -> Self {
        Arena::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alloc_across_chunks() {
        let a = Arena::new();
        let small: &mut u64 = a.alloc(7u64);
        assert_eq!(*small, 7);
        // Larger than the first chunk: gets its own (bigger) chunk.
        let big = a.bytes(&[7u8; 10_000]);
        assert_eq!(big.len(), 10_000);
        assert_eq!(big[9999], 7);
        *small = 9;
        assert_eq!(*small, 9); // the earlier allocation is intact
        let s = a.str("hé\n");
        assert_eq!(s, "hé\n");
        let nums = a.alloc_slice(&[1u32, 2, 3]);
        assert_eq!(nums, &[1, 2, 3]);
        // Two live allocations through one shared borrow.
        let x = a.alloc(1i32);
        let y = a.alloc(2i32);
        assert_eq!((*x, *y), (1, 2));
    }
}
