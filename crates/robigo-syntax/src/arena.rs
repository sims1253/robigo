//! A bump arena for syntax trees.
//!
//! Trees reference the arena (and, for plain identifiers and strings,
//! the source text) instead of owning heap allocations: parsing a source
//! costs one bump pointer per node and no per-node constructor or
//! destructor, and dropping the arena frees everything at once. Values
//! allocated here must not own memory and must not implement `Drop`
//! (all AST types below satisfy that: references and `Copy` data only).
//!
//! Allocation takes `&self`: the bump discipline hands out disjoint memory,
//! so several allocations can be alive at once through one shared borrow,
//! which is what lets a parser hold `&Arena` and build nodes throughout a
//! recursive descent.

/// Chunks grow as needed from a small first one, so short sources do not
/// pay for long ones; the arena is single-use (create one per parse).
pub struct Arena(bumpalo::Bump);

const FIRST_CHUNK: usize = 2048;

impl Arena {
    pub fn new() -> Arena {
        Arena(bumpalo::Bump::with_capacity(FIRST_CHUNK))
    }

    /// Place one value in the arena.
    #[inline]
    pub fn alloc<T>(&self, value: T) -> &mut T {
        self.0.alloc(value)
    }

    /// Place a copy of a slice in the arena (`T` must be `Copy`: the
    /// source is dropped by the caller as usual, the copy is never
    /// dropped).
    #[inline]
    pub fn alloc_slice<T: Copy>(&self, vals: &[T]) -> &[T] {
        self.0.alloc_slice_copy(vals)
    }

    /// Copy bytes into the arena.
    #[inline]
    pub fn bytes(&self, b: &[u8]) -> &[u8] {
        self.0.alloc_slice_copy(b)
    }

    /// Copy a string into the arena.
    #[inline]
    pub fn str(&self, s: &str) -> &str {
        self.0.alloc_str(s)
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
