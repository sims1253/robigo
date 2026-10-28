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

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The entry with hash `h` for which `eq` holds.
    #[inline]
    pub fn find(&self, h: u32, mut eq: impl FnMut(Sexp) -> bool) -> Option<Sexp> {
        let mask = self.slots.len() - 1;
        let mut i = h as usize & mask;
        loop {
            let c = unsafe { *self.slots.get_unchecked(i) };
            if c.is_null() {
                return None;
            }
            if unsafe { c.char_hash() } == h && eq(c) {
                return Some(c);
            }
            i = (i + 1) & mask;
        }
    }

    /// Insert a CHARSXP not in the table (its header holds its hash).
    pub fn insert(&mut self, c: Sexp) {
        if (self.len + 1) * 4 > self.slots.len() * 3 {
            self.rebuild(self.slots.len() * 2, |_| true);
        }
        Self::place(&mut self.slots, c);
        self.len += 1;
    }

    fn place(slots: &mut [Sexp], c: Sexp) {
        let mask = slots.len() - 1;
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
                Self::place(&mut self.slots, c);
                self.len += 1;
            }
        }
    }

    /// Drop the entries `live` rejects (after marking, before any memory
    /// is reused); the table shrinks to fit the survivors.
    pub fn prune(&mut self, live: impl Fn(Sexp) -> bool) {
        let survivors = self.slots.iter().filter(|c| !c.is_null() && live(**c)).count();
        let cap = (survivors * 2).next_power_of_two().max(MIN_SLOTS);
        if survivors == self.len && cap == self.slots.len() {
            return;
        }
        self.rebuild(cap, live);
    }
}

impl Default for CharTable {
    fn default() -> Self {
        Self::new()
    }
}
