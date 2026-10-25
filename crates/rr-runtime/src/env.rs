//! Environments: mutable frames of bindings with a parent pointer.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};
use std::rc::Rc;

use crate::sym::Sym;
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
#[derive(Default)]
pub struct Frame {
    pub entries: Vec<Binding>,
    index: Option<SymMap<usize>>,
    /// The frame belongs to an environment on the search path.
    pub search: bool,
}

thread_local! {
    /// Incremented whenever a binding that could hold a function changes
    /// in an environment on the search path (global, attached packages,
    /// base). Function lookups through the search path are cached against
    /// it.
    static SEARCH_EPOCH: Cell<u64> = const { Cell::new(0) };
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
    }
}

const INDEX_THRESHOLD: usize = 12;

impl Frame {
    pub fn position(&self, sym: Sym) -> Option<usize> {
        match &self.index {
            Some(ix) => ix.get(&sym).copied(),
            None => self.entries.iter().position(|b| b.sym == sym),
        }
    }

    pub fn get(&self, sym: Sym) -> Option<&Binding> {
        self.position(sym).map(|i| &self.entries[i])
    }

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
        if self.search && (function_like(&value) || flags & BIND_ACTIVE != 0) {
            bump_search_epoch();
        }
        self.entries.push(Binding { sym, value, flags });
        let n = self.entries.len();
        match &mut self.index {
            Some(ix) => {
                ix.insert(sym, n - 1);
            }
            None if n > INDEX_THRESHOLD => {
                self.index = Some(self.entries.iter().enumerate().map(|(i, b)| (b.sym, i)).collect());
            }
            None => {}
        }
    }

    pub fn remove(&mut self, sym: Sym) -> Option<Binding> {
        let i = self.position(sym)?;
        let b = self.entries.remove(i);
        if self.search && (function_like(&b.value) || b.flags & BIND_ACTIVE != 0) {
            bump_search_epoch();
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

pub struct EnvData {
    /// Shared so that the base namespace and the base package environment
    /// can be distinct environments over the same bindings, as in GNU R.
    pub frame: Rc<RefCell<Frame>>,
    pub parent: RefCell<Option<Env>>,
    pub attrs: RefCell<AttrBox>,
    pub locked: Cell<bool>,
    /// Special name used when printing (`R_GlobalEnv`, `base`, `namespace:foo`).
    pub name: RefCell<Option<Rc<str>>>,
    pub id: usize,
    /// Incremented when a binding in this frame that could hold a function
    /// is created, changed or removed (see [`Env::fn_version`]).
    pub fn_version: Cell<u32>,
}

#[derive(Clone)]
pub struct Env(pub Rc<EnvData>);

thread_local! {
    static NEXT_ENV_ID: Cell<usize> = const { Cell::new(1) };
}

impl Env {
    pub fn new(parent: Option<Env>) -> Env {
        Env::with_frame(
            parent,
            Rc::new(RefCell::new(Frame { entries: Vec::new(), index: None, search: false })),
        )
    }

    fn with_frame(parent: Option<Env>, frame: Rc<RefCell<Frame>>) -> Env {
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
        }))
    }

    /// A new environment sharing `other`'s bindings (the base namespace).
    pub fn alias(other: &Env, parent: Option<Env>, name: &str) -> Env {
        let e = Env::with_frame(parent, other.0.frame.clone());
        *e.0.name.borrow_mut() = Some(Rc::from(name));
        e
    }

    /// An environment on the search path (global, base, attached).
    pub fn new_search(parent: Option<Env>, name: &str) -> Env {
        let e = Env::named(parent, name);
        e.0.frame.borrow_mut().search = true;
        e
    }

    pub fn is_search(&self) -> bool {
        self.0.frame.borrow().search
    }

    pub fn set_search(&self, on: bool) {
        self.0.frame.borrow_mut().search = on;
        bump_search_epoch();
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
        *self.0.parent.borrow_mut() = p;
    }

    pub fn special_name(&self) -> Option<Rc<str>> {
        self.0.name.borrow().clone()
    }

    pub fn get_local(&self, sym: Sym) -> Option<Value> {
        self.0.frame.borrow().get(sym).map(|b| b.value.clone())
    }

    pub fn binding_flags(&self, sym: Sym) -> Option<u8> {
        self.0.frame.borrow().get(sym).map(|b| b.flags)
    }

    pub fn has_local(&self, sym: Sym) -> bool {
        self.0.frame.borrow().position(sym).is_some()
    }

    /// Define or overwrite a binding in this frame. Returns false if the
    /// binding (or the environment, for new bindings) is locked.
    pub fn set_local(&self, sym: Sym, value: Value) -> bool {
        self.set_local_flags(sym, value, 0)
    }

    pub fn set_local_flags(&self, sym: Sym, value: Value, flags: u8) -> bool {
        let mut f = self.0.frame.borrow_mut();
        let search = f.search;
        if let Some(b) = f.get_mut(sym) {
            if b.flags & BIND_LOCKED != 0 {
                return false;
            }
            if function_like(&b.value) || function_like(&value) || b.flags & BIND_ACTIVE != 0 {
                if search {
                    bump_search_epoch();
                }
                self.0.fn_version.set(self.0.fn_version.get().wrapping_add(1));
            }
            b.value = value;
            b.flags = flags;
            return true;
        }
        if self.0.locked.get() {
            return false;
        }
        if function_like(&value) || flags & BIND_ACTIVE != 0 {
            self.0.fn_version.set(self.0.fn_version.get().wrapping_add(1));
        }
        f.insert(sym, value, flags);
        true
    }

    /// Move a value out of a binding, leaving `NULL` behind, so that the
    /// caller holds the only reference and can modify it in place.
    pub fn take_local(&self, sym: Sym) -> Option<Value> {
        let mut f = self.0.frame.borrow_mut();
        let search = f.search;
        let b = f.get_mut(sym)?;
        if b.flags & (BIND_ACTIVE | BIND_LOCKED) != 0 {
            return Some(b.value.clone());
        }
        if function_like(&b.value) {
            if search {
                bump_search_epoch();
            }
            self.0.fn_version.set(self.0.fn_version.get().wrapping_add(1));
        }
        Some(std::mem::replace(&mut b.value, Value::Null))
    }

    pub fn remove(&self, sym: Sym) -> bool {
        let r = self.0.frame.borrow_mut().remove(sym);
        if r.as_ref().is_some_and(|b| function_like(&b.value)) {
            self.0.fn_version.set(self.0.fn_version.get().wrapping_add(1));
        }
        r.is_some()
    }

    /// See [`EnvData::fn_version`].
    #[inline]
    pub fn fn_version(&self) -> u32 {
        self.0.fn_version.get()
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
        self.0.frame.borrow().entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn symbols(&self) -> Vec<Sym> {
        self.0.frame.borrow().entries.iter().map(|b| b.sym).collect()
    }

    pub fn lock(&self, bindings: bool) {
        self.0.locked.set(true);
        if bindings {
            for b in self.0.frame.borrow_mut().entries.iter_mut() {
                b.flags |= BIND_LOCKED;
            }
        }
    }

    pub fn set_binding_locked(&self, sym: Sym, locked: bool) -> bool {
        let mut f = self.0.frame.borrow_mut();
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
