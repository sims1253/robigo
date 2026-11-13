//! Environments: mutable frames of bindings with a parent pointer.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};
use std::rc::Rc;

use crate::sym::{Sym, s};
use crate::value::{AttrBox, Value};

mod table;
mod trace;

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
    /// The bindings in the order they were made. In a frame with an
    /// `index`, a removed binding leaves a `VACANT` slot behind, so that
    /// removal does not move the bindings after it (and their positions in
    /// the index stay valid); `vacant` is set while there are any, the
    /// index maps `VACANT` to their number (`dead()`; a counter field would
    /// grow every environment), and the frame is compacted when they are
    /// half of it. Read through `bindings()`.
    entries: smallvec::SmallVec<[Binding; INLINE_BINDINGS]>,
    index: Option<SymMap<usize>>,
    /// A bit per symbol (`sym_bit`) that was ever bound here: most lookups
    /// of a symbol not in the frame stop at this test.
    mask: u64,
    /// The frame belongs to an environment on the search path.
    pub search: bool,
    // The order in which GNU R lists the bindings (`ls(sorted = FALSE)`,
    // `names()`, `as.list()`, `serialize()`); lookups do not depend on it.
    // These fields live in what was the struct's padding and a hashed
    // environment's table in `HASH_TABLES`, so that frames, their
    // construction and their drop glue are what they were (closure calls
    // are measured to the instruction).
    /// Without a table the frame is a GNU R frame pairlist
    /// (`NewEnvironment`): its first `nbase` bindings in the order they were
    /// given (a closure's matched arguments), each later one prepended by
    /// `defineVar`. `NBASE_OPEN` until the first binding that is not part
    /// of the initial frame (so `bind_arg` costs nothing more). For a
    /// hashed frame: the size its table starts with.
    pub(crate) nbase: u32,
    /// Listed in the order of insertion: the base environment, which GNU R
    /// lists in its global symbol table's order.
    insertion: bool,
    /// A hashed environment (`R_NewHashedEnv`): its table is made when
    /// first needed (environments that are never listed have none).
    pub(crate) hashed: bool,
    /// `entries` has `VACANT` slots (in the last byte of padding, so that
    /// `len()` stays a load and a subtraction).
    vacant: bool,
}

/// The symbol of a removed binding's slot: no interned symbol has it
/// (`sym.rs` interns fewer than 2^31 names).
const VACANT: Sym = Sym(u32::MAX);

#[inline(always)]
fn sym_bit(sym: Sym) -> u64 {
    1 << (sym.0 & 63)
}

