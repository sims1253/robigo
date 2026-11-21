//! A table of symbols by name: the heap's own (`Core::install`'s cache of
//! every symbol it has taken) and the arena's (every symbol of the
//! process). Flat open addressing with linear probing, one word per slot,
//! as the CHARSXP cache (`chars.rs`). A slot holds the symbol alone: the
//! key is the symbol's arena name record, whose header holds the name's
//! hash ([`crate::hash_bytes`]) and whose bytes are the name. So an insert
//! copies no key and a probe compares the hash before the bytes.
//!
//! **Why reading a record is sound.** A symbol in a table is published (the
//! arena's `install` returned it), and its name record:
//! - is made only by the arena (`arena::Inner::new_name`), under the arena's
//!   lock, complete (hash, length, bytes, NUL) before the symbol that names
//!   it is published;
//! - lives in an arena segment, which is never freed, moved or reallocated;
//! - is never written again.
//!
//! A thread that probes a record another thread made reads it after taking
//! the arena's lock in the `install` that returned the symbol to it (or
//! after its own heap got it from such an install), so the record's writes
//! happen before its reads. A forked child has the parent's memory, taken
//! while the fork handlers held the arena's lock, so no record is half
//! written there.
//!
//! Symbols are immortal and are never removed: the table only grows.

use crate::Sexp;

/// The slots a symbol table starts with: room for 3,072 symbols before it
/// grows (at three quarters full).
/// - Measured: a base boot (`robigo-rt --vanilla -e 1`, `ROBIGO_RT_BOOT=rdb`,
///   #922's head 2ca695862) makes 1,970 symbols. That is the callgrind count
///   of the symbol-bits rule, which runs once per new symbol of the process.
///   2,048 slots (room for 1,536) would grow during the boot; 4,096 leave
///   room for a session's own names.
/// - Cost: 4,096 × 8 bytes = 32 KiB for each heap, from its start, so an
///   embedder pays it per isolate; and 32 KiB once for the arena's table.
///   The tables they replace held one boxed copy of each name per heap and
///   per process, and grew by doubling to the same size.
pub(crate) const SLOTS: usize = 4096;

pub(crate) struct SymTable {
    /// Open addressing; the length is a power of two, which `find`'s masked
    /// probe relies on.
    slots: Box<[Sexp]>,
    len: usize,
}

impl SymTable {
    /// A table with `slots` slots (a power of two): room for three
    /// quarters of that before it grows.
    pub(crate) fn with_slots(slots: usize) -> SymTable {
        assert!(slots.is_power_of_two(), "robigo: a symbol table's size is not a power of two");
        SymTable { slots: vec![Sexp::NULL; slots].into_boxed_slice(), len: 0 }
    }

    pub(crate) fn len(&self) -> usize {
        self.len
    }

    /// The symbol named `name`, whose [`crate::hash_bytes`] is `h`.
    #[inline]
    pub(crate) fn find(&self, h: u32, name: &[u8]) -> Option<Sexp> {
        let mask = self.slots.len() - 1;
        let mut i = h as usize & mask;
        loop {
            // SAFETY: `i` is masked by `len - 1`, and the length is a power
            // of two (`with_slots`, `grow`).
            let s = unsafe { *self.slots.get_unchecked(i) };
            if s.is_null() {
                return None;
            }
            // SAFETY: entries are published symbols: their name records are
            // complete, never written again and never freed (the module's
            // doc).
            if unsafe { s.sym_name_hash() == h && s.sym_name_bytes() == name } {
                return Some(s);
            }
            i = (i + 1) & mask;
        }
    }

    /// Add `s`, a symbol not in the table.
    ///
    /// # Safety
    /// `s` must be a symbol the arena published (its name record complete).
    #[inline]
    pub(crate) unsafe fn insert(&mut self, s: Sexp) {
        if (self.len + 1) * 4 > self.slots.len() * 3 {
            self.grow();
        }
        // SAFETY: this function's contract.
        unsafe { place(&mut self.slots, s) };
        self.len += 1;
    }

    #[cold]
    fn grow(&mut self) {
        let mut slots = vec![Sexp::NULL; self.slots.len() * 2].into_boxed_slice();
        for &s in self.slots.iter().filter(|s| !s.is_null()) {
            // SAFETY: an entry: a published symbol.
            unsafe { place(&mut slots, s) };
        }
        self.slots = slots;
    }

    /// Every symbol in the table.
    pub(crate) fn iter(&self) -> impl Iterator<Item = Sexp> + '_ {
        self.slots.iter().copied().filter(|s| !s.is_null())
    }
}

/// # Safety
/// `s` must be a published symbol, and `slots` must have a free slot.
unsafe fn place(slots: &mut [Sexp], s: Sexp) {
    let mask = slots.len() - 1;
    // SAFETY: the caller's contract.
    let mut i = unsafe { s.sym_name_hash() } as usize & mask;
    while !slots[i].is_null() {
        i = (i + 1) & mask;
    }
    slots[i] = s;
}
