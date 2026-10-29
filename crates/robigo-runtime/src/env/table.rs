//! GNU R's hash tables of hashed environments, kept only to list their
//! names in GNU R's order (`ls(sorted = FALSE)`, `names()`, `as.list()`,
//! `serialize()`). In a module of its own so that `env.rs`, whose frame
//! code is on every closure call, compiles as it did.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use super::{Env, EnvData, FastHash};
use crate::sym::Sym;

type Tables = HashMap<usize, (std::rc::Weak<EnvData>, HashState), FastHash>;

thread_local! {
    /// The tables of hashed environments, by the address of their
    /// `EnvData`. The entry's weak reference keeps that address from being
    /// reused while the entry exists, and tells when the environment is
    /// gone; dead entries are swept when the map has doubled.
    static HASH_TABLES: RefCell<Tables> = RefCell::new(HashMap::default());
    static SWEEP_AT: Cell<usize> = const { Cell::new(64) };
}

const NIL: u32 = u32::MAX;

thread_local! {
    /// `R_Newhashpjw` of each symbol's name, by symbol index (`NO_HASH`
    /// until computed; the hash is below 2^28): a package's symbols are in
    /// its namespace, exports and package environments alike.
    static SYM_HASH: RefCell<Vec<u32>> = const { RefCell::new(Vec::new()) };
}

const NO_HASH: u32 = u32::MAX;

fn sym_hash(sym: Sym) -> u32 {
    SYM_HASH.with(|h| {
        let mut h = h.borrow_mut();
        let i = sym.0 as usize;
        if i >= h.len() {
            h.resize(i + 1, NO_HASH);
        }
        if h[i] == NO_HASH {
            h[i] = sym.with_name(hashpjw);
        }
        h[i]
    })
}

/// A hashed environment's table, kept lazily: `shape` is the table after
/// the bindings `entries[..at]`; the later ones were all added since, in
/// order (bindings are appended; removals bring the table up to date
/// first), so the current table is `shape` with them inserted. Making a
/// binding costs nothing more.
struct HashState {
    shape: HashShape,
    at: usize,
}

/// GNU R's hash table of an environment: its size, `HASHPRI` and the
/// chains, as linked lists over the nodes `syms` (`heads[b]` the first
/// node of chain `b`, `next[n]` the node after `n`, `NIL` at the end;
/// `hashes[n]` the hash code of `syms[n]`, so that growing the table only
/// takes remainders).
struct HashShape {
    size: u32,
    pri: u32,
    heads: Vec<u32>,
    next: Vec<u32>,
    syms: Vec<Sym>,
    hashes: Vec<u32>,
}

const HASHMINSIZE: u32 = 29;

/// The hash code of a symbol's name.
// R: envir.c:R_Newhashpjw (`*p` is a signed char)
fn hashpjw(name: &str) -> u32 {
    let mut h: u32 = 0;
    for &b in name.as_bytes() {
        h = (h << 4).wrapping_add(b as i8 as i32 as u32);
        let g = h & 0xf000_0000;
        if g != 0 {
            h ^= g >> 24;
            h ^= g;
        }
    }
    h
}

impl HashShape {
    // R: envir.c:R_NewHashTable
    fn new(size: i32) -> HashShape {
        let size = if size <= 0 { HASHMINSIZE } else { size as u32 };
        HashShape {
            size,
            pri: 0,
            heads: vec![NIL; size as usize],
            next: Vec::new(),
            syms: Vec::new(),
            hashes: Vec::new(),
        }
    }

    /// Put a new node at the head of `sym`'s chain; returns whether the
    /// chain was empty.
    fn link(&mut self, sym: Sym) -> bool {
        let h = sym_hash(sym);
        let b = (h % self.size) as usize;
        let node = self.syms.len() as u32;
        self.syms.push(sym);
        self.hashes.push(h);
        self.next.push(self.heads[b]);
        let was_empty = self.heads[b] == NIL;
        self.heads[b] = node;
        was_empty
    }