/// A value that can lead to an environment through reference-count edges;
/// binding or unbinding one can create or break a runtime cycle.
#[inline]
fn cycle_value(v: &Value) -> bool {
    matches!(v, Value::Env(_) | Value::Closure(_) | Value::ExtPtr(_))
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

pub(crate) fn bump_search_epoch() {
    // Frame::drop calls this, and frames held by other thread-locals are
    // dropped during TLS destruction at process exit. A destroyed TLS must
    // not abort the process; no lookup can be cached then anyway.
    let _ = SEARCH_EPOCH.try_with(|e| e.set(e.get() + 1));
}

/// Could this value act as (or evaluate to) a function?
#[inline]
fn function_like(v: &Value) -> bool {
    matches!(v, Value::Closure(_) | Value::Builtin(_) | Value::Promise(_))
}

/// The `VACANT` slot count an index keeps under the `VACANT` key.
#[inline]
fn vacant_count(ix: &SymMap<usize>) -> usize {
    ix.get(&VACANT).copied().unwrap_or(0)
}

/// Could a function lookup stop at this binding (a function, an active
/// binding, or a missing argument, which errors)? Binding or unbinding one
/// invalidates the VM's function caches (`Env::fn_version`).
#[inline]
fn may_hold_fun(b: &Binding) -> bool {
    function_like(&b.value) || b.flags & BIND_ACTIVE != 0 || b.value.is_missing_arg()
}

const NBASE_OPEN: u32 = u32::MAX;

impl Drop for Frame {
    fn drop(&mut self) {
        // Only search-path frames (global, attached packages) need work, so
        // the drop glue of closure frames, on every call, stays small
        // enough to be inlined however the crate is partitioned.
        if self.search {
            search_frame_dropped(&self.entries);
        }
    }
}

#[cold]
#[inline(never)]
fn search_frame_dropped(entries: &[Binding]) {
    if entries.iter().any(|b| function_like(&b.value)) {
        bump_search_epoch();
    }
}

impl Frame {
    /// A binding that is not part of the initial frame is being added.
    #[inline]
    fn close_base(&mut self) {
        if self.nbase == NBASE_OPEN && !self.hashed {
            self.nbase = self.entries.len() as u32;
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
        if cycle_value(&value) {
            trace::cycle_changed();
        }
        if let Some(i) = self.position(sym) {
            let b = &mut self.entries[i];
            if cycle_value(&b.value) {
                trace::cycle_changed();
            }
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
        if cycle_value(&value) {
            trace::cycle_changed();
        }
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
    pub(crate) fn frame_names(&self) -> Vec<Sym> {
        if self.insertion {
            return self.bindings().map(|b| b.sym).collect();
        }
        let (base, added) = self.entries.split_at((self.nbase as usize).min(self.entries.len()));
        added.iter().rev().chain(base).map(|b| b.sym).filter(|&s| s != VACANT).collect()
    }

    /// Remove the first binding of `sym` (with repeated tags, lookups then
    /// find the next one, as after `RemoveFromList`). O(1) amortized in a
    /// frame with an index: the slot is left `VACANT` and the bindings
    /// after it stay where they are.
    ///
    /// Returns the binding and whether a function lookup of `sym` here can
    /// now resolve differently: the removed binding, or the repeated one
    /// that takes its place, could hold a function (`may_hold_fun`). The
    /// caller bumps the environment's `fn_version` then; the search epoch
    /// is bumped here.
    pub fn remove(&mut self, sym: Sym) -> Option<(Binding, bool)> {
        let i = self.position(sym)?;
        trace::cycle_changed();
        let (b, exposed) = match &mut self.index {
            None => {
                let b = self.entries.remove(i);
                if !self.hashed && (i as u32) < self.nbase && self.nbase != NBASE_OPEN {
                    self.nbase -= 1;
                }
                // at most INDEX_THRESHOLD bindings to look at
                let exposed = self.entries[i..].iter().find(|e| e.sym == sym).is_some_and(may_hold_fun);
                (b, exposed)
            }
            Some(ix) => {
                let dead = vacant_count(ix);
                // More bindings than indexed symbols: some tag is repeated,
                // and a later binding of `sym` may take this one's place.
                let repeated = self.entries.len() - dead > ix.len() - usize::from(dead > 0);
                let next = if repeated { self.entries[i + 1..].iter().position(|e| e.sym == sym) } else { None };
                let exposed = match next {
                    Some(j) => {
                        ix.insert(sym, i + 1 + j);
                        may_hold_fun(&self.entries[i + 1 + j])
                    }
                    None => {
                        ix.remove(&sym);
                        false
                    }
                };
                let b = std::mem::replace(&mut self.entries[i], Binding { sym: VACANT, value: Value::Null, flags: 0 });
                ix.insert(VACANT, dead + 1);
                self.vacant = true;
                if 2 * (dead + 1) > self.entries.len() {
                    self.compact();
                }
                (b, exposed)
            }
        };
        let fun = exposed || may_hold_fun(&b);
        if self.search && fun {
            bump_search_epoch();
        }
        Some((b, fun))
    }

    /// Drop the `VACANT` slots of a frame with an index, keeping the order
    /// of the bindings: O(n) once per n/2 removals.
    #[cold]
    #[inline(never)]
    fn compact(&mut self) {
        if !self.hashed && self.nbase != NBASE_OPEN {
            let base = (self.nbase as usize).min(self.entries.len());
            self.nbase -= self.entries[..base].iter().filter(|b| b.sym == VACANT).count() as u32;
        }
        self.entries.retain(|b| b.sym != VACANT);
        self.vacant = false;
        if let Some(ix) = &mut self.index {
            // (`VACANT`'s count with the rest)
            ix.clear();
            // the first binding of a repeated tag is the one found
            for (j, b) in self.entries.iter().enumerate().rev() {
                ix.insert(b.sym, j);
            }
        }
    }

    /// The bindings, in the order they were made.
    #[inline]
    pub fn bindings(&self) -> impl Iterator<Item = &Binding> {
        self.entries.iter().filter(|b| b.sym != VACANT)
    }

    #[inline]
    pub fn bindings_mut(&mut self) -> impl Iterator<Item = &mut Binding> {
        self.entries.iter_mut().filter(|b| b.sym != VACANT)
    }

    /// The number of `VACANT` slots in `entries` (only a frame with an
    /// index has any).
    #[inline]
    fn dead(&self) -> usize {
        if self.vacant { self.dead_slots() } else { 0 }
    }

    #[cold]
    #[inline(never)]
    fn dead_slots(&self) -> usize {
        self.index.as_ref().map_or(0, vacant_count)
    }

    /// The number of bindings.
    #[inline]
    pub fn len(&self) -> usize {
        self.entries.len() - self.dead()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Forget every binding: the sweep of a frame no live environment
    /// reaches (memory.c's sweep frees the unreachable cells).
    pub(crate) fn clear(&mut self) {
        self.entries.clear();
        self.index = None;
        self.vacant = false;
        self.mask = 0;
        self.nbase = NBASE_OPEN;
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
    pub name: RefCell<Option<crate::value::RString>>,
    pub id: usize,
    /// Incremented when a binding in this frame that could hold a function
    /// is created, changed or removed (see [`Env::fn_version`]).
    fn_version: Cell<u32>,
    /// `Frame::search` of this environment's frame, readable without
    /// borrowing it (set at creation and by `set_search`).
    search: Cell<bool>,
    /// Already present in the weak environment inventory. Fits in padding.
    trace_recorded: Cell<bool>,
    /// R's RDEBUG bit of the environment: a function frame being browsed
    /// (`debug()`, or `n` at a `browser()` prompt) steps statement by
    /// statement. Fits in padding.
    debug: Cell<bool>,
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
    /// R's `RDEBUG(rho)` (see [`EnvData`]).
    #[inline(always)]
    pub fn rdebug(&self) -> bool {
        self.0.debug.get()
    }

    /// R's `SET_RDEBUG(rho, on)`.
    pub fn set_rdebug(&self, on: bool) {
        self.0.debug.set(on);
    }

    pub fn new(parent: Option<Env>) -> Env {
        let env = Env::with_capacity(parent, 0);
        env.track_escape();
        env
    }

    pub(crate) fn track_escape(&self) {
        trace::track(self);
    }

    /// Free a finished call frame whose only references beyond its
    /// `owners` come from its own bindings: an unforced promise (a
    /// default argument's) or a closure made in the frame that nothing
    /// but its binding holds, or the environment bound in itself. Nothing
    /// outside reaches such a frame once its owners let go, so its
    /// bindings are cleared (as `reachability` would, at the next
    /// collection), breaking the cycle. Returns whether it did. Any
    /// reference it cannot attribute keeps the frame as it is.
    pub(crate) fn release_self_cycle(&self, owners: usize) -> bool {
        let extra = Rc::strong_count(&self.0) - owners;
        let entries = {
            let Ok(mut frame) = self.0.frame().try_borrow_mut() else { return false };
            // (`VACANT` slots, holding `NULL`, count toward neither)
            if frame.search || frame.entries.len() < extra {
                return false;
            }
            let mut inner = 0;
            for b in frame.bindings() {
                inner += match &b.value {
                    Value::Promise(p) => usize::from(
                        Rc::strong_count(p) == 1
                            && !p.forcing.get()
                            && p.env.borrow().as_ref().is_some_and(|e| e.ptr_eq(self)),
                    ),
                    Value::Closure(c) => usize::from(Rc::strong_count(c) == 1 && c.env.ptr_eq(self)),
                    Value::Env(e) => usize::from(e.ptr_eq(self)),
                    _ => 0,
                };
            }
            if inner != extra {
                return false;
            }
            let entries = std::mem::take(&mut frame.entries);
            frame.clear();
            entries
        };
        trace::cycle_changed();
        // Dropped with the frame released: they only lead back here.
        drop(entries);
        true
    }

    /// The collector's view of "a cycle may have become unreachable".
    pub fn cycle_epoch() -> u64 {
        trace::cycle_epoch()
    }

    /// Mark that an environment or a path to one changed outside the
    /// binding paths (`gc()` dropping caches, a finalizer clearing a key).
    pub fn mark_cycle_dirty() {
        trace::cycle_changed();
    }

    pub(crate) fn visit_escaped(visitor: impl FnMut(&Env)) {
        trace::visit(visitor);
    }

    /// A new environment with room for `n` bindings.
    pub fn with_capacity(parent: Option<Env>, n: usize) -> Env {
        let frame = Frame {
            entries: smallvec::SmallVec::with_capacity(n),
            index: None,
            mask: 0,
            search: false,
            nbase: NBASE_OPEN,
            insertion: false,
            hashed: false,
            vacant: false,
        };
        Env::with_frame(parent, FrameCell::Own(RefCell::new(frame)))
    }

    fn with_frame(parent: Option<Env>, frame: FrameCell) -> Env {
        let search = match &frame {
            FrameCell::Own(f) => f.borrow().search,
            FrameCell::Shared(f) => f.borrow().search,
        };
        crate::counters::count(crate::counters::Counter::Envs);
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
            trace_recorded: Cell::new(false),
            debug: Cell::new(false),
        }))
    }

    /// Shared frames are separate owners in the GC edge graph.
    pub(crate) fn shared_frame(&self) -> Option<&Rc<RefCell<Frame>>> {
        match &self.0.frame {
            FrameCell::Shared(frame) => Some(frame),
            FrameCell::Own(_) => None,
        }
    }

    /// A new environment sharing `other`'s bindings (the base namespace).
    /// `other` must be an environment on the search path.
    pub fn alias(other: &Env, parent: Option<Env>, name: &str) -> Env {
        let FrameCell::Shared(frame) = &other.0.frame else {
            panic!("Env::alias: the frame of {:?} is not shareable", other.special_name());
        };
        let e = Env::with_frame(parent, FrameCell::Shared(frame.clone()));
        e.track_escape();
        *e.0.name.borrow_mut() = Some(crate::value::RString::from(name));
        e
    }

    /// An environment on the search path (global, base, attached).
    pub fn new_search(parent: Option<Env>, name: &str) -> Env {
        let frame = Frame {
            entries: Default::default(),
            index: None,
            mask: 0,
            search: true,
            nbase: NBASE_OPEN,
            insertion: false,
            hashed: false,
            vacant: false,
        };
        let e = Env::with_frame(parent, FrameCell::Shared(Rc::new(RefCell::new(frame))));
        e.track_escape();
        *e.0.name.borrow_mut() = Some(crate::value::RString::from(name));
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
        *e.0.name.borrow_mut() = Some(crate::value::RString::from(name));
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
        trace::cycle_changed();
        *self.0.parent.borrow_mut() = p;
    }

    /// Whether making `p` the parent of `self` would close a cycle in the
    /// parent chain (R's `SET_ENCLOS`, src/main/memory.c:4667).
    pub fn parent_cycle(&self, p: Option<&Env>) -> bool {
        let mut e = p.cloned();
        while let Some(cur) = e {
            if cur.ptr_eq(self) {
                return true;
            }
            e = cur.parent();
        }
        false
    }

    pub fn special_name(&self) -> Option<crate::value::RString> {
        self.0.name.borrow().clone()
    }

    /// `"package:foo"` for an attached package environment
    /// (R's `R_IsPackageEnv`/`R_PackageEnvName`).
    pub fn package_env_name(&self) -> Option<crate::value::RString> {
        match Value::Env(self.clone()).get_attr(s::NAME) {
            Some(Value::Str(s)) => s.data.first()?.clone().filter(|n| n.starts_with("package:")),
            _ => None,
        }
    }

    /// A package imports environment (R's `R_IsImportsEnv`, src/main/builtin.c):
    /// its parent is the base namespace and its `name` attribute is a single
    /// string starting with `"imports:"`.
    pub fn is_imports_env(&self, base_ns: &Env) -> bool {
        if !self.parent().is_some_and(|p| p.ptr_eq(base_ns)) {
            return false;
        }
        match Value::Env(self.clone()).get_attr(s::NAME) {
            Some(Value::Str(n)) if n.data.len() == 1 => n.data[0].as_deref().is_some_and(|n| n.starts_with("imports:")),
            _ => false,
        }
    }

    /// The package name of a namespace environment: `base` for the base
    /// namespace, otherwise the first element of `.__NAMESPACE__.$spec`
    /// (R's `R_IsNamespaceEnv`/`R_NamespaceEnvSpec`).
    pub fn namespace_name(&self) -> Option<crate::value::RString> {
        if let Some(n) = self.special_name() {
            if let Some(n) = n.strip_prefix("namespace:") {
                return Some(crate::value::RString::from(n));
            }
        }
        let Some(Value::Env(info)) = self.get_local(s::DOT_NAMESPACE) else { return None };
        match info.get_local(s::SPEC) {
            Some(Value::Str(s)) => s.data.first()?.clone(),
            _ => None,
        }
    }

    #[inline(always)]
    pub fn get_local(&self, sym: Sym) -> Option<Value> {
        self.binding_value(sym).map(|(v, _)| v)
    }

    /// The value bound to `sym` in this frame and the binding's flags, as R
    /// reads a variable's object. A shared small integer bound here (the
    /// VM binds unboxed scalars as they are) is first boxed into an object
    /// of its own and stored back into the binding, so this reader and
    /// every later one (`get`, `as.list(env)`, C's `findVar`, a promise of
    /// the variable) see one object, which native code may write into. The
    /// VM's unboxed reads (`vm::plain_lookup`) do not expand.
    // R: envir.c:BINDING_VALUE -> memory.c:R_expand_binding_value
    #[inline(always)]
    pub fn binding_value(&self, sym: Sym) -> Option<(Value, u8)> {
        let f = self.0.frame().borrow();
        let b = f.get(sym)?;
        if let Some(i) = b.value.shared_int() {
            let flags = b.flags;
            drop(f);
            return Some((self.expand_binding(sym, i), flags));
        }
        Some((b.value.clone(), b.flags))
    }

    /// [`Env::binding_value`]'s boxing: the binding's shared small integer
    /// `i` replaced by an object of its own (SET_BNDCELL).
    #[cold]
    #[inline(never)]
    fn expand_binding(&self, sym: Sym, i: i32) -> Value {
        let v = Value::int_fresh(i);
        // (A reader that holds a borrow of this frame meanwhile gets the
        // box without storing it: reading never panics.)
        if let Ok(mut f) = self.0.frame().try_borrow_mut()
            && let Some(b) = f.get_mut(sym)
            && b.value.shared_int() == Some(i)
        {
            b.value = v.clone();
        }
        v
    }

    pub fn binding_flags(&self, sym: Sym) -> Option<u8> {
        self.0.frame().borrow().get(sym).map(|b| b.flags)
    }

    /// Whether `sym` is bound here by an active binding as R sees it
    /// (`bindingIsActive`, `R_BindingIsActive`); `None` if unbound. A
    /// C-owned variable (`native::c_owned_binding`) is robigo's way of
    /// keeping C's object for a plain value binding, and GNU R has a plain
    /// value there.
    pub fn binding_is_active(&self, sym: Sym) -> Option<bool> {
        let frame = self.0.frame().borrow();
        let b = frame.get(sym)?;
        Some(b.flags & BIND_ACTIVE != 0 && crate::native::c_owned_holder(&b.value).is_none())
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
                // The temporary unlock bypassed bump_fn_version's locked
                // frame invalidation. Search frames invalidate in push_new.
                if was && !self.is_search() {
                    bump_search_epoch();
                }
            }
        }
    }

    pub fn set_local_flags(&self, sym: Sym, value: Value, flags: u8) -> bool {
        if cycle_value(&value) {
            trace::cycle_changed();
        }
        let mut f = self.0.frame().borrow_mut();
        let search = f.search;
        if let Some(b) = f.get_mut(sym) {
            if b.flags & BIND_LOCKED != 0 {
                return false;
            }
            if cycle_value(&b.value) {
                trace::cycle_changed();
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
        f.close_base();
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
        self.0.frame().borrow_mut().push_new(sym, value, flags);
    }

    /// Bind a new symbol without looking for an existing binding, as
    /// `defineVar` into a fresh frame does (`R_execMethod`): it is listed
    /// before the bindings made so far.
    pub fn bind_new(&self, sym: Sym, value: Value, flags: u8) {
        if function_like(&value) {
            self.bump_fn_version();
        }
        let mut f = self.0.frame().borrow_mut();
        f.close_base();
        f.push_new(sym, value, flags);
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
        if cycle_value(&b.value) {
            trace::cycle_changed();
        }
        Some(std::mem::replace(&mut b.value, Value::Null))
    }

    pub fn remove(&self, sym: Sym) -> bool {
        let hashed = self.0.frame().borrow().hashed;
        if hashed && self.has_local(sym) {
            self.table_delete(sym);
        }
        let r = self.0.frame().borrow_mut().remove(sym);
        if hashed && r.is_some() {
            // the table holds every binding left
            self.table_removed();
        }
        if r.as_ref().is_some_and(|&(_, fun)| fun) {
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
        // Guard memos may look through locked frames without recording
        // every frame's version. Locking forbids new bindings, but existing
        // function bindings can still be replaced: invalidate those memos.
        if self.0.locked.get() {
            bump_search_epoch();
        }
    }

    /// Whether `sym` is bound here to a plain value that assigning can
    /// replace without running code or failing.
    pub(crate) fn plain_binding(&self, sym: Sym) -> bool {
        match self.0.frame().borrow().get(sym) {
            Some(b) => b.flags == 0 && !function_like(&b.value),
            None => false,
        }
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

    // `Value::len` inlines this into its `Env` arm: the frame borrow is
    // released before the call for `VACANT` slots, so that the call is a
    // tail call and `Value::len` stays a leaf function (no prologue on
    // every `length()`).
    #[inline]
    pub fn len(&self) -> usize {
        let frame = self.0.frame().borrow();
        if frame.vacant {
            drop(frame);
            return self.len_with_vacant();
        }
        frame.entries.len()
    }

    #[cold]
    #[inline(never)]
    fn len_with_vacant(&self) -> usize {
        self.0.frame().borrow().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The bound symbols, in the order GNU R lists them.
    pub fn symbols(&self) -> Vec<Sym> {
        if self.0.frame().borrow().hashed {
            return self.table_names();
        }
        self.0.frame().borrow().frame_names()
    }

    /// The bound symbols in no particular order (for callers that sort).
    pub fn symbols_unordered(&self) -> Vec<Sym> {
        self.0.frame().borrow().bindings().map(|b| b.sym).collect()
    }

    /// List the bindings in the order they were made (the base environment).
    pub fn set_insertion_order(&self) {
        self.0.frame().borrow_mut().insertion = true;
    }

    pub fn lock(&self, bindings: bool) {
        self.0.locked.set(true);
        if bindings {
            for b in self.0.frame().borrow_mut().bindings_mut() {
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

#[cfg(test)]
mod review_regression_tests {
    use super::*;
    use crate::Interp;
    use crate::compile::GuardMemo;

    #[test]
    fn forced_binding_in_locked_ancestor_invalidates_guard() {
        let mut interp = Interp::with_output(Box::new(std::io::sink()), Box::new(std::io::sink()));
        let outer = Env::new(Some(interp.global.clone()));
        outer.lock(false);
        let inner = Env::new(Some(outer.clone()));
        inner.lock(false);
        let frame = Env::new(Some(inner));
        let plus = Sym::new("+");
        let memo = Cell::new(GuardMemo::NONE);
        assert!(crate::vm::guard_ok(&mut interp, plus, &frame, &memo));
        let epoch = search_epoch();
        let minus = crate::builtins::lookup_primitive("-").expect("base subtraction");
        outer.force_set(plus, Value::Builtin(minus));
        assert!(outer.0.locked.get());
        assert_ne!(search_epoch(), epoch);
        assert!(!crate::vm::guard_ok(&mut interp, plus, &frame, &memo));
    }
}

#[cfg(test)]
mod escape_tests {
    use super::*;
    use crate::Interp;

    fn tracked() -> usize {
        let mut n = 0;
        trace::visit(|_| n += 1);
        n
    }

    /// A frame left by `return()` is freed when its call ends, as one whose
    /// body falls off its end is, even when its own bindings hold it (here
    /// a closure made in it): the jump's reference to the frame is gone by
    /// then, so nothing outside seems to reach it and it is not tracked as
    /// escaped, to wait for a cycle collection.
    #[test]
    fn return_leaves_no_escaped_frame() {
        let mut interp = Interp::with_output(Box::new(std::io::sink()), Box::new(std::io::sink()));
        let env = interp.global.clone();
        let run = |interp: &mut Interp, src: &str| {
            for e in crate::lang::parse_to_values(src).unwrap() {
                assert!(interp.eval(&e, &env).is_ok(), "{src}");
            }
        };
        // (`return()` in a promise jumps as a `Flow::Return` in every tier)
        run(&mut interp, "id <- function(y) y");
        run(&mut interp, "f <- function(x) { h <- function() x; id(return(x)) }");
        run(&mut interp, "for (i in 1:3) f(i)");
        let before = tracked();
        run(&mut interp, "for (i in 1:50) f(i)");
        assert_eq!(tracked(), before);
    }
}

#[cfg(test)]
mod remove_tests {
    use super::*;

    fn syms(n: usize) -> Vec<Sym> {
        (0..n).map(|i| Sym::new(&format!("remove_test_{i}"))).collect()
    }

    #[test]
    fn removal_keeps_order_and_compacts() {
        let s = syms(100);
        let mut f = Frame::default();
        f.nbase = NBASE_OPEN;
        for (i, &sym) in s.iter().enumerate() {
            f.insert(sym, Value::int_fresh(i as i32), 0);
        }
        // remove every index except multiples of 3, one at a time
        for (i, &sym) in s.iter().enumerate() {
            if i % 3 != 0 {
                assert!(f.remove(sym).is_some());
                assert!(f.get(sym).is_none());
            }
        }
        let left: Vec<Sym> = f.bindings().map(|b| b.sym).collect();
        let want: Vec<Sym> = s.iter().copied().step_by(3).collect();
        assert_eq!(left, want);
        assert_eq!(f.len(), want.len());
        // compacted at least once: fewer vacant slots than bindings
        assert!(f.dead() <= f.len());
        for (k, &sym) in want.iter().enumerate() {
            let b = f.get(sym).expect("kept binding");
            assert_eq!(crate::coerce::as_integer(&b.value), 3 * k as i32);
        }
        for &sym in &want {
            f.remove(sym);
        }
        assert_eq!(f.len(), 0);
        assert!(f.bindings().next().is_none());
    }

    #[test]
    fn removal_of_a_repeated_tag_finds_the_next_binding() {
        let s = syms(20);
        let mut f = Frame::default();
        f.nbase = NBASE_OPEN;
        for &sym in &s {
            f.push_new(sym, Value::Null, 0);
        }
        let x = Sym::new("remove_test_repeated");
        for k in 0..3 {
            f.push_new(x, Value::int_fresh(k), 0);
        }
        for k in 0..3 {
            let b = f.get(x).expect("a binding of x");
            assert_eq!(crate::coerce::as_integer(&b.value), k);
            f.remove(x);
        }
        assert!(f.get(x).is_none());
        assert_eq!(f.len(), 20);
    }

    /// Removing a plain value that a repeated function binding replaces
    /// changes what a function lookup finds: `remove` reports it, so that
    /// `Env::remove` invalidates the VM's caches (`fn_version`).
    #[test]
    fn removal_that_exposes_a_function_reports_it() {
        let fun = || Value::Builtin(crate::builtins::lookup_primitive("c").expect("primitive c"));
        let x = Sym::new("remove_test_exposed");
        for pad in [0, 20] {
            let mut f = Frame::default();
            f.nbase = NBASE_OPEN;
            for &sym in &syms(pad) {
                f.push_new(sym, Value::Null, 0);
            }
            f.push_new(x, Value::Null, 0);
            f.push_new(x, fun(), 0);
            f.push_new(x, Value::Null, 0);
            assert_eq!(f.remove(x).map(|(_, changed)| changed), Some(true), "pad {pad}: exposes the function");
            assert_eq!(f.remove(x).map(|(_, changed)| changed), Some(true), "pad {pad}: removes the function");
            assert_eq!(f.remove(x).map(|(_, changed)| changed), Some(false), "pad {pad}: plain value, last");
            assert!(f.remove(x).is_none());
        }
    }
}
