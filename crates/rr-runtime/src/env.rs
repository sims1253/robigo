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
        self.0 = (n as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    }
}

type SymMap<V> = HashMap<Sym, V, BuildHasherDefault<SymHasher>>;

/// Frames are small vectors; large ones (globals, base, namespaces) get a
/// hash index.
#[derive(Default)]
pub struct Frame {
    pub entries: Vec<Binding>,
    index: Option<SymMap<usize>>,
    /// Bindings in this frame count towards [`shadow_count`] (all frames
    /// except the base environment's).
    counted: bool,
}

thread_local! {
    /// Number of bindings of each symbol in environments other than base.
    /// When it is zero, a lookup from any environment ends in base, so the
    /// evaluator can skip walking the environment chain.
    static SHADOW: RefCell<Vec<u32>> = const { RefCell::new(Vec::new()) };
}

/// See [`SHADOW`].
#[inline]
pub fn shadow_count(sym: Sym) -> u32 {
    SHADOW.with(|s| s.borrow().get(sym.0 as usize).copied().unwrap_or(0))
}

fn shadow_adjust(sym: Sym, up: bool) {
    SHADOW.with(|s| {
        let mut s = s.borrow_mut();
        let i = sym.0 as usize;
        if s.len() <= i {
            s.resize(i + 1, 0);
        }
        if up {
            s[i] += 1;
        } else {
            s[i] = s[i].saturating_sub(1);
        }
    });
}

impl Drop for Frame {
    fn drop(&mut self) {
        if self.counted {
            for b in &self.entries {
                shadow_adjust(b.sym, false);
            }
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
        self.entries.push(Binding { sym, value, flags });
        if self.counted {
            shadow_adjust(sym, true);
        }
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
        if self.counted {
            shadow_adjust(sym, false);
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
    pub frame: RefCell<Frame>,
    pub parent: RefCell<Option<Env>>,
    pub attrs: RefCell<AttrBox>,
    pub locked: Cell<bool>,
    /// Special name used when printing (`R_GlobalEnv`, `base`, `namespace:foo`).
    pub name: RefCell<Option<Rc<str>>>,
    pub id: usize,
}

#[derive(Clone)]
pub struct Env(pub Rc<EnvData>);

thread_local! {
    static NEXT_ENV_ID: Cell<usize> = const { Cell::new(1) };
}

impl Env {
    pub fn new(parent: Option<Env>) -> Env {
        let id = NEXT_ENV_ID.with(|c| {
            let v = c.get();
            c.set(v + 1);
            v
        });
        Env(Rc::new(EnvData {
            frame: RefCell::new(Frame { entries: Vec::new(), index: None, counted: true }),
            parent: RefCell::new(parent),
            attrs: RefCell::new(None),
            locked: Cell::new(false),
            name: RefCell::new(None),
            id,
        }))
    }

    /// The base environment: its bindings do not count as shadowing.
    pub fn new_base(parent: Option<Env>, name: &str) -> Env {
        let e = Env::named(parent, name);
        e.0.frame.borrow_mut().counted = false;
        e
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
        if let Some(b) = f.get_mut(sym) {
            if b.flags & BIND_LOCKED != 0 {
                return false;
            }
            b.value = value;
            b.flags = flags;
            return true;
        }
        if self.0.locked.get() {
            return false;
        }
        f.insert(sym, value, flags);
        true
    }

    /// Move a value out of a binding, leaving `NULL` behind, so that the
    /// caller holds the only reference and can modify it in place.
    pub fn take_local(&self, sym: Sym) -> Option<Value> {
        let mut f = self.0.frame.borrow_mut();
        let b = f.get_mut(sym)?;
        if b.flags & (BIND_ACTIVE | BIND_LOCKED) != 0 {
            return Some(b.value.clone());
        }
        Some(std::mem::replace(&mut b.value, Value::Null))
    }

    pub fn remove(&self, sym: Sym) -> bool {
        self.0.frame.borrow_mut().remove(sym).is_some()
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