    /// A new binding, then the table grown as `defineVar` does.
    /// `R_HashSet` tests for an empty chain after its search loop has run
    /// the chain pointer to its end, so `HASHPRI` counts every new binding.
    // R: envir.c:R_HashSet, defineVar
    fn insert(&mut self, sym: Sym) {
        self.link(sym);
        self.pri += 1;
        if self.needs_resize() {
            self.resize();
        }
    }

    /// Prepend to a chain, counting it if it was empty (`R_HashFrame`).
    fn prepend(&mut self, sym: Sym) {
        if self.link(sym) {
            self.pri += 1;
        }
    }

    // R: envir.c:R_HashDelete
    fn delete(&mut self, sym: Sym) {
        let b = (sym_hash(sym) % self.size) as usize;
        let (mut prev, mut n) = (NIL, self.heads[b]);
        while n != NIL && self.syms[n as usize] != sym {
            prev = n;
            n = self.next[n as usize];
        }
        if n == NIL {
            return;
        }
        let after = self.next[n as usize];
        if prev == NIL {
            self.heads[b] = after;
        } else {
            self.next[prev as usize] = after;
        }
        if self.heads[b] == NIL {
            self.pri -= 1;
        }
    }

    // R: envir.c:R_HashSizeCheck
    fn needs_resize(&self) -> bool {
        self.pri as f64 > self.size as f64 * 0.85
    }

    /// Each old chain, head first, is moved node by node to the head of
    /// its new chain.
    // R: envir.c:R_HashResize
    fn resize(&mut self) {
        let size = 1 + (self.size as f64 * 1.2) as u32;
        let old = std::mem::replace(&mut self.heads, vec![NIL; size as usize]);
        self.size = size;
        self.pri = 0;
        for mut n in old {
            while n != NIL {
                let after = self.next[n as usize];
                let b = (self.hashes[n as usize] % size) as usize;
                if self.heads[b] == NIL {
                    self.pri += 1;
                }
                self.next[n as usize] = self.heads[b];
                self.heads[b] = n;
                n = after;
            }
        }
    }

    /// The chains, each head first.
    fn chains(&self) -> Vec<Vec<Sym>> {
        self.heads
            .iter()
            .map(|&h| {
                let mut c = Vec::new();
                let mut n = h;
                while n != NIL {
                    c.push(self.syms[n as usize]);
                    n = self.next[n as usize];
                }
                c
            })
            .collect()
    }

    /// The names in `HashTableNames` order: chain by chain, head first.
    fn names(&self) -> Vec<Sym> {
        let mut out = Vec::with_capacity(self.syms.len());
        for &h in &self.heads {
            let mut n = h;
            while n != NIL {
                out.push(self.syms[n as usize]);
                n = self.next[n as usize];
            }
        }
        out
    }

    /// A table holding `chains` (head first) as they are.
    fn from_chains(chains: Vec<Vec<Sym>>) -> HashShape {
        let mut h = HashShape::new(chains.len().max(1) as i32);
        h.size = chains.len() as u32;
        h.heads = vec![NIL; chains.len()];
        for (b, c) in chains.into_iter().enumerate() {
            let mut prev = NIL;
            for sym in c {
                let node = h.syms.len() as u32;
                h.syms.push(sym);
                h.hashes.push(sym_hash(sym));
                h.next.push(NIL);
                if prev == NIL {
                    h.heads[b] = node;
                    h.pri += 1;
                } else {
                    h.next[prev as usize] = node;
                }
                prev = node;
            }
        }
        h
    }
}

/// Drop the tables of environments that are gone, once the map has doubled
/// since the last sweep.
fn sweep_tables(t: &mut Tables) {
    if t.len() >= SWEEP_AT.with(|c| c.get()) {
        t.retain(|_, e| e.0.strong_count() > 0);
        SWEEP_AT.with(|c| c.set((2 * t.len()).max(64)));
    }
}

