//! Promises, and argument slots as GNU R's promises (docs/g3.md §6.2-6.3).
//!
//! An argument is a slot of the callee's frame (N1b's kinds), not a PROMSXP:
//! delayed (expression, environment, PRSEEN, MISSING, an optional value
//! the caller computed early) until its first read, then forced (value and
//! expression). This module is what makes such a slot behave as the
//! PROMSXP GNU R's binding would hold:
//!
//! - [`Cx::force_arg`] is `forcePromise` (eval.c:954) on a delayed slot,
//!   in place: PRSEEN's "promise already under evaluation" and "restarting
//!   interrupted promise evaluation", the value written back, the caller's
//!   environment released (`SET_PRENV(e, R_NilValue)`). A delayed slot with
//!   a precomputed value becomes forced without evaluating anything.
//! - The **pending-force stack** (GNU R's `R_PendingPromises`): while a
//!   slot's expression runs, C or `get` may read the binding as an object,
//!   which materializes the slot into a PROMSXP ([`Cx::ensure_object`]).
//!   GNU R's binding held that promise all along, so the force completes
//!   the promise object, not the slot. The stack tells the materialization
//!   which force it joins. An exit through the force marks the promise (or
//!   the slot) interrupted, PRSEEN 2, as `R_restore_globals` does for the
//!   pending promises above the exit's target (every exit that passes a
//!   force in Rust passes its GNU R frame too).
//! - [`Cx::force_promise`] is `forcePromise` on a PROMSXP: a dots element,
//!   a materialized argument, `delayedAssign`'s, one made by C. A promise
//!   whose PRVALUE holds an unboxed value (GNU R's `PROMISE_TAG`,
//!   `IMMEDIATE_PROMISE_VALUES`) is evaluated; [`Cx::promise_value`] boxes
//!   it once and stores the box back (`R_expand_promise_value`).
//! - [`Cx::binding_type`] is `R_GetBindingType` (envir.c:807-882): a slot's
//!   kind is the kind of GNU R's binding, through promise chains
//!   (`promiseUnwrap`), without making the slot cell-backed.
//! - [`Cx::cleanup_envir`] is `R_CleanupEnvir` (eval.c:2132) on a slot
//!   frame: when a call's frame is unreachable after the call, its bindings
//!   are released, an argument slot as GNU R's `clearPromise` releases its
//!   promise; `unpromiseArgs`'s decrements are this release (§6.3).

use super::frame::{self, layout};
use super::*;

/// GNU R's `PRSEEN` values (a promise's gp; a delayed slot's meta bits).
pub(crate) const PRSEEN_EVALUATING: u16 = 1;
pub(crate) const PRSEEN_INTERRUPTED: u16 = 2;

const UNDER_EVALUATION: &str =
    "promise already under evaluation: recursive default argument reference or earlier problems?";
const RESTARTING: &str = "restarting interrupted promise evaluation";

/// One argument slot whose expression is being evaluated by
/// [`Cx::force_arg`]: the frame, the slot, and the PROMSXP the slot was
/// materialized into meanwhile (null until then). A precise root.
#[derive(Trace)]
pub(crate) struct PendingForce {
    frame: Sexp,
    #[trace(skip = "a slot index")]
    index: u32,
    promise: Sexp,
}

/// The pending forces, innermost last.
#[derive(Default, Trace)]
pub(crate) struct PendingForces {
    stack: UnsafeCell<Vec<PendingForce>>,
}

impl PendingForces {
    fn push(&self, frame: Sexp, index: u32) -> usize {
        // SAFETY: a shared access that ends with this call; nothing it calls
        // reaches the stack (a collection runs with no `&EnvState` in use).
        let v = unsafe { &mut *self.stack.get() };
        v.push(PendingForce { frame, index, promise: Sexp::NULL });
        v.len() - 1
    }

    /// Pop entry `at` (the innermost); the promise it was materialized
    /// into, if any.
    fn pop(&self, at: usize) -> Sexp {
        // SAFETY: as in `push`.
        let v = unsafe { &mut *self.stack.get() };
        debug_assert_eq!(v.len(), at + 1, "robigo: pending forces popped out of order");
        v.truncate(at + 1);
        v.pop().map_or(Sexp::NULL, |e| e.promise)
    }

    /// Record that slot `index` of `frame`, under evaluation, became the
    /// promise `p`.
    fn materialized(&self, frame: Sexp, index: u32, p: Sexp) {
        // SAFETY: as in `push`.
        let v = unsafe { &mut *self.stack.get() };
        if let Some(e) = v.iter_mut().rev().find(|e| e.frame == frame && e.index == index) {
            e.promise = p;
        }
    }

    pub(crate) fn for_each(&self, f: &mut dyn FnMut(Sexp)) {
        // SAFETY: as in `push`; `f` does not reach the stack (it is handed
        // copies).
        let v: Vec<(Sexp, Sexp)> = unsafe { (*self.stack.get()).iter().map(|e| (e.frame, e.promise)).collect() };
        for (a, b) in v {
            f(a);
            if !b.is_null() {
                f(b);
            }
        }
    }
}

/// GNU R's `R_BindingType_t`: what kind of binding `R_GetBindingType` and
/// `.Internal(getBindingType())` report.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BindingType {
    Unbound,
    Value,
    Missing,
    Delayed,
    Forced,
    Active,
}

