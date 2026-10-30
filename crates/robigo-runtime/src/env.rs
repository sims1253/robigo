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
    pub entries: smallvec::SmallVec<[Binding; INLINE_BINDINGS]>,
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
    pub(crate) fn frame_names(&self) -> Vec<Sym> {
        if self.insertion {
            return self.entries.iter().map(|b| b.sym).collect();
        }
        let (base, added) = self.entries.split_at((self.nbase as usize).min(self.entries.len()));
        added.iter().rev().chain(base).map(|b| b.sym).collect()
    }

    pub fn remove(&mut self, sym: Sym) -> Option<Binding> {
        let i = self.position(sym)?;
        let b = self.entries.remove(i);
        if self.search && (function_like(&b.value) || b.flags & BIND_ACTIVE != 0) {
            bump_search_epoch();
        }
        if !self.hashed && (i as u32) < self.nbase && self.nbase != NBASE_OPEN {
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
    pub name: RefCell<Option<crate::value::RString>>,
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
        let env = Env::with_capacity(parent, 0);
        env.track_escape();
        env
    }

    pub(crate) fn track_escape(&self) {
        trace::track(self);
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
        };
        let e = Env::with_frame(parent, FrameCell::Shared(Rc::new(RefCell::new(frame))));
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
        *self.0.parent.borrow_mut() = p;
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
        if self.0.frame().borrow().hashed {
            return self.table_names();
        }
        self.0.frame().borrow().frame_names()
    }

    /// The bound symbols in no particular order (for callers that sort).
    pub fn symbols_unordered(&self) -> Vec<Sym> {
        self.0.frame().borrow().entries.iter().map(|b| b.sym).collect()
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
