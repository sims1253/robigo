//! The rooted layer (docs/g3.md §2.3): `Scope<'s>`, `Root<'s>`, `Out<'s>`,
//! `Args<'a>` and `Global`, all of them value-stack slots except `Global`.
//!
//! - A scope is generative: [`Cx::scope`] runs a closure that is generic
//!   over the scope's lifetime, so no `Root<'s>` can leave it, and the
//!   value-stack top is restored when the closure returns, errs or panics.
//!   Roots are pushed only into the innermost open scope (checked in every
//!   build), so restoring a scope's top never removes a root of a scope
//!   that is still open.
//! - Root storage does not borrow the capability: a `Root` survives every
//!   `&mut Cx` call made inside its scope, and [`Root::bind`] mints a fresh
//!   `Ref` after it.
//! - Owner checks, in every build: binding or resetting a root, reading
//!   `Args`, writing an `Out` and binding a `Global` check that the handle
//!   is this isolate's; rooting, writing an `Out` and adding a global check
//!   that the object is this isolate's heap's. A failed check panics.
//! - Replacing a root's value takes `&mut Cx` ([`Root::reset`]), which ends
//!   every `Ref` and slice read from it.

#![allow(unsafe_code)]

use crate::handle::{Ref, Sym};
use crate::isolate::{Cx, Frame, Isolate, foreign_handle};
use crate::vstack::Slot;
use robigo_heap::{Sexp, Trace};
use std::fmt;
use std::marker::PhantomData;
use std::ptr::NonNull;

/// An open root scope of one isolate. `Copy`; usable only inside the
/// closure [`Cx::scope`] runs, and only while no inner scope is open.
#[derive(Clone, Copy)]
pub struct Scope<'s> {
    id: u64,
    iso: NonNull<Isolate>,
    _s: PhantomData<fn(&'s ()) -> &'s ()>,
}

impl Cx {
    /// Open a root scope for `f`: roots pushed into it live until `f`
    /// returns, unwinds or errs, and cannot be named after that.
    pub fn scope<R>(&mut self, f: impl for<'s> FnOnce(&mut Cx, Scope<'s>) -> R) -> R {
        let iso = self.iso();
        let frame = iso.frame();
        let id = iso.scope_serial.get() + 1;
        iso.scope_serial.set(id);
        iso.innermost.set(id);
        let _end = EndScope { iso: self.iso_ptr(), frame };
        let s = Scope { id, iso: self.iso_ptr(), _s: PhantomData };
        f(self, s)
    }

    /// Root `v` until [`Cx::release_global`]: the isolate's global table
    /// (GNU R's precious list and API globals live there).
    pub fn global<'g>(&'g self, v: impl Into<Ref<'g>>) -> Global {
        let x = v.into().sexp();
        let iso = self.iso();
        iso.check_owns(x);
        Global { iso: self.iso_ptr(), idx: iso.global_add(x) }
    }

    /// Unroot a global. Takes `&mut Cx`: no `Ref` read through it survives.
    pub fn release_global(&mut self, g: Global) {
        if g.iso != self.iso_ptr() {
            foreign_handle();
        }
        self.iso().global_remove(g.idx);
    }
}

/// Ends a scope: restores the value-stack top and the innermost scope.
struct EndScope {
    iso: NonNull<Isolate>,
    frame: Frame,
}

impl Drop for EndScope {
    fn drop(&mut self) {
        // SAFETY: made in `Cx::scope`, whose `&mut Cx` borrow keeps the
        // isolate alive until this guard is dropped.
        unsafe { self.iso.as_ref() }.restore(self.frame);
    }
}

impl<'s> Scope<'s> {
    /// The isolate, checked: `cx` is this scope's, and the scope is the
    /// innermost open one (so a push is above every slot of an open scope
    /// and below every slot of a closed one).
    #[inline(always)]
    fn open<'c>(&self, cx: &'c Cx) -> &'c Isolate {
        if cx.iso_ptr() != self.iso {
            foreign_handle();
        }
        let iso = cx.iso();
        if iso.innermost.get() != self.id {
            not_innermost();
        }
        iso
    }

    /// Root `v` in this scope.
    #[inline]
    pub fn root<'g>(&self, cx: &'g Cx, v: impl Into<Ref<'g>>) -> Root<'s> {
        let iso = self.open(cx);
        let x = v.into().sexp();
        iso.check_owns(x);
        Root { slot: iso.stack().push(Slot::object(x)), _s: PhantomData }
    }

    /// An empty slot in this scope, for a callee to write its result into.
    /// [`Out::root`] reads it afterwards (R_NilValue until written).
    #[inline]
    pub fn out(&self, cx: &Cx) -> Out<'s> {
        let iso = self.open(cx);
        Out { slot: iso.stack().push(Slot::EMPTY), _s: PhantomData }
    }

    /// An argument window in this scope: `values`, then (if given) one
    /// name per value, R_NilValue for none. Panics if the names do not
    /// match the values one for one.
    pub fn args<'g>(&self, cx: &'g Cx, values: &[Ref<'g>], names: Option<&[Option<Sym<'g>>]>) -> Args<'s> {
        let iso = self.open(cx);
        let n = u32::try_from(values.len()).expect("robigo: too many arguments");
        let mut any_alt = false;
        let mut base = None;
        for &v in values {
            iso.check_owns(v.sexp());
            any_alt |= v.is_package_altrep();
            let p = iso.stack().push(Slot::object(v.sexp()));
            base.get_or_insert(p);
        }
        let base = base.unwrap_or_else(|| iso.stack().top());
        let names = names.map(|ns| {
            assert_eq!(ns.len(), values.len(), "robigo: one name per argument");
            let first = iso.stack().top();
            for &s in ns {
                let x = s.map_or(iso.nil, |s| s.as_ref().sexp());
                iso.check_owns(x);
                iso.stack().push(Slot::object(x));
            }
            first
        });
        Args { base, names, n, any_alt, _a: PhantomData }
    }
}