impl BindingType {
    /// The string `do_bindingType` returns.
    pub fn name(self) -> &'static str {
        match self {
            BindingType::Unbound => "unbound",
            BindingType::Value => "value",
            BindingType::Missing => "missing",
            BindingType::Delayed => "delayed",
            BindingType::Forced => "forced",
            BindingType::Active => "active",
        }
    }
}

/// `PROMISE_IS_EVALUATED(p)`.
///
/// # Safety
/// `p` is a live promise of `iso`'s heap.
#[inline(always)]
pub(crate) unsafe fn is_evaluated(iso: &Isolate, p: Sexp) -> bool {
    // SAFETY: the caller's contract.
    unsafe { p.promise_tag() != 0 || p.car() != iso.unbound }
}

/// What a promise's PRVALUE holds, read without allocating.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum PrValue {
    /// Not evaluated (R_UnboundValue).
    Unevaluated,
    /// An object.
    Obj(Sexp),
    /// An unboxed value (`PROMISE_TAG`): an object only once boxed.
    Unboxed,
}

/// PRVALUE of `p`, without boxing.
///
/// # Safety
/// `p` is a live promise of `iso`'s heap.
#[inline(always)]
pub(crate) unsafe fn pr_value(iso: &Isolate, p: Sexp) -> PrValue {
    // SAFETY: the caller's contract.
    unsafe {
        if p.promise_tag() != 0 {
            return PrValue::Unboxed;
        }
        let v = p.car();
        if v == iso.unbound { PrValue::Unevaluated } else { PrValue::Obj(v) }
    }
}

/// `SET_PRVALUE(p, v)` for an object `v`: a tagged value's bits are
/// dropped first (`CLEAR_BNDCELL_TAG`), then the store counts.
///
/// # Safety
/// `p` a live promise, `v` a live object, of one heap.
unsafe fn set_prvalue(p: Sexp, v: Sexp, nil: Sexp) {
    // SAFETY: the caller's contract; a tagged PRVALUE holds no counted
    // object.
    unsafe {
        if p.promise_tag() != 0 {
            p.set_promise_tag(0);
            p.set_promise_bits(nil.addr() as u64);
        }
        p.set_car(v);
    }
}

/// GNU R's `clearPromise(p)` (eval.c:2085): a tagged promise is left as it
/// is; otherwise its value, environment and code are released.
///
/// # Safety
/// `p` is a live promise of `iso`'s heap.
// R: eval.c:clearPromise
pub(crate) unsafe fn clear_promise(iso: &Isolate, p: Sexp) {
    // SAFETY: the caller's contract.
    unsafe {
        if p.promise_tag() != 0 {
            return;
        }
        p.set_car(iso.unbound);
        p.set_tag(iso.nil);
        p.set_cdr(iso.nil);
    }
}

/// `promiseUnwrap(x, &forced)` (envir.c:785): whether the innermost promise
/// of the chain starting at the promise `x` is evaluated; `Err` for a
/// cycle (Floyd's check, as GNU R's).
///
/// # Safety
/// `x` is a live promise of `iso`'s heap.
// R: envir.c:promiseUnwrap
unsafe fn unwrap_forced(iso: &Isolate, mut x: Sexp) -> Result<bool, ()> {
    let mut slow = x;
    let mut advance_slow = false;
    // SAFETY: the caller's contract; PRCODE of a live promise is live.
    unsafe {
        loop {
            let code = x.cdr();
            if code.typ() != PROMSXP {
                return Ok(is_evaluated(iso, x));
            }
            x = code;
            if x == slow {
                return Err(());
            }
            if advance_slow {
                slow = slow.cdr();
            }
            advance_slow = !advance_slow;
        }
    }
}

/// `BINDING_TYPE` of a binding whose value is the object `v` (not active,
/// not unboxed).
///
/// # Safety
/// `v` is a live object of `iso`'s heap.
// R: envir.c:BINDING_TYPE
unsafe fn value_type(iso: &Isolate, v: Sexp) -> Result<BindingType, ()> {
    // SAFETY: the caller's contract.
    unsafe {
        if v == iso.missing_arg {
            return Ok(BindingType::Missing);
        }
        if v.typ() == PROMSXP {
            return Ok(if unwrap_forced(iso, v)? { BindingType::Forced } else { BindingType::Delayed });
        }
        Ok(BindingType::Value)
    }
}

/// An argument slot as the VM's raw lookup found it ([`RawBinding::Thunk`]):
/// a delayed slot of a frame, to force in place with [`Cx::force_arg`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ArgLoc {
    frame: Sexp,
    index: u32,
}

impl ArgLoc {
    pub(crate) fn of(s: SlotRef) -> ArgLoc {
        ArgLoc { frame: s.owner, index: s.index as u32 }
    }

    /// The frame (to root it across the force).
    pub fn frame(self) -> Sexp {
        self.frame
    }

    /// The slot's index in the frame.
    pub fn index(self) -> usize {
        self.index as usize
    }
}

