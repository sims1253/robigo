//! The CHARSXP cache: a flat open-addressing table of `Sexp`s with linear
//! probing (council item 10). The 32-bit hash lives in each CHARSXP's
//! header (the attribute slot, which CHARSXPs do not use), so a probe
//! compares hashes before bytes and the table holds one word per entry.
//! Entries are weak: the collector prunes dead CHARSXPs by rebuilding
//! the table from the survivors (no tombstones).

use crate::Sexp;

pub struct CharTable {
    slots: Vec<Sexp>,
    len: usize,
}

const MIN_SLOTS: usize = 1024;

impl CharTable {
    pub fn new() -> CharTable {
        CharTable { slots: vec![Sexp::NULL; MIN_SLOTS], len: 0 }
    }

    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.len
    }

    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The entry with hash `h` for which `eq` holds.
    #[inline]
    pub fn find(&self, h: u32, mut eq: impl FnMut(Sexp) -> bool) -> Option<Sexp> {
        let mask = self.slots.len() - 1;
        let mut i = h as usize & mask;
        loop {
            let c = self.slots[i];
            if c.is_null() {
                return None;
            }
            // SAFETY: entries are live CHARSXPs: `insert`'s contract, and the
            // collector prunes dead ones before it frees them.
            if unsafe { c.char_hash() } == h && eq(c) {
                return Some(c);
            }
            i = (i + 1) & mask;
        }
    }

    /// Insert a CHARSXP not in the table (its header holds its hash).
    ///
    /// # Safety
    /// `c` must be a live CHARSXP of the heap this table belongs to. The
    /// table reads its header from now on, until the collector prunes it.
    pub unsafe fn insert(&mut self, c: Sexp) {
        if (self.len + 1) * 4 > self.slots.len() * 3 {
            self.rebuild(self.slots.len() * 2, |_| true);
        }
        // SAFETY: `place` asks for what this function's contract gives.
        unsafe { Self::place(&mut self.slots, c) };
        self.len += 1;
    }

    /// # Safety
    /// `c` must be a live CHARSXP: its header holds the hash.
    unsafe fn place(slots: &mut [Sexp], c: Sexp) {
        let mask = slots.len() - 1;
        // SAFETY: `c` is a live CHARSXP, this function's contract.
        let mut i = unsafe { c.char_hash() } as usize & mask;
        while !slots[i].is_null() {
            i = (i + 1) & mask;
        }
        slots[i] = c;
    }

    fn rebuild(&mut self, cap: usize, keep: impl Fn(Sexp) -> bool) {
        let old = std::mem::replace(&mut self.slots, vec![Sexp::NULL; cap]);
        self.len = 0;
        for c in old {
            if !c.is_null() && keep(c) {
                // SAFETY: an entry, so a live CHARSXP (see `find`): `keep` runs
                // after marking and before the sweep frees anything.
                unsafe { Self::place(&mut self.slots, c) };
                self.len += 1;
            }
        }
    }

    /// Whether `f` holds for every entry.
    pub fn all(&self, f: impl Fn(Sexp) -> bool) -> bool {
        self.slots.iter().all(|c| c.is_null() || f(*c))
    }

    /// Drop the entries `live` rejects (after marking, before any memory
    /// is reused); the table shrinks to fit the survivors.
    pub fn prune(&mut self, live: impl Fn(Sexp) -> bool) {
        let survivors = self.slots.iter().filter(|c| !c.is_null() && live(**c)).count();
        // the smallest table within insert's 3/4 load factor: a sparser
        // one costs a word per slot (10M strings: 128 MiB more, measured
        // by bench/scale.sh)
        let cap = (survivors * 4 / 3 + 1).next_power_of_two().max(MIN_SLOTS);
        self.rebuild(cap, live);
    }
}

impl Default for CharTable {
    fn default() -> Self {
        Self::new()
    }
}