#[cold]
#[inline(never)]
fn not_innermost() -> ! {
    panic!("robigo: rooting into a scope that is not the innermost open one")
}

/// A root: a value-stack slot of an open scope. `Copy`, 8 bytes; survives
/// `&mut Cx` calls; cannot leave its scope.
#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(transparent)]
pub struct Root<'s> {
    slot: NonNull<Slot>,
    _s: PhantomData<&'s ()>,
}

impl fmt::Debug for Root<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Root({:p})", self.slot)
    }
}

impl<'s> Root<'s> {
    /// The object rooted here, readable for the `&Cx` interval `'g`
    /// (R_NilValue if the slot is still empty). Panics if the root is of
    /// another isolate, or holds an unboxed value.
    #[inline]
    pub fn bind<'g>(self, cx: &'g Cx) -> Ref<'g> {
        let iso = cx.iso();
        if !iso.stack().owns(self.slot) {
            foreign_handle();
        }
        // SAFETY: a slot of this isolate's stack, checked above.
        let s = unsafe { iso.stack().read(self.slot) };
        let x = match s.as_object() {
            Some(x) => x,
            None if s == Slot::EMPTY => iso.nil,
            None => unboxed(),
        };
        // SAFETY: the slot is live: its scope is open (the brand), and
        // scopes close innermost first, so no restore has removed it. An
        // object slot of a live root holds a live object of this isolate
        // (every write checks ownership), reachable for 'g.
        unsafe { Ref::from_sexp(x, cx) }
    }

    /// Empty this root and return the slot as an `Out` to write it again.
    /// Takes `&mut Cx`, so no `Ref` or slice read from the old value is
    /// still in use.
    #[inline]
    pub fn reset(self, cx: &mut Cx) -> Out<'s> {
        let iso = cx.iso();
        if !iso.stack().owns(self.slot) {
            foreign_handle();
        }
        // SAFETY: a slot of this stack, checked above; EMPTY holds no object.
        unsafe { iso.stack().write(self.slot, Slot::EMPTY) };
        Out { slot: self.slot, _s: PhantomData }
    }

    /// The slot, for the raw layer (the VM, the collector's tests).
    #[inline(always)]
    pub fn as_ptr(self) -> *const Slot {
        self.slot.as_ptr()
    }
}

#[cold]
#[inline(never)]
fn already_written() -> ! {
    panic!("robigo: an Out whose slot already holds a value")
}

#[cold]
#[inline(never)]
fn unboxed() -> ! {
    panic!("robigo: a root that holds an unboxed value (box it before binding)")
}

/// A slot to write one result into: the caller's VM register or a slot of
/// the caller's scope. Written before the callee's roots are released (the
/// callee writes it, then returns). Not `Copy`: writing consumes it.
pub struct Out<'s> {
    slot: NonNull<Slot>,
    _s: PhantomData<&'s ()>,
}

impl fmt::Debug for Out<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Out({:p})", self.slot)
    }
}

impl<'s> Out<'s> {
    /// Write the result. Panics if the slot or the object is of another
    /// isolate, or if the slot is no longer empty: an `Out` fills an empty
    /// slot once, so it never replaces a value (that takes [`Root::reset`],
    /// under `&mut Cx`), even when [`Root::reset`] made a second `Out` for
    /// the same slot.
    #[inline]
    pub fn set<'g>(self, cx: &'g Cx, v: impl Into<Ref<'g>>) {
        let iso = cx.iso();
        if !iso.stack().owns(self.slot) {
            foreign_handle();
        }
        let x = v.into().sexp();
        iso.check_owns(x);
        // SAFETY: a slot of this stack, checked above.
        if unsafe { iso.stack().read(self.slot) } != Slot::EMPTY {
            already_written();
        }
        // SAFETY: a slot of this stack, and a live object of this isolate.
        unsafe { iso.stack().write(self.slot, Slot::object(x)) };
    }

    /// The same slot as a root, to read the result once it is written.
    #[inline(always)]
    pub fn root(&self) -> Root<'s> {
        Root { slot: self.slot, _s: PhantomData }
    }

    /// An `Out` for a slot the raw layer owns (the VM's destination).
    ///
    /// # Safety
    /// `slot` is a live value-stack slot of the isolate that will be written
    /// through this `Out`, and stays live for `'s`. It should be empty:
    /// [`Out::set`] panics otherwise.
    #[inline(always)]
    pub unsafe fn from_raw(slot: NonNull<Slot>) -> Out<'s> {
        Out { slot, _s: PhantomData }
    }
}