impl Cx {
    /// GNU R's `PRVALUE(p)` of the evaluated promise `p`, written to `out`:
    /// an unboxed value (`PROMISE_TAG`) is boxed and stored back
    /// (`R_expand_promise_value`), so it has one identity. False (nothing
    /// written) if `p` is not evaluated.
    // R: memory.c:R_expand_promise_value
    pub fn promise_value(&mut self, p: Root<'_>, out: Out<'_>) -> RResult<bool> {
        let x = p.bind(self).sexp();
        assert_eq!(p.bind(self).sexptype(), PROMSXP, "robigo: promise_value of a non-promise");
        self.iso().check_owns(x);
        // SAFETY: a rooted promise.
        match unsafe { pr_value(self.iso(), x) } {
            PrValue::Unevaluated => Ok(false),
            PrValue::Obj(v) => {
                // SAFETY: PRVALUE of a rooted promise.
                out.set(self, unsafe { Ref::from_sexp(v, self) });
                Ok(true)
            }
            PrValue::Unboxed => {
                let b = alloc_retry(self, |cx| {
                    let p = p.bind(cx).sexp();
                    // SAFETY: a rooted tagged promise: its bits are the value
                    // of the type its tag names.
                    unsafe { box_bits(cx.iso().core(), p.promise_tag(), p.promise_bits()) }
                })?;
                let nil = self.iso().nil;
                let p = p.bind(self).sexp();
                // SAFETY: `b` is fresh (nothing collected since); the store
                // drops the tag and counts the box (SET_BNDCELL).
                unsafe {
                    if p.promise_tag() != 0 {
                        set_prvalue(p, b, nil);
                    }
                    out.set(self, Ref::from_sexp(p.car(), self));
                }
                Ok(true)
            }
        }
    }

    /// GNU R's `forcePromise(p)`, and the promise's value written to `out`.
    /// A value the caller computed early (§6.2, kept in the promise's 4th
    /// word by materialization) is taken without evaluating. The value is
    /// not marked: `ENSURE_NAMEDMAX` does nothing under reference counting
    /// (`Defn.h:337-340`).
    // R: eval.c:forcePromise
    pub fn force_promise(&mut self, p: Root<'_>, out: Out<'_>) -> RResult<()> {
        assert_eq!(p.bind(self).sexptype(), PROMSXP, "robigo: forcing a non-promise");
        // SAFETY: a rooted promise.
        if unsafe { is_evaluated(self.iso(), p.bind(self).sexp()) } {
            self.promise_value(p, out)?;
            return Ok(());
        }
        let iso = self.iso();
        let ps = p.bind(self).sexp();
        // a value computed early: forced without evaluating
        // SAFETY: a rooted promise; its 4th word is R_NilValue or a value.
        let early = unsafe { ps.aux() };
        if early != iso.nil && !early.is_null() {
            // SAFETY: as above; the stores count (SET_PRVALUE, SET_PRENV).
            unsafe {
                set_prvalue(ps, early, iso.nil);
                ps.set_aux(iso.nil);
                ps.set_gp(0);
                ps.set_tag(iso.nil);
                out.set(self, Ref::from_sexp(ps.car(), self));
            }
            return Ok(());
        }
        self.safepoint(robigo_heap::Safepoint::Entry)?;
        let ps = p.bind(self).sexp();
        // SAFETY: as above; PRSEEN is the gp field.
        let seen = unsafe { ps.gp() };
        if seen != 0 {
            self.prseen_check(seen)?;
            // SAFETY: as above.
            unsafe { p.bind(self).sexp().set_gp(PRSEEN_EVALUATING) };
        }
        // SAFETY: as above.
        unsafe { p.bind(self).sexp().set_gp(PRSEEN_EVALUATING) };
        let res = out.root();
        let r = self.scope(|cx, s| {
            let ps = p.bind(cx).sexp();
            // SAFETY: PRCODE and PRENV of a rooted promise.
            let (code, env) =
                unsafe { (s.root(cx, Ref::from_sexp(ps.cdr(), cx)), s.root(cx, Ref::from_sexp(ps.tag(), cx))) };
            cx.eval(code, env, out)
        });
        let iso = self.iso();
        let ps = p.bind(self).sexp();
        match r {
            Err(e) => {
                // R: context.c:R_restore_globals marks every pending promise
                // above the exit's target PRSEEN 2
                // SAFETY: a rooted promise.
                unsafe { ps.set_gp(PRSEEN_INTERRUPTED) };
                Err(e)
            }
            Ok(()) => {
                let val = res.bind(self).sexp();
                // SAFETY: a rooted promise and value: SET_PRVALUE (counted),
                // SET_PRSEEN(0), SET_PRENV(R_NilValue).
                unsafe {
                    set_prvalue(ps, val, iso.nil);
                    ps.set_gp(0);
                    ps.set_tag(iso.nil);
                }
                Ok(())
            }
        }
    }

    /// `forcePromise`'s PRSEEN check: "promise already under evaluation"
    /// for 1; the restart warning for 2 (after which the caller marks it 1
    /// again). Both are reported against `R_GlobalContext->call`.
    fn prseen_check(&mut self, seen: u16) -> RResult<()> {
        let call = self.global_context().call().sexp();
        self.scope(|cx, s| {
            // SAFETY: the innermost record's call, a live object.
            let c = s.root(cx, unsafe { Ref::from_sexp(call, cx) });
            if seen == PRSEEN_EVALUATING {
                return Err(cx.error_call(Some(c), UNDER_EVALUATION));
            }
            cx.warning_call(Some(c), RESTARTING)
        })
    }

