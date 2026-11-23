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
use layout::ALIAS;

/// GNU R's `PRSEEN` values (a promise's gp; a delayed slot's meta bits).
pub(crate) use robigo_heap::gp::promise::{PRSEEN_EVALUATING, PRSEEN_INTERRUPTED};

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
    /// The slot lost its binding while its expression ran ([`retire_arg`]):
    /// GNU R's promise still completes, setting its value and releasing
    /// its environment, which then stays with the residue or leaks.
    #[trace(skip = "a flag")]
    retired: Retired,
    /// The environment [`retire_arg`] kept counted for the promise.
    env: Sexp,
}

/// What [`retire_arg`] did with a slot under evaluation.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Retired {
    No,
    /// Its counts went to a residue entry of the live call.
    Residue,
    /// Its promise is dead: its counts leak.
    Leaked,
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
        v.push(PendingForce { frame, index, promise: Sexp::NULL, retired: Retired::No, env: Sexp::NULL });
        v.len() - 1
    }

    /// Pop entry `at` (the innermost): the promise it was materialized
    /// into (or null), and whether and how the slot was retired meanwhile.
    fn pop(&self, at: usize) -> (Sexp, Retired, Sexp) {
        // SAFETY: as in `push`.
        let v = unsafe { &mut *self.stack.get() };
        debug_assert_eq!(v.len(), at + 1, "robigo: pending forces popped out of order");
        v.truncate(at + 1);
        v.pop().map_or((Sexp::NULL, Retired::No, Sexp::NULL), |e| (e.promise, e.retired, e.env))
    }

    /// The environment a pending force of slot `index` of `frame` holds for
    /// a retired argument whose promise is not made yet (it may be null:
    /// already released).
    fn retired_residue(&self, frame: Sexp, index: u32) -> Option<Sexp> {
        // SAFETY: as in `push`.
        let v = unsafe { &*self.stack.get() };
        v.iter()
            .rev()
            .find(|e| e.frame == frame && e.index == index && e.retired == Retired::Residue && e.promise.is_null())
            .map(|e| e.env)
    }

    /// The retired argument of a pending force became the promise `p`,
    /// which counts the environment the force held: the force completes
    /// `p` (and no longer holds the environment).
    fn adopt(&self, frame: Sexp, index: u32, p: Sexp) {
        // SAFETY: as in `push`.
        let v = unsafe { &mut *self.stack.get() };
        if let Some(e) = v.iter_mut().rev().find(|e| e.frame == frame && e.index == index && e.promise.is_null()) {
            if !e.env.is_null() {
                // SAFETY: a live object whose count the entry held.
                unsafe { e.env.decref() };
                e.env = Sexp::NULL;
            }
            e.promise = p;
        }
    }

    /// Record that slot `index` of `frame`, under evaluation, was retired.
    fn retired(&self, frame: Sexp, index: u32, how: Retired, env: Sexp) {
        // SAFETY: as in `push`.
        let v = unsafe { &mut *self.stack.get() };
        if let Some(e) = v.iter_mut().rev().find(|e| e.frame == frame && e.index == index) {
            e.retired = how;
            e.env = env;
        }
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
        let v: Vec<Sexp> = unsafe { (*self.stack.get()).iter().flat_map(|e| [e.frame, e.promise, e.env]).collect() };
        for x in v.into_iter().filter(|x| !x.is_null()) {
            f(x);
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
    let _ = nil;
    // SAFETY: the caller's contract; the heap's store clears an unboxed
    // value's tag without reading its bits as an object.
    unsafe { p.set_promise_value(v) }
}

/// `bytecodeExpr(e)`: byte code's source expression (its first constant,
/// `nil` with none), anything else itself. A promise compiled code made
/// holds byte code as its code (`MAKEPROM`'s PRCODE); what reads the
/// promise's expression (GNU R's `PREXPR`, `R_PromiseExpr`) reads it
/// through this, what forces it runs the code.
///
/// # Safety
/// `e` is live.
// R: eval.c:bytecodeExpr, eval.c:R_PromiseExpr
#[inline]
pub unsafe fn bytecode_expr(e: Sexp, nil: Sexp) -> Sexp {
    // SAFETY: the caller's contract: a BCODESXP's CDR is its constants
    // (a VECSXP), live.
    unsafe {
        if e.typ() != robigo_heap::types::BCODESXP {
            return e;
        }
        let k = e.cdr();
        if k.std_len() > 0 { *(k.std_data() as *const Sexp) } else { nil }
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
/// a delayed slot of a frame, to force in place with [`Cx::force_arg`]
/// (or an S3 method's alias, which [`Cx::force_arg`] forces through to its
/// generic's slot).
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
                let b = self.retry_alloc(|cx| {
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

    /// GNU R's `SET_PRVALUE(p, v)` (counted; PRSEEN and PRENV unchanged):
    /// the dispatchers mark the promise of an argument they evaluated
    /// (`IF_PROMSXP_SET_PRVALUE`). Panics if `p` is not a promise.
    // R: memory.c:SET_PRVALUE
    pub fn set_promise_value(&mut self, p: Root<'_>, v: Root<'_>) {
        let (ps, vs) = (p.bind(self).sexp(), v.bind(self).sexp());
        assert_eq!(p.bind(self).sexptype(), PROMSXP, "robigo: SET_PRVALUE of a non-promise");
        let iso = self.iso();
        iso.check_owns(ps);
        iso.check_owns(vs);
        // SAFETY: a rooted promise and a rooted value of this heap.
        unsafe { set_prvalue(ps, vs, iso.nil) };
    }

    /// Whether `p` is a promise `forcePromise` would evaluate plainly: not
    /// evaluated, PRSEEN 0, its code not byte code.
    pub fn promise_unforced_ast(&self, p: Ref<'_>) -> bool {
        if p.sexptype() != PROMSXP {
            return false;
        }
        let ps = p.sexp();
        // SAFETY: a live promise (a `Ref` is live); its header fields read.
        unsafe {
            !is_evaluated(self.iso(), ps) && ps.gp() == 0 && ps.extra() & robigo_heap::extra::promise::BYTECODE == 0
        }
    }

    /// The end of `forcePromise` for a value the caller computed as the
    /// promise's code would (`SET_PRVALUE`, `SET_PRSEEN(0)`,
    /// `SET_PRENV(R_NilValue)`): for a promise [`Cx::promise_unforced_ast`]
    /// accepts, whose evaluation the caller has shown to be unobservable.
    pub fn set_promise_forced(&mut self, p: Root<'_>, v: Root<'_>) {
        let iso = self.iso();
        let (ps, val) = (p.bind(self).sexp(), v.bind(self).sexp());
        assert!(
            self.promise_unforced_ast(p.bind(self)),
            "robigo: set_promise_forced on a promise not plainly unforced"
        );
        // SAFETY: a rooted promise and value, as `force_promise`'s end.
        unsafe {
            set_prvalue(ps, val, iso.nil);
            ps.set_gp(0);
            ps.set_tag(iso.nil);
        }
    }

    /// GNU R's `forcePromise(p)`, and the promise's value written to `out`.
    /// The value is not marked: `ENSURE_NAMEDMAX` does nothing under
    /// reference counting (`Defn.h:337-340`).
    // R: eval.c:forcePromise
    pub fn force_promise(&mut self, p: Root<'_>, out: Out<'_>) -> RResult<()> {
        assert_eq!(p.bind(self).sexptype(), PROMSXP, "robigo: forcing a non-promise");
        // SAFETY: a rooted promise.
        if unsafe { is_evaluated(self.iso(), p.bind(self).sexp()) } {
            self.promise_value(p, out)?;
            return Ok(());
        }
        if self.inline_force_due() {
            // SAFETY: a rooted promise.
            if let Some(e) = unsafe { self.inline_promise_overflow(p.bind(self).sexp()) } {
                return Err(e);
            }
        }
        self.safepoint(robigo_heap::Safepoint::Entry)?;
        let ps = p.bind(self).sexp();
        // SAFETY: as above; PRSEEN is the gp field.
        let seen = unsafe { ps.gp() };
        // "set PRSEEN to 1 to avoid infinite recursion" before the restart
        // warning, so that a handler forcing it again gets the error; an
        // exit out of the warning leaves it 1 (GNU R pushes the promise on
        // its pending stack only after)
        // SAFETY: as above.
        unsafe { ps.set_gp(PRSEEN_EVALUATING) };
        if seen != 0 {
            self.prseen_check(seen)?;
        }
        let res = out.root();
        let r = self.scope(|cx, s| {
            let ps = p.bind(cx).sexp();
            // SAFETY: PRCODE and PRENV of a rooted promise; its header's
            // byte-code mark.
            let (code, env, bc) = unsafe {
                (
                    s.root(cx, Ref::from_sexp(ps.cdr(), cx)),
                    s.root(cx, Ref::from_sexp(ps.tag(), cx)),
                    ps.extra() & robigo_heap::extra::promise::BYTECODE != 0,
                )
            };
            cx.eval_promise_code(code, env, bc, out)
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
    /// for 1; the restart warning for 2 (the caller has marked it 1
    /// already). Both are reported against `R_GlobalContext->call`.
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

    /// Whether the promise GNU R's binding in slot `index` of the slot
    /// frame `frame` would hold is unevaluated (`PRVALUE(s) ==
    /// R_UnboundValue`): a delayed slot, or a slot materialized into a
    /// promise not yet evaluated. (GetObject evaluates only such a promise.)
    /// For the slot kinds an argument of the generic's own frame has:
    /// delayed, forced, a value (a materialized promise), an unboxed
    /// scalar. An alias (a method's argument, which reads its generic's
    /// slot) is false: GetObject reads the generic's frame, whose slot is
    /// never an alias.
    ///
    /// Unlike its checked siblings ([`Cx::force_arg`], `arg_promise`,
    /// `materialize_arg`), its checks are debug assertions: it runs on
    /// every S3 dispatch of an argument slot, and the force that follows
    /// it re-checks the frame and slot before anything is forced.
    ///
    /// # Safety
    /// `frame` is a live slot frame of this isolate and `index` is below
    /// its capacity (the checks [`Cx::force_arg`] makes, debug-asserted
    /// here: this runs on every S3 dispatch of an argument slot).
    #[inline]
    pub unsafe fn arg_unevaluated(&self, frame: Root<'_>, index: usize) -> bool {
        let f = frame.bind(self).sexp();
        #[cfg(debug_assertions)]
        self.iso().check_owns(f);
        debug_assert!(
            // SAFETY: a rooted environment.
            unsafe { f.typ() == ENVSXP && layout::is_slots(f) },
            "robigo: arg_unevaluated on a non-slot frame"
        );
        // SAFETY: as above.
        debug_assert!(index < unsafe { layout::capacity(f) }, "robigo: arg_unevaluated past the frame's slots");
        let s = SlotRef { owner: f, index };
        // SAFETY: the caller's contract: a slot of a live slot frame; a
        // value slot's `val` is live.
        unsafe {
            match s.kind() {
                DELAYED => true,
                VALUE => s.val().typ() == PROMSXP && !is_evaluated(self.iso(), s.val()),
                _ => false,
            }
        }
    }

    /// bcEval's inlined `GETVAR`/`DDVAL` force: a promise bcEval forces in
    /// place (not evaluated, of byte code, PRSEEN 0) whose evaluation's
    /// depth step overflows is left PRSEEN 1. `setup_bcframe_prom` marks it
    /// before `INCREMENT_EVAL_DEPTH` and puts it on the pending stack only
    /// after, so `R_restore_globals` never marks it interrupted: a later
    /// force is "promise already under evaluation", not the restart
    /// warning. Taken while byte code is active (`R_BCIntActive`); the AST
    /// interpreter's `eval` goes through `forcePromise`. The check is
    /// on the overflow's side only: the common path pays the depth compare.
    /// Its depth test must be the same as robigo-rt's `forced_as_getvar`
    /// (eval.rs), which asks the same question for GETVAR_MISSOK's reads:
    /// an edit of one is made in the other, until the fold of
    /// docs/ledger/g3/2026-11-23-eval-promise.md removes that compare.
    // R: eval.c:DO_GETVAR, eval.c:setup_bcframe_prom
    #[inline(always)]
    fn inline_force_due(&self) -> bool {
        self.force_depth_overflows() && self.iso().contexts().bc_active.get()
    }

    /// Whether a force's depth step here would overflow the evaluation
    /// depth limit (`eval_depth >= expressions`): the one test the
    /// inlined-force overflow above and robigo-rt's GETVAR_MISSOK reads
    /// (`eval::forced_as`) both make.
    #[inline(always)]
    pub fn force_depth_overflows(&self) -> bool {
        self.eval_depth() >= self.expressions()
    }

    /// The overflow of bcEval's inlined force of a delayed slot: `None` when
    /// the slot is not one bcEval forces in place (then `force_arg` forces
    /// it as `getvar` does). Only a delayed slot reaches it: `force_arg`
    /// hands a value slot's promise to `force_promise`, which makes its own
    /// check ([`Cx::inline_promise_overflow`]).
    ///
    /// # Safety
    /// `s` is a delayed slot of a live slot frame of this isolate.
    #[cold]
    #[inline(never)]
    unsafe fn inline_force_overflow(&mut self, s: SlotRef) -> Option<RError> {
        // SAFETY: the caller's contract.
        unsafe {
            debug_assert_eq!(s.kind(), DELAYED);
            if s.prseen() == 0 && s.has(layout::META_BYTECODE) {
                set_prseen(s, PRSEEN_EVALUATING);
                return Some(self.inline_depth_overflow());
            }
            None
        }
    }

    /// The overflow of bcEval's inlined force of a PROMSXP, `None` when `p`
    /// is not a promise bcEval forces in place.
    ///
    /// # Safety
    /// `p` is a live promise of this isolate.
    #[cold]
    #[inline(never)]
    unsafe fn inline_promise_overflow(&mut self, p: Sexp) -> Option<RError> {
        // SAFETY: the caller's contract; PRSEEN is the gp field.
        unsafe {
            if is_evaluated(self.iso(), p) || p.gp() != 0 || p.extra() & robigo_heap::extra::promise::BYTECODE == 0 {
                return None;
            }
            p.set_gp(PRSEEN_EVALUATING);
        }
        Some(self.inline_depth_overflow())
    }

    /// `setup_bcframe_prom`'s `INCREMENT_EVAL_DEPTH()` past the limit:
    /// one level deeper and `handle_eval_depth_overflow`'s error.
    fn inline_depth_overflow(&mut self) -> RError {
        match self.eval_depth_enter() {
            Err(e) => e,
            Ok(_) => unreachable!("robigo: an inline promise force past the limit that did not overflow"),
        }
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
            // SAFETY: a value slot's `val` is a live object.
            VALUE if unsafe { s.val().typ() } == PROMSXP => {
                return self.scope(|cx, sc| {
                    // SAFETY: the slot's value, a live promise.
                    let p = sc.root(cx, unsafe { Ref::from_sexp(s.val(), cx) });
                    cx.force_promise(p, out)
                });
            }
            // (a method's argument, an alias, out of line)
            _ => return self.force_alias(s, out),
        }
        if self.inline_force_due() {
            // SAFETY: a delayed slot of a rooted frame.
            if let Some(e) = unsafe { self.inline_force_overflow(s) } {
                return Err(e);
            }
        }
        // a value the caller computed early (§6.2): present only while no
        // effect has happened since (`note_effect` drops it), so it is the
        // value the expression would have; the force makes the evaluation's
        // counts and takes it, unless the evaluation would poll or fail
        // (then the expression is evaluated, as without one)
        // SAFETY: a delayed slot's `val` is null or the early value.
        let early = unsafe { s.val() };
        if !early.is_null() {
            self.early_taken(f);
            // SAFETY: a delayed slot's expression is live.
            let (bc, kind) = unsafe { (s.has(layout::META_BYTECODE), s.aux().typ()) };
            if (self.iso().hooks.get().early_force)(self, bc, kind) {
                let iso = self.iso();
                // SAFETY: the slot holds the value; it becomes the forced
                // value (nothing ran since it was read).
                unsafe { force_slot(iso, s, early) };
                // SAFETY: as above.
                out.set(self, unsafe { Ref::from_sexp(early, self) });
                return Ok(());
            }
            // SAFETY: as above; the store drops the value's count.
            unsafe { s.store_val(Sexp::NULL) };
        }
        self.safepoint(robigo_heap::Safepoint::Entry)?;
        let s = SlotRef { owner: frame.bind(self).sexp(), index };
        // SAFETY: as above.
        let seen = unsafe { s.prseen() };
        if seen == PRSEEN_EVALUATING {
            return self.prseen_check(seen);
        }
        let iso = self.iso();
        // PRSEEN 1 before the restart warning (see `force_promise`), and the
        // force pending from here, so that a handler's materialization of
        // the slot is the promise this force completes
        // SAFETY: a delayed slot of a rooted frame.
        unsafe { set_prseen(s, PRSEEN_EVALUATING) };
        let at = iso.envs().pending.push(s.owner, index as u32);
        let res = out.root();
        let mut evaluating = false;
        let r = self.scope(|cx, sc| {
            let s = SlotRef { owner: frame.bind(cx).sexp(), index };
            // SAFETY: a delayed slot of a rooted frame: its expression and
            // environment are live; rooted before anything can run.
            let (e, rho, bc) = unsafe {
                let env = if s.has(META_ORIGIN_SELF) { s.owner } else { layout::get(s.owner, layout::OFF_CALLER) };
                (
                    sc.root(cx, Ref::from_sexp(s.aux(), cx)),
                    sc.root(cx, Ref::from_sexp(env, cx)),
                    s.has(layout::META_BYTECODE),
                )
            };
            if seen != 0 {
                cx.prseen_check(seen)?;
            }
            evaluating = true;
            cx.eval_promise_code(e, rho, bc, out)
        });
        let iso = self.iso();
        let (promise, retired, renv) = iso.envs().pending.pop(at);
        let s = SlotRef { owner: frame.bind(self).sexp(), index };
        // SAFETY: a slot of the rooted frame; `promise` (if any) is the
        // object the slot was materialized into during the evaluation, kept
        // alive by the pending stack until the pop just now and by the slot
        // or `out`'s caller since (nothing collected in between).
        unsafe {
            let still = s.kind() == DELAYED && s.prseen() == PRSEEN_EVALUATING;
            match r {
                Err(e) => {
                    // (an exit out of the restart warning leaves PRSEEN 1:
                    // GNU R's promise was not pending yet)
                    if !promise.is_null() {
                        if evaluating {
                            promise.set_gp(PRSEEN_INTERRUPTED);
                        }
                    } else if still {
                        if evaluating {
                            set_prseen(s, PRSEEN_INTERRUPTED);
                        }
                    } else if retired == Retired::Residue && !renv.is_null() {
                        // the interrupted promise keeps its environment in
                        // the argument list until the call's end
                        iso.envs().residues.keep_env(s.owner, index as u32, renv);
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
                    } else if retired != Retired::No {
                        // the promise lost its binding during its own
                        // evaluation: it completes all the same, its value
                        // counted and its environment released
                        if retired == Retired::Residue {
                            iso.envs().residues.complete(s.owner, index as u32, val);
                        } else {
                            val.incref();
                        }
                        if !renv.is_null() {
                            renv.decref();
                        }
                    }
                    Ok(())
                }
            }
        }
    }

    /// [`Cx::force_arg`] of a slot of another kind than an argument's: a
    /// method's alias forces its generic's argument in place there (with
    /// that slot's early value and its call site's record). Out of line,
    /// off the arguments' path.
    #[cold]
    #[inline(never)]
    fn force_alias(&mut self, s: SlotRef, out: Out<'_>) -> RResult<()> {
        // SAFETY: a slot of a rooted frame.
        let k = unsafe { s.kind() };
        assert!(k == ALIAS, "robigo: force_arg on a slot of kind {k}");
        // SAFETY: a live alias, whose target's frame its call's record roots.
        let t = unsafe { self.read_through(s) };
        self.scope(|cx, sc| {
            // SAFETY: as above.
            let g = sc.root(cx, unsafe { Ref::from_sexp(t.owner, cx) });
            cx.force_arg(g, t.index, out)
        })
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
                // (an alias: the generic's argument it reads)
                At::FrameSlot(sl) => {
                    let sl = self.read_through(sl);
                    match sl.kind() {
                        VALUE => value_type(iso, sl.val()),
                        REAL | INT | LGL => Ok(BindingType::Value),
                        ACTIVE => Ok(BindingType::Active),
                        // the slot is the outer promise of the chain
                        DELAYED | FORCED => {
                            let code = sl.aux();
                            if code.typ() == PROMSXP {
                                unwrap_forced(iso, code)
                                    .map(|f| if f { BindingType::Forced } else { BindingType::Delayed })
                            } else if sl.kind() == FORCED {
                                Ok(BindingType::Forced)
                            } else {
                                Ok(BindingType::Delayed)
                            }
                        }
                        k => unreachable!("robigo: a bound slot of kind {k}"),
                    }
                }
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
            frame::walk_bound(r, nil, |s| {
                match s.kind() {
                    // BNDCELL_TAG: `continue`, the value stays
                    REAL | INT | LGL => {}
                    // an argument: its promise's clearPromise, then the
                    // binding's SETCAR(b, R_NilValue)
                    DELAYED | FORCED => {
                        // (the methods' aliases ended with their calls)
                        debug_assert!(!s.has(layout::META_ALIASED), "robigo: an alias outlived its method's call");
                        s.store_aux(Sexp::NULL);
                        s.store_val(Sexp::NULL);
                        s.set_flag(META_ORIGIN_SELF, false);
                        s.set_flag(layout::META_BYTECODE, false);
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
                            return false;
                        }
                        release_value(iso, c.car(), v);
                        chain::set_value(c, nil);
                    }
                    // a method's alias: dropped (the generic's argument
                    // holds what GNU R's promise holds; its list keeps it)
                    _ => super::alias::release(iso, s),
                }
                true
            });
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
        frame::walk_bound(r, iso.nil, |s| {
            let x = match s.kind() {
                // (an alias counts no frame)
                REAL | INT | LGL | DELAYED | FORCED | ALIAS => return true,
                CELL => {
                    let c = s.val();
                    if c.refcnt() != 1 {
                        return false;
                    }
                    c.car()
                }
                _ => s.val(),
            };
            if x != v && !x.is_null() {
                match x.typ() {
                    PROMSXP if x.refcnt() == 1 && x.tag() == r => crefs += 1,
                    CLOSXP if x.refcnt() == 1 && x.tag() == r => crefs += 1,
                    ENVSXP if x == r => crefs += 1,
                    _ => {}
                }
            }
            true
        });
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
        s.set_flag(layout::META_BYTECODE, false);
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
    /// The retired argument's expression is byte code's
    /// ([`layout::META_BYTECODE`]).
    #[trace(skip = "a flag")]
    bytecode: bool,
    /// An alias entry ([`super::alias`]): slot `index` of the method's
    /// frame `frame` is an [`ALIAS`] of slot `tindex` of the generic's frame
    /// `target`, and its other words are null. Null for a ghost or a
    /// retired argument.
    target: Sexp,
    #[trace(skip = "a slot index")]
    tindex: u32,
}

impl Residue {
    /// An entry of slot `index` of `frame` with every other word null.
    fn empty(frame: Sexp, index: u32) -> Residue {
        Residue {
            frame,
            index,
            promise: Sexp::NULL,
            val: Sexp::NULL,
            expr: Sexp::NULL,
            env: Sexp::NULL,
            bytecode: false,
            target: Sexp::NULL,
            tindex: 0,
        }
    }
}

/// The residues of the live calls, oldest first, and the aliases of the
/// live methods ([`super::alias`]).
#[derive(Default, Trace)]
pub(crate) struct Residues {
    list: UnsafeCell<Vec<Residue>>,
    /// The alias entries in `list`.
    #[trace(skip = "a count")]
    aliases: Cell<usize>,
    /// Promise nodes made ahead, at least one per alias entry, for
    /// [`Cx::end_call`] to make an alias the promise without allocating
    /// ([`Cx::reserve_alias_promises`]). Kept for the next method.
    reserve: UnsafeCell<Vec<Sexp>>,
}

impl Residues {
    fn push(&self, r: Residue) {
        // SAFETY: a shared access that ends with this call; nothing it calls
        // reaches the list (a collection runs with no `&EnvState` in use).
        unsafe { (*self.list.get()).push(r) }
    }

    /// The entry of slot `index` of `frame`, if any (a ghost or a retired
    /// argument), turned into a ghost by `ghost` when it is retired.
    fn find(
        &self,
        frame: Sexp,
        index: u32,
        ghost: impl FnOnce(&Residue) -> Result<Sexp, NeedGc>,
    ) -> Result<Sexp, NeedGc> {
        // SAFETY: as in `push`; `ghost` makes a promise from the entry's
        // words without reaching the list.
        let v = unsafe { &mut *self.list.get() };
        let Some(r) = v.iter_mut().find(|r| r.frame == frame && r.index == index && r.target.is_null()) else {
            return Ok(Sexp::NULL);
        };
        if r.promise.is_null() {
            // (a refused allocation changes nothing)
            let p = ghost(r)?;
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
            *r = Residue { promise: p, ..Residue::empty(frame, index) };
        }
        Ok(r.promise)
    }

    /// The entry of slot `index` of `frame`, read only: Some(its promise),
    /// null for a retired argument not yet made a promise (`find` makes
    /// it); None without one.
    pub(crate) fn promise_of(&self, frame: Sexp, index: u32) -> Option<Sexp> {
        // SAFETY: as in `push`; read only.
        let v = unsafe { &*self.list.get() };
        v.iter().find(|r| r.frame == frame && r.index == index && r.target.is_null()).map(|r| r.promise)
    }

    /// A retired argument's promise completed with `val` (its environment
    /// already released by the force).
    fn complete(&self, frame: Sexp, index: u32, val: Sexp) {
        // SAFETY: as in `push`; `val` is live.
        let v = unsafe { &mut *self.list.get() };
        if let Some(r) = v
            .iter_mut()
            .rev()
            .find(|r| r.frame == frame && r.index == index && r.promise.is_null() && r.target.is_null())
        {
            // SAFETY: a live object, now counted by the entry.
            unsafe { val.incref() };
            r.val = val;
        }
    }

    /// A retired argument's promise was interrupted: it keeps `env` (whose
    /// count the force held) until the call's end.
    fn keep_env(&self, frame: Sexp, index: u32, env: Sexp) {
        // SAFETY: as in `push`.
        let v = unsafe { &mut *self.list.get() };
        if let Some(r) = v
            .iter_mut()
            .rev()
            .find(|r| r.frame == frame && r.index == index && r.promise.is_null() && r.target.is_null())
        {
            r.env = env;
        }
    }

    /// Remove `frame`'s ghosts and retired arguments, oldest first,
    /// passing each to `each`, which must not reach the list; true if
    /// `frame` (a method's) still has alias entries, which are left for
    /// [`Cx::end_call`] to resolve. Nothing is allocated: the common case is
    /// none of this frame's, while outer calls keep theirs (a generic's
    /// frame keeps its ghost while its method runs).
    #[inline(always)]
    fn take(&self, frame: Sexp, mut each: impl FnMut(Residue)) -> bool {
        // SAFETY: as in `push`; `each` does not reach the list (the
        // caller's contract).
        let v = unsafe { &mut *self.list.get() };
        if !v.iter().any(|r| r.frame == frame) {
            return false;
        }
        let mut aliases = false;
        // (the other entries keep their order)
        for r in v.extract_if(.., |r| {
            let alias = !r.target.is_null();
            aliases |= alias && r.frame == frame;
            r.frame == frame && !alias
        }) {
            each(r);
        }
        aliases
    }

    // ---- alias entries (super::alias) -------------------------------------

    /// Slot `index` of the method's frame `method` became an alias of slot
    /// `tindex` of `target`.
    pub(super) fn push_alias(&self, method: Sexp, index: u32, target: Sexp, tindex: u32) {
        assert!(self.reserved() > self.aliases.get(), "robigo: an alias without a reserved promise");
        self.aliases.set(self.aliases.get() + 1);
        self.push(Residue { target, tindex, ..Residue::empty(method, index) });
    }

    /// The promise nodes in reserve.
    #[inline(always)]
    pub(super) fn reserved(&self) -> usize {
        // SAFETY: as in `push`; read only.
        unsafe { (*self.reserve.get()).len() }
    }

    /// Whether the reserve has a node for every alias entry and `more`.
    #[inline(always)]
    pub(super) fn reserve_covers(&self, more: usize) -> bool {
        self.reserved() >= self.aliases.get() + more
    }

    /// Put the fresh promise node `p` in reserve.
    pub(super) fn push_reserve(&self, p: Sexp) {
        // SAFETY: as in `push`.
        unsafe { (*self.reserve.get()).push(p) }
    }

    /// A promise node from the reserve (one per alias entry is there).
    pub(super) fn take_reserve(&self) -> Sexp {
        // SAFETY: as in `push`.
        unsafe { (*self.reserve.get()).pop() }.expect("robigo: no promise in reserve for an alias")
    }

    /// Remove the alias entry of slot `index` of `method` (the newest
    /// first: calls nest); its target frame and index.
    pub(super) fn remove_alias(&self, method: Sexp, index: u32) -> Option<(Sexp, u32)> {
        // SAFETY: as in `push`.
        let v = unsafe { &mut *self.list.get() };
        let at = v.iter().rposition(|r| r.frame == method && r.index == index && !r.target.is_null())?;
        let r = v.remove(at);
        self.aliases.set(self.aliases.get() - 1);
        Some((r.target, r.tindex))
    }

    /// Whether some alias reads slot `tindex` of `target`.
    pub(super) fn targeted(&self, target: Sexp, tindex: u32) -> bool {
        // SAFETY: as in `push`; read only.
        let v = unsafe { &*self.list.get() };
        v.iter().any(|r| r.target == target && r.tindex == tindex)
    }

    /// Remove every alias entry of slot `tindex` of `target`, passing each
    /// alias (the method's frame and slot) to `each`, which must not reach
    /// the list.
    pub(super) fn take_targeting(&self, target: Sexp, tindex: u32, mut each: impl FnMut(Sexp, u32)) {
        // SAFETY: as in `push`; `each` does not reach the list.
        let v = unsafe { &mut *self.list.get() };
        for r in v.extract_if(.., |r| r.target == target && r.tindex == tindex) {
            self.aliases.set(self.aliases.get() - 1);
            each(r.frame, r.index);
        }
    }

    /// The newest alias entry of the method's frame `method`: its slot, and
    /// the target frame and index.
    pub(super) fn alias_of(&self, method: Sexp) -> Option<(u32, Sexp, u32)> {
        // SAFETY: as in `push`; read only.
        let v = unsafe { &*self.list.get() };
        v.iter().rev().find(|r| r.frame == method && !r.target.is_null()).map(|r| (r.index, r.target, r.tindex))
    }

    /// The target of the alias in slot `index` of `method`, if its entry is
    /// there.
    #[cfg(debug_assertions)]
    pub(super) fn alias_target(&self, method: Sexp, index: u32) -> Option<(Sexp, u32)> {
        // SAFETY: as in `push`; read only.
        let v = unsafe { &*self.list.get() };
        v.iter()
            .rev()
            .find(|r| r.frame == method && r.index == index && !r.target.is_null())
            .map(|r| (r.target, r.tindex))
    }

    /// Whether an alias entry has `f` as its method's frame or its target.
    #[cfg(debug_assertions)]
    pub(super) fn any_alias_of(&self, f: Sexp) -> bool {
        // SAFETY: as in `push`; read only.
        let v = unsafe { &*self.list.get() };
        v.iter().any(|r| !r.target.is_null() && (r.frame == f || r.target == f))
    }

    pub(crate) fn for_each(&self, f: &mut dyn FnMut(Sexp)) {
        // SAFETY: as in `push`; `f` gets copies.
        let mut v: Vec<Sexp> = unsafe {
            (*self.list.get()).iter().flat_map(|r| [r.frame, r.promise, r.val, r.expr, r.env, r.target]).collect()
        };
        // SAFETY: as above.
        v.extend(unsafe { (*self.reserve.get()).iter().copied() });
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

/// The early value of slot `index` of frame `f` is released without being
/// taken (the slot becomes a promise object, or an assignment replaces the
/// argument): one fewer in the early frame, and one more dropped for the
/// call site's record ([`Cx::early_wasted`]).
pub(crate) fn early_lost(iso: &Isolate, f: Sexp, index: usize) {
    let st = iso.contexts();
    if st.early_frame.get() != f {
        return;
    }
    let n = st.early_left.get().saturating_sub(1);
    st.early_left.set(n);
    if n == 0 {
        st.early_frame.set(iso.nil);
    }
    let m = if st.early_wasted.get() == f { st.early_wasted_mask.get() } else { 0 };
    st.early_wasted.set(f);
    st.early_wasted_mask.set(m | if index < 32 { 1 << index } else { 0 });
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
            if !s.val().is_null() {
                early_lost(iso, f, s.index);
            }
            s.store_val(Sexp::NULL);
            let env = if s.has(META_ORIGIN_SELF) { f } else { layout::get(f, layout::OFF_CALLER) };
            (Sexp::NULL, env)
        } else {
            (s.val(), Sexp::NULL)
        };
        // the slot's words, cleared with their counts kept (nothing
        // collects before the caller stores over them). Cleared raw, so in
        // a frozen frame each cleared object is noted, as a barriered store
        // over it would be (the boot heap's freeze: Sol r2 S9)
        if f.flags() & robigo_heap::flags::FROZEN != 0 {
            robigo_heap::note_overwritten(val);
            robigo_heap::note_overwritten(expr);
        }
        s.clear_raw_val();
        s.clear_raw_aux();
        // the promise's environment, counted once per promise in GNU R
        // (here the frame's caller word, or nothing for a default)
        if !env.is_null() {
            env.incref();
        }
        let default = s.missing() == 2;
        let how = if !default && call_live(f) {
            // while the residue holds `env`, the completing force releases
            // it there
            iso.envs().residues.push(Residue {
                val,
                expr,
                env: if delayed && s.prseen() == PRSEEN_EVALUATING { Sexp::NULL } else { env },
                bytecode: delayed && s.has(layout::META_BYTECODE),
                ..Residue::empty(f, s.index as u32)
            });
            Retired::Residue
        } else {
            // the promise is dead and its counts leak (val, expr, env)
            Retired::Leaked
        };
        if delayed && s.prseen() == PRSEEN_EVALUATING {
            iso.envs().pending.retired(f, s.index as u32, how, env);
        }
        s.set_flag(META_ORIGIN_SELF, false);
        s.set_flag(layout::META_BYTECODE, false);
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
            iso.envs().residues.push(Residue { promise: p, ..Residue::empty(s.owner, s.index as u32) });
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
    /// Either way, a method's aliases that its frame's cleanup did not drop
    /// become the argument's promise ([`super::alias`]; this allocates).
    // R: eval.c:applyClosure_core (unpromiseArgs), eval.c:unpromiseArgs
    #[inline]
    pub fn end_call(&mut self, frame: Root<'_>, normal: bool) {
        let f = frame.bind(self).sexp();
        let iso = self.iso();
        // SAFETY: a rooted environment; the entries' objects are rooted by
        // the list until taken, and nothing below collects.
        let aliases = unsafe {
            f.set_extra(f.extra() & !layout::CALL_LIVE);
            // (clearing a promise and dropping counts reach no residue)
            iso.envs().residues.take(f, |r| {
                if !normal {
                    return;
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
            })
        };
        // a method's aliases the cleanup did not drop become the promise
        // (super::alias): the frame may outlive the call
        if aliases {
            self.end_aliases(frame.bind(self).sexp());
        }
        #[cfg(debug_assertions)]
        {
            let f = frame.bind(self).sexp();
            assert!(!self.iso().envs().residues.any_alias_of(f), "robigo: an alias outlived its method's call");
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
        if matches!(unsafe { s.kind() }, DELAYED | FORCED | ALIAS) {
            // (an alias: its generic's argument's promise, which it then
            // holds)
            let s = self.ensure_object(s)?;
            // SAFETY: a value slot now holding the new promise.
            out.set(self, unsafe { Ref::from_sexp(s.val(), self) });
            return Ok(true);
        }
        let mut under_force = false;
        let p = self.retry_alloc(|cx| {
            let iso = cx.iso();
            let f = frame.bind(cx).sexp();
            // a retired argument whose own force is still running: the
            // force holds its environment, and completes the promise
            let pending = iso.envs().pending.retired_residue(f, index as u32);
            under_force = pending.is_some();
            // SAFETY: the entry's words are live (it roots them), as is the
            // pending force's environment; a promise made from them counts
            // them (alloc_node), and nothing collects.
            iso.envs().residues.find(f, index as u32, |r| unsafe {
                let p = if let Some(env) = pending {
                    let p = iso.core().alloc_node(PROMSXP, iso.unbound, r.expr, env)?;
                    p.set_gp(PRSEEN_EVALUATING);
                    p
                } else if !r.val.is_null() {
                    iso.core().alloc_node(PROMSXP, r.val, r.expr, iso.nil)?
                } else {
                    iso.core().alloc_node(PROMSXP, iso.unbound, r.expr, r.env)?
                };
                if r.bytecode {
                    p.set_extra(p.extra() | robigo_heap::extra::promise::BYTECODE);
                }
                Ok(p)
            })
        })?;
        if under_force && !p.is_null() {
            // the promise now holds the environment the force held
            self.iso().envs().pending.adopt(frame.bind(self).sexp(), index as u32, p);
        }
        if p.is_null() {
            // SAFETY: a slot of a rooted frame.
            let v = unsafe { SlotRef { owner: frame.bind(self).sexp(), index }.val() };
            // a materialized argument still bound: its promise
            // SAFETY: as above.
            if matches!(unsafe { SlotRef { owner: frame.bind(self).sexp(), index }.kind() }, VALUE)
                // SAFETY: a value slot's `val` is a live object.
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

    /// Frame `f` (just made by a closure call) holds `n` early values in
    /// its delayed slots (§6.2's precomputed arguments): the values its
    /// caller computed for argument expressions that run no code, which the
    /// callee's forces take while nothing else has happened.
    ///
    /// # Safety
    /// `f` is a live slot frame of this isolate with `n` early values in its
    /// delayed slots: it is a root from here on, and dropping the values
    /// writes its slots.
    pub unsafe fn set_early_frame(&self, f: Sexp, n: u32) {
        let st = self.ctxs();
        debug_assert_eq!(st.early_frame.get(), self.iso().nil, "robigo: a second frame with early values");
        st.early_frame.set(f);
        st.early_left.set(n);
    }

    /// The record of dropped early values ([`Cx::early_wasted`]), taken
    /// (and cleared) by a call that is about to make early values of its
    /// own, which gives it back with [`Cx::restore_early_wasted`] when it
    /// ends: an outer call's record survives the calls its body makes.
    #[inline(always)]
    pub fn take_early_wasted(&self) -> (Sexp, u32) {
        let st = self.ctxs();
        let r = (st.early_wasted.get(), st.early_wasted_mask.get());
        st.early_wasted.set(self.iso().nil);
        st.early_wasted_mask.set(0);
        r
    }

    /// Put back a record [`Cx::take_early_wasted`] took.
    ///
    /// # Safety
    /// `r` is what `take_early_wasted` returned in the same call, whose
    /// frame is R_NilValue or still live: the record's frame is a root.
    #[inline(always)]
    pub unsafe fn restore_early_wasted(&self, r: (Sexp, u32)) {
        let st = self.ctxs();
        st.early_wasted.set(r.0);
        st.early_wasted_mask.set(r.1);
    }

    /// Which of frame `f`'s first 32 slots had early values that were
    /// dropped before the callee took them (0 for none): a call site's
    /// reason to stop computing those arguments early.
    ///
    /// # Safety
    /// `f` is a live object of this isolate (only its address is compared
    /// today; the contract keeps a read sound).
    #[inline(always)]
    pub unsafe fn early_wasted(&self, f: Sexp) -> u32 {
        let st = self.ctxs();
        if st.early_wasted.get() == f { st.early_wasted_mask.get() } else { 0 }
    }

    /// Something happened that may change what an argument expression
    /// evaluates to, or observe that it was not evaluated: R code ran (a
    /// closure call), a builtin ran, a binding outside the early frame was
    /// written. The early values are dropped, and their arguments are
    /// evaluated when forced, as GNU R evaluates them then.
    #[inline(always)]
    pub fn note_effect(&self) {
        let st = self.ctxs();
        if st.early_frame.get() != self.iso().nil {
            self.drop_early_values();
        }
    }

    /// A binding of `rho` is written: an effect ([`Cx::note_effect`])
    /// unless `rho` is the early frame itself (the callee's own locals,
    /// which no argument expression reads: its caller's environments do
    /// not reach the new frame).
    ///
    /// # Safety
    /// `rho` is a live environment of this isolate (only its address is
    /// compared today; the contract keeps a read sound).
    #[inline(always)]
    pub unsafe fn note_write(&self, rho: Sexp) {
        let st = self.ctxs();
        let e = st.early_frame.get();
        if e != self.iso().nil && e != rho {
            self.drop_early_values();
        }
    }

    /// The call that made frame `f` ends (returns or is left by a jump):
    /// early values its body did not take are dropped.
    ///
    /// # Safety
    /// `f` is a live object of this isolate (only its address is compared
    /// today; the contract keeps a read sound).
    #[inline(always)]
    pub unsafe fn end_early_frame(&self, f: Sexp) {
        if self.ctxs().early_frame.get() == f {
            self.drop_early_values();
        }
    }

    /// An early value of frame `f` is taken (or set aside by its force).
    #[inline(always)]
    fn early_taken(&self, f: Sexp) {
        let st = self.ctxs();
        if st.early_frame.get() == f {
            let n = st.early_left.get().saturating_sub(1);
            st.early_left.set(n);
            if n == 0 {
                st.early_frame.set(self.iso().nil);
            }
        }
    }

    #[cold]
    #[inline(never)]
    fn drop_early_values(&self) {
        let st = self.ctxs();
        let f = st.early_frame.get();
        st.early_frame.set(self.iso().nil);
        st.early_left.set(0);
        let mut wasted = 0u32;
        // SAFETY: the early frame is a live slot frame (a traced root of
        // the context state); clearing a delayed slot's value drops its
        // count and neither allocates nor collects.
        unsafe {
            for index in 0..layout::capacity(f) {
                let s = SlotRef { owner: f, index };
                if s.kind() == DELAYED && !s.val().is_null() {
                    s.store_val(Sexp::NULL);
                    if index < 32 {
                        wasted |= 1 << index;
                    }
                }
            }
        }
        if wasted != 0 {
            let m = if st.early_wasted.get() == f { st.early_wasted_mask.get() } else { 0 };
            st.early_wasted.set(f);
            st.early_wasted_mask.set(m | wasted);
        }
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
        if matches!(unsafe { s.kind() }, DELAYED | FORCED | ALIAS) {
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
