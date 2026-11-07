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

// ---- deferred package ALTREP strings and lists (#468) -----------------
//
// A package ALTREP string or list vector whose data GNU R has not been
// handed yet (`Dataptr_or_null` is NULL: vctrs' lazy character, vroom's
// columns) reaches R unread, as a wrapper of the core object (an `ExtPtr`
// the bridge makes): reading it would run the class's `Length`/`Elt`/
// `Dataptr`, which GNU R calls only where R uses the contents. The wrapper
// is forced, into #453's eager copy, where R uses its contents: before a
// builtin body runs (`Interp::invoke_builtin`, which the interpreter, the
// VM and `.Internal` all go through) and where the specials that read
// their operands (`if`, `while`, `for`, `switch`, `&&`, `||`, `[`, `[[`,
// `$`, the assignment forms, `rep`, `round`, `signif`, `log`) evaluate
// them. The builtins GNU R runs without touching an operand's contents
// keep it unread ([`may_stay_deferred`]).

static ANY_DEFERRED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Whether a deferred value was ever made (one load on every builtin call).
#[inline(always)]
pub fn any_deferred() -> bool {
    ANY_DEFERRED.load(std::sync::atomic::Ordering::Relaxed)
}

/// The bridge made a deferred value.
pub fn note_deferred() {
    ANY_DEFERRED.store(true, std::sync::atomic::Ordering::Relaxed);
}

/// The type a deferred value has (`Str` or `List`), if `v` is one; its
/// contents stay unread.
#[inline]
pub fn deferred_type(v: &Value) -> Option<crate::value::VType> {
    if !matches!(v, Value::ExtPtr(_)) || !any_deferred() {
        return None;
    }
    crate::native::bridge()?.deferred_type(v)
}

/// The contents of `v` if it is a deferred value (its class's methods run
/// now, and can raise or run R code), else `None`.
#[inline]
pub fn force(interp: &mut Interp, v: &Value) -> RResult<Option<Value>> {
    if !matches!(v, Value::ExtPtr(_)) || !any_deferred() {
        return Ok(None);
    }
    bridge()?.force_deferred(interp, v)
}

/// `v`, forced if it is a deferred value.
#[inline(always)]
pub fn forced(interp: &mut Interp, v: Value) -> RResult<Value> {
    if !matches!(v, Value::ExtPtr(_)) {
        return Ok(v);
    }
    Ok(match force(interp, &v)? {
        Some(f) => f,
        None => v,
    })
}

/// Force every deferred argument.
pub fn force_all(interp: &mut Interp, args: &mut [crate::value::Arg]) -> RResult<()> {
    if !any_deferred() {
        return Ok(());
    }
    for a in args.iter_mut() {
        if let Some(f) = force(interp, &a.value)? {
            a.value = f;
        }
    }
    Ok(())
}

/// Force the deferred arguments of builtin `b` but those GNU R's
/// implementation of `b` never reads ([`may_stay_deferred`]).
#[cold]
#[inline(never)]
pub fn force_builtin_args(
    interp: &mut Interp,
    b: &crate::value::Builtin,
    args: &mut [crate::value::Arg],
) -> RResult<()> {
    for k in 0..args.len() {
        if !matches!(args[k].value, Value::ExtPtr(_)) || may_stay_deferred(b, args, k) {
            continue;
        }
        if let Some(f) = force(interp, &args[k].value)? {
            args[k].value = f;
        }
    }
    Ok(())
}

/// Whether builtin `b` uses argument `k` without its contents in GNU R,
/// so that a deferred value there stays unread (council round 8):
/// - `list` stores its arguments (builtin.c:do_makelist), `.Call` and
///   `.External` hand them to C as they are (dotcode.c);
/// - `typeof`, `is.character`, `is.list` read the type (coerce.c:do_typeof,
///   do_is), `class`, `oldClass` and `inherits` the class attribute and
///   the type (attrib.c:R_data_class2, objects.c:inherits3), `attributes`
///   the attribute list (attrib.c:do_attributes);
/// - `identical` of an object and itself is TRUE before anything is read
///   (identical.c:R_compute_identical, `x == y`);
/// - `attr<-` stores its value as it is, but for the attributes
///   attrib.c:setAttrib checks or coerces (names, dim, dimnames, class,
///   tsp, comment); row.names only check the type (row_names_gets).
fn may_stay_deferred(b: &crate::value::Builtin, args: &[crate::value::Arg], k: usize) -> bool {
    match b.name {
        "list" | ".Call" | ".External" | ".External2" | ".Call.graphics" | ".External.graphics" => true,
        "typeof" | "is.character" | "is.list" | "class" | "oldClass" | "inherits" | "attributes" => k == 0,
        "identical" => {
            k < 2
                && args.len() >= 2
                && matches!((&args[0].value, &args[1].value), (Value::ExtPtr(a), Value::ExtPtr(b)) if Rc::ptr_eq(a, b))
        }
        "attr<-" => {
            k == 2
                && args.get(1).is_some_and(|w| match &w.value {
                    Value::Str(s) if s.data.len() == 1 => s.data[0]
                        .as_deref()
                        .is_some_and(|n| !matches!(n, "names" | "dim" | "dimnames" | "class" | "tsp" | "comment")),
                    _ => false,
                })
        }
        _ => false,
    }
}

/// One of the summaries an ALTREP class can answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Summary {
    Sum,
    Min,
    Max,
}