    /// `forcePromise` of the promise GNU R's binding in slot `index` of the
    /// slot frame `frame` would hold, in place (docs/g3.md §6.3, "first
    /// read"): a delayed slot's expression is evaluated in its environment
    /// (the caller's, or the frame itself for a default) and the slot
    /// becomes forced (expression kept, environment released, MISSING
    /// kept); a precomputed value is taken without evaluating. A forced
    /// slot, or a value slot holding a promise (materialized), is read or
    /// forced as that. The value is written to `out`.
    // R: eval.c:forcePromise
    pub fn force_arg(&mut self, frame: Root<'_>, index: usize, out: Out<'_>) -> RResult<()> {
        let f = frame.bind(self).sexp();
        let iso = self.iso();
        iso.check_owns(f);
        // SAFETY: a rooted environment.
        assert!(unsafe { f.typ() == ENVSXP && layout::is_slots(f) }, "robigo: force_arg on a non-slot frame");
        // SAFETY: as above.
        assert!(index < unsafe { layout::capacity(f) }, "robigo: force_arg past the frame's slots");
        let s = SlotRef { owner: f, index };
        // SAFETY: a slot of a rooted frame.
        match unsafe { s.kind() } {
            DELAYED => {}
            FORCED => {
                // SAFETY: a forced slot's value is an object.
                out.set(self, unsafe { Ref::from_sexp(s.val(), self) });
                return Ok(());
            }
            VALUE if unsafe { s.val().typ() } == PROMSXP => {
                return self.scope(|cx, sc| {
                    // SAFETY: the slot's value, a live promise.
                    let p = sc.root(cx, unsafe { Ref::from_sexp(s.val(), cx) });
                    cx.force_promise(p, out)
                });
            }
            k => panic!("robigo: force_arg on a slot of kind {k}"),
        }
        // a value computed early: forced, nothing evaluated
        // SAFETY: a delayed slot's `val` is null or the early value.
        let early = unsafe { s.val() };
        if !early.is_null() {
            // SAFETY: the slot holds the value; it becomes the forced value.
            unsafe { force_slot(iso, s, early) };
            // SAFETY: as above.
            out.set(self, unsafe { Ref::from_sexp(early, self) });
            return Ok(());
        }
        self.safepoint(robigo_heap::Safepoint::Entry)?;
        let s = SlotRef { owner: frame.bind(self).sexp(), index };
        // SAFETY: as above.
        let seen = unsafe { s.prseen() };
        if seen != 0 {
            self.prseen_check(seen)?;
        }
        let s = SlotRef { owner: frame.bind(self).sexp(), index };
        let iso = self.iso();
        // SAFETY: a delayed slot of a rooted frame: its expression and
        // environment are live.
        let (expr, env) = unsafe {
            set_prseen(s, PRSEEN_EVALUATING);
            let env = if s.has(META_ORIGIN_SELF) { s.owner } else { layout::get(s.owner, layout::OFF_CALLER) };
            (s.aux(), env)
        };
        let at = iso.envs().pending.push(s.owner, index as u32);
        let res = out.root();
        let r = self.scope(|cx, sc| {
            // SAFETY: the slot's expression and environment, live (above).
            let (e, rho) = unsafe { (sc.root(cx, Ref::from_sexp(expr, cx)), sc.root(cx, Ref::from_sexp(env, cx))) };
            cx.eval(e, rho, out)
        });
        let iso = self.iso();
        let promise = iso.envs().pending.pop(at);
        let s = SlotRef { owner: frame.bind(self).sexp(), index };
        // SAFETY: a slot of the rooted frame; `promise` (if any) is the
        // object the slot was materialized into during the evaluation, kept
        // alive by the pending stack until the pop just now and by the slot
        // or `out`'s caller since (nothing collected in between).
        unsafe {
            let still = s.kind() == DELAYED && s.prseen() == PRSEEN_EVALUATING;
            match r {
                Err(e) => {
                    if !promise.is_null() {
                        promise.set_gp(PRSEEN_INTERRUPTED);
                    } else if still {
                        set_prseen(s, PRSEEN_INTERRUPTED);
                    }
                    Err(e)
                }
                Ok(()) => {
                    let val = res.bind(self).sexp();
                    if !promise.is_null() {
                        // GNU R's binding held this promise all along
                        set_prvalue(promise, val, iso.nil);
                        promise.set_gp(0);
                        promise.set_tag(iso.nil);
                    } else if still {
                        force_slot(iso, s, val);
                    }
                    Ok(())
                }
            }
        }
    }

    /// GNU R's `R_GetBindingType(sym, env)`: the kind of `sym`'s binding in
    /// `env`'s own frame. A slot reports the kind GNU R's binding would have
    /// (§6.2): a delayed argument is "delayed" until its first read, through
    /// any chain of promises its expression is (a forwarded `...` element),
    /// and the slot is neither materialized nor made cell-backed.
    // R: envir.c:R_GetBindingType, R_GetVarLocType, BINDING_TYPE, SYMBOL_BINDING_TYPE
    pub fn binding_type(&mut self, sym: Root<'_>, env: Root<'_>) -> RResult<BindingType> {
        let (s, r) = (sym.bind(self), env.bind(self));
        // R: envir.c:R_findVarLocInFrameCheck
        if s.sexptype() != SYMSXP {
            return Err(self.error("not a symbol"));
        }
        if r.sexptype() != ENVSXP {
            return Err(self.error("not an environment"));
        }
        let (s, r) = (s.sexp(), r.sexp());
        let iso = self.iso();
        iso.check_owns(s);
        iso.check_owns(r);
        // SAFETY: rooted, live; frames are read only.
        let t = unsafe {
            let l = match loc_in_frame(iso, r, s) {
                Ok(Some(l)) => l,
                Ok(None) => return Ok(BindingType::Unbound),
                Err(k) => return Err(special_error(self, k)),
            };
            match l.at() {
                At::Base(sym) => {
                    let c = iso.envs().cell(sym);
                    if c.bits & ACTIVE_BINDING != 0 {
                        Ok(BindingType::Active)
                    } else if c.value.is_null() || c.value == iso.unbound {
                        Ok(BindingType::Unbound)
                    } else {
                        value_type(iso, c.value)
                    }
                }
                At::Cell(c) => {
                    if c.gp() & ACTIVE_BINDING != 0 {
                        Ok(BindingType::Active)
                    } else {
                        value_type(iso, c.car())
                    }
                }
                At::FrameSlot(sl) => match sl.kind() {
                    VALUE => value_type(iso, sl.val()),
                    REAL | INT | LGL => Ok(BindingType::Value),
                    ACTIVE => Ok(BindingType::Active),
                    // the slot is the outer promise of the chain
                    DELAYED | FORCED => {
                        let code = sl.aux();
                        if code.typ() == PROMSXP {
                            unwrap_forced(iso, code).map(|f| if f { BindingType::Forced } else { BindingType::Delayed })
                        } else if sl.kind() == FORCED {
                            Ok(BindingType::Forced)
                        } else {
                            Ok(BindingType::Delayed)
                        }
                    }
                    k => unreachable!("robigo: a bound slot of kind {k}"),
                },
            }
        };
        t.map_err(|()| self.error("cycle detected in promise chain"))
    }

