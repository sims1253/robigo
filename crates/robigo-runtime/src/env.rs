//! Environments: mutable frames of bindings with a parent pointer.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};
use std::rc::Rc;

use crate::sym::{Sym, s};
use crate::value::{AttrBox, Value};

pub const BIND_LOCKED: u8 = 1;
pub const BIND_ACTIVE: u8 = 2;
/// The binding holds a default-argument promise: `missing(x)` is TRUE.
pub const BIND_MISSING_DEFAULT: u8 = 4;

#[derive(Clone)]
pub struct Binding {
    pub sym: Sym,
    pub value: Value,
    pub flags: u8,
}

/// Hasher for interned symbols: they are small integers already, so a
/// multiplicative mix is enough (SipHash showed up in variable lookup).
#[derive(Default)]
pub struct SymHasher(u64);

impl Hasher for SymHasher {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        for b in bytes {
            self.0 = (self.0.rotate_left(5) ^ *b as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        }
    }
    fn write_u32(&mut self, n: u32) {
        self.0 = (self.0.rotate_left(26) ^ n as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    }
    fn write_u64(&mut self, n: u64) {
        self.0 = (self.0.rotate_left(26) ^ n).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    }
    fn write_usize(&mut self, n: usize) {
        self.write_u64(n as u64);
    }
}

/// A fast hasher for small integer keys.
pub type FastHash = BuildHasherDefault<SymHasher>;

type SymMap<V> = HashMap<Sym, V, BuildHasherDefault<SymHasher>>;

/// Frames are small vectors; large ones (globals, base, namespaces) get a
/// hash index.
/// Bindings are stored inline up to this count, so that the frame of a
/// typical closure call lives in the same allocation as its environment.
const INLINE_BINDINGS: usize = 4;

#[derive(Default)]
pub struct Frame {
    pub entries: smallvec::SmallVec<[Binding; INLINE_BINDINGS]>,
    index: Option<SymMap<usize>>,
    /// A bit per symbol (`sym_bit`) that was ever bound here: most lookups
    /// of a symbol not in the frame stop at this test.
    mask: u64,
    /// The frame belongs to an environment on the search path.
    pub search: bool,
    // The order in which GNU R lists the bindings (`ls(sorted = FALSE)`,
    // `names()`, `as.list()`, `serialize()`); lookups do not depend on it.
    // These fields live in what was the struct's padding, and a hashed
    // frame's table in `HASH_TABLES`, so that closure frames stay as small
    // as they were.
    /// Without a table the frame is a GNU R frame pairlist
    /// (`NewEnvironment`): its first `nbase` bindings in the order they were
    /// given (a closure's matched arguments), each later one prepended by
    /// `defineVar`.
    nbase: u32,
    /// Listed in the order of insertion: the base environment, which GNU R
    /// lists in its global symbol table's order.
    insertion: bool,
    /// A hashed environment (`R_NewHashedEnv`): its table is in
    /// `HASH_TABLES` under the frame's address.
    hashed: bool,
}

thread_local! {
    /// The tables of hashed frames, by frame address (frames do not move:
    /// they live in their environment's allocation).
    static HASH_TABLES: RefCell<HashMap<usize, HashState, FastHash>> = RefCell::new(HashMap::default());
}

const NIL: u32 = u32::MAX;

fn sym_hash(sym: Sym) -> u32 {
    sym.with_name(hashpjw)
}

/// Forget a dropped hashed frame's table (out of line: frames are dropped
/// on every closure return).
#[cold]
#[inline(never)]
fn drop_table(key: usize) {
    HASH_TABLES.with(|t| t.borrow_mut().remove(&key));
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
/// node of chain `b`, `next[n]` the node after `n`, `NIL` at the end).
struct HashShape {
    size: u32,
    pri: u32,
    heads: Vec<u32>,
    next: Vec<u32>,
    syms: Vec<Sym>,
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
        HashShape { size, pri: 0, heads: vec![NIL; size as usize], next: Vec::new(), syms: Vec::new() }
    }

    /// Put a new node at the head of `sym`'s chain; returns whether the
    /// chain was empty.
    fn link(&mut self, sym: Sym) -> bool {
        let b = (sym_hash(sym) % self.size) as usize;
        let node = self.syms.len() as u32;
        self.syms.push(sym);
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
                let b = (sym_hash(self.syms[n as usize]) % size) as usize;
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
        self.chains().into_iter().flatten().collect()
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

#[inline(always)]
fn sym_bit(sym: Sym) -> u64 {
    1 << (sym.0 & 63)
}

thread_local! {
    /// Incremented whenever a binding that could hold a function changes
    /// in an environment on the search path (global, attached packages,
    /// base). Function lookups through the search path are cached against
    /// it.
    static SEARCH_EPOCH: Cell<u64> = const { Cell::new(0) };
}

thread_local! {
    /// Incremented whenever R code may run (an evaluation, a closure
    /// call, forcing a promise): compiled code's cached variable values
    /// stay valid while it is unchanged (see `vm::Vars`).
    static R_EPOCH: Cell<u64> = const { Cell::new(0) };
}

#[inline]
pub fn r_epoch() -> u64 {
    R_EPOCH.with(|e| e.get())
}

#[inline]
pub fn bump_r_epoch() {
    R_EPOCH.with(|e| e.set(e.get().wrapping_add(1)));
}

#[inline]
pub fn search_epoch() -> u64 {
    SEARCH_EPOCH.with(|e| e.get())
}

fn bump_search_epoch() {
    SEARCH_EPOCH.with(|e| e.set(e.get() + 1));
}

/// Could this value act as (or evaluate to) a function?
#[inline]
fn function_like(v: &Value) -> bool {
    matches!(v, Value::Closure(_) | Value::Builtin(_) | Value::Promise(_))
}

impl Drop for Frame {
    fn drop(&mut self) {
        if self.search && self.entries.iter().any(|b| function_like(&b.value)) {
            bump_search_epoch();
        }
        if self.hashed {
            drop_table(self.key());
        }
    }
}

const INDEX_THRESHOLD: usize = 12;

impl Frame {
    #[inline]
    pub fn position(&self, sym: Sym) -> Option<usize> {
        if self.mask & sym_bit(sym) == 0 {
            return None;
        }
        match &self.index {
            Some(ix) => ix.get(&sym).copied(),
            None => self.entries.iter().position(|b| b.sym == sym),
        }
    }

    // Inline across codegen units: the evaluator's variable lookups go
    // through here, and whether LLVM inlines it otherwise depends on how
    // unrelated code happens to be partitioned.
    #[inline]
    pub fn get(&self, sym: Sym) -> Option<&Binding> {
        self.position(sym).map(|i| &self.entries[i])
    }

    #[inline]
    pub fn get_mut(&mut self, sym: Sym) -> Option<&mut Binding> {
        self.position(sym).map(move |i| &mut self.entries[i])
    }

    pub fn insert(&mut self, sym: Sym, value: Value, flags: u8) {
        if let Some(i) = self.position(sym) {
            let b = &mut self.entries[i];
            b.value = value;
            b.flags = (b.flags & BIND_LOCKED) | flags;
            return;
        }
        self.push_new(sym, value, flags);
    }

    /// Append a binding without looking for an existing one. Should `sym`
    /// be bound already, lookups keep finding the first binding, as in a
    /// GNU R frame with repeated tags.
    fn push_new(&mut self, sym: Sym, value: Value, flags: u8) {
        if self.search && (function_like(&value) || flags & BIND_ACTIVE != 0) {
            bump_search_epoch();
        }
        self.entries.push(Binding { sym, value, flags });
        self.mask |= sym_bit(sym);
        let n = self.entries.len();
        match &mut self.index {
            Some(ix) => {
                ix.entry(sym).or_insert(n - 1);
            }
            None if n > INDEX_THRESHOLD => {
                self.index = Some(self.entries.iter().enumerate().rev().map(|(i, b)| (b.sym, i)).collect());
            }
            None => {}
        }
    }

    /// The symbols in GNU R's order: `HashTableNames` for a hashed frame,
    /// `FrameNames` (head first) for a frame pairlist.
    // R: envir.c:R_lsInternal3
    fn names_in_order(&self) -> Vec<Sym> {
        if self.hashed {
            return self.with_table(|h| h.names());
        }
        if self.insertion {
            return self.entries.iter().map(|b| b.sym).collect();
        }
        let (base, added) = self.entries.split_at((self.nbase as usize).min(self.entries.len()));
        added.iter().rev().chain(base).map(|b| b.sym).collect()
    }

    fn key(&self) -> usize {
        self as *const Frame as usize
    }

    /// Call `f` with this hashed frame's table, brought up to date.
    fn with_table<R>(&self, f: impl FnOnce(&mut HashShape) -> R) -> R {
        HASH_TABLES.with(|t| {
            let mut t = t.borrow_mut();
            let st = t.get_mut(&self.key()).expect("a hashed frame has a table");
            for b in &self.entries[st.at.min(self.entries.len())..] {
                st.shape.insert(b.sym);
            }
            st.at = self.entries.len();
            f(&mut st.shape)
        })
    }

    fn set_table(&mut self, shape: HashShape) {
        self.hashed = true;
        let (key, at) = (self.key(), self.entries.len());
        HASH_TABLES.with(|t| t.borrow_mut().insert(key, HashState { shape, at }));
    }

    pub fn remove(&mut self, sym: Sym) -> Option<Binding> {
        let i = self.position(sym)?;
        if self.hashed {
            self.with_table(|h| h.delete(sym));
        }
        let b = self.entries.remove(i);
        if self.search && (function_like(&b.value) || b.flags & BIND_ACTIVE != 0) {
            bump_search_epoch();
        }
        if self.hashed {
            let (key, at) = (self.key(), self.entries.len());
            HASH_TABLES.with(|t| t.borrow_mut().get_mut(&key).map(|st| st.at = at));
        } else if (i as u32) < self.nbase {
            self.nbase -= 1;
        }
        if let Some(ix) = &mut self.index {
            ix.remove(&sym);
            for (j, e) in self.entries.iter().enumerate().skip(i) {
                ix.insert(e.sym, j);
            }
        }
        Some(b)
    }
}

/// An environment's frame: inline, or shared between environments.
enum FrameCell {
    Own(RefCell<Frame>),
    /// The base namespace and the base package environment are distinct
    /// environments over the same bindings, as in GNU R.
    Shared(Rc<RefCell<Frame>>),
}

pub struct EnvData {
    frame: FrameCell,
    pub parent: RefCell<Option<Env>>,
    pub attrs: RefCell<AttrBox>,
    pub locked: Cell<bool>,
    /// Special name used when printing (`R_GlobalEnv`, `base`, `namespace:foo`).
    pub name: RefCell<Option<Rc<str>>>,
    pub id: usize,
    /// Incremented when a binding in this frame that could hold a function
    /// is created, changed or removed (see [`Env::fn_version`]).
    fn_version: Cell<u32>,
    /// `Frame::search` of this environment's frame, readable without
    /// borrowing it (set at creation and by `set_search`).
    search: Cell<bool>,
}

impl EnvData {
    #[inline]
    pub fn frame(&self) -> &RefCell<Frame> {
        match &self.frame {
            FrameCell::Own(f) => f,
            FrameCell::Shared(f) => f,
        }
    }
}

#[derive(Clone)]
pub struct Env(pub Rc<EnvData>);

thread_local! {
    static NEXT_ENV_ID: Cell<usize> = const { Cell::new(1) };
}

impl Env {
    pub fn new(parent: Option<Env>) -> Env {
        Env::with_capacity(parent, 0)
    }

    /// A new environment with room for `n` bindings.
    pub fn with_capacity(parent: Option<Env>, n: usize) -> Env {
        let frame = Frame {
            entries: smallvec::SmallVec::with_capacity(n),
            index: None,
            mask: 0,
            search: false,
            nbase: 0,
            insertion: false,
            hashed: false,
        };
        Env::with_frame(parent, FrameCell::Own(RefCell::new(frame)))
    }

    fn with_frame(parent: Option<Env>, frame: FrameCell) -> Env {
        let search = match &frame {
            FrameCell::Own(f) => f.borrow().search,
            FrameCell::Shared(f) => f.borrow().search,
        };
        let id = NEXT_ENV_ID.with(|c| {
            let v = c.get();
            c.set(v + 1);
            v
        });
        Env(Rc::new(EnvData {
            frame,
            parent: RefCell::new(parent),
            attrs: RefCell::new(None),
            locked: Cell::new(false),
            name: RefCell::new(None),
            id,
            fn_version: Cell::new(0),
            search: Cell::new(search),
        }))
    }

    /// A new environment sharing `other`'s bindings (the base namespace).
    /// `other` must be an environment on the search path.
    pub fn alias(other: &Env, parent: Option<Env>, name: &str) -> Env {
        let FrameCell::Shared(frame) = &other.0.frame else {
            panic!("Env::alias: the frame of {:?} is not shareable", other.special_name());
        };
        let e = Env::with_frame(parent, FrameCell::Shared(frame.clone()));
        *e.0.name.borrow_mut() = Some(Rc::from(name));
        e
    }

    /// An environment on the search path (global, base, attached).
    pub fn new_search(parent: Option<Env>, name: &str) -> Env {
        let frame = Frame {
            entries: Default::default(),
            index: None,
            mask: 0,
            search: true,
            nbase: 0,
            insertion: false,
            hashed: false,
        };
        let e = Env::with_frame(parent, FrameCell::Shared(Rc::new(RefCell::new(frame))));
        *e.0.name.borrow_mut() = Some(Rc::from(name));
        e
    }

    #[inline]
    pub fn is_search(&self) -> bool {
        self.0.search.get()
    }

    /// (Not used on environments sharing a frame, whose other environment
    /// would keep the old flag.)
    pub fn set_search(&self, on: bool) {
        self.0.frame().borrow_mut().search = on;
        self.0.search.set(on);
        bump_search_epoch();
    }

    /// The parent, borrowed (no reference count traffic).
    #[inline]
    pub fn parent_ref(&self) -> std::cell::Ref<'_, Option<Env>> {
        self.0.parent.borrow()
    }

    pub fn named(parent: Option<Env>, name: &str) -> Env {
        let e = Env::new(parent);
        *e.0.name.borrow_mut() = Some(Rc::from(name));
        e
    }

    pub fn ptr_eq(&self, other: &Env) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }

    pub fn parent(&self) -> Option<Env> {
        self.0.parent.borrow().clone()
    }

    pub fn set_parent(&self, p: Option<Env>) {
        if self.is_search() {
            bump_search_epoch();
        }
        // Function lookups through this frame may now resolve elsewhere.
        self.bump_fn_version();
        *self.0.parent.borrow_mut() = p;
    }

    pub fn special_name(&self) -> Option<Rc<str>> {
        self.0.name.borrow().clone()
    }

    /// `"package:foo"` for an attached package environment
    /// (R's `R_IsPackageEnv`/`R_PackageEnvName`).
    pub fn package_env_name(&self) -> Option<Rc<str>> {
        match Value::Env(self.clone()).get_attr(s::NAME) {
            Some(Value::Str(s)) => s.data.first()?.clone().filter(|n| n.starts_with("package:")),
            _ => None,
        }
    }

    /// The package name of a namespace environment: `base` for the base
    /// namespace, otherwise the first element of `.__NAMESPACE__.$spec`
    /// (R's `R_IsNamespaceEnv`/`R_NamespaceEnvSpec`).
    pub fn namespace_name(&self) -> Option<Rc<str>> {
        if let Some(n) = self.special_name() {
            if let Some(n) = n.strip_prefix("namespace:") {
                return Some(Rc::from(n));
            }
        }
        let Some(Value::Env(info)) = self.get_local(s::DOT_NAMESPACE) else { return None };
        match info.get_local(s::SPEC) {
            Some(Value::Str(s)) => s.data.first()?.clone(),
            _ => None,
        }
    }

    pub fn get_local(&self, sym: Sym) -> Option<Value> {
        self.0.frame().borrow().get(sym).map(|b| b.value.clone())
    }

    pub fn binding_flags(&self, sym: Sym) -> Option<u8> {
        self.0.frame().borrow().get(sym).map(|b| b.flags)
    }

    pub fn has_local(&self, sym: Sym) -> bool {
        self.0.frame().borrow().position(sym).is_some()
    }

    /// Define or overwrite a binding in this frame. Returns false if the
    /// binding (or the environment, for new bindings) is locked.
    pub fn set_local(&self, sym: Sym, value: Value) -> bool {
        self.set_local_flags(sym, value, 0)
    }

    /// Set a binding regardless of locks, keeping its flags, as GNU R's C
    /// code does with `SET_SYMVALUE` for variables it maintains in the
    /// (locked) base environment, such as `last.warning`.
    pub fn force_set(&self, sym: Sym, value: Value) {
        let flags = {
            let f = self.0.frame().borrow();
            f.get(sym).map(|b| b.flags & !BIND_LOCKED)
        };
        match flags {
            Some(flags) if flags & BIND_ACTIVE == 0 => {
                let locked = self.binding_flags(sym).unwrap_or(0) & BIND_LOCKED;
                self.set_binding_locked(sym, false);
                self.set_local_flags(sym, value, flags);
                if locked != 0 {
                    self.set_binding_locked(sym, true);
                }
            }
            Some(_) => {}
            None => {
                let was = self.0.locked.replace(false);
                self.set_local(sym, value);
                self.0.locked.set(was);
            }
        }
    }

    pub fn set_local_flags(&self, sym: Sym, value: Value, flags: u8) -> bool {
        let mut f = self.0.frame().borrow_mut();
        let search = f.search;
        if let Some(b) = f.get_mut(sym) {
            if b.flags & BIND_LOCKED != 0 {
                return false;
            }
            if function_like(&b.value) || function_like(&value) || b.flags & BIND_ACTIVE != 0 {
                if search {
                    bump_search_epoch();
                }
                self.bump_fn_version();
            }
            b.value = value;
            b.flags = flags;
            return true;
        }
        if self.0.locked.get() {
            return false;
        }
        if function_like(&value) || flags & BIND_ACTIVE != 0 {
            self.bump_fn_version();
        }
        f.push_new(sym, value, flags);
        true
    }

    /// Bind a formal argument in a new function frame: no lookup for an
    /// existing binding (R's NewEnvironment builds the frame from the
    /// matched arguments; with repeated formals the first one is found).
    pub fn bind_arg(&self, sym: Sym, value: Value, flags: u8) {
        if function_like(&value) {
            self.bump_fn_version();
        }
        let mut f = self.0.frame().borrow_mut();
        f.push_new(sym, value, flags);
        f.nbase += 1;
    }

    /// Bind a new symbol without looking for an existing binding, as
    /// `defineVar` into a fresh frame does (`R_execMethod`): it is listed
    /// before the bindings made so far.
    pub fn bind_new(&self, sym: Sym, value: Value, flags: u8) {
        if function_like(&value) {
            self.bump_fn_version();
        }
        self.0.frame().borrow_mut().push_new(sym, value, flags);
    }

    /// Move a value out of a binding, leaving `NULL` behind, so that the
    /// caller holds the only reference and can modify it in place.
    pub fn take_local(&self, sym: Sym) -> Option<Value> {
        let mut f = self.0.frame().borrow_mut();
        let search = f.search;
        let b = f.get_mut(sym)?;
        if b.flags & (BIND_ACTIVE | BIND_LOCKED) != 0 {
            return Some(b.value.clone());
        }
        if function_like(&b.value) {
            if search {
                bump_search_epoch();
            }
            self.bump_fn_version();
        }
        Some(std::mem::replace(&mut b.value, Value::Null))
    }

    pub fn remove(&self, sym: Sym) -> bool {
        let r = self.0.frame().borrow_mut().remove(sym);
        if r.as_ref().is_some_and(|b| function_like(&b.value) || b.flags & BIND_ACTIVE != 0) {
            self.bump_fn_version();
        }
        r.is_some()
    }

    /// Changes whenever a binding of this frame that could hold a function
    /// (a function, promise or active binding) is created, changed or
    /// removed through `Env`'s methods.
    #[inline]
    pub fn fn_version(&self) -> u32 {
        self.0.fn_version.get()
    }

    fn bump_fn_version(&self) {
        self.0.fn_version.set(self.0.fn_version.get().wrapping_add(1));
    }

    /// Could a function lookup of `sym` stop in this frame (a function,
    /// promise, active binding or missing argument bound to it)?
    #[inline]
    pub fn local_may_bind_fun(&self, sym: Sym) -> bool {
        match self.0.frame().borrow().get(sym) {
            Some(b) => function_like(&b.value) || b.flags & BIND_ACTIVE != 0 || b.value.is_missing_arg(),
            None => false,
        }
    }

    /// Find the innermost environment binding `sym`.
    pub fn lookup(&self, sym: Sym) -> Option<(Value, Env)> {
        let mut env = self.clone();
        loop {
            if let Some(v) = env.get_local(sym) {
                return Some((v, env));
            }
            let p = env.parent()?;
            env = p;
        }
    }

    pub fn len(&self) -> usize {
        self.0.frame().borrow().entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The bound symbols, in the order GNU R lists them.
    pub fn symbols(&self) -> Vec<Sym> {
        self.0.frame().borrow().names_in_order()
    }

    /// Is this a hashed environment (`R_NewHashedEnv`)? Then its size.
    pub fn hash_size(&self) -> Option<usize> {
        let f = self.0.frame().borrow();
        f.hashed.then(|| f.with_table(|h| h.size as usize))
    }

    /// The chains of a hashed environment's table, head first (what
    /// `serialize()` writes as its `HASHTAB`).
    pub fn hash_chains(&self) -> Option<Vec<Vec<Sym>>> {
        let f = self.0.frame().borrow();
        f.hashed.then(|| f.with_table(|h| h.chains()))
    }

    /// A new hashed environment with a table of `size` (29 if `size <= 0`).
    // R: envir.c:R_NewHashedEnv
    pub fn new_hashed(parent: Option<Env>, size: i32) -> Env {
        let e = Env::new(parent);
        e.0.frame().borrow_mut().set_table(HashShape::new(size));
        e
    }

    /// Give this environment a hash table of `size`, its current bindings
    /// entered in `order`, head first, each at the head of its chain, then
    /// grown while the load factor is above the threshold (`attach()`).
    /// An unhashed environment's own order is used for `None`.
    // R: envir.c:R_HashFrame, do_attach
    pub fn hash_frame(&self, size: i32, order: Option<Vec<Sym>>) {
        let mut f = self.0.frame().borrow_mut();
        let order = order.unwrap_or_else(|| f.names_in_order());
        let mut h = HashShape::new(size);
        for sym in order {
            h.prepend(sym);
        }
        while h.needs_resize() {
            h.resize();
        }
        f.set_table(h);
    }

    /// Set a hashed environment's table as read by `unserialize()`:
    /// `chains` head first.
    pub fn set_hash_chains(&self, chains: Vec<Vec<Sym>>) {
        self.0.frame().borrow_mut().set_table(HashShape::from_chains(chains));
    }

    /// List the bindings in the order they were made (the base environment).
    pub fn set_insertion_order(&self) {
        self.0.frame().borrow_mut().insertion = true;
    }

    pub fn lock(&self, bindings: bool) {
        self.0.locked.set(true);
        if bindings {
            for b in self.0.frame().borrow_mut().entries.iter_mut() {
                b.flags |= BIND_LOCKED;
            }
        }
    }

    pub fn set_binding_locked(&self, sym: Sym, locked: bool) -> bool {
        let mut f = self.0.frame().borrow_mut();
        match f.get_mut(sym) {
            Some(b) => {
                if locked {
                    b.flags |= BIND_LOCKED;
                } else {
                    b.flags &= !BIND_LOCKED;
                }
                true
            }
            None => false,
        }
    }

    pub fn id(&self) -> usize {
        self.0.id
    }
}
