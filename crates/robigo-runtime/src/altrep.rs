//! Package-defined ALTREP vectors (#422, council round 7).
//!
//! A vector a package's ALTREP class made (cli's `progress_along`, its
//! `cli_timer_t`) reaches R as a copy of its data, *registered* here with
//! an opaque handle on the original core object, which the bridge keeps.
//! Where GNU R reads such an object through its class (`INTEGER_ELT` in
//! `for`, `[[`, lapply's `X[[i]]`, the scalar reads of `asLogical` and
//! friends, `sum`/`min`/`max`, the sortedness hints, `inspect`), robigo
//! calls the class's method on the original instead of reading the copy.
//!
//! Registration is by allocation, with weak entries, like the compact
//! sequences: an ordinary vector carries nothing, the `Weak` keeps the
//! address from being reused while the entry lives, and R modifying the
//! vector (`Rc::make_mut`) moves it to a new allocation that is not
//! registered, as GNU R's modified copy of an ALTREP object is a plain
//! vector. Every lookup is gated on [`any`] (no package ALTREP object was
//! ever seen: one load), then on the value's weak count.
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};

use crate::error::{Flow, RResult};
use crate::interp::Interp;
use crate::value::{Value, Vector};

enum WeakVec {
    I(Weak<Vector<i32>>),
    D(Weak<Vector<f64>>),
}

impl WeakVec {
    fn alive(&self) -> bool {
        match self {
            WeakVec::I(w) => w.strong_count() > 0,
            WeakVec::D(w) => w.strong_count() > 0,
        }
    }
}

/// Set once the first package ALTREP object is registered (the R thread
/// is the only writer; a relaxed load is all the hot paths pay).
static ANY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

thread_local! {
    static REGISTRY: RefCell<(HashMap<usize, (WeakVec, usize)>, usize)> = RefCell::new((HashMap::new(), 1024));
}

/// Whether a package ALTREP object was ever registered.
#[inline(always)]
pub fn any() -> bool {
    ANY.load(std::sync::atomic::Ordering::Relaxed)
}

/// The allocation and weak count of a value that can be registered.
#[inline(always)]
fn key(v: &Value) -> Option<(usize, usize)> {
    match v {
        Value::Int(x) | Value::Lgl(x) => Some((Rc::as_ptr(x) as usize, Rc::weak_count(x))),
        Value::Real(x) => Some((Rc::as_ptr(x) as usize, Rc::weak_count(x))),
        _ => None,
    }
}

/// Register `v`, the copy of a package ALTREP object, under `handle` (the
/// bridge's identity of the original). Integer, logical and double
/// vectors only: those are the classes whose elements R reads one at a
/// time (`Elt`); a string or list class is read in full at conversion.
pub fn register(v: &Value, handle: usize) {
    let w = match v {
        Value::Int(x) | Value::Lgl(x) => WeakVec::I(Rc::downgrade(x)),
        Value::Real(x) => WeakVec::D(Rc::downgrade(x)),
        _ => return,
    };
    let Some((k, _)) = key(v) else { return };
    REGISTRY.with(|r| {
        let mut r = r.borrow_mut();
        if r.0.len() >= r.1 {
            r.0.retain(|_, (w, _)| w.alive());
            r.1 = (r.0.len() * 2).max(1024);
        }
        r.0.insert(k, (w, handle));
    });
    ANY.store(true, std::sync::atomic::Ordering::Relaxed);
}

/// The handle of a registered value.
#[inline]
pub fn lookup(v: &Value) -> Option<usize> {
    if !any() {
        return None;
    }
    lookup_slow(v)
}

#[cold]
#[inline(never)]
fn lookup_slow(v: &Value) -> Option<usize> {
    let (k, weak) = key(v)?;
    if weak == 0 {
        return None;
    }
    // the Weak pins the address: an entry for it is this vector
    REGISTRY.with(|r| r.borrow().0.get(&k).map(|(_, h)| *h))
}

/// The handles still registered (the bridge releases the others' originals).
pub fn live_handles() -> Vec<usize> {
    REGISTRY.with(|r| {
        let mut r = r.borrow_mut();
        r.0.retain(|_, (w, _)| w.alive());
        r.0.values().map(|(_, h)| *h).collect()
    })
}

fn bridge() -> RResult<&'static dyn crate::native::NativeBridge> {
    crate::native::bridge().ok_or_else(|| Flow::error("robigo: ALTREP classes require the native bridge"))
}

/// Element `i` of a registered value, from its class (`*_ELT`).
pub fn elt(interp: &mut Interp, handle: usize, i: usize) -> RResult<Value> {
    bridge()?.altrep_elt(interp, handle, i)
}

/// `Sum`, `Min` or `Max` of a registered value, if its class has the
/// method and it answers (R: summary.c's ALTINTEGER_SUM etc.).
pub fn summary(interp: &mut Interp, handle: usize, op: Summary, narm: bool) -> RResult<Option<Value>> {
    bridge()?.altrep_summary(interp, handle, op, narm)
}

/// `Is_sorted` (or `No_NA`) of a registered value, if its class has the method.
pub fn sorted(interp: &mut Interp, handle: usize, no_na: bool) -> RResult<Option<i32>> {
    bridge()?.altrep_sorted(interp, handle, no_na)
}

/// `Extract_subset` of a registered value, if its class has the method
/// and it answers (R: subset.c's ALTVEC_EXTRACT_SUBSET).
pub fn extract_subset(interp: &mut Interp, handle: usize, indx: &Value) -> RResult<Option<Value>> {
    bridge()?.altrep_subset(interp, handle, indx)
}

/// The class info and `Serialized_state` of a registered value, if its
/// class has the method and it answers (R: serialize.c's version 3 path).
pub fn serialized_state(interp: &mut Interp, handle: usize) -> RResult<Option<(Value, Value)>> {
    bridge()?.altrep_serialized(interp, handle)
}

/// Print a registered value as `.Internal(inspect)` does, through its
/// class's `Inspect` (R: inspect.c's ALTREP_INSPECT). False: the class has
/// no method or it declined, and inspect prints the vector itself.
pub fn inspect(interp: &mut Interp, handle: usize, pre: i32, deep: i32, pvec: i32) -> RResult<bool> {
    bridge()?.altrep_inspect(interp, handle, pre, deep, pvec)
}

/// One of the summaries an ALTREP class can answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Summary {
    Sum,
    Min,
    Max,
}