    /// GNU R's `R_CleanupEnvir(rho, val)` after a closure call returned
    /// `val` (eval.c:2132, with `ADJUST_ENVIR_REFCNTS`): if the call's frame
    /// is unreachable (its count, less the cycles through its own bindings,
    /// is zero), its bindings are released in listing order up to the first
    /// shared binding cell (an argument slot as `clearPromise` releases its
    /// promise, a promise or `...` list only it holds cleared, every other
    /// value dropped), and its enclosure becomes the empty environment.
    /// Unboxed values stay, as GNU R's `BNDCELL_TAG` cells do. Only slot
    /// frames (every closure frame) are cleaned.
    // R: eval.c:R_CleanupEnvir, countCycleRefs, cleanupEnvDots
    pub fn cleanup_envir(&mut self, rho: Root<'_>, val: Option<Root<'_>>) {
        let r = rho.bind(self).sexp();
        let v = val.map_or(Sexp::NULL, |v| v.bind(self).sexp());
        let iso = self.iso();
        iso.check_owns(r);
        if r == v {
            return;
        }
        // SAFETY: a rooted environment; the walk reads slots and drops
        // counts, which neither allocates nor collects.
        unsafe {
            if r.typ() != ENVSXP || !layout::is_slots(r) {
                return;
            }
            let mut refs = r.refcnt() as i32;
            if refs > 0 {
                refs -= cycle_refs(iso, r, v);
            }
            if refs != 0 {
                return;
            }
            let nil = iso.nil;
            for s in frame::bindings(r, nil) {
                match s.kind() {
                    // BNDCELL_TAG: `continue`, the value stays
                    REAL | INT | LGL => continue,
                    // an argument: its promise's clearPromise, then the
                    // binding's SETCAR(b, R_NilValue)
                    DELAYED | FORCED => {
                        s.store_aux(Sexp::NULL);
                        s.store_val(Sexp::NULL);
                        s.set_flag(META_ORIGIN_SELF, false);
                        set_prseen(s, 0);
                        s.set_key(s.sym(), VALUE);
                        s.store_val(nil);
                    }
                    VALUE | ACTIVE => {
                        release_value(iso, s.val(), v);
                        s.store_val(nil);
                    }
                    CELL => {
                        let c = s.val();
                        if c.refcnt() != 1 {
                            break;
                        }
                        release_value(iso, c.car(), v);
                        chain::set_value(c, nil);
                    }
                    k => unreachable!("robigo: a bound slot of kind {k}"),
                }
            }
            frame::release_caller(r, nil);
            env::set_enclos(r, iso.envs().empty);
        }
    }
}

/// `countCycleRefs(rho, val)`: the references to frame `r` from its own
/// bindings' promises (PRENV), closures (CLOENV) and itself, each held by
/// that binding alone. A delayed slot of a default evaluates in the frame
/// without counting it (no edge), so it needs no discount.
///
/// # Safety
/// `r` is a live slot frame of `iso`'s heap; `v` null or live.
// R: eval.c:countCycleRefs
unsafe fn cycle_refs(iso: &Isolate, r: Sexp, v: Sexp) -> i32 {
    let mut crefs = 0;
    // SAFETY: the caller's contract.
    unsafe {
        for s in frame::bindings(r, iso.nil) {
            let x = match s.kind() {
                REAL | INT | LGL | DELAYED | FORCED => continue,
                CELL => {
                    let c = s.val();
                    if c.refcnt() != 1 {
                        break;
                    }
                    c.car()
                }
                _ => s.val(),
            };
            if x == v || x.is_null() {
                continue;
            }
            match x.typ() {
                PROMSXP if x.refcnt() == 1 && x.tag() == r => crefs += 1,
                CLOSXP if x.refcnt() == 1 && x.tag() == r => crefs += 1,
                ENVSXP if x == r => crefs += 1,
                _ => {}
            }
        }
    }
    crefs
}