impl Env {
    fn table_key(&self) -> usize {
        Rc::as_ptr(&self.0) as usize
    }

    /// Call `f` with this hashed environment's table, brought up to date.
    fn with_table<R>(&self, f: impl FnOnce(&mut HashShape) -> R) -> R {
        let key = self.table_key();
        let frame = self.0.frame().borrow();
        HASH_TABLES.with(|t| {
            let mut t = t.borrow_mut();
            if t.get(&key).is_none_or(|e| e.0.strong_count() == 0) {
                sweep_tables(&mut t);
                let st = HashState { shape: HashShape::new(frame.nbase as i32), at: 0 };
                t.insert(key, (Rc::downgrade(&self.0), st));
            }
            let st = &mut t.get_mut(&key).unwrap().1;
            for b in &frame.entries[st.at.min(frame.entries.len())..] {
                st.shape.insert(b.sym);
            }
            st.at = frame.entries.len();
            f(&mut st.shape)
        })
    }

    fn set_table(&self, shape: HashShape) {
        let at = {
            let mut f = self.0.frame().borrow_mut();
            f.hashed = true;
            f.entries.len()
        };
        let key = self.table_key();
        HASH_TABLES.with(|t| {
            let mut t = t.borrow_mut();
            sweep_tables(&mut t);
            t.insert(key, (Rc::downgrade(&self.0), HashState { shape, at }));
        });
    }

    /// The names of a hashed environment, in its table's order.
    pub(super) fn table_names(&self) -> Vec<Sym> {
        self.with_table(|h| h.names())
    }

    /// Is this a hashed environment (`R_NewHashedEnv`)? Then its size.
    pub fn hash_size(&self) -> Option<usize> {
        let hashed = self.0.frame().borrow().hashed;
        hashed.then(|| self.with_table(|h| h.size as usize))
    }

    /// The chains of a hashed environment's table, head first (what
    /// `serialize()` writes as its `HASHTAB`).
    pub fn hash_chains(&self) -> Option<Vec<Vec<Sym>>> {
        let hashed = self.0.frame().borrow().hashed;
        hashed.then(|| self.with_table(|h| h.chains()))
    }

    /// A new hashed environment with a table of `size` (29 if `size <= 0`).
    // R: envir.c:R_NewHashedEnv
    pub fn new_hashed(parent: Option<Env>, size: i32) -> Env {
        let e = Env::new(parent);
        let mut f = e.0.frame().borrow_mut();
        f.hashed = true;
        f.nbase = size.max(0) as u32;
        drop(f);
        e
    }

    /// Give this environment a hash table of `size`, its current bindings
    /// entered in `order`, head first, each at the head of its chain, then
    /// grown while the load factor is above the threshold (`attach()`).
    /// An unhashed environment's own order is used for `None`.
    // R: envir.c:R_HashFrame, do_attach
    pub fn hash_frame(&self, size: i32, order: Option<Vec<Sym>>) {
        let order = order.unwrap_or_else(|| self.0.frame().borrow().frame_names());
        let mut h = HashShape::new(size);
        for sym in order {
            h.prepend(sym);
        }
        while h.needs_resize() {
            h.resize();
        }
        self.set_table(h);
    }

    /// Set a hashed environment's table as read by `unserialize()`:
    /// `chains` head first.
    pub fn set_hash_chains(&self, chains: Vec<Vec<Sym>>) {
        self.set_table(HashShape::from_chains(chains));
    }

    /// Forget `sym` in a hashed environment's table before its binding is
    /// removed (`R_HashDelete`); `table_removed` after.
    pub(super) fn table_delete(&self, sym: Sym) {
        self.with_table(|h| h.delete(sym));
    }

    pub(super) fn table_removed(&self) {
        let (key, at) = (self.table_key(), self.len());
        HASH_TABLES.with(|t| t.borrow_mut().get_mut(&key).map(|e| e.1.at = at));
    }
}