/// A call's arguments: a window of value-stack slots the caller rooted for
/// `'a`, with optional names, and whether any argument is a package ALTREP
/// object (so a builtin takes one branch for the common case, §2.4).
#[derive(Clone, Copy)]
pub struct Args<'a> {
    base: NonNull<Slot>,
    names: Option<NonNull<Slot>>,
    n: u32,
    any_alt: bool,
    _a: PhantomData<&'a ()>,
}

impl fmt::Debug for Args<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Args({:p}, {})", self.base, self.n)
    }
}

impl<'a> Args<'a> {
    #[inline(always)]
    pub fn len(&self) -> usize {
        self.n as usize
    }

    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.n == 0
    }

    /// Argument `i` as a root. Panics if `i` is out of range.
    #[inline(always)]
    pub fn root(&self, i: usize) -> Root<'a> {
        assert!(i < self.len(), "robigo: argument {i} of {}", self.n);
        // SAFETY: the window has `n` slots from `base`.
        Root { slot: unsafe { self.base.add(i) }, _s: PhantomData }
    }

    /// Argument `i`, readable for `'g`. Panics if `i` is out of range or
    /// the window is of another isolate.
    #[inline(always)]
    pub fn get<'g>(&self, cx: &'g Cx, i: usize) -> Ref<'g> {
        self.root(i).bind(cx)
    }

    /// The name of argument `i`, if it has one.
    pub fn name<'g>(&self, cx: &'g Cx, i: usize) -> Option<Sym<'g>> {
        assert!(i < self.len(), "robigo: argument {i} of {}", self.n);
        let names = self.names?;
        // SAFETY: the names window has `n` slots too.
        let r = Root { slot: unsafe { names.add(i) }, _s: PhantomData }.bind(cx);
        r.as_sym()
    }

    /// Whether any argument is an object of a package ALTREP class.
    #[inline(always)]
    pub fn any_package_altrep(&self) -> bool {
        self.any_alt
    }

    /// The window the VM's dispatcher built.
    ///
    /// # Safety
    /// `base` (and `names`, if given) start `n` live object slots of the
    /// isolate the window is read with, rooted for `'a`; `names` hold
    /// symbols or R_NilValue; `any_alt` is true if any value is a package
    /// ALTREP object.
    #[inline(always)]
    pub unsafe fn from_raw(base: NonNull<Slot>, names: Option<NonNull<Slot>>, n: u32, any_alt: bool) -> Args<'a> {
        Args { base, names, n, any_alt, _a: PhantomData }
    }
}

/// A root in the isolate's global table, until [`Cx::release_global`]. Not
/// `Copy`: releasing consumes it. Dropping it without releasing keeps the
/// object rooted (GNU R's `R_PreserveObject` without a release).
#[must_use = "a Global roots its object until released"]
#[derive(Debug)]
pub struct Global {
    iso: NonNull<Isolate>,
    idx: u32,
}

impl Global {
    /// The object, readable for `'g`. Panics if the global is of another
    /// isolate.
    #[inline]
    pub fn bind<'g>(&self, cx: &'g Cx) -> Ref<'g> {
        if self.iso != cx.iso_ptr() {
            foreign_handle();
        }
        let x = cx.iso().global_get(self.idx);
        // SAFETY: the table roots `x` (a live object of this isolate, checked
        // when added) until this `Global` is released, which consumes it.
        unsafe { Ref::from_sexp(x, cx) }
    }
}

/// The global table: entries with free ones reused. A free entry is null
/// (a null edge, which a collection skips).
#[derive(Default, Trace)]
pub(crate) struct Globals {
    slots: Vec<Sexp>,
    free: Vec<u32>,
}

impl Globals {
    pub(crate) fn add(&mut self, x: Sexp) -> u32 {
        match self.free.pop() {
            Some(i) => {
                self.slots[i as usize] = x;
                i
            }
            None => {
                let i = u32::try_from(self.slots.len()).expect("robigo: too many globals");
                self.slots.push(x);
                i
            }
        }
    }
    pub(crate) fn get(&self, i: u32) -> Sexp {
        let x = self.slots[i as usize];
        assert!(!x.is_null(), "robigo: a released global");
        x
    }
    pub(crate) fn remove(&mut self, i: u32) {
        let x = &mut self.slots[i as usize];
        assert!(!x.is_null(), "robigo: a global released twice");
        *x = Sexp::NULL;
        self.free.push(i);
    }
    pub(crate) fn slot(&self, i: usize) -> Option<Sexp> {
        self.slots.get(i).copied().filter(|x| !x.is_null())
    }
    pub(crate) fn slots(&self) -> usize {
        self.slots.len()
    }
}