/// `R_CleanupEnvir`'s treatment of a binding's value `x` before the
/// binding drops it: a promise or `...` list that only the binding holds,
/// and that is not the call's value, is cleared.
///
/// # Safety
/// `x` is a live object (or R_NilValue) of `iso`'s heap.
unsafe fn release_value(iso: &Isolate, x: Sexp, v: Sexp) {
    // SAFETY: the caller's contract.
    unsafe {
        if x.is_null() || x == v || x.refcnt() != 1 {
            return;
        }
        match x.typ() {
            PROMSXP => clear_promise(iso, x),
            DOTSXP => {
                // R: eval.c:cleanupEnvDots
                let mut d = x;
                while d != iso.nil && d.refcnt() == 1 {
                    let e = d.car();
                    if e.refcnt() == 1 && e.typ() == PROMSXP {
                        clear_promise(iso, e);
                    }
                    d.set_car(iso.nil);
                    d = d.cdr();
                }
            }
            // (VECSXP: cleanupEnvVector is disabled in GNU R)
            _ => {}
        }
    }
}

/// Set a delayed slot's PRSEEN.
///
/// # Safety
/// `s` is a live slot.
unsafe fn set_prseen(s: SlotRef, v: u16) {
    // SAFETY: the caller's contract.
    unsafe {
        let m = s.meta() & !(3 << layout::META_PRSEEN_SHIFT);
        s.set_meta(m | ((v as u64 & 3) << layout::META_PRSEEN_SHIFT));
    }
}

/// A delayed slot becomes forced with `val`: GNU R's `SET_PRVALUE`,
/// `SET_PRSEEN(0)`, `SET_PRENV(R_NilValue)` on the promise the binding
/// holds. The expression and MISSING stay; the caller's environment is
/// released once no delayed slot needs it.
///
/// # Safety
/// `s` a delayed slot of a live frame, `val` a live object.
unsafe fn force_slot(iso: &Isolate, s: SlotRef, val: Sexp) {
    // SAFETY: the caller's contract; a delayed slot's `val` is null or the
    // early value (counted), replaced by the store.
    unsafe {
        let was_caller = !s.has(META_ORIGIN_SELF);
        s.set_flag(META_ORIGIN_SELF, false);
        set_prseen(s, 0);
        s.set_key(s.sym(), FORCED);
        s.store_val(val);
        if was_caller {
            frame::release_caller(s.owner, iso.nil);
        }
    }
}

/// A box of the unboxed value `bits` of type `t` (REALSXP, INTSXP or
/// LGLSXP).
///
/// # Safety
/// `t` and `bits` come from a tagged promise.
unsafe fn box_bits(core: &mut Core, t: u8, bits: u64) -> Result<Sexp, NeedGc> {
    let x = core.alloc_vector(t, 1)?;
    // SAFETY: a fresh vector of one element of type `t`.
    unsafe {
        match t {
            REALSXP => *(x.std_data() as *mut f64) = f64::from_bits(bits),
            _ => *(x.std_data() as *mut i32) = bits as u32 as i32,
        }
    }
    Ok(x)
}

// ---- the argument list GNU R keeps per call ------------------------------
//
// GNU R binds a supplied argument as a promise that two places hold while
// the call runs: the call's argument list (`promiseArgs`, `matchArgs_RC`)
// and the frame's binding. At the call's end `R_CleanupEnvir` drops the
// binding (if the frame is unreachable) and `unpromiseArgs` clears every
// promise that only the list still holds, then drops the list's
// reference. A promise that loses its last reference keeps its own
// children's counts: dead containers are never decremented (docs/g3.md
// §4). A slot frame has no such list, so the counts it implies are kept
// here, per slot and only when they differ from the slot's own:
//
// - while the call runs (the frame's `CALL_LIVE` bit), an argument slot
//   that is overwritten or removed leaves its value, expression and
//   environment counted in a *residue* entry, which the call's end clears
//   as `unpromiseArgs` would; a slot materialized into a PROMSXP gives the
//   promise the list's reference (a *ghost* entry), dropped at the end;
// - after the call (or for a default, which no list holds) the promise
//   that loses the binding is dead: its counts leak, exactly as GNU R's.
//
// An exit that leaves the call by a jump (an error, a `return` to an
// outer function) skips GNU R's cleanup: the entries are dropped with
// their counts leaked.

/// One entry of a live call's argument list that the frame no longer
/// holds the GNU R way (see above). A precise root.
#[derive(Trace)]
pub(crate) struct Residue {
    frame: Sexp,
    #[trace(skip = "a slot index")]
    index: u32,
    /// A ghost: the materialized promise (its list reference counted).
    promise: Sexp,
    /// A retired argument: its value, expression and environment (each
    /// counted by this entry), or null.
    val: Sexp,
    expr: Sexp,
    env: Sexp,
}

/// The residues of the live calls, oldest first.
#[derive(Default, Trace)]
pub(crate) struct Residues {
    list: UnsafeCell<Vec<Residue>>,
}

impl Residues {
    fn push(&self, r: Residue) {
        // SAFETY: a shared access that ends with this call; nothing it calls
        // reaches the list (a collection runs with no `&EnvState` in use).
        unsafe { (*self.list.get()).push(r) }
    }

    /// The entry of slot `index` of `frame`, if any (a ghost or a retired
    /// argument), turned into a ghost by `ghost` when it is retired.
    fn find(&self, frame: Sexp, index: u32, ghost: impl FnOnce(&Residue) -> Sexp) -> Sexp {
        // SAFETY: as in `push`; `ghost` makes a promise from the entry's
        // words without reaching the list.
        let v = unsafe { &mut *self.list.get() };
        let Some(r) = v.iter_mut().find(|r| r.frame == frame && r.index == index) else { return Sexp::NULL };
        if r.promise.is_null() {
            let p = ghost(r);
            // the promise now counts the words; the entry's counts go
            // SAFETY: the entry's words are live objects it counted.
            unsafe {
                for x in [r.val, r.expr, r.env] {
                    if !x.is_null() {
                        x.decref();
                    }
                }
                p.incref();
            }
            *r = Residue { frame, index, promise: p, val: Sexp::NULL, expr: Sexp::NULL, env: Sexp::NULL };
        }
        r.promise
    }

    /// Take the entries of `frame`.
    fn take(&self, frame: Sexp) -> Vec<Residue> {
        // SAFETY: as in `push`.
        let v = unsafe { &mut *self.list.get() };
        if v.is_empty() {
            return Vec::new();
        }
        let (mine, rest): (Vec<_>, Vec<_>) = std::mem::take(v).into_iter().partition(|r| r.frame == frame);
        *v = rest;
        mine
    }

    pub(crate) fn for_each(&self, f: &mut dyn FnMut(Sexp)) {
        // SAFETY: as in `push`; `f` gets copies.
        let v: Vec<Sexp> =
            unsafe { (*self.list.get()).iter().flat_map(|r| [r.frame, r.promise, r.val, r.expr, r.env]).collect() };
        for x in v.into_iter().filter(|x| !x.is_null()) {
            f(x);
        }
    }
}

/// Whether the slot is an argument some call's list held in GNU R: delayed
/// or forced, supplied (a default, MISSING 2, is the frame's alone).
///
/// # Safety
/// `s` is a live slot.
#[inline(always)]
unsafe fn supplied(s: SlotRef) -> bool {
    // SAFETY: the caller's contract.
    unsafe { matches!(s.kind(), DELAYED | FORCED) && s.missing() != 2 }
}

/// Whether the call that made frame `f` is still running.
///
/// # Safety
/// `f` is a live slot frame.
#[inline(always)]
unsafe fn call_live(f: Sexp) -> bool {
    // SAFETY: the caller's contract.
    unsafe { f.extra() & layout::CALL_LIVE != 0 }
}

/// The argument in the delayed or forced slot `s` loses its binding (an
/// assignment or `rm` replaces it), with GNU R's counts (see above): its
/// words are cleared without dropping their counts, which a residue entry
/// takes over while the call runs and which leak otherwise. A precomputed
/// early value, robigo's alone, is dropped. The slot keeps its kind; the
/// caller stores over it ([`frame::store_value`], [`frame::unbind`]).
///
/// # Safety
/// `s` is a delayed or forced slot of a live frame of `iso`'s heap.
pub(crate) unsafe fn retire_arg(iso: &Isolate, s: SlotRef) {
    // SAFETY: the caller's contract.
    unsafe {
        let f = s.owner;
        let delayed = s.kind() == DELAYED;
        let expr = s.aux();
        let (val, env) = if delayed {
            // the early value is not GNU R's: dropped
            s.store_val(Sexp::NULL);
            let env = if s.has(META_ORIGIN_SELF) { f } else { layout::get(f, layout::OFF_CALLER) };
            (Sexp::NULL, env)
        } else {
            (s.val(), Sexp::NULL)
        };
        // the slot's words, cleared with their counts kept (nothing
        // collects before the caller stores over them)
        s.clear_raw_val();
        s.clear_raw_aux();
        // the promise's environment, counted once per promise in GNU R
        // (here the frame's caller word, or nothing for a default)
        if !env.is_null() {
            env.incref();
        }
        let default = s.missing() == 2;
        if !default && call_live(f) {
            iso.envs().residues.push(Residue { frame: f, index: s.index as u32, promise: Sexp::NULL, val, expr, env });
        }
        // else: the promise is dead and its counts leak (val, expr, env)
        s.set_flag(META_ORIGIN_SELF, false);
        set_prseen(s, 0);
    }
}

/// A supplied argument slot was materialized into the promise `p` while
/// its call runs: `p` gets the argument list's reference (a ghost entry).
/// If `p` was under evaluation by a [`Cx::force_arg`], that force
/// completes `p`.
///
/// # Safety
/// `s` is the delayed or forced slot (still in that state) of a live frame;
/// `p` its new promise, of `iso`'s heap.
pub(crate) unsafe fn note_materialized(iso: &Isolate, s: SlotRef, p: Sexp) {
    // SAFETY: the caller's contract.
    unsafe {
        if s.kind() == DELAYED && s.prseen() == PRSEEN_EVALUATING {
            iso.envs().pending.materialized(s.owner, s.index as u32, p);
        }
        if supplied(s) && call_live(s.owner) {
            p.incref();
            iso.envs().residues.push(Residue {
                frame: s.owner,
                index: s.index as u32,
                promise: p,
                val: Sexp::NULL,
                expr: Sexp::NULL,
                env: Sexp::NULL,
            });
        }
    }
}

impl Cx {
    /// A closure call starts running in the slot frame `frame`: until
    /// [`Cx::end_call`], its arguments are held by the call's (implicit)
    /// argument list.
    #[inline]
    pub fn begin_call(&mut self, frame: Root<'_>) {
        let f = frame.bind(self).sexp();
        // SAFETY: a rooted environment; a header bit.
        unsafe {
            debug_assert!(layout::is_slots(f));
            f.set_extra(f.extra() | layout::CALL_LIVE);
        }
    }

    /// The call in `frame` ends. `normal`: it returned (GNU R then runs
    /// `unpromiseArgs` after [`Cx::cleanup_envir`]: every promise only the
    /// argument list holds is cleared, and the list's references dropped);
    /// otherwise an exit jumped past it, and the list's counts leak.
    // R: eval.c:applyClosure_core (unpromiseArgs), eval.c:unpromiseArgs
    #[inline]
    pub fn end_call(&mut self, frame: Root<'_>, normal: bool) {
        let f = frame.bind(self).sexp();
        let iso = self.iso();
        // SAFETY: a rooted environment; the entries' objects are rooted by
        // the list until taken, and nothing below collects.
        unsafe {
            f.set_extra(f.extra() & !layout::CALL_LIVE);
            for r in iso.envs().residues.take(f) {
                if !normal {
                    continue;
                }
                if !r.promise.is_null() {
                    if r.promise.refcnt() == 1 {
                        clear_promise(iso, r.promise);
                    }
                    r.promise.decref();
                } else {
                    for x in [r.val, r.expr, r.env] {
                        if !x.is_null() {
                            x.decref();
                        }
                    }
                }
            }
        }
    }
}

impl Cx {
    /// The promise GNU R's argument list holds for the argument bound to
    /// slot `index` of `frame` while its call runs, written to `out`
    /// (`promargs`, `ROBIGO_MATERIALIZE=1`): a delayed or forced slot is
    /// materialized (§6.3); one that was overwritten since is the promise
    /// its residue stands for, made now. False (nothing written) when the
    /// slot holds no argument promise (a value bound directly: a constant
    /// under the `Bc` profile, a promise list's value).
    pub fn arg_promise(&mut self, frame: Root<'_>, index: usize, out: Out<'_>) -> RResult<bool> {
        let f = frame.bind(self).sexp();
        self.iso().check_owns(f);
        // SAFETY: a rooted environment.
        assert!(unsafe { f.typ() == ENVSXP && layout::is_slots(f) }, "robigo: arg_promise of a non-slot frame");
        let s = SlotRef { owner: f, index };
        // SAFETY: a slot of a rooted frame.
        if matches!(unsafe { s.kind() }, DELAYED | FORCED) {
            let s = self.ensure_object(s)?;
            // SAFETY: a value slot now holding the new promise.
            out.set(self, unsafe { Ref::from_sexp(s.val(), self) });
            return Ok(true);
        }
        let p = alloc_retry(self, |cx| {
            let iso = cx.iso();
            let f = frame.bind(cx).sexp();
            let mut refused = None;
            // SAFETY: the entry's words are live (it roots them); a promise
            // made from them counts them (alloc_node), and nothing collects.
            let p = iso.envs().residues.find(f, index as u32, |r| unsafe {
                let made = if r.expr.is_null() {
                    Ok(iso.nil)
                } else if !r.val.is_null() {
                    iso.core().alloc_node(PROMSXP, r.val, r.expr, iso.nil)
                } else {
                    iso.core().alloc_node(PROMSXP, iso.unbound, r.expr, r.env)
                };
                match made {
                    Ok(p) => p,
                    Err(e) => {
                        refused = Some(e);
                        Sexp::NULL
                    }
                }
            });
            match refused {
                Some(e) => Err(e),
                None => Ok(p),
            }
        })?;
        if p.is_null() {
            // SAFETY: a slot of a rooted frame.
            let v = unsafe { SlotRef { owner: frame.bind(self).sexp(), index }.val() };
            // a materialized argument still bound: its promise
            // SAFETY: as above.
            if matches!(unsafe { SlotRef { owner: frame.bind(self).sexp(), index }.kind() }, VALUE)
                && unsafe { v.typ() } == PROMSXP
            {
                // SAFETY: as above.
                out.set(self, unsafe { Ref::from_sexp(v, self) });
                return Ok(true);
            }
            return Ok(false);
        }
        // SAFETY: the ghost's promise, rooted by the residue list.
        out.set(self, unsafe { Ref::from_sexp(p, self) });
        Ok(true)
    }

    /// GNU R's binding of slot `index` of `frame` as an object from now on:
    /// a delayed or forced argument becomes its PROMSXP (§6.3's
    /// "materialize"; what `ROBIGO_MATERIALIZE=1` does to every argument at
    /// call entry).
    pub fn materialize_arg(&mut self, frame: Root<'_>, index: usize) -> RResult<()> {
        let f = frame.bind(self).sexp();
        self.iso().check_owns(f);
        // SAFETY: a rooted environment.
        assert!(unsafe { f.typ() == ENVSXP && layout::is_slots(f) }, "robigo: materialize_arg of a non-slot frame");
        let s = SlotRef { owner: f, index };
        // SAFETY: a slot of a rooted frame.
        if matches!(unsafe { s.kind() }, DELAYED | FORCED) {
            self.ensure_object(s)?;
        }
        Ok(())
    }
}

impl Cx {
    /// GNU R's `unpromiseArgs(pargs)` after a call whose argument list was a
    /// real pairlist (a promise list handed to `applyClosure`): every
    /// promise only the list holds is cleared, and the list drops its
    /// elements.
    // R: eval.c:unpromiseArgs
    pub fn unpromise_args(&mut self, pargs: Root<'_>) {
        let iso = self.iso();
        let mut a = pargs.bind(self).sexp();
        iso.check_owns(a);
        // SAFETY: a rooted pairlist; clearing and dropping counts neither
        // allocates nor collects.
        unsafe {
            while a != iso.nil {
                let v = a.car();
                if v.typ() == PROMSXP && v.refcnt() == 1 {
                    clear_promise(iso, v);
                }
                a.set_car(iso.nil);
                a = a.cdr();
            }
        }
    }
}
